use crate::{BridgeError, ConfiguredMod, JdkInfo, LaunchConfig, Result};
use jni::objects::{Global, JObject, JObjectArray, JString};
use jni::{Env, InitArgsBuilder, JNIVersion, JValue, jni_sig, jni_str};
use morizen_compat::{CompatReport, ModCompatStatus, ModReport};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct LaunchSummary {
    pub jdk: JdkInfo,
    pub minecraft_version: String,
    pub loader_version: String,
    pub mods: Vec<ConfiguredMod>,
}

enum Command {
    Shutdown,
}

pub struct JavaModHost {
    command: Sender<Command>,
    finished: Receiver<()>,
    thread: Option<JoinHandle<Result<()>>>,
    summary: LaunchSummary,
    shutdown_timeout: Duration,
}

impl JavaModHost {
    pub fn start(config: LaunchConfig) -> Result<Self> {
        static JVM_CLAIMED: AtomicBool = AtomicBool::new(false);

        config.validate()?;
        let jdk = JdkInfo::discover(config.jdk_home.as_deref())?;
        if JVM_CLAIMED.swap(true, Ordering::AcqRel) {
            return Err(BridgeError::JvmStart(
                "a JVM session was already attempted in this process; JNI does not support restart"
                    .into(),
            ));
        }
        let timeout = Duration::from_millis(config.startup_timeout_ms);
        let shutdown_timeout = Duration::from_millis(config.shutdown_timeout_ms);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let thread_jdk = jdk.clone();
        let handle = thread::Builder::new()
            .name("Java mods".into())
            .spawn(move || {
                let result = owner_thread(config, thread_jdk, ready_tx, command_rx);
                let _ = finished_tx.send(());
                result
            })
            .map_err(|error| {
                JVM_CLAIMED.store(false, Ordering::Release);
                BridgeError::Thread(format!("could not spawn owner thread: {error}"))
            })?;

        let summary = match ready_rx.recv_timeout(timeout) {
            Ok(Ok(summary)) => summary,
            Ok(Err(error)) => {
                let _ = handle.join();
                return Err(error);
            }
            Err(error) => {
                let _ = command_tx.send(Command::Shutdown);
                return Err(BridgeError::Thread(format!(
                    "JVM startup did not finish within {} ms: {error}",
                    timeout.as_millis()
                )));
            }
        };
        Ok(Self {
            command: command_tx,
            finished: finished_rx,
            thread: Some(handle),
            summary,
            shutdown_timeout,
        })
    }

    pub fn start_from_environment() -> Result<Option<Self>> {
        LaunchConfig::from_environment()?
            .map(Self::start)
            .transpose()
    }

    pub fn summary(&self) -> &LaunchSummary {
        &self.summary
    }

    pub fn shutdown(mut self) -> Result<()> {
        self.finish()
    }

    fn finish(&mut self) -> Result<()> {
        let Some(handle) = self.thread.take() else {
            return Ok(());
        };
        let _ = self.command.send(Command::Shutdown);
        if let Err(error) = self.finished.recv_timeout(self.shutdown_timeout) {
            drop(handle);
            return Err(BridgeError::Thread(format!(
                "Java VM did not stop within {} ms: {error}; it will be reclaimed at process exit",
                self.shutdown_timeout.as_millis()
            )));
        }
        handle
            .join()
            .map_err(|_| BridgeError::Thread("Java mod thread panicked".into()))?
    }
}

impl Drop for JavaModHost {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("morizen-java: shutdown error: {error}");
        }
    }
}

fn owner_thread(
    config: LaunchConfig,
    jdk: JdkInfo,
    ready: Sender<Result<LaunchSummary>>,
    commands: Receiver<Command>,
) -> Result<()> {
    if let Err(error) = fs::create_dir_all(&config.game_dir) {
        let error = BridgeError::Config(format!(
            "could not create game directory {}: {error}",
            config.game_dir.display()
        ));
        let _ = ready.send(Err(error));
        return Ok(());
    }
    if let Some(parent) = config.marker_path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        let error = BridgeError::Config(format!(
            "could not create marker directory {}: {error}",
            parent.display()
        ));
        let _ = ready.send(Err(error));
        return Ok(());
    }
    let _ = fs::remove_file(&config.marker_path);

    println!(
        "morizen-java: jdk={} home={}",
        jdk.version,
        jdk.home.display()
    );
    let vm = match create_vm(&config, &jdk) {
        Ok(vm) => vm,
        Err(error) => {
            write_failure_report(&config, &error);
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };

    let target_loader = match vm.attach_current_thread(launch_fabric) {
        Ok(target_loader) => target_loader,
        Err(error) => {
            write_failure_report(&config, &error);
            let _ = ready.send(Err(error));
            destroy_vm(vm)?;
            return Ok(());
        }
    };

    let report = success_report(&config);
    if let Err(error) = report.write_atomic(&config.report_path) {
        let error = BridgeError::Report(format!(
            "could not write {}: {error}",
            config.report_path.display()
        ));
        let _ = ready.send(Err(error));
        drop(target_loader);
        destroy_vm(vm)?;
        return Ok(());
    }
    let summary = LaunchSummary {
        jdk,
        minecraft_version: config.minecraft_version.clone(),
        loader_version: config.loader_version.clone(),
        mods: config.mods.clone(),
    };
    if ready.send(Ok(summary)).is_err() {
        drop(target_loader);
        destroy_vm(vm)?;
        return Ok(());
    }

    let _ = commands.recv();
    println!("morizen-java: shutdown begin");
    let stop_result = vm.attach_current_thread(stop_fabric);
    drop(target_loader);
    let destroy_result = destroy_vm(vm);
    stop_result?;
    destroy_result?;
    println!("morizen-java: shutdown end");
    Ok(())
}

fn create_vm(config: &LaunchConfig, jdk: &JdkInfo) -> Result<jni::JavaVM> {
    let classpath = env::join_paths(
        std::iter::once(config.provider_jar.as_path())
            .chain(config.fabric_libraries.iter().map(PathBuf::as_path))
            .chain(config.game_libraries.iter().map(PathBuf::as_path)),
    )
    .map_err(|error| BridgeError::Config(format!("invalid JVM class path: {error}")))?;
    let mods = env::join_paths(config.mod_jars.iter())
        .map_err(|error| BridgeError::Config(format!("invalid Fabric mod path: {error}")))?;
    let options = [
        format!("-Xmx{}m", config.max_heap_mb),
        "-Xms32m".into(),
        format!("-Djava.class.path={}", classpath.to_string_lossy()),
        "-Dfile.encoding=UTF-8".into(),
        "-Djava.awt.headless=true".into(),
        "-Dfabric.skipMcProvider=true".into(),
        "-Dfabric.noGui=true".into(),
        format!("-Dfabric.gameVersion={}", config.minecraft_version),
        "-Dfabric.runtimeMappingNamespace=official".into(),
        "-Dfabric.defaultModDistributionNamespace=official".into(),
        format!("-Dfabric.addMods={}", mods.to_string_lossy()),
        format!("-Dmorizen.gameDir={}", config.game_dir.display()),
        format!("-Dmorizen.bootstrapJar={}", config.bootstrap_jar.display()),
        format!("-Dmorizen.marker={}", config.marker_path.display()),
    ];
    let mut builder = InitArgsBuilder::new().version(JNIVersion::V1_8);
    for option in &options {
        builder = builder.option(option.as_str());
    }
    let args = builder
        .build()
        .map_err(|error| BridgeError::JvmStart(error.to_string()))?;
    let _dll_directory = DllDirectory::for_jdk(&jdk.home)?;
    jni::JavaVM::with_libjvm(args, || Ok(jdk.libjvm.clone()))
        .map_err(|error| BridgeError::JvmStart(error.to_string()))
}

fn launch_fabric(env: &mut Env<'_>) -> Result<Global<JObject<'static>>> {
    let null = JString::default();
    let arguments = JObjectArray::<JString>::new(env, 0, &null)?;
    env.call_static_method(
        jni_str!("dev/morizen/compat/provider/MorizenGameProvider"),
        jni_str!("launchKnot"),
        jni_sig!("([Ljava/lang/String;)V"),
        &[JValue::Object(arguments.as_ref())],
    )?;
    let target_loader = env
        .call_static_method(
            jni_str!("dev/morizen/compat/provider/MorizenGameProvider"),
            jni_str!("getActiveTargetLoader"),
            jni_sig!("()Ljava/lang/ClassLoader;"),
            &[],
        )?
        .l()?;
    Ok(env.new_global_ref(target_loader)?)
}

fn stop_fabric(env: &mut Env<'_>) -> Result<()> {
    env.call_static_method(
        jni_str!("dev/morizen/compat/provider/MorizenGameProvider"),
        jni_str!("stopActive"),
        jni_sig!(() -> void),
        &[],
    )?;
    Ok(())
}

fn success_report(config: &LaunchConfig) -> CompatReport {
    CompatReport::fabric(
        &config.minecraft_version,
        &config.loader_version,
        config
            .mods
            .iter()
            .map(|configured| ModReport {
                id: configured.id.clone(),
                version: configured.version.clone(),
                status: if configured.verified_compatible {
                    ModCompatStatus::Works
                } else {
                    ModCompatStatus::Partial
                },
                unsupported: Vec::new(),
                error: None,
            })
            .collect(),
    )
}

fn write_failure_report(config: &LaunchConfig, error: &BridgeError) {
    let report = CompatReport::fabric(
        &config.minecraft_version,
        &config.loader_version,
        config
            .mods
            .iter()
            .map(|configured| ModReport {
                id: configured.id.clone(),
                version: configured.version.clone(),
                status: ModCompatStatus::LoaderFailure,
                unsupported: Vec::new(),
                error: Some(error.to_string()),
            })
            .collect(),
    );
    if let Err(report_error) = report.write_atomic(&config.report_path) {
        eprintln!("morizen-java: could not write failure report: {report_error}");
    }
}

fn destroy_vm(vm: jni::JavaVM) -> Result<()> {
    // SAFETY: this is the sole owner thread; Java callbacks have returned, the
    // bootstrap has dropped its target-loader reference, and no JNI reference
    // wrapper is retained outside the completed attachment closures above.
    let result = unsafe { vm.destroy() }.map_err(BridgeError::from);
    drop(vm);
    result
}

#[cfg(windows)]
struct DllDirectory {
    previous: Option<Vec<u16>>,
}

#[cfg(windows)]
impl DllDirectory {
    fn for_jdk(home: &Path) -> Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::LibraryLoader::{GetDllDirectoryW, SetDllDirectoryW};

        let needed = unsafe { GetDllDirectoryW(0, std::ptr::null_mut()) };
        let previous = if needed == 0 {
            None
        } else {
            let mut buffer = vec![0u16; needed as usize + 1];
            let written = unsafe { GetDllDirectoryW(buffer.len() as u32, buffer.as_mut_ptr()) };
            if written == 0 {
                None
            } else {
                buffer.truncate(written as usize);
                buffer.push(0);
                Some(buffer)
            }
        };
        let mut path: Vec<u16> = home.join("bin").as_os_str().encode_wide().collect();
        path.push(0);
        if unsafe { SetDllDirectoryW(path.as_ptr()) } == 0 {
            return Err(BridgeError::JvmStart(format!(
                "could not add {} to the Windows DLL search path: {}",
                home.join("bin").display(),
                std::io::Error::last_os_error()
            )));
        }
        Ok(Self { previous })
    }
}

#[cfg(windows)]
impl Drop for DllDirectory {
    fn drop(&mut self) {
        use windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW;

        let pointer = self
            .previous
            .as_ref()
            .map_or(std::ptr::null(), |path| path.as_ptr());
        unsafe {
            SetDllDirectoryW(pointer);
        }
    }
}

#[cfg(not(windows))]
struct DllDirectory;

#[cfg(not(windows))]
impl DllDirectory {
    fn for_jdk(_home: &Path) -> Result<Self> {
        Ok(Self)
    }
}

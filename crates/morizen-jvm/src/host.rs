use crate::{BridgeError, ConfiguredMod, JdkInfo, LaunchConfig, Result};
use jni::objects::{Global, JObject, JObjectArray, JString};
use jni::{Env, InitArgsBuilder, JNIVersion, JValue, jni_sig, jni_str};
use morizen_compat::{
    ActionBatch, BridgeEvent, CompatReport, EventBatch, EventKind, ModCompatStatus, ModReport,
    RegistrationSet,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct LaunchSummary {
    pub jdk: JdkInfo,
    pub minecraft_version: String,
    pub loader_version: String,
    pub loader_id: String,
    pub mods: Vec<ConfiguredMod>,
    /// Content-mod folders built from what Java mods registered; the game
    /// adds them to its `--dev-mod` list.
    pub content_mods: Vec<PathBuf>,
}

enum Command {
    Dispatch(EventBatch, Sender<Result<ActionBatch>>),
    Shutdown,
}

/// Cloneable, thread-safe sender for coarse Java callback batches. It owns no
/// JNI state; the dedicated JVM thread performs every Java call.
#[derive(Clone)]
pub struct JavaModDispatcher {
    command: Sender<Command>,
    timeout: Duration,
    next_sequence: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    dispatch_lock: Arc<Mutex<()>>,
}

impl JavaModDispatcher {
    /// Creates an event. Fill its snapshot fields, then dispatch it; sequence
    /// numbers are assigned under the dispatch lock so concurrent game/server
    /// threads cannot reach Java out of order.
    pub fn event(&self, kind: EventKind) -> BridgeEvent {
        BridgeEvent::new(0, kind)
    }

    pub fn dispatch_event(&self, event: BridgeEvent) -> Result<ActionBatch> {
        self.dispatch(EventBatch::single(event))
    }

    pub fn dispatch(&self, mut batch: EventBatch) -> Result<ActionBatch> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(BridgeError::Event(
                "callback dispatch was attempted after stop".into(),
            ));
        }
        if !self.enabled.load(Ordering::Acquire) {
            return Ok(ActionBatch::default());
        }
        let _guard = self
            .dispatch_lock
            .lock()
            .map_err(|_| BridgeError::Event("the Java dispatch lock was poisoned".into()))?;
        for event in &mut batch.events {
            event.sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        }
        batch.validate().map_err(BridgeError::Event)?;
        let (reply_tx, reply_rx) = mpsc::channel();
        self.command
            .send(Command::Dispatch(batch, reply_tx))
            .map_err(|_| BridgeError::Event("the Java mod thread has stopped".into()))?;
        reply_rx.recv_timeout(self.timeout).map_err(|error| {
            BridgeError::Event(format!(
                "Java callbacks did not finish within {} ms: {error}",
                self.timeout.as_millis()
            ))
        })?
    }
}

pub struct JavaModHost {
    command: Sender<Command>,
    finished: Receiver<()>,
    thread: Option<JoinHandle<Result<()>>>,
    summary: LaunchSummary,
    shutdown_timeout: Duration,
    dispatcher: JavaModDispatcher,
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
        let event_timeout = Duration::from_millis(config.event_timeout_ms);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let next_sequence = Arc::new(AtomicU64::new(1));
        let stopped = Arc::new(AtomicBool::new(false));
        let enabled = Arc::new(AtomicBool::new(false));
        let dispatch_lock = Arc::new(Mutex::new(()));
        let thread_jdk = jdk.clone();
        let thread_sequence = Arc::clone(&next_sequence);
        let thread_stopped = Arc::clone(&stopped);
        let thread_enabled = Arc::clone(&enabled);
        let handle = thread::Builder::new()
            .name("Java mods".into())
            .spawn(move || {
                let result = owner_thread(
                    config,
                    thread_jdk,
                    ready_tx,
                    command_rx,
                    thread_sequence,
                    thread_stopped,
                    thread_enabled,
                );
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
        let dispatcher = JavaModDispatcher {
            command: command_tx.clone(),
            timeout: event_timeout,
            next_sequence,
            stopped,
            enabled,
            dispatch_lock,
        };
        Ok(Self {
            command: command_tx,
            finished: finished_rx,
            thread: Some(handle),
            summary,
            shutdown_timeout,
            dispatcher,
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

    pub fn dispatcher(&self) -> JavaModDispatcher {
        self.dispatcher.clone()
    }

    pub fn shutdown(mut self) -> Result<()> {
        self.finish()
    }

    fn finish(&mut self) -> Result<()> {
        let Some(handle) = self.thread.take() else {
            return Ok(());
        };
        self.dispatcher.stopped.store(true, Ordering::Release);
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
    next_sequence: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
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
    // A previous session's registrations must never leak into this one.
    for directory in [config.registry_directory(), config.content_directory()] {
        if directory.exists()
            && let Err(error) = fs::remove_dir_all(&directory)
        {
            let error =
                BridgeError::Config(format!("could not clear {}: {error}", directory.display()));
            let _ = ready.send(Err(error));
            return Ok(());
        }
    }

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

    let target_loader = match vm.attach_current_thread(|env| launch_loader(env, &config)) {
        Ok(target_loader) => target_loader,
        Err(error) => {
            write_failure_report(&config, &error);
            let _ = ready.send(Err(error));
            destroy_vm(vm)?;
            return Ok(());
        }
    };

    let has_callbacks = vm.attach_current_thread(|env| loader_has_callbacks(env, &config))?;
    enabled.store(has_callbacks, Ordering::Release);
    if has_callbacks {
        // Registration has completed, so callbacks are now frozen. The first
        // event is always start and runs before the game receives a dispatcher.
        let start = EventBatch::single(BridgeEvent::new(0, EventKind::Start));
        if let Err(error) = vm.attach_current_thread(|env| dispatch_loader(env, &config, &start)) {
            write_failure_report(&config, &error);
            let _ = ready.send(Err(error));
            drop(target_loader);
            destroy_vm(vm)?;
            return Ok(());
        }
    }

    let content_mods = match write_registrations(&config) {
        Ok(paths) => paths,
        Err(error) => {
            write_failure_report(&config, &error);
            let _ = ready.send(Err(error));
            drop(target_loader);
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
        loader_id: config.loader_id.clone(),
        mods: config.mods.clone(),
        content_mods,
    };
    if ready.send(Ok(summary)).is_err() {
        drop(target_loader);
        destroy_vm(vm)?;
        return Ok(());
    }

    while let Ok(Command::Dispatch(batch, reply)) = commands.recv() {
        let result = vm.attach_current_thread(|env| dispatch_loader(env, &config, &batch));
        let _ = reply.send(result);
    }
    println!("morizen-java: shutdown begin");
    let stop = EventBatch::single(BridgeEvent::new(
        next_sequence.fetch_add(1, Ordering::Relaxed),
        EventKind::Stop,
    ));
    let event_stop_result = if has_callbacks {
        vm.attach_current_thread(|env| dispatch_loader(env, &config, &stop))
    } else {
        Ok(ActionBatch::default())
    };
    stopped.store(true, Ordering::Release);
    let stop_result = vm.attach_current_thread(|env| stop_loader(env, &config));
    drop(target_loader);
    let destroy_result = destroy_vm(vm);
    event_stop_result?;
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
    let mut options = vec![
        format!("-Xmx{}m", config.max_heap_mb),
        "-Xms32m".into(),
        format!("-Djava.class.path={}", classpath.to_string_lossy()),
        "-Dfile.encoding=UTF-8".into(),
        "-Djava.awt.headless=true".into(),
        format!("-Dmorizen.gameDir={}", config.game_dir.display()),
        format!(
            "-Dmorizen.registryDir={}",
            config.registry_directory().display()
        ),
        format!("-Dmorizen.bootstrapJar={}", config.bootstrap_jar.display()),
        format!("-Dmorizen.marker={}", config.marker_path.display()),
    ];
    match config.loader_id.as_str() {
        "fabric" => options.extend([
            "-Dfabric.skipMcProvider=true".into(),
            "-Dfabric.noGui=true".into(),
            format!("-Dfabric.gameVersion={}", config.minecraft_version),
            "-Dfabric.runtimeMappingNamespace=official".into(),
            "-Dfabric.defaultModDistributionNamespace=official".into(),
            format!("-Dfabric.addMods={}", mods.to_string_lossy()),
        ]),
        "quilt" => options.extend([
            "-Dloader.skipMcProvider=true".into(),
            format!("-Dloader.gameVersion={}", config.minecraft_version),
            format!("-Dloader.addMods={}", mods.to_string_lossy()),
        ]),
        _ => unreachable!("LaunchConfig validates loader_id"),
    }
    if let Some(shims) = &config.shims_jar {
        options.push(format!("-Dmorizen.shimsJar={}", shims.display()));
    }
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

fn launch_loader(env: &mut Env<'_>, config: &LaunchConfig) -> Result<Global<JObject<'static>>> {
    let null = JString::default();
    let loader_arguments = if config.loader_id == "quilt" {
        let mods = env::join_paths(config.mod_jars.iter())
            .map_err(|error| BridgeError::Config(format!("invalid Quilt mod path: {error}")))?;
        vec![
            "--loader.gameVersion".to_owned(),
            config.minecraft_version.clone(),
            "--loader.addMods".to_owned(),
            mods.to_string_lossy().into_owned(),
        ]
    } else {
        Vec::new()
    };
    let arguments = JObjectArray::<JString>::new(env, loader_arguments.len(), &null)?;
    for (index, argument) in loader_arguments.iter().enumerate() {
        let value = env.new_string(argument)?;
        arguments.set_element(env, index, &value)?;
    }
    let target_loader = if config.loader_id == "quilt" {
        env.call_static_method(
            jni_str!("dev/morizen/compat/quilt/provider/MorizenQuiltGameProvider"),
            jni_str!("launchKnot"),
            jni_sig!("([Ljava/lang/String;)V"),
            &[JValue::Object(arguments.as_ref())],
        )?;
        env.call_static_method(
            jni_str!("dev/morizen/compat/quilt/provider/MorizenQuiltGameProvider"),
            jni_str!("getActiveTargetLoader"),
            jni_sig!("()Ljava/lang/ClassLoader;"),
            &[],
        )?
        .l()?
    } else {
        env.call_static_method(
            jni_str!("dev/morizen/compat/provider/MorizenGameProvider"),
            jni_str!("launchKnot"),
            jni_sig!("([Ljava/lang/String;)V"),
            &[JValue::Object(arguments.as_ref())],
        )?;
        env.call_static_method(
            jni_str!("dev/morizen/compat/provider/MorizenGameProvider"),
            jni_str!("getActiveTargetLoader"),
            jni_sig!("()Ljava/lang/ClassLoader;"),
            &[],
        )?
        .l()?
    };
    Ok(env.new_global_ref(target_loader)?)
}

fn dispatch_loader(
    env: &mut Env<'_>,
    config: &LaunchConfig,
    batch: &EventBatch,
) -> Result<ActionBatch> {
    let wire = batch.to_wire().map_err(BridgeError::Event)?;
    let input = env.new_string(wire)?;
    let output = if config.loader_id == "quilt" {
        env.call_static_method(
            jni_str!("dev/morizen/compat/quilt/provider/MorizenQuiltGameProvider"),
            jni_str!("dispatchActive"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
            &[JValue::Object(input.as_ref())],
        )?
    } else {
        env.call_static_method(
            jni_str!("dev/morizen/compat/provider/MorizenGameProvider"),
            jni_str!("dispatchActive"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
            &[JValue::Object(input.as_ref())],
        )?
    }
    .l()?;
    let output = JString::cast_local(env, output)?;
    let output = output.try_to_string(env)?;
    ActionBatch::from_wire(&output).map_err(BridgeError::Event)
}

fn loader_has_callbacks(env: &mut Env<'_>, config: &LaunchConfig) -> Result<bool> {
    let value = if config.loader_id == "quilt" {
        env.call_static_method(
            jni_str!("dev/morizen/compat/quilt/provider/MorizenQuiltGameProvider"),
            jni_str!("hasCallbacksActive"),
            jni_sig!("()Z"),
            &[],
        )?
    } else {
        env.call_static_method(
            jni_str!("dev/morizen/compat/provider/MorizenGameProvider"),
            jni_str!("hasCallbacksActive"),
            jni_sig!("()Z"),
            &[],
        )?
    };
    Ok(value.z()?)
}

fn stop_loader(env: &mut Env<'_>, config: &LaunchConfig) -> Result<()> {
    if config.loader_id == "quilt" {
        env.call_static_method(
            jni_str!("dev/morizen/compat/quilt/provider/MorizenQuiltGameProvider"),
            jni_str!("stopActive"),
            jni_sig!(() -> void),
            &[],
        )?;
    } else {
        env.call_static_method(
            jni_str!("dev/morizen/compat/provider/MorizenGameProvider"),
            jni_str!("stopActive"),
            jni_sig!(() -> void),
            &[],
        )?;
    }
    Ok(())
}

fn write_registrations(config: &LaunchConfig) -> Result<Vec<PathBuf>> {
    let sets = RegistrationSet::read_directory(&config.registry_directory())
        .map_err(BridgeError::Registry)?;
    let content = std::path::absolute(config.content_directory())
        .unwrap_or_else(|_| config.content_directory());
    let mut paths = Vec::new();
    for set in &sets {
        if set.is_empty() {
            continue;
        }
        let package = set.content_package().map_err(BridgeError::Registry)?;
        let path = package.write(&content.join(&set.mod_id)).map_err(|error| {
            BridgeError::Registry(format!(
                "could not write generated content for {}: {error}",
                set.mod_id
            ))
        })?;
        println!(
            "morizen-java: content={} id={} blocks={} items={}",
            path.display(),
            set.mod_id,
            set.blocks.len(),
            set.items.len()
        );
        paths.push(path);
    }
    Ok(paths)
}

fn success_report(config: &LaunchConfig) -> CompatReport {
    CompatReport::loader(
        &config.minecraft_version,
        &config.loader_id,
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
    let report = CompatReport::loader(
        &config.minecraft_version,
        &config.loader_id,
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
    //
    // `JavaVM` has no destructor: `destroy` is the whole cleanup, so letting the
    // handle leave scope afterwards is the same as dropping it.
    unsafe { vm.destroy() }.map_err(BridgeError::from)
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

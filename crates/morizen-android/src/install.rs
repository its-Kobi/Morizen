//! Handing a downloaded update to Android's package installer.
//!
//! There is no way to install an APK without the system's own confirmation dialog, so the last
//! step of an update always asks the player. What this module does is open that dialog on the
//! APK the launcher just downloaded, instead of making the player hunt for the file.

use std::path::Path;

#[cfg(target_os = "android")]
mod android {
    use std::ffi::c_void;
    use std::path::Path;
    use std::sync::atomic::{AtomicPtr, Ordering};

    /// The process's Java VM and its activity, kept from `android_main`. The installer is opened
    /// from a tapped button, which can be any call inside the event loop, so they are remembered
    /// here rather than carried around.
    static VM: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static ACTIVITY: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

    pub fn remember(app: &winit::platform::android::activity::AndroidApp) {
        VM.store(app.vm_as_ptr(), Ordering::Release);
        ACTIVITY.store(app.activity_as_ptr(), Ordering::Release);
    }

    /// `startActivity(new Intent(ACTION_VIEW).setDataAndType(file, package-archive))`, which is
    /// the installer Android shows for a sideloaded APK.
    pub fn start(apk: &Path) -> Result<(), String> {
        use jni::JavaVM;
        use jni::objects::{JObject, JValue};

        let (vm, activity) = (VM.load(Ordering::Acquire), ACTIVITY.load(Ordering::Acquire));
        if vm.is_null() || activity.is_null() {
            return Err("the app is not ready to open the installer yet".to_owned());
        }
        let vm = unsafe { JavaVM::from_raw(vm.cast()) }.map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let activity = unsafe { JObject::from_raw(activity.cast()) };
        // The APK lives in this app's own folder, so the intent carries a file:// URI. Android
        // throws on those by default (FileUriExposedException); an empty VM policy clears the
        // check for this process only, which is what a self-updating sideloaded app needs.
        let policy = env
            .new_object("android/os/StrictMode$VmPolicy$Builder", "()V", &[])
            .and_then(|builder| {
                env.call_method(&builder, "build", "()Landroid/os/StrictMode$VmPolicy;", &[])
            })
            .map_err(|e| e.to_string())?;
        let policy = policy.l().map_err(|e| e.to_string())?;
        env.call_static_method(
            "android/os/StrictMode",
            "setVmPolicy",
            "(Landroid/os/StrictMode$VmPolicy;)V",
            &[JValue::Object(&policy)],
        )
        .map_err(|e| e.to_string())?;
        let path = env
            .new_string(apk.to_string_lossy().as_ref())
            .map_err(|e| e.to_string())?;
        let file = env
            .new_object(
                "java/io/File",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&path)],
            )
            .and_then(|file| {
                env.call_static_method(
                    "android/net/Uri",
                    "fromFile",
                    "(Ljava/io/File;)Landroid/net/Uri;",
                    &[JValue::Object(&file)],
                )
            })
            .map_err(|e| e.to_string())?;
        let uri = file.l().map_err(|e| e.to_string())?;
        let intent = env
            .new_object("android/content/Intent", "()V", &[])
            .map_err(|e| e.to_string())?;
        let view = env
            .new_string("android.intent.action.VIEW")
            .map_err(|e| e.to_string())?;
        env.call_method(
            &intent,
            "setAction",
            "(Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&view)],
        )
        .map_err(|e| e.to_string())?;
        let archive = env
            .new_string("application/vnd.android.package-archive")
            .map_err(|e| e.to_string())?;
        env.call_method(
            &intent,
            "setDataAndType",
            "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&uri), JValue::Object(&archive)],
        )
        .map_err(|e| e.to_string())?;
        // FLAG_ACTIVITY_NEW_TASK: the installer runs in another task.
        env.call_method(
            &intent,
            "addFlags",
            "(I)Landroid/content/Intent;",
            &[JValue::Int(0x1000_0000)],
        )
        .map_err(|e| e.to_string())?;
        env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(target_os = "android")]
pub use android::{remember, start};

/// Desktop builds of this crate (the workspace's clippy and test jobs) have no installer; the
/// function is here so the shell's code stays one piece.
#[cfg(not(target_os = "android"))]
pub fn start(_apk: &Path) -> Result<(), String> {
    Err("installing an update is only done on Android".to_owned())
}

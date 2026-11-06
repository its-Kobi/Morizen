//! Native Android entry point for Morizen.
//!
//! The desktop game loop is included rather than forked: Android supplies Winit's `AndroidApp`
//! and an app-private working directory, then runs the same Rust game. The mobile bootstrap will
//! prepare that directory before entering the game; until then the missing-resources error is
//! deliberately logged instead of bundling Mojang files in the APK.

pub mod manifest;
pub mod sandbox;

#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("Morizen")
            .with_max_level(log::LevelFilter::Info),
    );
    horizon_client::telemetry::mark_start();
    let Some(home) = app.internal_data_path() else {
        log::error!("Android did not provide an internal data folder");
        return;
    };
    if let Err(error) =
        std::fs::create_dir_all(&home).and_then(|_| std::env::set_current_dir(&home))
    {
        log::error!("Could not open Morizen's Android data folder: {error}");
        return;
    }
    if let Err(error) = horizon_client::game::run_android(app) {
        log::error!("Morizen Android stopped: {error}");
    }
}

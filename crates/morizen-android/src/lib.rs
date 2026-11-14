//! Native Android entry point for Morizen.
//!
//! The app opens its own screen first (`shell`): it does not need any game data, so tapping the
//! icon always shows something. That screen downloads the player's Minecraft files and edits the
//! touch layout; when the player taps Play it hands the process's one event loop to the same Rust
//! game the desktop runs (`horizon_client::game::AndroidGame`).

pub mod font8x8;
pub mod gfx2d;
pub mod manifest;
pub mod sandbox;
pub mod shell;
pub mod tables;

#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("Morizen")
            .with_max_level(log::LevelFilter::Info),
    );
    horizon_client::telemetry::mark_start();
    if let Err(error) = shell::run(app) {
        log::error!("Morizen could not start: {error}");
    }
}

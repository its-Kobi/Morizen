use morizen_jvm::{JavaModHost, LaunchConfig};
use std::process::ExitCode;

fn run() -> morizen_jvm::Result<()> {
    let path = std::env::args_os().nth(1).ok_or_else(|| {
        morizen_jvm::BridgeError::Config("usage: empty_fabric <run-config.json>".into())
    })?;
    let config = LaunchConfig::from_path(path)?;
    let host = JavaModHost::start(config)?;
    let summary = host.summary();
    println!(
        "morizen-java: ready minecraft={} loader={} mods={}",
        summary.minecraft_version,
        summary.loader_version,
        summary.mods.len()
    );
    host.shutdown()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("morizen-java: {error}");
            ExitCode::FAILURE
        }
    }
}

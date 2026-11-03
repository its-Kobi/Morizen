//! Phase 2 proof: a real Fabric mod registers blocks and items through the
//! generated 26.3 shims, and the host turns the transaction into content mods.
//!
//! The Python harness (`tools/java_mod_bridge.py test`) checks the JSON and
//! the generated package; this example prints what the host produced.

use morizen_jvm::{JavaModHost, LaunchConfig};
use std::process::ExitCode;

fn run() -> morizen_jvm::Result<()> {
    let path = std::env::args_os().nth(1).ok_or_else(|| {
        morizen_jvm::BridgeError::Config("usage: registry_mod <run-config.json>".into())
    })?;
    let config = LaunchConfig::from_path(path)?;
    let host = JavaModHost::start(config)?;
    let (minecraft_version, loader_version, mods, content_mods) = {
        let summary = host.summary();
        (
            summary.minecraft_version.clone(),
            summary.loader_version.clone(),
            summary.mods.len(),
            summary.content_mods.clone(),
        )
    };
    println!(
        "morizen-java: ready minecraft={} loader={} mods={} content={}",
        minecraft_version,
        loader_version,
        mods,
        content_mods.len()
    );
    for path in &content_mods {
        println!("morizen-java: content-mod={}", path.display());
    }
    let result = host.shutdown();
    if content_mods.is_empty() && result.is_ok() {
        return Err(morizen_jvm::BridgeError::Registry(
            "the registry test mod produced no content mod".into(),
        ));
    }
    result
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

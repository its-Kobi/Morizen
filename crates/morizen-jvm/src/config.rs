use crate::{BridgeError, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_ENV: &str = "MORIZEN_JAVA_MOD_CONFIG";

fn default_minecraft_version() -> String {
    "26.3".into()
}

fn default_loader_version() -> String {
    "0.19.5".into()
}

fn default_heap_mb() -> u32 {
    256
}

fn default_startup_timeout_ms() -> u64 {
    60_000
}

fn default_shutdown_timeout_ms() -> u64 {
    5_000
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredMod {
    pub id: String,
    pub version: String,
    /// Set only by a passing compatibility test for this exact artifact.
    #[serde(default)]
    pub verified_compatible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchConfig {
    #[serde(default)]
    pub jdk_home: Option<PathBuf>,
    pub provider_jar: PathBuf,
    pub bootstrap_jar: PathBuf,
    pub fabric_libraries: Vec<PathBuf>,
    /// Third-party libraries from the user's pinned game install, never the game jar.
    #[serde(default)]
    pub game_libraries: Vec<PathBuf>,
    #[serde(default)]
    pub mod_jars: Vec<PathBuf>,
    #[serde(default)]
    pub mods: Vec<ConfiguredMod>,
    pub game_dir: PathBuf,
    pub marker_path: PathBuf,
    pub report_path: PathBuf,
    #[serde(default = "default_minecraft_version")]
    pub minecraft_version: String,
    #[serde(default = "default_loader_version")]
    pub loader_version: String,
    #[serde(default = "default_heap_mb")]
    pub max_heap_mb: u32,
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
    #[serde(default = "default_shutdown_timeout_ms")]
    pub shutdown_timeout_ms: u64,
}

impl LaunchConfig {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|error| {
            BridgeError::Config(format!("could not read {}: {error}", path.display()))
        })?;
        let config: Self = serde_json::from_slice(&bytes).map_err(|error| {
            BridgeError::Config(format!("could not parse {}: {error}", path.display()))
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn from_environment() -> Result<Option<Self>> {
        let Some(path) = env::var_os(CONFIG_ENV) else {
            return Ok(None);
        };
        Self::from_path(path).map(Some)
    }

    pub fn validate(&self) -> Result<()> {
        if self.minecraft_version != "26.3" {
            return Err(BridgeError::Config(format!(
                "expected Minecraft 26.3, got {}",
                self.minecraft_version
            )));
        }
        if self.loader_version != "0.19.5" {
            return Err(BridgeError::Config(format!(
                "expected Fabric Loader 0.19.5, got {}",
                self.loader_version
            )));
        }
        if !(32..=4096).contains(&self.max_heap_mb) {
            return Err(BridgeError::Config(format!(
                "max_heap_mb must be between 32 and 4096, got {}",
                self.max_heap_mb
            )));
        }
        if self.startup_timeout_ms == 0 || self.shutdown_timeout_ms == 0 {
            return Err(BridgeError::Config(
                "startup_timeout_ms and shutdown_timeout_ms must be nonzero".into(),
            ));
        }
        require_file("provider jar", &self.provider_jar)?;
        require_file("bootstrap jar", &self.bootstrap_jar)?;
        if self.fabric_libraries.is_empty() {
            return Err(BridgeError::Config(
                "fabric_libraries must not be empty".into(),
            ));
        }
        for library in &self.fabric_libraries {
            require_file("Fabric library", library)?;
        }
        for library in &self.game_libraries {
            require_file("game runtime library", library)?;
        }
        for mod_jar in &self.mod_jars {
            require_file("mod jar", mod_jar)?;
        }
        if self.marker_path.as_os_str().is_empty() || self.report_path.as_os_str().is_empty() {
            return Err(BridgeError::Config(
                "marker_path and report_path must not be empty".into(),
            ));
        }
        Ok(())
    }
}

fn require_file(kind: &str, path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err(BridgeError::Config(format!(
            "{kind} does not exist: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_pinned_to_phase_one_loader() {
        let json = r#"{
            "provider_jar":"missing-provider.jar",
            "bootstrap_jar":"missing-bootstrap.jar",
            "fabric_libraries":[],
            "game_dir":"game",
            "marker_path":"marker",
            "report_path":"report"
        }"#;
        let config: LaunchConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.minecraft_version, "26.3");
        assert_eq!(config.loader_version, "0.19.5");
        assert_eq!(config.max_heap_mb, 256);
        assert_eq!(config.startup_timeout_ms, 60_000);
        assert_eq!(config.shutdown_timeout_ms, 5_000);
    }
}

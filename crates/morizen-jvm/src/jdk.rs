use crate::{BridgeError, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REQUIRED_JAVA_MAJOR: u32 = 25;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JdkInfo {
    pub home: PathBuf,
    pub version: String,
    pub major: u32,
    pub libjvm: PathBuf,
}

impl JdkInfo {
    pub fn discover(explicit: Option<&Path>) -> Result<Self> {
        let home = if let Some(path) = explicit {
            path.to_path_buf()
        } else if let Some(path) = env::var_os("MORIZEN_JAVA_HOME") {
            PathBuf::from(path)
        } else if let Some(path) = env::var_os("JAVA_HOME") {
            PathBuf::from(path)
        } else {
            discover_java_home()?
        };
        Self::from_home(home)
    }

    pub fn from_home(home: impl Into<PathBuf>) -> Result<Self> {
        let home = home.into();
        let release_path = home.join("release");
        let release = fs::read_to_string(&release_path).map_err(|error| {
            BridgeError::Jdk(format!(
                "could not read {}: {error}",
                release_path.display()
            ))
        })?;
        let version = release_value(&release, "JAVA_VERSION").ok_or_else(|| {
            BridgeError::Jdk(format!("{} has no JAVA_VERSION", release_path.display()))
        })?;
        let major = java_major(&version)
            .ok_or_else(|| BridgeError::Jdk(format!("could not parse Java version {version:?}")))?;
        if major != REQUIRED_JAVA_MAJOR {
            return Err(BridgeError::Jdk(format!(
                "{} is Java {major} ({version}); Java {REQUIRED_JAVA_MAJOR} is required",
                home.display()
            )));
        }
        let libjvm = libjvm_path(&home);
        if !libjvm.is_file() {
            return Err(BridgeError::Jdk(format!(
                "JVM library does not exist: {}",
                libjvm.display()
            )));
        }
        Ok(Self {
            home,
            version,
            major,
            libjvm,
        })
    }
}

fn discover_java_home() -> Result<PathBuf> {
    let output = Command::new("java")
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .map_err(|error| BridgeError::Jdk(format!("could not run java: {error}")))?;
    if !output.status.success() {
        return Err(BridgeError::Jdk(format!(
            "java -version failed with {}",
            output.status
        )));
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        if key.trim() == "java.home" {
            return Ok(PathBuf::from(value.trim()));
        }
    }
    Err(BridgeError::Jdk(
        "java did not report its java.home; set MORIZEN_JAVA_HOME".into(),
    ))
}

fn release_value(release: &str, wanted: &str) -> Option<String> {
    release.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == wanted).then(|| value.trim().trim_matches('"').to_owned())
    })
}

fn java_major(version: &str) -> Option<u32> {
    let first = version.split(['.', '-', '_']).next()?;
    first.parse().ok()
}

#[cfg(target_os = "windows")]
fn libjvm_path(home: &Path) -> PathBuf {
    home.join("bin/server/jvm.dll")
}

#[cfg(target_os = "linux")]
fn libjvm_path(home: &Path) -> PathBuf {
    home.join("lib/server/libjvm.so")
}

#[cfg(target_os = "macos")]
fn libjvm_path(home: &Path) -> PathBuf {
    home.join("lib/server/libjvm.dylib")
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn libjvm_path(home: &Path) -> PathBuf {
    home.join("unsupported-libjvm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jdk_release_version() {
        let release = "IMPLEMENTOR=\"Example\"\nJAVA_VERSION=\"25.0.1\"\n";
        assert_eq!(
            release_value(release, "JAVA_VERSION").as_deref(),
            Some("25.0.1")
        );
        assert_eq!(java_major("25.0.1"), Some(25));
        assert_eq!(java_major("17-ea"), Some(17));
    }
}

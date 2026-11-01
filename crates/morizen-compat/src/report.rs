use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The on-disk compatibility report schema.
pub const REPORT_SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderInfo {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModCompatStatus {
    #[default]
    Works,
    Partial,
    Unsupported,
    #[serde(rename = "loader_failure")]
    LoaderFailure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedCall {
    pub member: String,
    pub phase: String,
    pub calls: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModReport {
    pub id: String,
    pub version: String,
    pub status: ModCompatStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<UnsupportedCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatReport {
    pub schema: u32,
    pub minecraft: String,
    pub loader: LoaderInfo,
    pub mods: Vec<ModReport>,
}

impl CompatReport {
    pub fn fabric(
        minecraft: impl Into<String>,
        loader_version: impl Into<String>,
        mods: Vec<ModReport>,
    ) -> Self {
        Self {
            schema: REPORT_SCHEMA,
            minecraft: minecraft.into(),
            loader: LoaderInfo {
                id: "fabric".into(),
                version: loader_version.into(),
            },
            mods,
        }
    }

    /// Writes and syncs a complete report beside the destination, then atomically
    /// replaces the previous report on Unix and Windows.
    pub fn write_atomic(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = temporary_path(path);
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        match replace_file(&temporary, path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(error)
            }
        }
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let mut source: Vec<u16> = source.as_os_str().encode_wide().collect();
    source.push(0);
    let mut destination: Vec<u16> = destination.as_os_str().encode_wide().collect();
    destination.push(0);
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnsupportedKey {
    pub mod_id: String,
    pub member: String,
    pub phase: String,
}

/// Deduplicates unsupported calls while retaining an exact call count.
#[derive(Default)]
pub struct UnsupportedCollector {
    calls: BTreeMap<UnsupportedKey, u64>,
}

impl UnsupportedCollector {
    /// Returns `true` only for the first occurrence, which lets callers log once.
    pub fn record(&mut self, key: UnsupportedKey) -> bool {
        let count = self.calls.entry(key).or_default();
        let first = *count == 0;
        *count = count.saturating_add(1);
        first
    }

    pub fn calls_for(&self, mod_id: &str) -> Vec<UnsupportedCall> {
        self.calls
            .iter()
            .filter(|(key, _)| key.mod_id == mod_id)
            .map(|(key, calls)| UnsupportedCall {
                member: key.member.clone(),
                phase: key.phase.clone(),
                calls: *calls,
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "morizen-compat-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn unsupported_calls_log_once_and_count_every_call() {
        let mut collector = UnsupportedCollector::default();
        let key = UnsupportedKey {
            mod_id: "example".into(),
            member: "net.minecraft.Level.getBlockState(BlockPos)".into(),
            phase: "main".into(),
        };
        assert!(collector.record(key.clone()));
        assert!(!collector.record(key));
        assert_eq!(
            collector.calls_for("example"),
            vec![UnsupportedCall {
                member: "net.minecraft.Level.getBlockState(BlockPos)".into(),
                phase: "main".into(),
                calls: 2,
            }]
        );
    }

    #[test]
    fn report_round_trips_through_disk() {
        let directory = scratch("report");
        let path = directory.join("java-mods/compat-report.json");
        let report = CompatReport::fabric(
            "26.3",
            "0.19.5",
            vec![ModReport {
                id: "empty".into(),
                version: "1.0.0".into(),
                status: ModCompatStatus::Works,
                unsupported: Vec::new(),
                error: None,
            }],
        );
        report.write_atomic(&path).unwrap();
        let saved: CompatReport = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, report);
        let replacement = CompatReport::fabric("26.3", "0.19.5", Vec::new());
        replacement.write_atomic(&path).unwrap();
        let saved: CompatReport = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, replacement);
        fs::remove_dir_all(directory).unwrap();
    }
}

//! Checking GitHub for a newer Morizen and downloading its APK.
//!
//! The check runs on a thread of its own at startup (and again when the player asks), because a
//! phone's first screen must be up before the network answers. The download uses the screen's job
//! machinery, so the progress bar and the status line work exactly like the game set-up's.

use std::io::{Read, Write};
use std::path::Path;

/// The release feed: the newest published Morizen release with its assets.
const LATEST: &str = "https://api.github.com/repos/its-Kobi/Morizen/releases/latest";
/// The APK inside it. The app is built for arm64 only, so that is the one to take.
const ASSET: &str = "morizen-android-arm64.apk";
/// This build's version, from `Cargo.toml`: what the release's tag is compared with.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The tag without its `v`, for example `1.0.4`.
    pub version: String,
    pub url: String,
    pub size: u64,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent("Morizen-Android")
        .build()
}

/// The newest release, when it is newer than this build and carries an APK. `Ok(None)` means
/// this build is the newest one.
pub fn check() -> Result<Option<Release>, String> {
    let body = agent()
        .get(LATEST)
        .call()
        .map_err(|e| format!("GitHub did not answer: {e}"))?
        .into_string()
        .map_err(|e| format!("GitHub's answer was unreadable: {e}"))?;
    let release: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("GitHub's answer was unreadable: {e}"))?;
    let tag = release
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let version = tag.trim_start_matches(['v', 'V']).to_owned();
    if version.is_empty() || !newer(&version, VERSION) {
        return Ok(None);
    }
    let asset = release
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset.get("name").and_then(serde_json::Value::as_str) == Some(ASSET))
        })
        .ok_or_else(|| format!("release {tag} has no {ASSET} to install"))?;
    let url = asset
        .get("browser_download_url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("release {tag}'s {ASSET} has no download link"))?;
    let size = asset
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok(Some(Release {
        version,
        url: url.to_owned(),
        size,
    }))
}

/// Whether `a` names a later version than `b`. The numbers decide; `v` and any suffix are
/// ignored, so `v1.0.4` is newer than `1.0.3` and `1.0.4-beta.2` counts as newer too.
pub fn newer(a: &str, b: &str) -> bool {
    numbers(a) > numbers(b)
}

fn numbers(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Downloads the release's APK to `to`, replacing whatever an earlier attempt left. `progress`
/// gets the bytes written and the total, the same shape the set-up's progress bar draws.
pub fn download(
    release: &Release,
    to: &Path,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), String> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let response = agent()
        .get(&release.url)
        .call()
        .map_err(|e| format!("Could not download the update: {e}"))?;
    let total = response
        .header("Content-Length")
        .and_then(|length| length.parse::<u64>().ok())
        .filter(|length| *length > 0)
        .unwrap_or(release.size);
    let mut file = std::fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut reported = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("The update download stopped: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("{}: {e}", to.display()))?;
        done += read as u64;
        if done - reported >= 1 << 20 {
            reported = done;
            progress(done, total);
        }
    }
    file.flush().map_err(|e| format!("{}: {e}", to.display()))?;
    progress(done, total);
    if total > 0 && done != total {
        return Err(format!("The update stopped at {done} of {total} bytes."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn a_later_tag_is_newer_and_the_same_one_is_not() {
        assert!(newer("1.0.4", "1.0.3"));
        assert!(newer("v1.0.4", "1.0.3"));
        assert!(newer("1.1", "1.0.9"));
        assert!(!newer("1.0.3", "1.0.3"));
        assert!(!newer("v1.0.3", "1.0.3"));
        assert!(!newer("1.0.2", "1.0.3"));
        assert!(!newer("", "1.0.3"));
    }
}

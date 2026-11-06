//! Native local/offline launch context. Morizen does not launch Mojang's Java main class, so Java
//! launcher placeholders such as `--accessToken` and `--userType` do not belong in its process.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalSandbox {
    pub username: String,
    pub uuid: u128,
    pub game_dir: PathBuf,
    pub resources: PathBuf,
}

impl LocalSandbox {
    pub fn new(
        username: impl Into<String>,
        game_dir: impl Into<PathBuf>,
        resources: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        let username = username.into();
        if !(3..=16).contains(&username.len())
            || !username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("An offline name must be 3 to 16 letters, digits or underscores.".into());
        }
        Ok(Self {
            uuid: horizon_protocol::crypto::offline_uuid(&username),
            username,
            game_dir: game_dir.into(),
            resources: resources.into(),
        })
    }

    /// Arguments understood by the native Rust game.
    pub fn args(&self) -> Vec<String> {
        vec![
            "--resources".into(),
            self.resources.to_string_lossy().into_owned(),
            "--name".into(),
            self.username.clone(),
        ]
    }

    pub fn prepare_directory(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.game_dir)
            .map_err(|e| format!("{}: {e}", self.game_dir.display()))
    }

    pub fn uuid_text(&self) -> String {
        let raw = format!("{:032x}", self.uuid);
        format!(
            "{}-{}-{}-{}-{}",
            &raw[..8],
            &raw[8..12],
            &raw[12..16],
            &raw[16..20],
            &raw[20..]
        )
    }

    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sandbox_has_a_stable_offline_identity_and_real_game_arguments() {
        let sandbox = LocalSandbox::new("MobilePlayer", "/game", "/resources").unwrap();
        assert_eq!(
            sandbox.uuid,
            horizon_protocol::crypto::offline_uuid("MobilePlayer")
        );
        assert_eq!(sandbox.uuid_text().len(), 36);
        assert_eq!(
            sandbox.args(),
            ["--resources", "/resources", "--name", "MobilePlayer"]
        );
        assert!(LocalSandbox::new("no spaces", "/game", "/resources").is_err());
    }
}

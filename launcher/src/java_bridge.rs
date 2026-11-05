//! The opt-in embedded Java mod bridge (`docs/java-mod-bridge.md`), off in every ordinary build.
//!
//! The launcher runs no JVM itself: the game's `java-mods` feature starts one and reads the
//! config named by `MORIZEN_JAVA_MOD_CONFIG`. This module only points the game at that config,
//! either the player's choice from Settings or the instance's own `java-mods/config.json`.
//! Without a config it sets nothing, so the launcher and the game behave as before.
use std::path::{Path, PathBuf};
use std::process::Command;

/// The variable the game reads at start (`morizen_jvm::config::CONFIG_ENV`).
pub const ENV: &str = "MORIZEN_JAVA_MOD_CONFIG";
/// The folder under an instance's home that holds the bridge's config and mod jars.
pub const FOLDER: &str = "java-mods";
/// The config's name inside [`FOLDER`].
pub const CONFIG: &str = "config.json";

/// The config an instance uses when the player has not chosen one.
pub fn config_path(home: &Path) -> PathBuf {
    home.join(FOLDER).join(CONFIG)
}

/// The config the game should read: the chosen one when it is a file, else the instance's own,
/// else none.
pub fn configured(home: &Path, chosen: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = chosen.filter(|path| path.is_file()) {
        return Some(path.to_path_buf());
    }
    let own = config_path(home);
    own.is_file().then_some(own)
}

/// Sets the bridge's variable on the game's command. Returns the config that was set, if any.
pub fn apply(cmd: &mut Command, home: &Path, chosen: Option<&Path>) -> Option<PathBuf> {
    let config = configured(home, chosen)?;
    cmd.env(ENV, &config);
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn has_config(cmd: &Command) -> bool {
        cmd.get_envs()
            .any(|(name, _)| name.to_string_lossy() == ENV)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("morizen-java-bridge-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_instance_uses_its_own_config_when_it_has_one() {
        let home = scratch("own");
        let own = config_path(&home);
        assert_eq!(configured(&home, None), None);
        fs::create_dir_all(own.parent().unwrap()).unwrap();
        fs::write(&own, "{}").unwrap();
        assert_eq!(configured(&home, None), Some(own));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_chosen_config_wins_and_a_missing_one_is_ignored() {
        let home = scratch("chosen");
        let own = config_path(&home);
        fs::create_dir_all(own.parent().unwrap()).unwrap();
        fs::write(&own, "{}").unwrap();
        let chosen = home.join("elsewhere.json");
        assert_eq!(configured(&home, Some(&chosen)), Some(own.clone()));
        fs::write(&chosen, "{}").unwrap();
        assert_eq!(configured(&home, Some(&chosen)), Some(chosen.clone()));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn the_game_command_only_gets_the_variable_with_a_config() {
        let home = scratch("command");
        let mut cmd = Command::new("horizon-client");
        assert_eq!(apply(&mut cmd, &home, None), None);
        assert!(!has_config(&cmd));
        let own = config_path(&home);
        fs::create_dir_all(own.parent().unwrap()).unwrap();
        fs::write(&own, "{}").unwrap();
        assert_eq!(apply(&mut cmd, &home, None), Some(own.clone()));
        let mut set = None;
        for (name, value) in cmd.get_envs() {
            if name.to_string_lossy() == ENV {
                set = value.map(PathBuf::from);
            }
        }
        assert_eq!(set, Some(own));
        let _ = fs::remove_dir_all(&home);
    }
}

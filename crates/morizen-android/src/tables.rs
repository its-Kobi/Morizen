//! The Rust edition's generated tables, bundled with the app as `tables.zip` and installed beside
//! Mojang's download on the device.
//!
//! The game reads tables that only Java exporters and the vanilla data generator can produce
//! (`facts.json`, `gameplay-facts.json`, the model and behaviour catalogs, the datagen reports).
//! A phone cannot run them, so the Android build's workflow runs `horizon-prepare` on a computer
//! and ships its output; everything that is Mojang's own data (the client JAR's `assets/` and
//! `data/`, the world generator's bundles and structure templates) is rebuilt here from the JAR
//! the player downloaded.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// The file that makes a folder a prepared game root (`horizon-prepare`'s output).
pub const MANIFEST: &str = "horizon-resources.json";

/// Unpacks `tables` into `root` and rebuilds the world generator's bundle from `client_jar`.
/// Returns how many files the archive held.
pub fn install(
    tables: &Path,
    root: &Path,
    client_jar: &Path,
    log: horizon_prepare::Log<'_>,
) -> Result<usize, String> {
    let file = fs::File::open(tables).map_err(|e| format!("{}: {e}", tables.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", tables.display()))?;
    let mut written = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        let target = root.join(member(&name)?);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut out =
            fs::File::create(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        io::copy(&mut entry, &mut out).map_err(|e| format!("{}: {e}", target.display()))?;
        written += 1;
    }
    if !root.join(MANIFEST).is_file() {
        return Err("The bundled tables have no horizon-resources.json.".into());
    }
    horizon_prepare::assemble::worldgen_from_client(root, client_jar, log)?;
    Ok(written)
}

/// A member path with no `..` and not absolute.
fn member(name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    let safe = !name.contains('\\')
        && !name.starts_with('/')
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
    safe.then(|| path.to_path_buf())
        .ok_or_else(|| format!("Unsafe path {name:?} in the bundled tables."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn refuses_a_member_that_climbs_out_of_the_root() {
        assert!(member("worldgen/biome.json").is_ok());
        assert!(member("../escape").is_err());
        assert!(member("/absolute").is_err());
        assert!(member("a/../../b").is_err());
    }

    /// A tiny table bundle is unpacked where the game reads it, and the manifest is required.
    #[test]
    fn unpacks_the_tables_beside_mojang_files() {
        let dir = std::env::temp_dir().join(format!(
            "morizen-tables-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("prepared");
        fs::create_dir_all(&root).unwrap();
        let tables = dir.join("tables.zip");
        let mut bytes = io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("facts.json", options).unwrap();
            zip.write_all(b"{}").unwrap();
            zip.start_file("worldgen/block_flags.json", options)
                .unwrap();
            zip.write_all(b"[]").unwrap();
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(b"{\"data_version\":5023}").unwrap();
            zip.finish().unwrap();
        }
        fs::write(&tables, bytes.into_inner()).unwrap();
        let client = dir.join("client.jar");
        fs::write(&client, b"not a jar").unwrap();
        // Without a readable client JAR the world generator cannot be written, but the tables are.
        let installed = install(&tables, &root, &client, &|_| {});
        assert!(installed.is_err());
        assert_eq!(fs::read_to_string(root.join("facts.json")).unwrap(), "{}");
        assert!(root.join("worldgen/block_flags.json").is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}

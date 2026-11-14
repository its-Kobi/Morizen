//! Mojang's public launcher-manifest and asset pipeline for Android.
//!
//! Nothing is embedded or redistributed: the caller chooses an app-private cache, and every
//! downloaded file is checked against the size and SHA-1 Mojang publishes. This module has no
//! account policy; a player-facing caller must perform the project's ownership gate before it is
//! invoked. Tests inject a local fetcher and contain no Mojang data.

use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const VERSION_MANIFEST: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const ASSET_OBJECTS: &str = "https://resources.download.minecraft.net";
const MANIFEST_LIMIT: u64 = 16 << 20;
const FILE_LIMIT: u64 = 1 << 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Manifest,
    Version,
    Client,
    AssetIndex,
    Assets,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Downloaded {
    pub version: String,
    pub root: PathBuf,
    pub version_json: PathBuf,
    pub client_jar: PathBuf,
    pub asset_index: PathBuf,
}

#[derive(Deserialize)]
struct LauncherManifest {
    versions: Vec<ManifestVersion>,
}

#[derive(Deserialize)]
struct ManifestVersion {
    id: String,
    url: String,
    sha1: String,
}

#[derive(Clone, Debug)]
struct Artifact {
    url: String,
    sha1: String,
    size: Option<u64>,
    path: PathBuf,
}

pub fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    // A phone network can stall for a moment without being gone, so a failed download is tried
    // again twice. The timeout is per socket read, never for the whole file: a big client JAR on
    // a slow connection takes minutes and is not a timeout (the first Android build used one
    // 90-second limit and stopped mid-download).
    for attempt in 0..3 {
        match fetch_once(url) {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                last = error;
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_secs(2 << attempt));
                }
            }
        }
    }
    Err(format!(
        "{last} (the connection stalled or was too slow; what was already downloaded is kept, so \
         tap DOWNLOAD & SET UP again)"
    ))
}

fn fetch_once(url: &str) -> Result<Vec<u8>, String> {
    let response = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout_read(std::time::Duration::from_secs(120))
        .timeout_write(std::time::Duration::from_secs(60))
        .user_agent("Morizen-Android/1.0")
        .build()
        .get(url)
        .call()
        .map_err(|e| format!("Could not download {url}: {e}"))?;
    let length = response
        .header("Content-Length")
        .and_then(|n| n.parse::<u64>().ok());
    if length.is_some_and(|n| n > FILE_LIMIT) {
        return Err(format!("{url} is larger than the 1 GiB safety limit."));
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Download interrupted: {e}"))?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(format!("{url} is larger than the 1 GiB safety limit."));
    }
    Ok(bytes)
}

/// Download one Minecraft version into the official-launcher-shaped cache below `root`.
pub fn download(
    version: &str,
    root: &Path,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<Downloaded, String> {
    download_with(version, root, &fetch_url, progress)
}

pub fn download_with<F>(
    version: &str,
    root: &Path,
    fetch: &F,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<Downloaded, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String> + Sync,
{
    if !plain(version) {
        return Err("The Minecraft version id is not a plain name.".into());
    }
    progress(Progress {
        phase: Phase::Manifest,
        done: 0,
        total: 1,
    });
    let manifest_bytes = limited(fetch(VERSION_MANIFEST)?, MANIFEST_LIMIT, "version manifest")?;
    let manifest: LauncherManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|e| format!("Version manifest: {e}"))?;
    let selected = manifest
        .versions
        .into_iter()
        .find(|entry| entry.id == version)
        .ok_or_else(|| format!("Mojang's manifest has no Minecraft {version}."))?;
    progress(Progress {
        phase: Phase::Version,
        done: 0,
        total: 1,
    });
    let versions = root.join("versions").join(version);
    let version_json = versions.join(format!("{version}.json"));
    let version_artifact = Artifact {
        url: selected.url,
        sha1: selected.sha1,
        size: None,
        path: version_json.clone(),
    };
    download_one(&version_artifact, fetch)?;
    let version_data: serde_json::Value = serde_json::from_slice(
        &fs::read(&version_json).map_err(|e| format!("{}: {e}", version_json.display()))?,
    )
    .map_err(|e| format!("Minecraft {version} metadata: {e}"))?;
    let client = artifact(
        &version_data["downloads"]["client"],
        versions.join(format!("{version}.jar")),
    )?;
    progress(Progress {
        phase: Phase::Client,
        done: 0,
        total: 1,
    });
    download_one(&client, fetch)?;
    let index = &version_data["assetIndex"];
    let index_id = text(index, "id")?;
    if !plain(&index_id) {
        return Err("The asset index id is not a plain name.".into());
    }
    let index_artifact = Artifact {
        url: text(index, "url")?,
        sha1: text(index, "sha1")?,
        size: index["size"].as_u64(),
        path: root.join("assets/indexes").join(format!("{index_id}.json")),
    };
    progress(Progress {
        phase: Phase::AssetIndex,
        done: 0,
        total: 1,
    });
    download_one(&index_artifact, fetch)?;
    let assets = asset_artifacts(&index_artifact.path, root, is_game_asset)?;
    download_many(&assets, fetch, progress)?;
    Ok(Downloaded {
        version: version.into(),
        root: root.into(),
        version_json,
        client_jar: client.path,
        asset_index: index_artifact.path,
    })
}

/// The assets only the asset index holds, which the game reads even without sound: the full title
/// panorama (the JAR has 1×1 placeholders), the Unifont fallback, the built-in resource packs, and
/// every bundled language (the JAR carries `en_us` alone). Sounds are 360 MB and come on their own
/// (`download_sounds`).
fn is_game_asset(name: &str) -> bool {
    name == "pack.mcmeta"
        || name.starts_with("minecraft/lang/")
        || name.starts_with("minecraft/font/")
        || name.starts_with("minecraft/resourcepacks/")
        || name.starts_with("minecraft/textures/gui/title/background/panorama")
}

/// Whether the game plays this asset: sounds.json and everything under sounds/.
fn is_sound(name: &str) -> bool {
    name == "minecraft/sounds.json" || name.starts_with("minecraft/sounds/")
}

/// Downloads the version's sound effects and music (about 360 MB) after the game itself is
/// installed; the store and the version JSON are already there.
pub fn download_sounds(
    version: &str,
    root: &Path,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<usize, String> {
    download_sounds_with(version, root, &fetch_url, progress)
}

pub fn download_sounds_with<F>(
    version: &str,
    root: &Path,
    fetch: &F,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<usize, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String> + Sync,
{
    let index = asset_index_path(root, version)?;
    let objects = asset_artifacts(&index, root, is_sound)?;
    download_many(&objects, fetch, progress)?;
    Ok(objects.len())
}

/// The asset index the version's metadata names, once it has been downloaded.
pub fn asset_index_path(root: &Path, version: &str) -> Result<PathBuf, String> {
    let version_json = root
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    let data: serde_json::Value = serde_json::from_slice(
        &fs::read(&version_json).map_err(|e| format!("{}: {e}", version_json.display()))?,
    )
    .map_err(|e| format!("Minecraft {version} metadata: {e}"))?;
    let index_id = text(&data["assetIndex"], "id")?;
    if !plain(&index_id) {
        return Err("The asset index id is not a plain name.".into());
    }
    Ok(root.join("assets/indexes").join(format!("{index_id}.json")))
}

/// Extract the raw `assets/` and `data/` trees and materialise indexed assets (notably sounds).
/// This intentionally does not write `horizon-resources.json`: the Rust edition's tables are
/// installed by `tables::install`, which also rebuilds the world generator from the client JAR.
pub fn extract_raw(download: &Downloaded, output: &Path) -> Result<(usize, usize), String> {
    let extracted = extract_jar_tree(&download.client_jar, output)?;
    let materialised = materialise_assets(&download.root, &download.asset_index, output)?;
    Ok((extracted, materialised))
}

/// Every `assets/` and `data/` member of Mojang's client JAR, at its own path.
fn extract_jar_tree(client_jar: &Path, output: &Path) -> Result<usize, String> {
    let file = fs::File::open(client_jar).map_err(|e| format!("{}: {e}", client_jar.display()))?;
    let mut jar = zip::ZipArchive::new(file).map_err(|e| format!("Client JAR: {e}"))?;
    let mut extracted = 0;
    for index in 0..jar.len() {
        let mut entry = jar.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        if entry.is_dir() || !(name.starts_with("assets/") || name.starts_with("data/")) {
            continue;
        }
        if entry.size() > FILE_LIMIT {
            return Err(format!("JAR member {name:?} is unexpectedly large."));
        }
        let relative = safe_path(&name)?;
        write_stream(&output.join(relative), &mut entry)?;
        extracted += 1;
    }
    Ok(extracted)
}

/// Copies every asset-index object the store below `root` has into `output/assets/<name>`, the
/// layout the game reads. Objects that were not downloaded (sounds before their own download)
/// are skipped.
pub fn materialise_assets(root: &Path, asset_index: &Path, output: &Path) -> Result<usize, String> {
    let index: serde_json::Value = serde_json::from_slice(
        &fs::read(asset_index).map_err(|e| format!("{}: {e}", asset_index.display()))?,
    )
    .map_err(|e| format!("Asset index: {e}"))?;
    let objects = index["objects"]
        .as_object()
        .ok_or("The asset index has no objects.")?;
    let mut materialised = 0;
    for (name, value) in objects {
        let hash = value["hash"].as_str().ok_or("An asset has no hash.")?;
        valid_hash(hash)?;
        let from = root.join("assets/objects").join(&hash[..2]).join(hash);
        if !from.is_file() {
            continue;
        }
        let to = output.join("assets").join(safe_path(name)?);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::copy(&from, &to).map_err(|e| format!("{}: {e}", to.display()))?;
        materialised += 1;
    }
    Ok(materialised)
}

fn artifact(value: &serde_json::Value, path: PathBuf) -> Result<Artifact, String> {
    Ok(Artifact {
        url: text(value, "url")?,
        sha1: text(value, "sha1")?,
        size: value["size"].as_u64(),
        path,
    })
}

fn text(value: &serde_json::Value, field: &str) -> Result<String, String> {
    value[field]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Minecraft metadata has no {field}."))
}

fn asset_artifacts(
    index: &Path,
    root: &Path,
    wanted: fn(&str) -> bool,
) -> Result<Vec<Artifact>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(index).map_err(|e| format!("{}: {e}", index.display()))?)
            .map_err(|e| format!("Asset index: {e}"))?;
    let objects = value["objects"]
        .as_object()
        .ok_or("The asset index has no objects.")?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (name, object) in objects {
        if !wanted(name) {
            continue;
        }
        let hash = object["hash"].as_str().ok_or("An asset has no hash.")?;
        valid_hash(hash)?;
        if !seen.insert(hash.to_owned()) {
            continue;
        }
        let relative = format!("{}/{hash}", &hash[..2]);
        out.push(Artifact {
            url: format!("{ASSET_OBJECTS}/{relative}"),
            sha1: hash.into(),
            size: object["size"].as_u64(),
            path: root.join("assets/objects").join(relative),
        });
    }
    Ok(out)
}

fn download_many<F>(
    files: &[Artifact],
    fetch: &F,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<(), String>
where
    F: Fn(&str) -> Result<Vec<u8>, String> + Sync,
{
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let error = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..8.min(files.len().max(1)) {
            scope.spawn(|| {
                while let Some(file) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if error.lock().unwrap().is_some() {
                        break;
                    }
                    if let Err(failure) = download_one(file, fetch) {
                        *error.lock().unwrap() = Some(failure);
                        break;
                    }
                    let count = done.fetch_add(1, Ordering::Relaxed) + 1;
                    progress(Progress {
                        phase: Phase::Assets,
                        done: count,
                        total: files.len(),
                    });
                }
            });
        }
    });
    error.into_inner().unwrap().map_or(Ok(()), Err)
}

fn download_one<F>(file: &Artifact, fetch: &F) -> Result<(), String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    valid_hash(&file.sha1)?;
    if sha1_file(&file.path).as_deref() == Some(file.sha1.as_str()) {
        return Ok(());
    }
    let bytes = fetch(&file.url)?;
    if file.size.is_some_and(|size| size != bytes.len() as u64) {
        return Err(format!("{} has the wrong size.", file.url));
    }
    let actual = sha1_bytes(&bytes);
    if actual != file.sha1 {
        return Err(format!("{} failed its Mojang SHA-1 check.", file.url));
    }
    if let Some(parent) = file.path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let partial = file.path.with_extension("part");
    fs::write(&partial, bytes).map_err(|e| format!("{}: {e}", partial.display()))?;
    if file.path.exists() {
        fs::remove_file(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
    }
    fs::rename(&partial, &file.path).map_err(|e| format!("{}: {e}", file.path.display()))
}

fn sha1_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hash = Sha1::new();
    let mut buffer = [0; 1 << 16];
    loop {
        let size = file.read(&mut buffer).ok()?;
        if size == 0 {
            return Some(format!("{:x}", hash.finalize()));
        }
        hash.update(&buffer[..size]);
    }
}

fn sha1_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

fn valid_hash(hash: &str) -> Result<(), String> {
    (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(())
        .ok_or_else(|| format!("Invalid SHA-1 {hash:?}."))
}

fn plain(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn safe_path(name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    (!name.contains('\\')
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_))))
    .then(|| path.to_path_buf())
    .ok_or_else(|| format!("Unsafe path {name:?}."))
}

fn limited(bytes: Vec<u8>, limit: u64, what: &str) -> Result<Vec<u8>, String> {
    (bytes.len() as u64 <= limit)
        .then_some(bytes)
        .ok_or_else(|| format!("The {what} is unexpectedly large."))
}

fn write_stream(path: &Path, reader: &mut impl Read) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut file = fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::copy(reader, &mut file).map_err(|e| format!("{}: {e}", path.display()))?;
    file.flush().map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Cursor;

    fn jar() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            zip.start_file(
                "assets/minecraft/lang/en_us.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"{}").unwrap();
            zip.finish().unwrap();
        }
        bytes.into_inner()
    }

    #[test]
    fn manifest_client_index_and_assets_are_checked_cached_and_extracted() {
        let client = jar();
        let lang = b"{}".to_vec();
        let sound = b"sound".to_vec();
        let client_hash = sha1_bytes(&client);
        let lang_hash = sha1_bytes(&lang);
        let sound_hash = sha1_bytes(&sound);
        let index = serde_json::to_vec(&serde_json::json!({"objects": {
            "minecraft/lang/de_de.json": {"hash": lang_hash.clone(), "size": lang.len()},
            "minecraft/sounds/a.ogg": {"hash": sound_hash.clone(), "size": sound.len()}
        }}))
        .unwrap();
        let index_hash = sha1_bytes(&index);
        let version = serde_json::to_vec(&serde_json::json!({
            "downloads": {"client": {"url": "client", "sha1": client_hash, "size": client.len()}},
            "assetIndex": {"id": "26", "url": "index", "sha1": index_hash, "size": index.len()}
        }))
        .unwrap();
        let version_hash = sha1_bytes(&version);
        let manifest = serde_json::to_vec(&serde_json::json!({"versions": [{
            "id": "26.3", "url": "version", "sha1": version_hash
        }]}))
        .unwrap();
        let mut replies = HashMap::from([
            (VERSION_MANIFEST.to_owned(), manifest),
            ("version".into(), version),
            ("client".into(), client),
            ("index".into(), index),
        ]);
        replies.insert(
            format!("{ASSET_OBJECTS}/{}/{sound_hash}", &sound_hash[..2]),
            sound,
        );
        replies.insert(
            format!("{ASSET_OBJECTS}/{}/{lang_hash}", &lang_hash[..2]),
            lang,
        );
        let fetch = |url: &str| replies.get(url).cloned().ok_or_else(|| url.to_owned());
        let root = std::env::temp_dir().join(format!("morizen-mobile-{}", std::process::id()));
        let downloaded = download_with("26.3", &root, &fetch, &|_| {}).unwrap();
        let output = root.join("prepared");
        assert_eq!(extract_raw(&downloaded, &output).unwrap(), (1, 1));
        assert_eq!(
            fs::read(output.join("assets/minecraft/lang/de_de.json")).unwrap(),
            b"{}"
        );
        // Sound effects are a separate, much larger download, and are materialised later.
        assert!(!output.join("assets/minecraft/sounds/a.ogg").exists());
        assert_eq!(
            download_sounds_with("26.3", &root, &fetch, &|_| {}).unwrap(),
            1
        );
        let index_path = asset_index_path(&root, "26.3").unwrap();
        assert_eq!(materialise_assets(&root, &index_path, &output).unwrap(), 2);
        assert_eq!(
            fs::read(output.join("assets/minecraft/sounds/a.ogg")).unwrap(),
            b"sound"
        );
        assert!(output.join("assets/minecraft/lang/en_us.json").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_downloads_and_unsafe_names_are_refused() {
        let path = std::env::temp_dir().join("morizen-mobile-bad");
        let file = Artifact {
            url: "bad".into(),
            sha1: sha1_bytes(b"right"),
            size: Some(5),
            path,
        };
        assert!(download_one(&file, &|_| Ok(b"wrong".to_vec())).is_err());
        assert!(safe_path("../outside").is_err());
        assert!(safe_path("assets\\outside").is_err());
    }
}

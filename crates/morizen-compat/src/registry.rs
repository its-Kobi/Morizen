//! Registry content a Java mod contributes, translated to the declarative
//! content-mod format the native engine already loads.
//!
//! The Java side commits one JSON transaction at the end of its entrypoints
//! (`java-mods/registry.json`). This module validates that transaction and
//! writes it as a content mod folder: declarations, models, textures and
//! translations. The native game then loads it through the same
//! `horizon_mods` / `mod_content` path as any content mod, so there is no
//! second registry.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The registry transaction schema written by `dev.morizen.compat.NativeBridge`.
pub const REGISTRY_SCHEMA: u32 = 1;

/// A block a Java mod registered, expressed as a copy of a vanilla template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockDeclaration {
    /// `namespace:path`; the namespace is the mod id.
    pub id: String,
    /// The vanilla block whose state/behaviour tables are copied.
    #[serde(default = "default_block_template")]
    pub template: String,
    /// `BlockBehaviour.Properties#strength`, also used for explosion
    /// resistance as vanilla's `strength(float)` does.
    #[serde(default = "default_strength")]
    pub strength: f32,
    /// `BlockBehaviour.Properties#lightLevel` at the block's default state.
    #[serde(default)]
    pub light: u8,
    /// `BlockBehaviour.Properties#requiresCorrectToolForDrops`.
    #[serde(default)]
    pub requires_tool: bool,
    /// `BlockBehaviour.Properties#friction`.
    #[serde(default = "default_friction")]
    pub friction: f32,
    /// Whether a `BlockItem` was registered for the same id.
    #[serde(default)]
    pub item: bool,
}

/// An item a Java mod registered, copying a vanilla template's components.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemDeclaration {
    pub id: String,
    #[serde(default = "default_item_template")]
    pub template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stack_size: Option<u8>,
}

/// Everything one Java mod registered during loader entrypoints.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RegistrationSet {
    pub mod_id: String,
    #[serde(default)]
    pub blocks: Vec<BlockDeclaration>,
    #[serde(default)]
    pub items: Vec<ItemDeclaration>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryTransaction {
    schema: u32,
    /// The namespace whose file this is; the Java bridge writes one file per
    /// namespace so a mod cannot contribute content under another mod's id.
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    blocks: Vec<BlockDeclaration>,
    #[serde(default)]
    items: Vec<ItemDeclaration>,
}

fn default_block_template() -> String {
    "minecraft:stone".into()
}

fn default_item_template() -> String {
    "minecraft:paper".into()
}

fn default_strength() -> f32 {
    1.0
}

fn default_friction() -> f32 {
    0.6
}

impl RegistrationSet {
    /// Reads every `namespace.json` transaction the Java bridge wrote.
    /// Files are validated independently so one bad mod does not hide the
    /// others' declarations.
    pub fn read_directory(directory: &Path) -> Result<Vec<Self>, String> {
        let mut sets = Vec::new();
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(sets),
            Err(error) => {
                return Err(format!(
                    "could not read the registry transaction directory {}: {error}",
                    directory.display()
                ));
            }
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        paths.sort();
        for path in paths {
            let namespace = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| format!("{} has no UTF-8 namespace", path.display()))?;
            let bytes = fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            sets.push(Self::from_json(namespace, &bytes)?);
        }
        Ok(sets)
    }

    /// Parses the file `dev.morizen.compat.NativeBridge` writes at the end of
    /// the entrypoints. Unknown fields are refused so a newer Java bridge
    /// cannot silently lose registrations.
    pub fn from_json(mod_id: &str, bytes: &[u8]) -> Result<Self, String> {
        let transaction: RegistryTransaction = serde_json::from_slice(bytes)
            .map_err(|error| format!("registry transaction is not valid JSON: {error}"))?;
        if transaction.schema != REGISTRY_SCHEMA {
            return Err(format!(
                "unsupported registry transaction schema {}",
                transaction.schema
            ));
        }
        if let Some(namespace) = &transaction.namespace
            && namespace != mod_id
        {
            return Err(format!(
                "registry transaction for {namespace:?} was filed under {mod_id:?}"
            ));
        }
        let set = Self {
            mod_id: mod_id.to_owned(),
            blocks: transaction.blocks,
            items: transaction.items,
        };
        set.validate()?;
        Ok(set)
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty() && self.items.is_empty()
    }

    /// Checks ids, namespaces, duplicates and limits before anything reaches
    /// the engine. A refused transaction disables Java mods rather than
    /// applying half of it.
    pub fn validate(&self) -> Result<(), String> {
        if !valid_mod_id(&self.mod_id) {
            return Err(format!(
                "mod id {:?} is not a content-mod id ([a-z0-9_]{{1,64}}); its blocks and items cannot be named",
                self.mod_id
            ));
        }
        if self.blocks.len() + self.items.len() > 4096 {
            return Err("a Java mod may register at most 4096 blocks and items".into());
        }
        let mut seen = BTreeMap::new();
        for block in &self.blocks {
            check_id(&self.mod_id, &block.id, "block")?;
            check_template(&block.template, "block")?;
            if !block.strength.is_finite() || !(0.0..=1_000_000.0).contains(&block.strength) {
                return Err(format!("{}: strength must be a finite number", block.id));
            }
            if !block.friction.is_finite() || !(0.0..=2.0).contains(&block.friction) {
                return Err(format!(
                    "{}: friction must be a finite number between 0 and 2",
                    block.id
                ));
            }
            if seen.insert(block.id.clone(), "block").is_some() {
                return Err(format!("{}: registered twice (as a block)", block.id));
            }
        }
        for item in &self.items {
            check_id(&self.mod_id, &item.id, "item")?;
            check_template(&item.template, "item")?;
            if let Some(size) = item.max_stack_size
                && !(1..=99).contains(&size)
            {
                return Err(format!(
                    "{}: max stack size {size} is outside 1..=99",
                    item.id
                ));
            }
            if let Some(previous) = seen.insert(item.id.clone(), "item") {
                return Err(format!(
                    "{}: registered twice (as a {previous}, then an item)",
                    item.id
                ));
            }
        }
        Ok(())
    }

    /// The content-mod folder this transaction becomes.
    pub fn content_package(&self) -> Result<ContentPackage, String> {
        self.validate()?;
        let mut files = BTreeMap::new();
        let mut lang = BTreeMap::new();
        for block in &self.blocks {
            let (namespace, path) = split(&block.id);
            files.insert(
                format!("data/{namespace}/horizon/block/{path}.json"),
                declaration_json(block)?,
            );
            let rgba = texture_color(&block.id);
            let texture = format!("assets/{namespace}/textures/block/{path}.png");
            files.insert(texture, png_solid(16, 16, rgba));
            files.insert(
                format!("assets/{namespace}/blockstates/{path}.json"),
                serde_json::to_vec(&serde_json::json!({
                    "variants": { "": { "model": format!("{namespace}:block/{path}") } }
                }))
                .map_err(|error| error.to_string())?,
            );
            files.insert(
                format!("assets/{namespace}/models/block/{path}.json"),
                serde_json::to_vec(&serde_json::json!({
                    "parent": "minecraft:block/cube_all",
                    "textures": { "all": format!("{namespace}:block/{path}") }
                }))
                .map_err(|error| error.to_string())?,
            );
            if block.item {
                files.insert(
                    format!("assets/{namespace}/models/item/{path}.json"),
                    serde_json::to_vec(&serde_json::json!({
                        "parent": format!("{namespace}:block/{path}")
                    }))
                    .map_err(|error| error.to_string())?,
                );
            }
            let name = pretty_name(path);
            lang.insert(format!("block.{namespace}.{path}"), name.clone());
            if block.item {
                lang.insert(format!("item.{namespace}.{path}"), name);
            }
        }
        for item in &self.items {
            let (namespace, path) = split(&item.id);
            files.insert(
                format!("data/{namespace}/horizon/item/{path}.json"),
                item_json(item)?,
            );
            files.insert(
                format!("assets/{namespace}/models/item/{path}.json"),
                serde_json::to_vec(&serde_json::json!({
                    "parent": "minecraft:item/generated",
                    "textures": { "layer0": format!("{namespace}:item/{path}") }
                }))
                .map_err(|error| error.to_string())?,
            );
            files.insert(
                format!("assets/{namespace}/textures/item/{path}.png"),
                png_solid(16, 16, texture_color(&item.id)),
            );
            lang.insert(format!("item.{namespace}.{path}"), pretty_name(path));
        }
        // Mods may own their own assets and data only, so one language file
        // per namespace (the mod id) is enough.
        let namespace = &self.mod_id;
        files.insert(
            format!("assets/{namespace}/lang/en_us.json"),
            serde_json::to_vec(&lang).map_err(|error| error.to_string())?,
        );
        Ok(ContentPackage {
            manifest: manifest(&self.mod_id),
            files,
        })
    }
}

fn declaration_json(block: &BlockDeclaration) -> Result<Vec<u8>, String> {
    let mut properties = serde_json::Map::new();
    properties.insert("hardness".into(), block.strength.into());
    properties.insert("explosion_resistance".into(), block.strength.into());
    if block.light > 0 {
        properties.insert("light".into(), u64::from(block.light).into());
    }
    if block.requires_tool {
        properties.insert("requires_tool".into(), true.into());
    }
    if (block.friction - 0.6).abs() > f32::EPSILON {
        properties.insert("friction".into(), block.friction.into());
    }
    serde_json::to_vec(&serde_json::json!({
        "template": block.template,
        "properties": properties,
        "item": block.item,
    }))
    .map_err(|error| error.to_string())
}

fn item_json(item: &ItemDeclaration) -> Result<Vec<u8>, String> {
    let mut value = serde_json::json!({ "template": item.template });
    if let Some(size) = item.max_stack_size {
        value["max_stack_size"] = size.into();
    }
    serde_json::to_vec(&value).map_err(|error| error.to_string())
}

fn manifest(mod_id: &str) -> String {
    format!(
        "id = \"{mod_id}\"\nname = \"{mod_id}\"\nversion = \"1.0.0\"\n\
         api = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n"
    )
}

/// A parsed content-mod package, written as a folder under the game directory.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentPackage {
    pub manifest: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl ContentPackage {
    /// Writes the package into `directory`, replacing a previous package.
    /// Returns the folder to hand to the game's `--dev-mod` list.
    pub fn write(&self, directory: &Path) -> io::Result<PathBuf> {
        if directory.exists() {
            fs::remove_dir_all(directory)?;
        }
        fs::create_dir_all(directory)?;
        fs::write(directory.join("horizon-mod.toml"), &self.manifest)?;
        for (name, bytes) in &self.files {
            let path = directory.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)?;
        }
        Ok(directory.to_owned())
    }
}

fn valid_mod_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn check_id(mod_id: &str, id: &str, kind: &str) -> Result<(), String> {
    let Some((namespace, path)) = id.split_once(':') else {
        return Err(format!("{kind} id {id:?} is not namespace:path"));
    };
    if namespace != mod_id {
        return Err(format!(
            "{kind} id {id:?} is outside the mod's namespace {mod_id:?}"
        ));
    }
    if path.is_empty()
        || path.len() > 128
        || !path.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b'/')
        })
    {
        return Err(format!("{kind} id {id:?} has an invalid path"));
    }
    Ok(())
}

fn check_template(template: &str, kind: &str) -> Result<(), String> {
    let Some(path) = template.strip_prefix("minecraft:") else {
        return Err(format!(
            "{kind} template {template:?} must name a minecraft: id"
        ));
    };
    if path.is_empty()
        || !path.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b'/')
        })
    {
        return Err(format!("{kind} template {template:?} is not a vanilla id"));
    }
    Ok(())
}

fn split(id: &str) -> (&str, &str) {
    id.split_once(':').unwrap_or(("minecraft", id))
}

fn pretty_name(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A stable colour per id, so two blocks never share a texture.
fn texture_color(id: &str) -> [u8; 4] {
    let mut hash = 0x811c_9dc5u32;
    for byte in id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    [
        96 + (hash & 0x7f) as u8,
        96 + ((hash >> 7) & 0x7f) as u8,
        96 + ((hash >> 15) & 0x7f) as u8,
        0xff,
    ]
}

/// A minimal, valid 8-bit RGBA PNG with stored (uncompressed) deflate blocks.
/// Textures are generated, never copied from Mojang's assets.
pub fn png_solid(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut row = Vec::with_capacity(width as usize * 4 + 1);
    row.push(0); // filter: none
    for _ in 0..width {
        row.extend_from_slice(&rgba);
    }
    let mut raw = Vec::with_capacity(row.len() * height as usize);
    for _ in 0..height {
        raw.extend_from_slice(&row);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut offset = 0;
    while offset < raw.len() {
        let length = (raw.len() - offset).min(0xffff);
        let final_block = offset + length == raw.len();
        zlib.push(u8::from(final_block));
        zlib.extend_from_slice(&(length as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(length as u16)).to_le_bytes());
        zlib.extend_from_slice(&raw[offset..offset + length]);
        offset += length;
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RegistrationSet {
        RegistrationSet {
            mod_id: "morizen_registry".into(),
            blocks: vec![BlockDeclaration {
                id: "morizen_registry:ruby_block".into(),
                template: "minecraft:stone".into(),
                strength: 4.0,
                light: 3,
                requires_tool: true,
                friction: 0.6,
                item: true,
            }],
            items: vec![ItemDeclaration {
                id: "morizen_registry:ruby".into(),
                template: "minecraft:paper".into(),
                max_stack_size: Some(16),
            }],
        }
    }

    #[test]
    fn a_transaction_becomes_a_loadable_content_package() {
        let package = sample().content_package().unwrap();
        assert!(package.manifest.contains("id = \"morizen_registry\""));
        let files: Vec<&str> = package.files.keys().map(String::as_str).collect();
        for expected in [
            "data/morizen_registry/horizon/block/ruby_block.json",
            "data/morizen_registry/horizon/item/ruby.json",
            "assets/morizen_registry/blockstates/ruby_block.json",
            "assets/morizen_registry/models/block/ruby_block.json",
            "assets/morizen_registry/models/item/ruby_block.json",
            "assets/morizen_registry/models/item/ruby.json",
            "assets/morizen_registry/textures/block/ruby_block.png",
            "assets/morizen_registry/textures/item/ruby.png",
            "assets/morizen_registry/lang/en_us.json",
        ] {
            assert!(files.contains(&expected), "missing {expected}");
        }
        let block: serde_json::Value = serde_json::from_slice(
            &package.files["data/morizen_registry/horizon/block/ruby_block.json"],
        )
        .unwrap();
        assert_eq!(block["template"], "minecraft:stone");
        assert_eq!(block["properties"]["hardness"], 4.0);
        assert_eq!(block["properties"]["light"], 3);
        assert_eq!(block["item"], true);
        let lang: serde_json::Value =
            serde_json::from_slice(&package.files["assets/morizen_registry/lang/en_us.json"])
                .unwrap();
        assert_eq!(lang["block.morizen_registry.ruby_block"], "Ruby Block");
        assert_eq!(lang["item.morizen_registry.ruby"], "Ruby");
    }

    #[test]
    fn png_is_a_valid_rgba_image() {
        let png = png_solid(16, 16, [1, 2, 3, 255]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 16);
        assert_eq!(png[24], 8);
        assert_eq!(png[25], 6);
        assert!(png.windows(4).any(|w| w == b"IEND"));
    }

    #[test]
    fn ids_outside_the_mods_namespace_are_refused() {
        let mut set = sample();
        set.blocks[0].id = "minecraft:stone".into();
        assert!(
            set.validate()
                .unwrap_err()
                .contains("outside the mod's namespace")
        );
        let mut set = sample();
        set.mod_id = "Not-A-Content-Id".into();
        assert!(set.validate().is_err());
        let mut set = sample();
        set.items.push(set.items[0].clone());
        let error = set.validate().unwrap_err();
        assert!(error.contains("registered twice"), "{error}");
    }

    #[test]
    fn the_transaction_file_round_trips() {
        let json = serde_json::json!({
            "schema": 1,
            "namespace": "morizen_registry",
            "blocks": [{
                "id": "morizen_registry:ruby_block",
                "template": "minecraft:stone",
                "strength": 4.0,
                "light": 3,
                "requires_tool": true,
                "friction": 0.6,
                "item": true
            }],
            "items": [{
                "id": "morizen_registry:ruby",
                "template": "minecraft:paper",
                "max_stack_size": 16
            }]
        });
        let parsed =
            RegistrationSet::from_json("morizen_registry", &serde_json::to_vec(&json).unwrap())
                .unwrap();
        assert_eq!(parsed, sample());
        assert!(RegistrationSet::from_json("morizen_registry", b"{\"schema\":2}").is_err());
    }

    #[test]
    fn a_package_writes_a_folder_the_mod_loader_can_read() {
        let directory =
            std::env::temp_dir().join(format!("morizen-registry-package-{}", std::process::id()));
        let path = sample()
            .content_package()
            .unwrap()
            .write(&directory)
            .unwrap();
        assert!(path.join("horizon-mod.toml").is_file());
        assert!(
            path.join("data/morizen_registry/horizon/block/ruby_block.json")
                .is_file()
        );
        assert!(
            path.join("assets/morizen_registry/textures/block/ruby_block.png")
                .is_file()
        );
        fs::remove_dir_all(directory).unwrap();
    }
}

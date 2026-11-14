//! The parts of a prepared resource root that need no Java: the world generator's bundle, rebuilt
//! from a client JAR. A desktop checkout gets them from `prepare_worldgen` (which also runs the
//! exporters); an Android device, whose Java exporters cannot run, downloads the generated tables
//! from the build and assembles the rest from the JAR it downloaded from Mojang.

use crate::{
    copy, json_files, json_resources, member_path, mkdir, open_zip, pyjson, write_if_changed,
    write_text, zip_bytes, zip_names, Log, WORLDGEN_BUNDLES,
};
use std::fs;
use std::path::Path;

/// Writes `<resources>/worldgen` for `client_jar`: the 19 resource bundles, every structure
/// template, and the data generator's multi-noise biome parameter reports. The client JAR holds
/// exactly the server's `data/minecraft` tree for these (checked by `prepare_worldgen`), so a
/// device with only Mojang's client can still generate the same worlds.
pub fn worldgen_from_client(resources: &Path, client_jar: &Path, log: Log) -> Result<(), String> {
    log("Writing the world generator's bundle");
    let out = resources.join("worldgen");
    mkdir(&out)?;
    let mut client = open_zip(client_jar)?;
    structures(&mut client, &out)?;
    bundles(&mut client, &out)?;
    biome_parameters(resources, &out)?;
    Ok(())
}

/// Structure templates: gzip NBT files under `structure/<namespace>/<path>.nbt`.
pub(crate) fn structures(
    archive: &mut zip::ZipArchive<fs::File>,
    out: &Path,
) -> Result<(), String> {
    let prefix = "data/minecraft/structure/";
    for name in zip_names(archive) {
        if name.starts_with(prefix) && name.ends_with(".nbt") {
            let relative = member_path(&name[prefix.len()..], "Invalid structure path")?;
            let data = zip_bytes(archive, &name)?;
            write_if_changed(&out.join("structure/minecraft").join(relative), &data)?;
        }
    }
    Ok(())
}

/// The chunk generator's JSON bundles (`GeneratorData::load`): `<name>.json` holding
/// `{"minecraft:<path>": text}` for every `<prefix><path>.json` in the JAR.
pub(crate) fn bundles(archive: &mut zip::ZipArchive<fs::File>, out: &Path) -> Result<(), String> {
    for (name, path) in WORLDGEN_BUNDLES {
        let prefix = format!("data/minecraft/{path}/");
        let resources = json_resources(archive, &prefix)?;
        write_text(
            &out.join(format!("{name}.json")),
            &pyjson::dumps(&resources, &pyjson::COMPACT),
        )?;
    }
    Ok(())
}

/// The data generator's `biome_parameters` reports, in the layout `GeneratorData::load` reads
/// (`worldgen/biome_parameters/<name>.json`). Missing reports are not an error: `presets.json`
/// carries the same tables when a preparation had no data generator run.
pub(crate) fn biome_parameters(resources: &Path, out: &Path) -> Result<(), String> {
    let source = resources.join("biome-parameters");
    if !source.join("overworld.json").exists() {
        return Ok(());
    }
    let reports = out.join("biome_parameters");
    mkdir(&reports)?;
    for report in json_files(&source)? {
        copy(&report, &reports.join(report.file_name().unwrap()))?;
    }
    Ok(())
}

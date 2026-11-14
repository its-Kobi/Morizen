#!/usr/bin/env python3
"""Packs the Rust edition's generated tables for the Android app.

The APK ships only what a phone cannot produce: the tables Horizon's Java exporters and Mojang's
data generator wrote. Everything else is rebuilt on the device from the client JAR it downloads
from Mojang, so no Mojang file travels in the app:

* `asset-index.json` is Mojang's asset index verbatim, and `pack.png` is the client JAR's own
  icon: both are left out (the device writes the index itself and copies `pack.png` from the JAR).
* `assets/` and `data/` come from the client JAR.
* `worldgen/<bundle>.json` and `worldgen/structure/**` are rebuilt from the JAR by
  `horizon_prepare::assemble::worldgen_from_client`.
* `worldgen/biome_parameters/**` is a copy of the shipped `biome-parameters/**`.

    python3 tools/ci/android_tables.py --root .local/horizon-client/26.3 \\
        --output crates/morizen-android/assets/tables.zip
"""

import argparse
import pathlib
import sys
import zipfile

# Mojang's own files, which the device gets from the download instead of from the app.
EXCLUDED = {"asset-index.json", "pack.png"}
# Directly under `worldgen/`, these are generated and shipped; every other bundle is Mojang's
# data, rebuilt from the client JAR on the device.
SHIPPED_BUNDLES = {"block_flags.json", "presets.json"}
REQUIRED = [
    "horizon-resources.json",
    "facts.json",
    "gameplay-facts.json",
    "block-tags.json",
    "block-behaviours.json",
    "items.json",
    "registries.json",
    "commands.json",
    "worldgen/block_flags.json",
    "worldgen/blocks/structure-blocks.json",
]
# A fixed timestamp keeps the archive identical between runs, so the APK cache key holds.
STAMP = (2020, 1, 1, 0, 0, 0)


def wanted(relative: pathlib.PurePosixPath) -> bool:
    parts = relative.parts
    if not parts or parts[0] in EXCLUDED:
        return False
    top = parts[0]
    if top in ("assets", "data", "classes", "client-classes"):
        return False
    if top == "biome-parameters":
        return len(parts) == 2 and relative.suffix == ".json"
    if top == "worldgen":
        if parts[1:2] in (("structure",), ("biome_parameters",)):
            return False
        if len(parts) == 2 and relative.suffix == ".json":
            return relative.name in SHIPPED_BUNDLES
        return True
    return len(parts) == 1 and relative.suffix in (".json", ".nbt")


def collect(root: pathlib.Path) -> list[tuple[pathlib.Path, str]]:
    files = []
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(root)
        if wanted(pathlib.PurePosixPath(relative.as_posix())):
            files.append((path, relative.as_posix()))
    return files


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".local/horizon-client/26.3")
    parser.add_argument(
        "--output", default="crates/morizen-android/assets/tables.zip", type=pathlib.Path
    )
    args = parser.parse_args()

    root = pathlib.Path(args.root)
    missing = [name for name in REQUIRED if not (root / name).is_file()]
    if missing:
        for name in missing:
            print(f"missing {root / name}", file=sys.stderr)
        raise SystemExit("the prepared root is incomplete; run horizon-prepare --prepare first")

    files = collect(root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    raw = 0
    with zipfile.ZipFile(args.output, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path, name in files:
            info = zipfile.ZipInfo(name, date_time=STAMP)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            data = path.read_bytes()
            raw += len(data)
            archive.writestr(info, data)
    packed = args.output.stat().st_size
    print(f"{len(files)} files, {raw / 1048576:.1f} MB raw -> {packed / 1048576:.1f} MB")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

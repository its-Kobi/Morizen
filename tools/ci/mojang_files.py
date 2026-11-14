#!/usr/bin/env python3
"""Downloads the pinned Minecraft release's files for the Android table build.

The Android app ships no Mojang file. Its workflow runs `horizon-prepare` on a runner to write
the tables a phone cannot generate (the Java exporters' catalogs and Mojang's data generator
reports), and the preparer needs the player's own game files: the version JSON, the client JAR,
the server jar and the version's library artifacts. This script fetches exactly those from
Mojang into a `.minecraft`-shaped folder, checking each against the SHA-1 (and size) Mojang
publishes and the client/server SHA-1s pinned in `research-lock.json`.

    python3 tools/ci/mojang_files.py --dir .local/ci-minecraft

Only the generated tables leave the build; the jars stay in the workspace and are dropped with
the runner.
"""

import argparse
import concurrent.futures
import hashlib
import json
import pathlib
import sys
import urllib.request

MANIFEST_URL = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
USER_AGENT = "Morizen-Android-CI/1.0 (+https://github.com/its-Kobi/Morizen)"
CHUNK = 1 << 20


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=180) as response:
        return response.read()


def fetch_checked(url: str, path: pathlib.Path, sha1: str, size: int | None) -> None:
    """Downloads `url` to `path`, checking the size and SHA-1 Mojang publishes."""
    if path.is_file():
        data = path.read_bytes()
        if hashlib.sha1(data).hexdigest() == sha1 and (size is None or len(data) == size):
            return
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    path.parent.mkdir(parents=True, exist_ok=True)
    digest = hashlib.sha1()
    written = 0
    with urllib.request.urlopen(request, timeout=180) as response, path.open("wb") as out:
        while True:
            chunk = response.read(CHUNK)
            if not chunk:
                break
            written += len(chunk)
            digest.update(chunk)
            out.write(chunk)
    if size is not None and written != size:
        raise SystemExit(f"{url} is {written} bytes, not the published {size}")
    if digest.hexdigest() != sha1:
        raise SystemExit(f"{url} does not match its published SHA-1")


def libraries(version: dict) -> list[tuple[str, str, str, int | None]]:
    """`(url, sha1, relative path, size)` of every library artifact the version JSON names."""
    found = []
    for library in version.get("libraries", []):
        artifact = library.get("downloads", {}).get("artifact")
        if not artifact:
            continue
        url = artifact.get("url")
        sha1 = artifact.get("sha1")
        path = artifact.get("path")
        if not (url and sha1 and path):
            continue
        found.append((url, sha1, path, artifact.get("size")))
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", default=".local/ci-minecraft", help="the .minecraft-shaped folder")
    parser.add_argument("--version", default=None, help="the release (default: the lock's)")
    parser.add_argument("--lock", default="research-lock.json")
    parser.add_argument("--jobs", type=int, default=8)
    args = parser.parse_args()

    lock = json.loads(pathlib.Path(args.lock).read_text())["minecraft"]
    version_id = args.version or lock["version"]
    root = pathlib.Path(args.dir)

    manifest = json.loads(fetch(MANIFEST_URL))
    entry = next((v for v in manifest["versions"] if v["id"] == version_id), None)
    if entry is None:
        raise SystemExit(f"Mojang's manifest has no {version_id}")

    versions = root / "versions" / version_id
    version_json = versions / f"{version_id}.json"
    fetch_checked(entry["url"], version_json, entry["sha1"], None)
    version = json.loads(version_json.read_text())
    downloads = version.get("downloads", {})
    for side, pinned in (("client", lock["client_sha1"]), ("server", lock["server_sha1"])):
        published = downloads.get(side, {}).get("sha1")
        if published != pinned:
            raise SystemExit(
                f"{version_id}'s {side} is {published}, but research-lock.json pins {pinned}"
            )
    print(f"{version_id}: version JSON checked against the manifest")

    client = versions / f"{version_id}.jar"
    fetch_checked(
        downloads["client"]["url"], client, downloads["client"]["sha1"], downloads["client"].get("size")
    )
    server = root / "server.jar"
    fetch_checked(
        downloads["server"]["url"], server, downloads["server"]["sha1"], downloads["server"].get("size")
    )
    print(f"{version_id}: client and server jars checked against the lock")

    artifacts = libraries(version)
    errors = []

    def download(artifact):
        url, sha1, path, size = artifact
        try:
            fetch_checked(url, root / "libraries" / path, sha1, size)
        except Exception as error:  # noqa: BLE001 - reported together below
            errors.append(f"{path}: {error}")

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        list(pool.map(download, artifacts))
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        raise SystemExit(f"{len(errors)} of {len(artifacts)} libraries failed")
    print(f"{version_id}: {len(artifacts)} library artifacts checked")
    print(f"Minecraft files in {root}")
    print(f"  --minecraft-dir {root}")
    print(f"  --server-jar {server}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Set up the launcher to play the Java mod bridge test build.

The launcher passes `MORIZEN_JAVA_MOD_CONFIG` to the game; the game's opt-in `java-mods`
feature starts the embedded JVM. This script takes a kit built by the `java mod bridge`
GitHub Actions workflow (or a local build), copies the newest prepared launcher version
into a test version, puts the kit's game binaries in it, and gives the launcher the bridge
config. It builds nothing and downloads nothing.

    py -3 tools/java_mods_launcher.py --kit . --config path/to/config.json

The config comes from `py -3 tools/java_mod_bridge.py prepare`, which needs the player's
own Minecraft 26.3 jar and a JDK (never Mojang files in Git). Without `--config` the
script tries `.local/java-mod-bridge/in-game/config.json` in a checkout and
`java-mods/config.json` beside itself.

Nothing here touches the version the player already plays: the test build gets its own
tag (`morizen-java-mods`), which can be removed from the launcher's Versions page.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Optional

DEFAULT_TAG = "morizen-java-mods"
FOLDER = "java-mods"
CONFIG = "config.json"
BINARIES = ("horizon-launcher", "horizon-client", "horizon-ctl")
COMPLETE = ".installed"
RESOURCES_MANIFEST = "horizon-resources.json"


def _exe(name: str) -> str:
    return name + (".exe" if sys.platform == "win32" else "")


def data_dir(env: Optional[dict] = None) -> Path:
    """The folder dirs::data_dir gives the launcher: `%APPDATA%`, XDG's data dir, or macOS."""
    env = os.environ if env is None else env
    if sys.platform == "win32":
        return Path(env.get("APPDATA") or Path.home() / "AppData" / "Roaming")
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support"
    return Path(env.get("XDG_DATA_HOME") or Path.home() / ".local" / "share")


def launcher_home(env: Optional[dict] = None) -> Path:
    env = os.environ if env is None else env
    override = env.get("HORIZON_LAUNCHER_HOME")
    return Path(override) if override else data_dir(env) / "horizon-launcher"


def game_home(env: Optional[dict] = None) -> Path:
    env = os.environ if env is None else env
    override = env.get("HORIZON_HOME")
    return Path(override) if override else data_dir(env) / "horizon"


def installed(home: Path) -> list:
    """The version folders the launcher shows, newest name first only for deterministic tests."""
    versions = home / "versions"
    if not versions.is_dir():
        return []
    return sorted(
        (
            path
            for path in versions.iterdir()
            if path.is_dir() and not path.name.startswith(".")
        ),
        key=lambda path: path.name,
    )


def prepared(version: Path) -> bool:
    """Whether the version already has the resources the game reads (no preparation needed)."""
    resources = version / ".local" / "horizon-client"
    if not resources.is_dir():
        return False
    return any(child.joinpath(RESOURCES_MANIFEST).is_file() for child in resources.iterdir())


def pick_source(home: Path) -> Optional[Path]:
    """The newest prepared, complete version: the one whose resources the test build reuses."""
    found = [
        path
        for path in installed(home)
        if path.joinpath(COMPLETE).is_file()
        and path.joinpath(_exe("horizon-client")).is_file()
        and prepared(path)
    ]
    if not found:
        return None
    return max(found, key=lambda path: path.joinpath(COMPLETE).stat().st_mtime)


def copy_version(source: Path, target: Path) -> None:
    """Copy a version folder: its resources, assets and programs, but not the logs."""
    shutil.copytree(
        source,
        target,
        dirs_exist_ok=True,
        ignore=shutil.ignore_patterns("logs", "crash-reports"),
    )


def install_binaries(version: Path, kit: Path) -> None:
    """Put the kit's game in the version, replacing only those programs."""
    for name in BINARIES[1:]:
        program = kit / _exe(name)
        if program.is_file():
            shutil.copy2(program, version / _exe(name))


def bridge_config(
    kit: Path, checkout: Optional[Path], explicit: Optional[Path]
) -> Optional[Path]:
    """The bridge config to use: the one given, a checkout's known-good one, or the kit's."""
    candidates = []
    if explicit is not None:
        candidates.append(Path(explicit))
    if checkout is not None:
        candidates.append(checkout / ".local" / "java-mod-bridge" / "in-game" / CONFIG)
    candidates.append(kit / ".local" / "java-mod-bridge" / "in-game" / CONFIG)
    candidates.append(kit / FOLDER / CONFIG)
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    return None


def save_settings(home: Path, tag: str, config: Path) -> Path:
    """Mark the test version selected and give the launcher its bridge config."""
    path = home / "settings.json"
    try:
        settings = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(settings, dict):
            settings = {}
    except (OSError, ValueError):
        settings = {}
    settings["selected"] = tag
    settings["java_mods_config"] = str(config)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(settings, indent=2) + "\n", encoding="utf-8")
    return path


def install(
    kit: Path,
    home: Path,
    tag: str = DEFAULT_TAG,
    config: Optional[Path] = None,
    checkout: Optional[Path] = None,
) -> Path:
    """Build the test version and return its folder. Raises SystemExit with a readable reason."""
    if not (kit / _exe("horizon-client")).is_file():
        raise SystemExit(f"{kit} has no {_exe('horizon-client')}: point --kit at the kit folder.")
    source = pick_source(home)
    if source is None:
        raise SystemExit(
            "No prepared Morizen version in the launcher. Install and play any version once, "
            "then run this again."
        )
    found = bridge_config(kit, checkout, config)
    if found is None:
        raise SystemExit(
            "No bridge config. Run `py -3 tools/java_mod_bridge.py prepare` with your own "
            "Minecraft 26.3 jar, then pass --config .local/java-mod-bridge/in-game/config.json."
        )
    target = home / "versions" / tag
    if source.resolve() != target.resolve():
        copy_version(source, target)
    target.joinpath(COMPLETE).touch()
    install_binaries(target, kit)
    # The default instance's own copy, so Play also works if the setting is cleared later.
    own = game_home() / FOLDER
    own.mkdir(parents=True, exist_ok=True)
    shutil.copy2(found, own / CONFIG)
    save_settings(home, tag, found)
    return target


def checkout_of(script: Path) -> Optional[Path]:
    """A checkout when the script runs from its `tools/` folder; None inside a kit."""
    if script.parent.name == "tools" and (script.parent.parent / "Cargo.toml").is_file():
        return script.parent.parent
    return None


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kit", type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument("--config", type=Path)
    parser.add_argument("--tag", default=DEFAULT_TAG)
    parser.add_argument("--no-start", action="store_true", help="set things up without starting it")
    args = parser.parse_args(argv)
    kit = args.kit.resolve()
    home = launcher_home()
    target = install(kit, home, args.tag, args.config, checkout_of(Path(__file__).resolve()))
    print(f"Test version: {target}")
    print(f"Config: {home / 'settings.json'}")
    launcher = kit / _exe("horizon-launcher")
    if args.no_start:
        return 0
    if not launcher.is_file():
        print(f"Start {_exe('horizon-launcher')} and press Play.", file=sys.stderr)
        return 0
    subprocess.Popen([str(launcher)], cwd=str(kit))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

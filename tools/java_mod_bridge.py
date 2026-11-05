#!/usr/bin/env python3
"""Build and exercise the local Phase 1 Java-mod bridge artifacts.

All class files and jars are written under .local/java-mod-bridge. The Fabric
dependencies come from the repository's hash-pinned benchmark lock.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

try:
    from . import lab, runtime
except ImportError:
    import lab
    import runtime


ROOT = Path(__file__).resolve().parents[1]
JAVA_ROOT = ROOT / "compat-java"
OUT = ROOT / ".local" / "java-mod-bridge"
BUILD = OUT / "build"
JAVA_MAJOR = 25
LOADER_VERSION = "0.19.5"
QUILT_LOCK = JAVA_ROOT / "quilt.lock.json"


def java_major(version):
    try:
        return int(version.split(".", 1)[0].split("-", 1)[0].split("_", 1)[0])
    except (TypeError, ValueError):
        return None


def release_values(home):
    path = Path(home) / "release"
    values = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip().strip('"')
    return values


def discover_jdk_home(explicit=None):
    candidate = explicit or os.environ.get("MORIZEN_JAVA_HOME") or os.environ.get("JAVA_HOME")
    if candidate:
        home = Path(candidate).expanduser().resolve()
    else:
        result = subprocess.run(
            ["java", "-XshowSettings:properties", "-version"],
            text=True,
            capture_output=True,
            check=True,
        )
        home = None
        for line in (result.stdout + "\n" + result.stderr).splitlines():
            if "=" not in line:
                continue
            key, value = line.strip().split("=", 1)
            if key.strip() == "java.home":
                home = Path(value.strip()).resolve()
                break
        if home is None:
            raise RuntimeError("java did not report java.home; pass --jdk-home")
    values = release_values(home)
    version = values.get("JAVA_VERSION")
    if java_major(version) != JAVA_MAJOR:
        raise RuntimeError(f"{home} is Java {version!r}; Java {JAVA_MAJOR} is required")
    return home


def jdk_tool(home, name):
    suffix = ".exe" if os.name == "nt" else ""
    path = Path(home) / "bin" / f"{name}{suffix}"
    if not path.is_file():
        raise RuntimeError(f"JDK tool does not exist: {path}")
    return path


def run(command, *, env=None, capture=False, check=True, show_capture=True, timeout=None):
    shown = " ".join(str(part) for part in command)
    print(f"+ {shown}")
    result = subprocess.run(
        [str(part) for part in command],
        cwd=ROOT,
        env=env,
        text=True,
        capture_output=capture,
        timeout=timeout,
    )
    if capture and show_capture:
        if result.stdout:
            print(result.stdout, end="")
        if result.stderr:
            print(result.stderr, end="", file=sys.stderr)
    if check and result.returncode:
        raise RuntimeError(f"command failed with exit code {result.returncode}: {shown}")
    return result


def compile_java(jdk, sources, destination, classpath):
    sources = sorted(map(Path, sources))
    if not sources:
        raise RuntimeError(f"no Java sources selected for {destination}")
    destination.mkdir(parents=True, exist_ok=True)
    command = [
        jdk_tool(jdk, "javac"),
        "--release",
        str(JAVA_MAJOR),
        "-proc:none",
        "-encoding",
        "UTF-8",
        "-cp",
        os.pathsep.join(map(str, classpath)),
        "-d",
        destination,
        *sources,
    ]
    run(command)


def copy_resources(source, destination):
    if not source.is_dir():
        return
    for path in sorted(source.rglob("*")):
        if path.is_file():
            output = destination / path.relative_to(source)
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, output)


def zip_tree(destination, roots):
    """Create a reproducible jar from directory roots, in precedence order."""
    entries = {}
    for root in roots:
        root = Path(root)
        if not root.is_dir():
            continue
        for path in sorted(root.rglob("*")):
            if path.is_file():
                entries[path.relative_to(root).as_posix()] = path.read_bytes()
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name, data in sorted(entries.items()):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, data)


def pinned_fabric_libraries():
    libraries = list(runtime.fabric_libraries())
    loader = [path for path in libraries if path.name == f"fabric-loader-{LOADER_VERSION}.jar"]
    if len(loader) != 1:
        raise RuntimeError(f"expected one Fabric Loader {LOADER_VERSION} jar, found {loader}")
    return libraries, loader[0]


def pinned_quilt_libraries():
    lock = json.loads(QUILT_LOCK.read_text(encoding="utf-8"))
    paths = []
    for entry in lock["libraries"]:
        path = runtime.fetch(
            entry["url"],
            ROOT / ".local" / "runtime" / "quilt" / entry["name"],
            entry["sha256"],
            "sha256",
        )
        if path.stat().st_size != entry["size"]:
            raise RuntimeError(f"Quilt runtime library has the wrong size: {path}")
        paths.append(path)
    return lock["loader"], paths


def game_runtime_libraries(minecraft_jar):
    """Official third-party artifacts beside the user's jar, never the game jar itself."""
    minecraft_jar = Path(minecraft_jar).resolve()
    manifest = minecraft_jar.with_suffix(".json")
    if not manifest.is_file():
        return []
    root = minecraft_jar.parents[2]
    libraries = []
    for entry in json.loads(manifest.read_text(encoding="utf-8")).get("libraries", []):
        artifact = entry.get("downloads", {}).get("artifact", {})
        relative = artifact.get("path")
        if not relative:
            continue
        path = root / "libraries" / relative
        if path.is_file():
            libraries.append(path.resolve())
    return libraries


def build(jdk_home=None, minecraft_jar=None):
    jdk = discover_jdk_home(jdk_home)
    libraries, loader = pinned_fabric_libraries()
    quilt, quilt_own_libraries = pinned_quilt_libraries()
    quilt_loader = quilt_own_libraries[0]
    quilt_libraries = [
        *quilt_own_libraries,
        *(path for path in libraries if "fabric-loader" not in path.name),
    ]
    game_libraries = game_runtime_libraries(minecraft_jar)
    if BUILD.exists():
        shutil.rmtree(BUILD)
    BUILD.mkdir(parents=True)

    main = JAVA_ROOT / "src" / "main" / "java"
    provider_sources = list((main / "dev" / "morizen" / "compat" / "provider").rglob("*.java"))
    bootstrap_sources = [
        path for path in main.rglob("*.java")
        if "provider" not in path.relative_to(main).parts
    ]
    # The registry package and its exception import Minecraft types, so they
    # compile in the same javac run as the generated shims.
    registry_sources = [
        path for path in bootstrap_sources
        if "registry" in path.relative_to(main).parts
        or path.name == "UnsupportedCompatException.java"
    ]
    provider_classes = BUILD / "provider-classes"
    quilt_provider_classes = BUILD / "quilt-provider-classes"
    bootstrap_classes = BUILD / "bootstrap-classes"
    quilt_bootstrap_classes = BUILD / "quilt-bootstrap-classes"
    compile_java(jdk, provider_sources, provider_classes, libraries)
    copy_resources(JAVA_ROOT / "src" / "main" / "resources", provider_classes)
    quilt_root = JAVA_ROOT / "src" / "quilt"
    compile_java(
        jdk,
        (quilt_root / "java" / "dev" / "morizen" / "compat" / "quilt" / "provider").rglob("*.java"),
        quilt_provider_classes,
        quilt_libraries,
    )
    copy_resources(quilt_root / "resources", quilt_provider_classes)

    # The generated shims are signatures validated against the user's own jar.
    # They are loaded by Knot beside the bootstrapped bridge.
    try:
        from . import gen_compat
    except ImportError:
        import gen_compat
    shims = gen_compat.build(
        minecraft_jar,
        jdk,
        gen_compat.GENERATED,
        extra_sources=registry_sources,
    )
    shim_classes = shims["classes"]
    shim_jar = shims["shim_jar"]

    compile_java(jdk, bootstrap_sources, bootstrap_classes, [shim_classes, *libraries])
    compile_java(
        jdk,
        [
            path
            for path in (quilt_root / "java").rglob("*.java")
            if "provider" not in path.relative_to(quilt_root / "java").parts
        ],
        quilt_bootstrap_classes,
        [bootstrap_classes, shim_classes, *quilt_libraries],
    )

    contract_classes = BUILD / "contract-classes"
    compile_java(
        jdk,
        (JAVA_ROOT / "src" / "test" / "java").rglob("*.java"),
        contract_classes,
        [provider_classes, *libraries],
    )

    test_mod_jars = {}
    for name in (
        "empty",
        "failing",
        "registry",
        "lifecycle",
        "lifecycle-failing",
        "mixin-head",
        "quilt",
    ):
        source = JAVA_ROOT / "test-mods" / name / "src" / "main"
        classes = BUILD / f"{name}-mod-classes"
        classpath = [*libraries]
        if name in {"registry", "mixin-head"}:
            classpath = [shim_classes, *classpath]
        elif name.startswith("lifecycle"):
            classpath = [bootstrap_classes, *classpath]
        elif name == "quilt":
            classpath = [bootstrap_classes, *quilt_libraries]
        compile_java(jdk, (source / "java").rglob("*.java"), classes, classpath)
        copy_resources(source / "resources", classes)
        jar = BUILD / f"morizen-{name}-test-mod.jar"
        zip_tree(jar, [classes])
        test_mod_jars[name] = jar

    provider_jar = BUILD / "morizen-provider.jar"
    bootstrap_jar = BUILD / "morizen-bootstrap.jar"
    zip_tree(provider_jar, [provider_classes, quilt_provider_classes])
    zip_tree(bootstrap_jar, [bootstrap_classes, quilt_bootstrap_classes])

    def write_config(name, entries, *, loader_id="fabric", loader_version=LOADER_VERSION):
        run_root = OUT / "test-runs" / name
        if run_root.exists():
            shutil.rmtree(run_root)
        game = run_root / "game"
        game.mkdir(parents=True)
        config = {
            "jdk_home": str(jdk),
            "provider_jar": str(provider_jar.resolve()),
            "bootstrap_jar": str(bootstrap_jar.resolve()),
            "shims_jar": str(Path(shim_jar).resolve()),
            "fabric_libraries": [
                str(path.resolve())
                for path in (
                    quilt_libraries
                    if loader_id == "quilt"
                    else libraries
                )
            ],
            "game_libraries": [],
            "mod_jars": [str(test_mod_jars[entry[0]].resolve()) for entry in entries],
            "mods": [
                {
                    "id": entry[1],
                    "version": "1.0.0",
                    "verified_compatible": entry[2],
                }
                for entry in entries
            ],
            "game_dir": str(game.resolve()),
            "marker_path": str((run_root / "entrypoints.txt").resolve()),
            "report_path": str((game / "java-mods" / "compat-report.json").resolve()),
            "minecraft_version": "26.3",
            "loader_id": loader_id,
            "loader_version": loader_version,
            "max_heap_mb": 256,
            "startup_timeout_ms": 60_000,
            "shutdown_timeout_ms": 5_000,
            "event_timeout_ms": 2_000,
        }
        config_path = run_root / "run-config.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        return {"root": run_root, "values": config, "path": config_path}

    configs = {
        "empty": write_config("empty", [("empty", "morizen_empty", True)]),
        "failing": write_config("failing", [("failing", "morizen_failing", False)]),
        "registry": write_config("registry", [("registry", "morizen_registry", True)]),
        "lifecycle": write_config(
            "lifecycle",
            [
                ("lifecycle", "morizen_lifecycle", True),
                ("lifecycle-failing", "morizen_lifecycle_failing", False),
            ],
        ),
        "mixin-head": write_config(
            "mixin-head", [("mixin-head", "morizen_mixin_head", True)]
        ),
        "quilt": write_config(
            "quilt",
            [("quilt", "morizen_quilt", True)],
            loader_id="quilt",
            loader_version=quilt["version"],
        ),
    }
    return {
        "jdk": jdk,
        "libraries": libraries,
        "loader": loader,
        "quilt_loader": quilt_loader,
        "game_libraries": game_libraries,
        "provider_jar": provider_jar,
        "bootstrap_jar": bootstrap_jar,
        "provider_classes": provider_classes,
        "contract_classes": contract_classes,
        "shims": shims,
        "shim_jar": shim_jar,
        "configs": configs,
    }


def mod_metadata(path):
    """A mod jar's id and version from its Fabric or Quilt metadata, when it has any."""
    try:
        with zipfile.ZipFile(path) as jar:
            names = set(jar.namelist())
            if "fabric.mod.json" in names:
                data = json.loads(jar.read("fabric.mod.json"))
                return data.get("id"), data.get("version")
            if "quilt.mod.json" in names:
                data = json.loads(jar.read("quilt.mod.json"))
                loader = data.get("quilt_loader", {})
                return loader.get("id"), loader.get("version")
    except (OSError, ValueError, zipfile.BadZipFile):
        pass
    return None, None


def write_game_config(artifacts, mods=None, out=None):
    """Write the config the game (and the launcher) reads, pointed at `mods`.

    `mods` is a folder of jars or one jar. Test mods built here keep their verified mark;
    every other jar starts `verified_compatible: false`, as the ledger requires.
    """
    out = Path(out) if out else OUT / "in-game" / "config.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    base = artifacts["configs"]["empty"]["values"]
    jars = []
    if mods is not None:
        mods = Path(mods)
        jars = [mods] if mods.is_file() else sorted(mods.glob("*.jar"))
    entries = []
    for jar in jars:
        mod_id, version = mod_metadata(jar)
        mod_id = mod_id or jar.stem
        entries.append(
            {
                "id": mod_id,
                "version": str(version or "0"),
                "verified_compatible": mod_id.startswith("morizen_"),
            }
        )
    game = out.parent / "game"
    game.mkdir(parents=True, exist_ok=True)
    config = {
        "jdk_home": base.get("jdk_home"),
        "provider_jar": base["provider_jar"],
        "bootstrap_jar": base["bootstrap_jar"],
        "shims_jar": base.get("shims_jar"),
        "fabric_libraries": base["fabric_libraries"],
        "game_libraries": base.get("game_libraries", []),
        "mod_jars": [str(jar.resolve()) for jar in jars],
        "mods": entries,
        "game_dir": str(game.resolve()),
        "marker_path": str((out.parent / "entrypoints.txt").resolve()),
        "report_path": str((out.parent / "compat-report.json").resolve()),
        "minecraft_version": "26.3",
        "loader_id": base.get("loader_id", "fabric"),
        "loader_version": base.get("loader_version", LOADER_VERSION),
        "max_heap_mb": 1024,
        "startup_timeout_ms": 60_000,
        "shutdown_timeout_ms": 5_000,
        "event_timeout_ms": 2_000,
    }
    out.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    return out


def run_contract(artifacts):
    classpath = [
        artifacts["contract_classes"],
        artifacts["provider_classes"],
        *artifacts["libraries"],
    ]
    result = run(
        [
            jdk_tool(artifacts["jdk"], "java"),
            "-cp",
            os.pathsep.join(map(str, classpath)),
            "dev.morizen.compat.LoaderContractTest",
        ],
        capture=True,
    )
    expected = "morizen-java: loader contract=0.19.5 methods=22"
    if expected not in result.stdout:
        raise RuntimeError(f"loader contract marker missing: {expected}")


def cargo_environment(run_root):
    environment = os.environ.copy()
    rustc = subprocess.run(
        ["rustc", "--print", "sysroot"], text=True, capture_output=True, check=True
    ).stdout.strip()
    target = subprocess.run(
        ["rustc", "-vV"], text=True, capture_output=True, check=True
    ).stdout
    host = next(line.split(":", 1)[1].strip() for line in target.splitlines() if line.startswith("host:"))
    bundled_tools = Path(rustc) / "lib" / "rustlib" / host / "bin" / "self-contained"
    if bundled_tools.is_dir():
        environment["PATH"] = str(bundled_tools) + os.pathsep + environment.get("PATH", "")

    homes = Path(run_root) / "homes"
    values = {
        "HORIZON_HOME": homes / "game",
        "HORIZON_LAUNCHER_HOME": homes / "launcher",
        "XDG_DATA_HOME": homes / "xdg-data",
        "XDG_CONFIG_HOME": homes / "xdg-config",
        "HORIZON_LAUNCHER_TRASH": homes / "trash",
        "HORIZON_LAUNCHER_CLIPBOARD": homes / "clipboard",
        "HORIZON_LAUNCHER_DESKTOP": homes / "desktop",
    }
    for path in values.values():
        path.mkdir(parents=True, exist_ok=True)
    return lab.game_env(base=environment, **values)


def run_empty_mod(artifacts):
    config = artifacts["configs"]["empty"]
    environment = cargo_environment(config["root"])
    result = run(
        [
            "cargo",
            "run",
            "-p",
            "morizen-jvm",
            "--example",
            "empty_fabric",
            "--",
            config["path"],
        ],
        env=environment,
        capture=True,
    )
    required = [
        "morizen-java: jdk=25.",
        "morizen-java: provider=minecraft class=dev.morizen.compat.provider.MorizenGameProvider",
        "morizen-java: loader=0.19.5",
        "morizen-java: mod=morizen_empty@1.0.0",
        "morizen-java: entrypoint=main begin mod=morizen_empty@1.0.0 thread=Java mods",
        "morizen-java: entrypoint=main end mod=morizen_empty@1.0.0 thread=Java mods",
        "morizen-java: entrypoint=client begin mod=morizen_empty@1.0.0 thread=Java mods",
        "morizen-java: entrypoint=client end mod=morizen_empty@1.0.0 thread=Java mods",
        "morizen-java: shutdown end",
    ]
    output = result.stdout + "\n" + result.stderr
    missing = [line for line in required if line not in output]
    if missing:
        raise RuntimeError(f"bridge demo log markers missing: {missing}")

    marker = Path(config["values"]["marker_path"]).read_text(encoding="utf-8")
    if marker != "main\nclient\n":
        raise RuntimeError(f"unexpected entrypoint marker: {marker!r}")
    report = json.loads(Path(config["values"]["report_path"]).read_text(encoding="utf-8"))
    expected_mod = {
        "id": "morizen_empty",
        "version": "1.0.0",
        "status": "works",
    }
    actual = {key: report["mods"][0][key] for key in expected_mod}
    if actual != expected_mod or report["mods"][0].get("unsupported"):
        raise RuntimeError(f"unexpected compatibility report: {report}")
    print("morizen-java: Phase 1 empty Fabric mod passed")


def run_failing_mod(artifacts):
    config = artifacts["configs"]["failing"]
    result = run(
        [
            "cargo",
            "run",
            "-p",
            "morizen-jvm",
            "--example",
            "empty_fabric",
            "--",
            config["path"],
        ],
        env=cargo_environment(config["root"]),
        capture=True,
        check=False,
    )
    if result.returncode == 0:
        raise RuntimeError("failing Fabric mod unexpectedly returned success")
    output = result.stdout + "\n" + result.stderr
    required = [
        "Java exception java.lang.IllegalStateException: Java mod callback failed",
        "Could not execute entrypoint stage 'main'",
        "expected Phase 1 entrypoint failure",
        "dev.morizen.testmod.FailingMod.onInitialize",
    ]
    missing = [line for line in required if line not in output]
    if missing:
        raise RuntimeError(f"translated Java exception markers missing: {missing}")
    report = json.loads(Path(config["values"]["report_path"]).read_text(encoding="utf-8"))
    mod = report["mods"][0]
    if mod["status"] != "loader_failure" or "expected Phase 1 entrypoint failure" not in mod["error"]:
        raise RuntimeError(f"failing mod was not reported clearly: {report}")
    print("morizen-java: Phase 1 exception translation passed")


def pinned_real_mods():
    lock = json.loads((ROOT / "tools" / "benchmark-deps" / "mods.lock.json").read_text())
    downloads = OUT / "real-mods" / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    mods = {}
    for slug, project in lock["projects"].items():
        expected = project["hashes"]["sha1"]
        cached = ROOT / ".local" / "pack-cache" / project["filename"]
        path = cached if cached.is_file() else downloads / project["filename"]
        if path.is_file() and hashlib.sha1(path.read_bytes()).hexdigest() != expected:
            raise RuntimeError(f"hash mismatch for cached real mod {path}")
        if not path.is_file():
            path = runtime.fetch(project["url"], path, expected, "sha1")
        mods[slug] = {"path": Path(path), "metadata": project}
    return mods


def first_gap(output):
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    causes = [line for line in lines if line.lower().startswith("caused by:")]
    if causes:
        return causes[-1][:500]
    priorities = (
        "morizen-java: Java exception",
        "Mod resolution encountered an incompatible mod set",
        "Mixin apply failed",
        "ClassNotFoundException",
        "NoClassDefFoundError",
    )
    for wanted in priorities:
        for line in lines:
            if wanted in line:
                return line[:500]
    return lines[-1][:500] if lines else "no diagnostic output"


def run_registry_mod(artifacts):
    config = artifacts["configs"]["registry"]
    result = run(
        [
            "cargo",
            "run",
            "-p",
            "morizen-jvm",
            "--example",
            "registry_mod",
            "--",
            config["path"],
        ],
        env=cargo_environment(config["root"]),
        capture=True,
    )
    output = result.stdout + "\n" + result.stderr
    required = [
        "morizen-java: registry block morizen_registry:ruby_block",
        "morizen-java: registry block morizen_registry:ruby_lamp",
        "morizen-java: registry item morizen_registry:ruby",
        "morizen-java: registry item morizen_registry:ruby_block (block item)",
        "morizen-java: registry entrypoints complete",
        "morizen-java: registry transaction",
        "morizen-java: content=",
        "morizen-java: ready",
    ]
    missing = [line for line in required if line not in output]
    if missing:
        raise RuntimeError(f"registry demo log markers missing: {missing}")

    marker = Path(config["values"]["marker_path"]).read_text(encoding="utf-8")
    expected_marker = (
        "block morizen_registry:ruby_block\n"
        "block morizen_registry:ruby_lamp\n"
        "item morizen_registry:ruby_block\n"
        "item morizen_registry:ruby\n"
    )
    if marker != expected_marker:
        raise RuntimeError(f"unexpected registry marker: {marker!r}")

    game = Path(config["values"]["game_dir"])
    transaction = json.loads(
        (game / "java-mods" / "registry" / "morizen_registry.json").read_text(encoding="utf-8")
    )
    if transaction["schema"] != 1 or transaction["namespace"] != "morizen_registry":
        raise RuntimeError(f"unexpected registry transaction header: {transaction}")
    blocks = {block["id"]: block for block in transaction["blocks"]}
    items = {item["id"]: item for item in transaction["items"]}
    if set(blocks) != {"morizen_registry:ruby_block", "morizen_registry:ruby_lamp"}:
        raise RuntimeError(f"unexpected blocks: {blocks}")
    if set(items) != {"morizen_registry:ruby"}:
        raise RuntimeError(f"unexpected items: {items}")
    ruby = blocks["morizen_registry:ruby_block"]
    if ruby["template"] != "minecraft:stone" or ruby["light"] != 3 or not ruby["item"]:
        raise RuntimeError(f"ruby_block declaration lost properties: {ruby}")
    lamp = blocks["morizen_registry:ruby_lamp"]
    if lamp["template"] != "minecraft:glowstone" or lamp["light"] != 15:
        raise RuntimeError(f"ruby_lamp declaration lost properties: {lamp}")
    if items["morizen_registry:ruby"]["max_stack_size"] != 16:
        raise RuntimeError(f"ruby item lost its stack size: {items['morizen_registry:ruby']}")

    content = game / "java-mods" / "content" / "morizen_registry"
    manifest = (content / "horizon-mod.toml").read_text(encoding="utf-8")
    if 'id = "morizen_registry"' not in manifest or 'api = "0.2"' not in manifest:
        raise RuntimeError(f"unexpected content manifest: {manifest!r}")
    declaration = json.loads(
        (
            content
            / "data"
            / "morizen_registry"
            / "horizon"
            / "block"
            / "ruby_block.json"
        ).read_text(encoding="utf-8")
    )
    if declaration["template"] != "minecraft:stone" or declaration["item"] is not True:
        raise RuntimeError(f"unexpected block declaration: {declaration}")
    texture = content / "assets" / "morizen_registry" / "textures" / "block" / "ruby_block.png"
    if texture.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise RuntimeError(f"generated texture is not a PNG: {texture}")
    report = json.loads(Path(config["values"]["report_path"]).read_text(encoding="utf-8"))
    if report["mods"][0]["status"] != "works":
        raise RuntimeError(f"registry mod was not reported as working: {report}")
    print("morizen-java: Phase 2 registry transaction and content package passed")


def run_lifecycle_mod(artifacts):
    config = artifacts["configs"]["lifecycle"]
    result = run(
        [
            "cargo",
            "run",
            "-p",
            "morizen-jvm",
            "--example",
            "lifecycle_mod",
            "--",
            config["path"],
        ],
        env=cargo_environment(config["root"]),
        capture=True,
    )
    output = result.stdout + "\n" + result.stderr
    required = [
        "morizen-java: event-batch events=1 actions=0",
        "morizen-java: event-batch events=2 actions=1",
        "morizen-java: event-batch events=5 actions=5",
        "morizen-java: event-batch events=256 actions=0",
        "morizen-java: disabled callbacks for morizen_lifecycle_failing",
        "morizen-java: Phase 3 lifecycle/event batch passed",
        "morizen-java: Phase 4 world snapshot and stale handles passed",
        "morizen-java: Phase 5 network/UI/sound/data call budget passed",
    ]
    missing = [line for line in required if line not in output]
    if missing:
        raise RuntimeError(f"lifecycle demo log markers missing: {missing}")
    marker = Path(config["values"]["marker_path"]).read_text(encoding="utf-8")
    expected = (
        "ok 0 start Java mods\n"
        "fail 0 start\n"
        "ok 1 client_tick Java mods\n"
        "fail 1 client_tick\n"
        "ok 2 server_tick Java mods\n"
        "fail 2 server_tick\n"
        "ok 3 player_join Java mods\n"
        "fail 3 player_join\n"
        "ok 4 block_interact Java mods\n"
        "ok 5 command Java mods\n"
        "ok 6 player_leave Java mods\n"
        "ok 7 world_snapshot Java mods\n"
        "ok 8 network_payload Java mods\n"
        "ok 9 screen Java mods\n"
        "ok 10 sound Java mods\n"
        "ok 11 recipe Java mods\n"
        "ok 12 loot Java mods\n"
        "ok 269 stop Java mods\n"
    )
    if marker != expected:
        raise RuntimeError(f"unexpected lifecycle order/isolation marker: {marker!r}")
    print("morizen-java: Phase 3 lifecycle ordering and isolation passed")


def run_mixin_head_mod(artifacts):
    config = artifacts["configs"]["mixin-head"]
    result = run(
        [
            "cargo",
            "run",
            "-p",
            "morizen-jvm",
            "--example",
            "empty_fabric",
            "--",
            config["path"],
        ],
        env=cargo_environment(config["root"]),
        capture=True,
    )
    output = result.stdout + "\n" + result.stderr
    for marker in [
        "morizen-java: mixin-head head",
        "morizen-java: mixin-head main",
        "morizen-java: shutdown end",
    ]:
        if marker not in output:
            raise RuntimeError(f"transformed-shim marker missing: {marker}")
    written = Path(config["values"]["marker_path"]).read_text(encoding="utf-8")
    if written != "head\nmain\n":
        raise RuntimeError(f"unexpected transformed-shim marker: {written!r}")
    print("morizen-java: Phase 6 transformed-shim HEAD callback passed")


def run_quilt_mod(artifacts):
    config = artifacts["configs"]["quilt"]
    result = run(
        [
            "cargo",
            "run",
            "-p",
            "morizen-jvm",
            "--example",
            "empty_fabric",
            "--",
            config["path"],
        ],
        env=cargo_environment(config["root"]),
        capture=True,
    )
    output = result.stdout + "\n" + result.stderr
    for marker in [
        "morizen-java: quilt-loader=0.30.2",
        "morizen-java: quilt-mod=morizen_quilt@1.0.0",
        "morizen-java: quilt entrypoint=morizen thread=Java mods",
        "morizen-java: quilt bootstrap stop",
    ]:
        if marker not in output:
            raise RuntimeError(f"Quilt provider/entrypoint marker missing: {marker}")
    written = Path(config["values"]["marker_path"]).read_text(encoding="utf-8")
    if written != "quilt\n":
        raise RuntimeError(f"unexpected Quilt marker: {written!r}")
    report = json.loads(Path(config["values"]["report_path"]).read_text(encoding="utf-8"))
    if report["loader"] != {"id": "quilt", "version": "0.30.2"}:
        raise RuntimeError(f"unexpected Quilt compatibility report: {report}")
    print("morizen-java: Phase 7 embedded Quilt provider/entrypoint passed")


def run_real_mods(artifacts):
    real_mods = pinned_real_mods()
    fabric_api = real_mods["fabric-api"]["path"]
    root = OUT / "real-mods" / "runs"
    results = []
    for slug, entry in real_mods.items():
        trial = root / slug
        if trial.exists():
            shutil.rmtree(trial)
        game = trial / "game"
        game.mkdir(parents=True)
        mod_jars = [entry["path"]]
        # Sodium declares three Fabric API module dependencies. The pinned API
        # aggregate supplies them as nested jars, as it does in a normal install.
        if slug == "sodium":
            mod_jars.append(fabric_api)
        project = entry["metadata"]
        config = {
            "jdk_home": str(artifacts["jdk"]),
            "provider_jar": str(artifacts["provider_jar"].resolve()),
            "bootstrap_jar": str(artifacts["bootstrap_jar"].resolve()),
            "shims_jar": str(Path(artifacts["shim_jar"]).resolve()),
            "fabric_libraries": [str(path.resolve()) for path in artifacts["libraries"]],
            "game_libraries": [str(path) for path in artifacts["game_libraries"]],
            "mod_jars": [str(path.resolve()) for path in mod_jars],
            "mods": [{"id": slug, "version": project["version"]}],
            "game_dir": str(game.resolve()),
            "marker_path": str((trial / "entrypoints.txt").resolve()),
            "report_path": str((game / "java-mods" / "compat-report.json").resolve()),
            "minecraft_version": "26.3",
            "loader_version": LOADER_VERSION,
            "loader_id": "fabric",
            "max_heap_mb": 256,
            "startup_timeout_ms": 60_000,
            "shutdown_timeout_ms": 5_000,
            "event_timeout_ms": 2_000,
        }
        config_path = trial / "run-config.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        try:
            completed = run(
                [
                    "cargo",
                    "run",
                    "-p",
                    "morizen-jvm",
                    "--example",
                    "empty_fabric",
                    "--",
                    config_path,
                ],
                env=cargo_environment(trial),
                capture=True,
                check=False,
                show_capture=False,
                timeout=120,
            )
            output = completed.stdout + "\n" + completed.stderr
            if completed.returncode == 0:
                outcome = "partial" if "ClassNotFoundException" in output else "loaded"
            elif "Loading Morizen 26.3" in output:
                outcome = "unsupported"
            else:
                outcome = "loader_failure"
        except subprocess.TimeoutExpired as error:
            output = (error.stdout or "") + "\n" + (error.stderr or "")
            outcome = "timeout"
        gap = first_gap(output)
        (trial / "run.log").write_text(output, encoding="utf-8")
        result = {
            "slug": slug,
            "version": project["version"],
            "outcome": outcome,
            "first_gap": gap,
        }
        results.append(result)
        print(f"{slug}: {outcome}: {gap}")
    result_path = OUT / "real-mods" / "results.json"
    result_path.write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    print(f"real-mod results: {result_path}")
    return results


def default_minecraft_jar():
    explicit = os.environ.get("MORIZEN_MINECRAFT_JAR")
    if explicit:
        return Path(explicit)
    appdata = os.environ.get("APPDATA")
    if appdata:
        candidate = (
            Path(appdata)
            / "horizon-launcher"
            / "linked-minecraft"
            / "versions"
            / "26.3"
            / "26.3.jar"
        )
        if candidate.is_file():
            return candidate
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jdk-home", type=Path)
    parser.add_argument("--minecraft-jar", type=Path)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("prepare", help="compile local provider/bootstrap/test-mod jars")
    subparsers.add_parser("contract", help="build and reflect-check Fabric's provider interface")
    subparsers.add_parser("test", help="run the provider contract, embedded-JVM and registry mods")
    subparsers.add_parser("real-mods", help="try the five hash-pinned 26.3 Fabric artifacts")
    game = subparsers.add_parser(
        "game-config", help="write the config the native game and the launcher read"
    )
    game.add_argument("--mods", type=Path, help="a folder of mod jars, or one jar")
    game.add_argument("--out", type=Path, help="where to write it")
    args = parser.parse_args()

    try:
        minecraft_jar = args.minecraft_jar or default_minecraft_jar()
        if minecraft_jar is None:
            raise RuntimeError(
                "pass --minecraft-jar (or MORIZEN_MINECRAFT_JAR): the user's 26.3 client jar "
                "is needed to generate the signature shims"
            )
        artifacts = build(args.jdk_home, minecraft_jar)
        if args.command in {"contract", "test"}:
            run_contract(artifacts)
        if args.command == "test":
            run_empty_mod(artifacts)
            run_failing_mod(artifacts)
            run_registry_mod(artifacts)
            run_lifecycle_mod(artifacts)
            run_mixin_head_mod(artifacts)
            run_quilt_mod(artifacts)
        if args.command == "real-mods":
            run_contract(artifacts)
            run_real_mods(artifacts)
        if args.command == "prepare":
            print(artifacts["configs"]["empty"]["path"])
        if args.command == "game-config":
            print(write_game_config(artifacts, args.mods, args.out))
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"java_mod_bridge: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

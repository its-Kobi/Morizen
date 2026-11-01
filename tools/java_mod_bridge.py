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


def build(jdk_home=None):
    jdk = discover_jdk_home(jdk_home)
    libraries, loader = pinned_fabric_libraries()
    if BUILD.exists():
        shutil.rmtree(BUILD)
    BUILD.mkdir(parents=True)

    main = JAVA_ROOT / "src" / "main" / "java"
    provider_sources = list((main / "dev" / "morizen" / "compat" / "provider").rglob("*.java"))
    bootstrap_sources = [
        path for path in main.rglob("*.java")
        if "provider" not in path.relative_to(main).parts
    ]
    provider_classes = BUILD / "provider-classes"
    bootstrap_classes = BUILD / "bootstrap-classes"
    compile_java(jdk, provider_sources, provider_classes, libraries)
    copy_resources(JAVA_ROOT / "src" / "main" / "resources", provider_classes)
    compile_java(jdk, bootstrap_sources, bootstrap_classes, libraries)

    contract_classes = BUILD / "contract-classes"
    compile_java(
        jdk,
        (JAVA_ROOT / "src" / "test" / "java").rglob("*.java"),
        contract_classes,
        [provider_classes, *libraries],
    )

    test_mod_jars = {}
    for name in ("empty", "failing"):
        source = JAVA_ROOT / "test-mods" / name / "src" / "main"
        classes = BUILD / f"{name}-mod-classes"
        compile_java(jdk, (source / "java").rglob("*.java"), classes, libraries)
        copy_resources(source / "resources", classes)
        jar = BUILD / f"morizen-{name}-test-mod.jar"
        zip_tree(jar, [classes])
        test_mod_jars[name] = jar

    provider_jar = BUILD / "morizen-provider.jar"
    bootstrap_jar = BUILD / "morizen-bootstrap.jar"
    zip_tree(provider_jar, [provider_classes])
    zip_tree(bootstrap_jar, [bootstrap_classes])

    def write_config(name, mod_id, mod_jar):
        run_root = OUT / "test-runs" / name
        if run_root.exists():
            shutil.rmtree(run_root)
        game = run_root / "game"
        game.mkdir(parents=True)
        config = {
            "jdk_home": str(jdk),
            "provider_jar": str(provider_jar.resolve()),
            "bootstrap_jar": str(bootstrap_jar.resolve()),
            "fabric_libraries": [str(path.resolve()) for path in libraries],
            "game_libraries": [],
            "mod_jars": [str(mod_jar.resolve())],
            "mods": [{
                "id": mod_id,
                "version": "1.0.0",
                "verified_compatible": name == "empty",
            }],
            "game_dir": str(game.resolve()),
            "marker_path": str((run_root / "entrypoints.txt").resolve()),
            "report_path": str((game / "java-mods" / "compat-report.json").resolve()),
            "minecraft_version": "26.3",
            "loader_version": LOADER_VERSION,
            "max_heap_mb": 256,
            "startup_timeout_ms": 60_000,
            "shutdown_timeout_ms": 5_000,
        }
        config_path = run_root / "run-config.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        return {"root": run_root, "values": config, "path": config_path}

    configs = {
        "empty": write_config("empty", "morizen_empty", test_mod_jars["empty"]),
        "failing": write_config("failing", "morizen_failing", test_mod_jars["failing"]),
    }
    return {
        "jdk": jdk,
        "libraries": libraries,
        "loader": loader,
        "provider_jar": provider_jar,
        "bootstrap_jar": bootstrap_jar,
        "provider_classes": provider_classes,
        "contract_classes": contract_classes,
        "configs": configs,
    }


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
            "fabric_libraries": [str(path.resolve()) for path in artifacts["libraries"]],
            "game_libraries": [],
            "mod_jars": [str(path.resolve()) for path in mod_jars],
            "mods": [{"id": slug, "version": project["version"]}],
            "game_dir": str(game.resolve()),
            "marker_path": str((trial / "entrypoints.txt").resolve()),
            "report_path": str((game / "java-mods" / "compat-report.json").resolve()),
            "minecraft_version": "26.3",
            "loader_version": LOADER_VERSION,
            "max_heap_mb": 256,
            "startup_timeout_ms": 60_000,
            "shutdown_timeout_ms": 5_000,
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jdk-home", type=Path)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("prepare", help="compile local provider/bootstrap/test-mod jars")
    subparsers.add_parser("contract", help="build and reflect-check Fabric's provider interface")
    subparsers.add_parser("test", help="run the provider contract and embedded-JVM empty mod")
    subparsers.add_parser("real-mods", help="try the five hash-pinned 26.3 Fabric artifacts")
    args = parser.parse_args()

    try:
        artifacts = build(args.jdk_home)
        if args.command in {"contract", "test"}:
            run_contract(artifacts)
        if args.command == "test":
            run_empty_mod(artifacts)
            run_failing_mod(artifacts)
        if args.command == "real-mods":
            run_contract(artifacts)
            run_real_mods(artifacts)
        if args.command == "prepare":
            print(artifacts["configs"]["empty"]["path"])
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"java_mod_bridge: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

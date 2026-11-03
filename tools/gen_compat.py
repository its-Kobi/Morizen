#!/usr/bin/env python3
"""Generate the signature-only Minecraft shims from the user's own 26.3 jar.

`compat-java/shim-surface.json` lists the classes and members the Java-mod
bridge re-implements. Generation checks each declaration against the class
signatures in the user's client jar (and its SHA-1 against
`research-lock.json`) before writing any source. Method bodies are Morizen's
own code and live in `BODIES` below; nothing here reads Mojang bytecode,
assets or source.

Output stays under `.local/java-mod-bridge/generated-src` (never committed).
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import sys
import zipfile

try:
    from . import java_mod_bridge as bridge
except ImportError:  # pragma: no cover - direct script use
    import java_mod_bridge as bridge


ROOT = Path(__file__).resolve().parents[1]
SURFACE = ROOT / "compat-java" / "shim-surface.json"
LOCK = ROOT / "research-lock.json"
OUT = ROOT / ".local" / "java-mod-bridge"
GENERATED = OUT / "generated-src"
SHIM_JAR = OUT / "build" / "morizen-shims.jar"

ACC_INTERFACE = 0x0200
ACC_FINAL = 0x0010


class SurfaceError(RuntimeError):
    pass


def class_signatures(data):
    """Reads class, field and method signatures; every Code attribute is skipped."""
    if data[:4] != b"\xca\xfe\xba\xbe":
        raise SurfaceError("invalid class file magic")
    offset = 8
    count = int.from_bytes(data[offset:offset + 2], "big")
    offset += 2
    pool = [None] * count
    index = 1
    while index < count:
        tag = data[offset]
        offset += 1
        if tag == 1:
            length = int.from_bytes(data[offset:offset + 2], "big")
            offset += 2
            pool[index] = ("utf8", data[offset:offset + length].decode("utf-8", "replace"))
            offset += length
        elif tag in (3, 4):
            offset += 4
        elif tag in (5, 6):
            offset += 8
            index += 1
        elif tag in (7, 8, 16, 19, 20):
            if tag == 7:
                pool[index] = ("class", int.from_bytes(data[offset:offset + 2], "big"))
            offset += 2
        elif tag in (9, 10, 11, 12, 17, 18):
            offset += 4
        elif tag == 15:
            offset += 3
        else:
            raise SurfaceError(f"unknown constant-pool tag {tag}")
        index += 1

    def utf8(i):
        entry = pool[i]
        if not entry or entry[0] != "utf8":
            raise SurfaceError(f"constant-pool entry {i} is not UTF-8")
        return entry[1]

    def class_name(i):
        entry = pool[i]
        if not entry or entry[0] != "class":
            raise SurfaceError(f"constant-pool entry {i} is not a Class")
        return utf8(entry[1])

    def u(size):
        nonlocal offset
        value = int.from_bytes(data[offset:offset + size], "big")
        offset += size
        return value

    def skip_attributes():
        nonlocal offset
        for _ in range(u(2)):
            u(2)
            length = u(4)
            offset += length

    access = u(2)
    name = class_name(u(2))
    super_index = u(2)
    super_name = class_name(super_index) if super_index else None
    interfaces = [class_name(u(2)) for _ in range(u(2))]
    fields = {}
    for _ in range(u(2)):
        field_access = u(2)
        field_name = utf8(u(2))
        descriptor = utf8(u(2))
        skip_attributes()
        fields[(field_name, descriptor)] = field_access
    methods = {}
    for _ in range(u(2)):
        method_access = u(2)
        method_name = utf8(u(2))
        descriptor = utf8(u(2))
        skip_attributes()
        methods[(method_name, descriptor)] = method_access
    return {
        "access": access,
        "name": name,
        "super": super_name,
        "interfaces": interfaces,
        "fields": fields,
        "methods": methods,
    }


def jar_sha1(jar):
    return hashlib.sha1(Path(jar).read_bytes()).hexdigest()


def verify_jar(jar):
    expected = json.loads(LOCK.read_text(encoding="utf-8"))["minecraft"]["client_sha1"]
    actual = jar_sha1(jar)
    if actual != expected:
        raise SurfaceError(
            f"{jar} is not the pinned 26.3 client jar (sha1 {actual}, expected {expected})"
        )
    return actual


def surface():
    return json.loads(SURFACE.read_text(encoding="utf-8"))


def load_signatures(jar):
    signatures = {}
    with zipfile.ZipFile(jar) as archive:
        for name in archive.namelist():
            if name.endswith(".class"):
                signatures[name[:-6].replace("/", ".")] = class_signatures(archive.read(name))
    return signatures


def check_member(class_name, member, signature):
    key = (member.get("name"), member["descriptor"])
    if member["kind"] == "constructor":
        key = ("<init>", member["descriptor"])
    if member["kind"] == "field":
        found = signature["fields"].get(key)
    else:
        found = signature["methods"].get(key)
    if found is None:
        kind = member["kind"]
        raise SurfaceError(f"{class_name}: {kind} {key[0]}{key[1]} is not in the 26.3 jar")


def validate(manifest, signatures):
    classes = {entry["name"]: entry for entry in manifest["classes"]}
    for entry in manifest["classes"]:
        name = entry["name"]
        signature = signatures.get(name)
        if signature is None:
            raise SurfaceError(f"{name} is not in the 26.3 jar")
        is_interface = bool(signature["access"] & ACC_INTERFACE)
        if (entry["kind"] == "interface") != is_interface:
            raise SurfaceError(f"{name}: the jar and the surface disagree about class kind")
        declared_final = bool(signature["access"] & ACC_FINAL)
        wants_final = "final" in entry.get("access", "").split()
        if wants_final != declared_final:
            raise SurfaceError(f"{name}: the jar and the surface disagree about final")
        outer = entry.get("outer")
        if outer and outer not in classes:
            raise SurfaceError(f"{name}: nested class has no outer {outer} in the surface")
        for member in entry["members"]:
            if member.get("extra"):
                continue
            check_member(name, member, signature)
    return classes


def type_name(descriptor):
    if not descriptor:
        raise SurfaceError("empty descriptor")
    if descriptor.startswith("["):
        return type_name(descriptor[1:]) + "[]"
    primitives = {
        "B": "byte", "C": "char", "D": "double", "F": "float", "I": "int",
        "J": "long", "S": "short", "Z": "boolean", "V": "void",
    }
    if descriptor in primitives:
        return primitives[descriptor]
    if not (descriptor.startswith("L") and descriptor.endswith(";")):
        raise SurfaceError(f"bad descriptor {descriptor!r}")
    return descriptor[1:-1].replace("/", ".").replace("$", ".")


def split_descriptor(descriptor):
    if not descriptor.startswith("("):
        raise SurfaceError(f"bad method descriptor {descriptor!r}")
    end = descriptor.index(")")
    params = []
    index = 1
    while index < end:
        start = index
        while descriptor[index] == "[":
            index += 1
        if descriptor[index] == "L":
            index = descriptor.index(";", index) + 1
        else:
            index += 1
        params.append(descriptor[start:index])
    return params, descriptor[end + 1:]


BODIES = {
    # Identifier: a real value type, not a shim that throws.
    ("net.minecraft.resources.Identifier", "<init>(Ljava/lang/String;Ljava/lang/String;)V"): [
        "if (arg0 == null || arg1 == null) {",
        '    throw new NullPointerException("Identifier namespace and path are required");',
        "}",
        'if (arg0.isEmpty() || arg1.isEmpty() || arg0.indexOf(\':\') >= 0) {',
        '    throw new IllegalArgumentException("invalid identifier " + arg0 + ":" + arg1);',
        "}",
        "this.namespace = arg0;",
        "this.path = arg1;",
    ],
    ("net.minecraft.resources.Identifier", "fromNamespaceAndPath(Ljava/lang/String;Ljava/lang/String;)Lnet/minecraft/resources/Identifier;"): [
        "return new net.minecraft.resources.Identifier(arg0, arg1);",
    ],
    ("net.minecraft.resources.Identifier", "parse(Ljava/lang/String;)Lnet/minecraft/resources/Identifier;"): [
        "int colon = arg0.indexOf(':');",
        'return colon < 0 ? new net.minecraft.resources.Identifier("minecraft", arg0)',
        "    : new net.minecraft.resources.Identifier(arg0.substring(0, colon), arg0.substring(colon + 1));",
    ],
    ("net.minecraft.resources.Identifier", "tryParse(Ljava/lang/String;)Lnet/minecraft/resources/Identifier;"): [
        "try {",
        "    return parse(arg0);",
        "} catch (RuntimeException ignored) {",
        "    return null;",
        "}",
    ],
    ("net.minecraft.resources.Identifier", "getNamespace()Ljava/lang/String;"): ["return namespace;"],
    ("net.minecraft.resources.Identifier", "getPath()Ljava/lang/String;"): ["return path;"],
    ("net.minecraft.resources.Identifier", "withPrefix(Ljava/lang/String;)Lnet/minecraft/resources/Identifier;"): [
        "return new net.minecraft.resources.Identifier(namespace, arg0 + path);",
    ],
    ("net.minecraft.resources.Identifier", "withSuffix(Ljava/lang/String;)Lnet/minecraft/resources/Identifier;"): [
        "return new net.minecraft.resources.Identifier(namespace, path + arg0);",
    ],
    ("net.minecraft.resources.Identifier", "toString()Ljava/lang/String;"): ['return namespace + ":" + path;'],
    ("net.minecraft.resources.Identifier", "equals(Ljava/lang/Object;)Z"): [
        "if (this == arg0) return true;",
        "if (!(arg0 instanceof net.minecraft.resources.Identifier other)) return false;",
        "return namespace.equals(other.namespace) && path.equals(other.path);",
    ],
    ("net.minecraft.resources.Identifier", "hashCode()I"): ["return 31 * namespace.hashCode() + path.hashCode();"],
    ("net.minecraft.resources.Identifier", "compareTo(Lnet/minecraft/resources/Identifier;)I"): [
        "return toString().compareTo(arg0.toString());",
    ],
    # Registry statics delegate to the Morizen registry collector.
    ("net.minecraft.core.Registry", "register(Lnet/minecraft/core/Registry;Ljava/lang/String;Ljava/lang/Object;)Ljava/lang/Object;"): [
        "return (T) dev.morizen.compat.registry.MorizenRegistries.register(",
        "    registry, net.minecraft.resources.Identifier.parse(name), entry);",
    ],
    ("net.minecraft.core.Registry", "register(Lnet/minecraft/core/Registry;Lnet/minecraft/resources/Identifier;Ljava/lang/Object;)Ljava/lang/Object;"): [
        "return (T) dev.morizen.compat.registry.MorizenRegistries.register(registry, id, entry);",
    ],
    # BlockBehaviour.
    ("net.minecraft.world.level.block.state.BlockBehaviour", "<init>(Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;)V"): [
        "this.properties = arg0;",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour", "properties()Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "return properties;",
    ],
    # BlockBehaviour.Properties.
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "<init>()V"): [],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "of()Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "return new net.minecraft.world.level.block.state.BlockBehaviour.Properties();",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "strength(F)Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "this.destroyTime = arg0;",
        "this.explosionResistance = arg0;",
        "return this;",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "strength(FF)Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "this.destroyTime = arg0;",
        "this.explosionResistance = arg1;",
        "return this;",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "lightLevel(Ljava/util/function/ToIntFunction;)Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "this.lightEmission = arg0;",
        "return this;",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "requiresCorrectToolForDrops()Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "this.requiresCorrectToolForDrops = true;",
        "return this;",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "friction(F)Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "this.friction = arg0;",
        "return this;",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "noCollision()Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "this.hasCollision = false;",
        "return this;",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "morizenTemplate(Ljava/lang/String;)Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;"): [
        "this.template = arg0;",
        "return this;",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "morizenStrength()F"): ["return destroyTime;"],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "morizenLight()I"): [
        "if (lightEmission == null) return 0;",
        "try {",
        "    int value = lightEmission.applyAsInt(",
        "        new net.minecraft.world.level.block.state.BlockState(null));",
        "    return Math.max(0, Math.min(15, value));",
        "} catch (RuntimeException ignored) {",
        "    return 0;",
        "}",
    ],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "morizenFriction()F"): ["return friction;"],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "morizenRequiresTool()Z"): ["return requiresCorrectToolForDrops;"],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "morizenHasCollision()Z"): ["return hasCollision;"],
    ("net.minecraft.world.level.block.state.BlockBehaviour$Properties", "morizenTemplateName()Ljava/lang/String;"): ["return template;"],
    # Block.
    ("net.minecraft.world.level.block.Block", "<init>(Lnet/minecraft/world/level/block/state/BlockBehaviour$Properties;)V"): [
        "super(arg0);",
    ],
    ("net.minecraft.world.level.block.Block", "asItem()Lnet/minecraft/world/item/Item;"): [
        "return net.minecraft.world.item.Item.BY_BLOCK.get(this);",
    ],
    ("net.minecraft.world.level.block.Block", "defaultBlockState()Lnet/minecraft/world/level/block/state/BlockState;"): [
        "return new net.minecraft.world.level.block.state.BlockState(this);",
    ],
    # BlockState.
    ("net.minecraft.world.level.block.state.BlockState", "<init>(Lnet/minecraft/world/level/block/Block;)V"): [
        "this.block = arg0;",
    ],
    ("net.minecraft.world.level.block.state.BlockState", "getBlock()Lnet/minecraft/world/level/block/Block;"): ["return block;"],
    # Item.
    ("net.minecraft.world.item.Item", "<init>(Lnet/minecraft/world/item/Item$Properties;)V"): [
        "this.properties = arg0;",
    ],
    ("net.minecraft.world.item.Item", "asItem()Lnet/minecraft/world/item/Item;"): ["return this;"],
    ("net.minecraft.world.item.Item", "morizenProperties()Lnet/minecraft/world/item/Item$Properties;"): [
        "return properties;",
    ],
    # Item.Properties.
    ("net.minecraft.world.item.Item$Properties", "<init>()V"): [],
    ("net.minecraft.world.item.Item$Properties", "stacksTo(I)Lnet/minecraft/world/item/Item$Properties;"): [
        "this.maxStackSize = arg0;",
        "return this;",
    ],
    ("net.minecraft.world.item.Item$Properties", "morizenItemTemplate(Ljava/lang/String;)Lnet/minecraft/world/item/Item$Properties;"): [
        "this.template = arg0;",
        "return this;",
    ],
    ("net.minecraft.world.item.Item$Properties", "morizenMaxStackSize()I"): ["return maxStackSize;"],
    ("net.minecraft.world.item.Item$Properties", "morizenItemTemplateName()Ljava/lang/String;"): ["return template;"],
    # BlockItem.
    ("net.minecraft.world.item.BlockItem", "<init>(Lnet/minecraft/world/level/block/Block;Lnet/minecraft/world/item/Item$Properties;)V"): [
        "super(arg1);",
        "this.block = arg0;",
        "net.minecraft.world.item.Item.BY_BLOCK.put(arg0, this);",
    ],
    ("net.minecraft.world.item.BlockItem", "getBlock()Lnet/minecraft/world/level/block/Block;"): ["return block;"],
}


def body_for(class_name, member):
    if member["kind"] == "constructor":
        key = (class_name, "<init>" + member["descriptor"])
    else:
        key = (class_name, member["name"] + member["descriptor"])
    body = BODIES.get(key)
    if body is None:
        label = f"{class_name.replace('/', '.')}.{key[1]}"
        return [f'throw new dev.morizen.compat.UnsupportedCompatException("{label}");']
    return list(body)


def render_member(class_name, member):
    if member["kind"] == "field":
        declaration = member.get("signature")
        if declaration is None:
            declaration = f'{member["access"]} {type_name(member["descriptor"])} {member["name"]}'
        init = member.get("init")
        if init:
            return f"    {declaration} = {init};"
        return f"    {declaration};"
    if member["kind"] == "constructor":
        params, _ = split_descriptor(member["descriptor"])
        simple = class_name.rsplit(".", 1)[-1].rsplit("$", 1)[-1]
        names = [f"arg{i}" for i in range(len(params))]
        typed = ", ".join(f"{type_name(p)} {n}" for p, n in zip(params, names))
        body = body_for(class_name, member)
        return render_block(f'    {member["access"]} {simple}({typed})', body)
    declaration = member.get("signature")
    if declaration is None:
        params, result = split_descriptor(member["descriptor"])
        names = [f"arg{i}" for i in range(len(params))]
        typed = ", ".join(f"{type_name(p)} {n}" for p, n in zip(params, names))
        declaration = f'{member["access"]} {type_name(result)} {member["name"]}({typed})'
    if member.get("abstract") or "abstract" in member.get("access", ""):
        return f"    {declaration};"
    body = body_for(class_name, member)
    return render_block(f"    {declaration}", body)


def render_block(header, body):
    lines = [header + " {"]
    lines.extend(f"        {line}" for line in body)
    lines.append("    }")
    return "\n".join(lines)


def render_class(node, classes):
    simple = node["name"].rsplit(".", 1)[-1].rsplit("$", 1)[-1]
    params = node.get("typeParameters") or []
    header = f'{node["access"]} {node["kind"]} {simple}'
    if params:
        header += "<" + ", ".join(params) + ">"
    if node.get("extends"):
        extends = node["extends"]
        if isinstance(extends, str):
            extends = [extends]
        header += " extends " + ", ".join(extends)
    if node.get("implements"):
        header += " implements " + ", ".join(node["implements"])
    body = [render_member(node["name"], member) for member in node["members"]]
    for child in classes.values():
        if child.get("outer") == node["name"]:
            body.append(render_class(child, classes))
    return header + " {\n" + "\n\n".join(part for part in body if part) + "\n}"


def generate(manifest, signatures, destination):
    classes = validate(manifest, signatures)
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    written = []
    for node in sorted(classes.values(), key=lambda entry: entry["name"]):
        if node.get("outer"):
            continue
        package, _, simple = node["name"].rpartition(".")
        path = destination / (package.replace(".", "/") + f"/{simple}.java")
        path.parent.mkdir(parents=True, exist_ok=True)
        source = (
            f"// Generated by tools/gen_compat.py from compat-java/shim-surface.json.\n"
            f"// Signature-only shim for Minecraft 26.3; do not commit.\n"
            f"package {package};\n\n"
            + render_class(node, classes)
            + "\n"
        )
        path.write_text(source, encoding="utf-8")
        written.append(path)
    return written


def build(jar=None, jdk_home=None, destination=None, extra_sources=(), classpath=()):
    jar = Path(jar or os.environ.get("MORIZEN_MINECRAFT_JAR", "")).resolve()
    if not jar.is_file():
        raise SurfaceError("pass the user's 26.3 client jar (--jar or MORIZEN_MINECRAFT_JAR)")
    expected = verify_jar(jar)
    manifest = surface()
    if manifest.get("minecraft") != "26.3":
        raise SurfaceError("the shim surface must target 26.3")
    signatures = load_signatures(jar)
    destination = Path(destination or GENERATED)
    sources = generate(manifest, signatures, destination)
    jdk = bridge.discover_jdk_home(jdk_home)
    classes = OUT / "build" / "shim-classes"
    if classes.exists():
        shutil.rmtree(classes)
    classes.mkdir(parents=True)
    # The registry classes use Minecraft types, so they are compiled with the
    # shims in one javac run; the rest of the bootstrap then compiles against
    # the result.
    bridge.compile_java(jdk, [*sources, *extra_sources], classes, list(classpath))
    SHIM_JAR.parent.mkdir(parents=True, exist_ok=True)
    bridge.zip_tree(SHIM_JAR, [classes])
    return {
        "jar": jar,
        "sha1": expected,
        "sources": sources,
        "classes": classes,
        "shim_jar": SHIM_JAR,
    }


def check(jar=None):
    jar = Path(jar or os.environ.get("MORIZEN_MINECRAFT_JAR", "")).resolve()
    expected = verify_jar(jar)
    manifest = surface()
    signatures = load_signatures(jar)
    classes = validate(manifest, signatures)
    return {"jar": jar, "sha1": expected, "classes": len(classes)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", help="the user's 26.3 client jar")
    parser.add_argument("--jdk-home")
    parser.add_argument("--out", help="generated source directory")
    parser.add_argument("--check", action="store_true", help="validate the surface only")
    arguments = parser.parse_args(argv)
    try:
        if arguments.check:
            result = check(arguments.jar)
            print(
                f"morizen-java: shim surface matches {result['classes']} classes "
                f"of 26.3 (sha1 {result['sha1']})"
            )
        else:
            result = build(arguments.jar, arguments.jdk_home, arguments.out)
            print(
                f"morizen-java: generated {len(result['sources'])} shim sources "
                f"and {result['shim_jar']}"
            )
    except (SurfaceError, RuntimeError) as error:
        print(f"gen_compat: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

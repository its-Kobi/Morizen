# Java mod bridge: embedded Fabric in native Morizen

Status: **vertical phase gates 1-7 implemented and demonstrated locally** (2026-11-02). Real Fabric
Loader 0.19.5 and Quilt Loader 0.30.2 run in the embedded JVM. Generated 26.3 shims support the
tracked block/item surface, lifecycle batches, generation handles, immutable snapshots and one
transformed-shim Mixin pattern. This is measured partial compatibility, not a claim that arbitrary
mods or Mixins work. NeoForge has no 26.3 release; Forge 26.3 needs the separate launch-service
design recorded below and is still reported unsupported. The implementation remains only on
`feature/java-mod-bridge` and off unless Cargo feature `java-mods` is selected.

## Goal and hard boundary

The bridge hosts one Java 25 VM inside the Morizen process, runs the real Fabric Loader with a
Morizen game provider, and presents JVM classes with the names and descriptors that 26.3 Fabric
mods expect. Those classes delegate supported operations to Rust. The real Minecraft Java client
is never started or put on the runtime class path.

This is not the launcher's existing Java-instance support. Java instances run Mojang's game in a
separate process. The bridge runs mod code beside the native Rust engine.

The following are non-negotiable:

- no Mojang class implementation, asset, jar, decompiled source or generated shim is committed;
- shim generation reads only the user's own 26.3 class signatures and writes under `.local/`;
- all Rust integration is behind `java-mods`, which is off by default;
- with `java-mods` off there is no JVM lookup, thread, branch, JNI dependency in the linked game,
  mod scan or change in behaviour;
- an unsupported bridge member throws `UnsupportedCompatException` and is reported once;
- Fabric, Mixin and mods are untrusted native-process code. The bridge can isolate ordinary Java
  exceptions, but cannot honestly promise process isolation (see [Security limits](#security-limits)).

## Phase 0 evidence

### Minecraft 26.3 uses Mojang's unobfuscated names

**Answer:** 26.3 Fabric mods are built against the readable `net.minecraft.*` classes and members
shipped by Mojang, not `class_1234` intermediary names. Fabric calls this unobfuscated namespace
`official` (not `named`), so the bridge runtime and distribution namespaces must be `official` and
no Tiny/Yarn/intermediary remap belongs in the 26.3 path.

Evidence:

1. [`version-ports.md`](version-ports.md#prepare) records that releases since 26.1 ship
   unobfuscated. The preparation pipeline stops if a candidate contains `client_mappings` or
   `server_mappings`; 26.3 passed that gate.
2. Fabric's 26.2 loader documentation says mappings are relevant only for obfuscated pre-26.1
   games. Fabric Loader does not remap mods to an arbitrary runtime namespace: released mods must
   already match it.
3. The repository's 26.3 benchmark mod compiles directly against the user's client jar and imports
   `net.minecraft.client.Minecraft`; its Mixin injects into named method `runTick`.
4. A signature-only scan of the five hash-pinned, released 26.3 Fabric artifacts in
   `tools/benchmark-deps/mods.lock.json` found the following unique class references:

   | Artifact | Readable `net/minecraft/...` | Intermediary `class_N` |
   |---|---:|---:|
   | Fabric API 0.161.0+26.3 (44 nested modules) | 1,125 | 0 |
   | Sodium 0.9.2 | 303 | 0 |
   | Lithium 0.26.2 | 406 | 0 |
   | ImmediatelyFast 1.17.1 | 70 | 0 |
   | FerriteCore 9.0.0 | 28 | 0 |

The official 26.3 client jar is not installed on this workstation, so Phase 0 did not perform a new
direct jar inventory here. The evidence above comes from the repository's already validated version
pipeline, its compile-and-run benchmark path, Fabric's documentation, and released mod class
signatures. `tools/gen_compat` must still refuse generation unless the local jar matches
`research-lock.json`'s 26.3 SHA-1 and has no mapping downloads.

### What Fabric mods expect

For 26.3, a production mod jar's own classes use Mojang's readable names in Fabric's `official`
namespace. `fabric.mod.json` still
declares loader and Minecraft version constraints, entrypoint keys, Mixin configurations, nested
jars and an optional class tweaker/access widener. Fabric Loader is responsible for discovery,
dependency solving, nested jars, side selection, language adapters and creating entrypoints.
Morizen must not reimplement those pieces.

The bridge advertises two built-in mods to Fabric:

- `minecraft` version `26.3`, so ordinary `depends.minecraft` constraints resolve;
- `morizen` at the native client version, so bridge-aware mods can detect the host.

`MorizenGameProvider.getGameId()` returns `minecraft` for loader/mapping compatibility and
`getGameName()` returns `Morizen`. Its runtime and default distribution namespaces are `official`.
Phase 1 confirmed the label directly: Fabric API, Sodium, Lithium and ImmediatelyFast class
tweakers all declare `official` and reject a provider that incorrectly advertises `named`.

### Fabric Loader contract (pinned 0.19.5)

`tools/benchmark-deps/fabric.lock.json` pins Fabric Loader 0.19.5 and all of its runtime libraries by
SHA-256. Phase 0 downloaded those public artifacts to ignored `.local/runtime/fabric/` and inspected
class signatures with JDK 25 `javap`; no Fabric source was read or copied.

The current `net.fabricmc.loader.impl.game.GameProvider` is an internal, version-specific interface.
It is discovered through Java `ServiceLoader` from
`META-INF/services/net.fabricmc.loader.impl.game.GameProvider`. In 0.19.5 its required surface is:

- identity/version: `getGameId`, `getGameName`, `getRawGameVersion`,
  `getNormalizedGameVersion`, `getBuiltinMods`;
- launch shape: `getEntrypoint`, `getLaunchDirectory`, `requiresUrlClassLoader`, `isEnabled`;
- discovery/setup: `locateGame(FabricLauncher, String[])`, `initialize(FabricLauncher)`,
  `getEntrypointTransformer`, `unlockClassPath(FabricLauncher)`;
- execution/arguments: `launch(ClassLoader)`, `getArguments`, `getLaunchArguments(boolean)`;
- transforms/namespaces: `getBuiltinTransforms(String)` plus defaults for runtime namespace,
  distribution namespace, error GUI and AWT support.

This differs from old examples (there is no `isObfuscated` method in 0.19.5). The Java provider is
compiled against exactly the pinned loader and an integration test reflects over the interface so a
loader bump cannot silently change the contract.

Provider bootstrap:

1. `morizen-jvm` starts `KnotClient` through the provider's exception-formatting wrapper on the
   dedicated JVM/mod thread.
2. The provider service is on the parent class path and `fabric.skipMcProvider=true`; Mojang's
   client jar is absent. The test asserts the selected provider class before loading a mod.
3. `locateGame` validates the game directory and local bootstrap path; Rust validates every mod and
   library path before starting the VM.
4. `getBuiltinMods` uses Fabric's `BuiltinModMetadata.Builder` for `minecraft` and `morizen`.
5. `unlockClassPath` adds the bootstrap jar to Knot's target class loader. Phase 2 adds the locally
   generated shim jar there too; neither belongs on the parent class path.
6. `launch(ClassLoader)` loads `dev.morizen.compat.Bootstrap` through the supplied Knot loader.
7. Bootstrap establishes the target context class loader, initializes loader entrypoints and hands
   the target loader back to Rust as a JNI global reference.

The old public Fabric integration guide calls `Hooks.startClient` to prepare and invoke `main` and
`client` entrypoints. That class is internal. `compat-java` isolates this call in one adapter covered
by the loader-contract test; mod-facing code uses only Fabric's public `FabricLoader` and initializer
interfaces. A loader upgrade is its own small phase.

### Mixin and class tweaker behaviour

Fabric 0.19.5's Knot loader owns the transforming class loader. Signature inspection shows that it
loads raw class bytes, applies pre-Mixin transforms/class tweakers, then delegates to Mixin's
`IMixinTransformer` before defining a class. Mixin's documentation confirms that target bytecode
must be transformed before the JVM defines the target class.

Therefore every targetable shim class must be loaded by Knot, not the JVM parent loader. Mixin can
then patch that JVM class normally. It never patches Rust machine code.

The shim method itself must contain ordinary bytecode that calls a separate native bridge method:

```java
// Generated locally from the user's class signature.
public BlockState defaultBlockState() {
    return NativeBridge.blockDefaultState(this.morizen$handle);
}
```

It must **not** merely declare the target method `native`: boundary injections need a Code attribute
and instructions to transform.

Compatibility by injection kind:

| Mixin use | Expected bridge support |
|---|---|
| `HEAD`, normal return/tail callbacks | Feasible when the exact target method exists in the shim |
| accessors, invokers, `@Shadow`, interface injection, access wideners | Feasible once fields/methods and hierarchy are represented |
| full overwrite | Feasible only when all APIs reached by the replacement body are supported |
| redirect/modify an invocation or field | Not generic: the delegate wrapper does not contain vanilla's instruction sequence |
| local capture, `ModifyVariable`, `ModifyConstant`, slices and ordinal-sensitive injections | Not generic: shim locals/constants/ordinals differ from vanilla |
| constructor injection | Partial: generated constructors cannot reproduce vanilla constructor bytecode |

This is the largest compatibility limit. Shipping signatures cannot reproduce vanilla method
instruction patterns without shipping Mojang code. Phase 6 must translate high-value
instruction-sensitive Mixins to explicit bridge hooks one module/mod at a time and report the rest;
it must never claim arbitrary Mixins are supported.

## Repository layout and feature fence

Planned tracked layout:

```text
crates/morizen-jvm/       JVM discovery, invocation, JNI adapters, mod thread
crates/morizen-compat/    pure Rust handles, batched requests/actions, report and ledger types
compat-java/              committed provider/bootstrap/native declarations and test-mod sources
  src/main/java/dev/morizen/compat/
  src/main/resources/META-INF/services/...
  test-mods/empty/
tools/gen_compat/         local signature reader and shim emitter
tools/compat_ledger.py    status/report tool
docs/java-mod-bridge.md   this design and phase results
```

Local/generated layout (all ignored and rejected by public-tree checks if accidentally staged):

```text
.local/java-mod-bridge/
  deps/                   hash-pinned Fabric/Mixin jars
  signatures/26.3.json    class/member signatures from the user's jar
  generated-src/          generated net.minecraft.* Java stubs
  generated-classes/
  morizen-shims.jar
  test-runs/<name>/
```

`horizon-client` gets optional dependencies on `morizen-jvm` and `morizen-compat` and one feature:

```toml
java-mods = ["dep:morizen-jvm", "dep:morizen-compat"]
```

Every engine field, module, startup call and hook is inside `#[cfg(feature = "java-mods")]`. The
feature is absent from `default`, release and server feature sets until a maintainer explicitly
changes that. Acceptance requires comparing a clean feature-off client build with the base commit:
the executable hash and parity selection must be unchanged. No no-op runtime branch is acceptable.

`morizen-compat` has no dependency on `horizon-client`, preventing a cycle. It owns neutral DTOs and
numeric handles. The client maps those batches to engine resources at narrow feature-gated seams.
`morizen-jvm` depends on `morizen-compat` and `jni`, and converts JNI values to those DTOs.

## Local shim generation

`tools/gen_compat` takes the user's exact 26.3 client jar and:

1. checks the jar SHA-1 against `research-lock.json`;
2. rejects a manifest with mapping downloads (the 26.3 path is unobfuscated `official`);
3. reads class access, hierarchy, interfaces, field names/descriptors/access and method
   names/descriptors/access/exceptions;
4. records nest/inner-class, record, enum and permitted-subclass shape needed by the JVM verifier;
5. does **not** read or emit method Code attributes, assets or implementation constants;
6. emits exact skeletons under `.local/java-mod-bridge/generated-src`;
7. gives a member a delegating body only when the tracked compatibility binding table supports it;
   every other callable body throws
   `UnsupportedCompatException("net.minecraft.Class.method(descriptor)")`;
8. compiles the generated source with the user's JDK 25 and packages `morizen-shims.jar` locally.

The existing `tools/class_index.py` is evidence that the repository can parse signatures, but the
generator gets a separate signatures-only reader: it must not hash, retain or inspect method code.
Generated source, signature inventories and jars never enter Git. Public CI cannot build shims and
must run only feature-off checks; the trusted local compatibility runner generates them from its own
copy before feature-on tests.

This has a distribution consequence: a public Morizen release cannot contain generated Mojang shim
classes under this policy. During these phases `java-mods` is a local, source-built experimental
feature. Shipping it needs a separate legal/distribution decision; implementation work must not
quietly change that boundary.

## Embedded JVM design

### Crate and JDK

Use `jni = 0.22.4` with feature `invocation`. It supports the workspace's Rust 1.85 floor and
dynamically loads `jvm.dll`, `libjvm.so` or `libjvm.dylib`. Phase 1 accepts an explicit JDK home,
then `MORIZEN_JAVA_HOME`, then `JAVA_HOME`; it requires major 25 and reports the detected `release`
file. It does not bundle or download Java.

Use `JavaVM::with_libjvm` with the platform's explicit server-VM path. On Windows, the JDK `bin`
directory must also be on the DLL search path before loading `jvm.dll`. VM arguments include a
bounded heap, the provider/Fabric parent class path, UTF-8, headless AWT and bridge paths as system
properties. Unknown options are errors.

### One VM, one owner thread

The JNI Invocation API supports only one VM per process. Morizen creates it once on a dedicated OS
thread named `Java mods`; this is the logical Java game/mod thread for the life of the process.
`JNIEnv` never crosses threads.

```text
Morizen game/server threads             Java mods thread
---------------------------             ----------------
snapshot/event batch  ───────────────►  channel → JNI dispatcher → Fabric callbacks
engine stays unborrowed                 Java/native calls append compat actions
apply action batch      ◄──────────────  one reply after the callback batch
```

The game does not call into Java while holding world/chunk/resource locks. Each tick or lifecycle
point sends an immutable batch and later applies a bounded result batch. Registration is a distinct
startup transaction that commits only if every id and property validates.

Mods may create their own Java threads. A native method invoked there receives that call's `JNIEnv`,
but may only enqueue thread-safe requests; it cannot borrow `Game`. Rust-originated calls use cached
global class/object references and cached method/field IDs. Attached native threads have the
bootstrap context loader by default, so class lookup never relies on `FindClass` from an arbitrary
Rust thread: Bootstrap supplies Knot's target loader as a global reference.

### Exceptions and shutdown

After each Rust-to-Java call, `morizen-jvm` checks for a pending exception, extracts class, message
and a bounded stack through the Java bootstrap helper, clears it and returns a Rust `JavaError`.
Rust native entrypoints use `catch_unwind`; no panic crosses JNI. A failure becomes a Java bridge
exception associated with the current mod/event.

Shutdown order:

1. stop accepting event batches;
2. dispatch Fabric/Morizen stop callbacks with a deadline;
3. ask bridge-owned executors to stop and join the Java mod thread;
4. drop every JNI local/global reference and attachment guard;
5. call `DestroyJavaVM` only when no Java non-daemon thread remains.

`DestroyJavaVM` waits for non-daemon threads and the JNI spec permits only one VM. A mod that leaves
a non-daemon thread alive can otherwise hang shutdown. The first implementation reports that mod and
lets process exit reclaim the VM after a short deadline rather than blocking Morizen forever. It
never attempts to destroy and recreate a VM in one process.

## JNI bridge: handles and batches

Java objects that represent Rust-owned game objects carry opaque nonzero `long` handles. A handle is
`(generation, slot)`; stale/world-after-close handles fail cleanly. Java never receives a Rust
pointer. Global registries use stable numeric ids plus resource names.

JNI calls are coarse:

- registry transaction: arrays of block/item descriptors and one result array;
- tick/lifecycle batch: compact primitive arrays or direct read-only buffers;
- world reads: batched positions/ids, never one JNI call per block;
- actions: one bounded command buffer returned after an event batch;
- asset handoff: paths and metadata once during resource loading, not bytes per frame.

Method/class IDs and global references are cached after bootstrap. Local frames bound temporary JNI
references. Strings are converted at batch boundaries only. No Java callback is made per entity,
block, draw call or mesh vertex.

## Compatibility API and engine seams

The Java surface has three layers:

1. exact generated `net.minecraft.*` signatures;
2. committed clean-room Fabric API compatibility classes, one module at a time;
3. `dev.morizen.compat.NativeBridge`, the small registered-native ABI.

The Rust surface in `morizen-compat` begins with:

- resource ids, registry transactions, block/item property DTOs;
- asset/data roots contributed by each mod;
- immutable lifecycle/event snapshots and bounded actions;
- handle tables and unsupported-call reporting.

Initial engine seams reuse existing code instead of creating parallel registries:

- registry output is converted to the content declarations already consumed by
  `horizon-client::mod_content` before `load_resources` builds `BlockFacts`, `Items`, creative tabs,
  recipes and pack models;
- a mod's `assets/` and `data/` are mounted through the existing content/resource root overlay;
- later event calls attach to the stable, empty native hook points in `game/native_hooks.rs`, each
  still compiled out completely without `java-mods`.

Phase 2 must start the registration transaction before `game/startup_menu.rs::spawn_loader` calls
`mods::content_root`; once `Resources` is built, registry mutation is refused until a full resource
reload creates a new generation.

## Unsupported calls and compatibility report

Every generated unsupported body calls a single Java helper which:

1. identifies the active mod/entrypoint when known;
2. records `(mod id, class, member descriptor, phase)` in `morizen-compat`;
3. increments a count but logs the same key only once;
4. throws `UnsupportedCompatException` with the exact member.

The bridge writes `java-mods/compat-report.json` atomically in the game directory:

```json
{
  "schema": 1,
  "minecraft": "26.3",
  "loader": { "id": "fabric", "version": "0.19.5" },
  "mods": [{
    "id": "example", "version": "1.0.0", "status": "partial",
    "unsupported": [{
      "member": "net.minecraft.world.level.Level.getBlockState(BlockPos)",
      "phase": "main", "calls": 4
    }]
  }]
}
```

An exception during one initializer or callback disables that mod's bridge callbacks and leaves
Morizen running. A required Mixin configuration that cannot apply can abort the Java loader before
an individual mod is attributable; that disables the whole Java-mod session, reports the Mixin
error and starts/continues native Morizen without Java mods.

## API-area priority from the 26.3 corpus

Phase 0 scanned class-name references and metadata only (not source or method bodies) in the five
hash-pinned benchmark artifacts. This is a performance-oriented corpus, not a claim about all mods,
but it gives a reproducible first priority signal:

| Rank | Area | Artifacts referencing it (of 5) | First phase | Notes |
|---:|---|---:|---:|---|
| 1 | blocks/world and item/components | 5 | 2/4 | Also the base for ordinary content mods |
| 2 | registries/resources/ids | 4 | 2 | Needed before Fabric registration can commit |
| 3 | lifecycle/events | 4 | 3 | Loader init plus ticks, joins and interaction |
| 4 | networking | 4 | 5 | Many client mods reference vanilla packet classes even without custom payloads |
| 5 | entities | 3 | 4 | Lithium alone references 111 entity classes |
| 6 | rendering and screens/GUI | 3 | 5/6 | Sodium has 211 rendering and 63 GUI class references |
| 7 | NBT/data | 3 | 4 | Item stacks, entities and persistent data |
| 8 | commands | 3 | 3 | Usually a small surface once Brigadier wrappers exist |
| 9 | sounds | 2 | 5 | Lower frequency in this corpus |

Mixin/class-tweaker pressure is high: Fabric API has 59 Mixin configs and 33 access wideners;
Sodium 12/8, FerriteCore 5/0, ImmediatelyFast 2/1 and Lithium 2/1. This is why Phase 1 proves only
loader/entrypoints and Phase 2 deliberately uses a purpose-built no-Mixin registry mod. The first
real-mod trials after each phase use the pinned five plus 1-2 content mods relevant to that phase;
failures are results, never silently excluded.

## Compatibility ledger

`tools/compat_ledger.py` is added in Phase 1 with a tracked area manifest and test-mod evidence.
Statuses are:

- **supported**: every committed test mod for the area passes and no member in its declared surface
  throws unsupported;
- **partial**: the manifest lists supported and unsupported members explicitly;
- **unsupported**: no stable bridge contract yet.

The percentage is supported member weight divided by all inventoried member weight in the pinned
test/real-mod corpus. Partial areas contribute only their supported members, never an arbitrary
half-credit. Adding a newly observed API expands the denominator, so the score may fall honestly.
The ledger prints the evidence test and phase for every supported area.

## Phase gates and demos

### Phase 1 — JVM, real Fabric Loader, empty entrypoint

Deliver both Rust crates, the committed provider/bootstrap, hash-pinned local dependency preparation,
JDK 25 boot/shutdown, real Fabric discovery and `main`/`client` entrypoints, exception conversion and
an empty report. `compat-java/test-mods/empty` writes a deterministic marker. Its headless log must
name the JDK, selected provider, Loader 0.19.5, mod id/version, entrypoint begin/end and shutdown. No
`net.minecraft` shim is needed yet.

#### Phase 1 result

Phase 1 now provides:

- `morizen-jvm`: JDK discovery (explicit path, `MORIZEN_JAVA_HOME`, `JAVA_HOME`, then `java.home`),
  strict Java 25 validation, platform `libjvm` lookup, bounded heap/options, one owner thread named
  `Java mods`, JNI exception conversion with a bounded nested cause chain, a retained global target
  class-loader reference and bounded shutdown;
- `morizen-compat`: engine-neutral report types, unsupported-call deduplication/counting and atomic
  JSON report replacement on Unix and Windows;
- the real hash-pinned Fabric Loader 0.19.5, Mixin 0.8.7 and ASM libraries, with
  `MorizenGameProvider` selected through `ServiceLoader`, `minecraft@26.3` and `morizen@0.1.0`
  built-ins, `official` runtime/distribution namespaces and Bootstrap loaded by Knot rather than the
  parent loader;
- `compat-java/test-mods/empty`, whose real Fabric `main` and `client` entrypoints append
  `main\nclient\n` to a marker; and `test-mods/failing`, which proves a nested initializer exception
  reaches Rust and `compat-report.json` without crashing the process;
- `tools/compat_ledger.py`, currently **9.6% (8/83 weighted members)**. Loader discovery,
  dependency resolution and the two client entrypoints have evidence; the loader area remains
  partial because server entrypoints, nested-jar evidence, Mixins and access wideners are not yet
  supported by bridge shims;
- a minimal `horizon-client` hook controlled by `MORIZEN_JAVA_MOD_CONFIG`, compiled only with Cargo
  feature `java-mods`. A loader/mod failure logs and disables Java mods rather than stopping the
  native game.

No Mojang jar or real Minecraft process was used. Java classes, test-mod jars and reports were built
only under ignored `.local/java-mod-bridge/`.

The successful empty-mod log contained:

```text
morizen-java: jdk=25.0.5.0.2
FabricLoader/GameProvider: Loading Morizen 26.3 with Fabric Loader 0.19.5
morizen-java: provider=minecraft class=dev.morizen.compat.provider.MorizenGameProvider
morizen-java: mod=morizen_empty@1.0.0
morizen-java: entrypoint=main begin ... thread=Java mods
morizen-java: entrypoint=main end ... thread=Java mods
morizen-java: entrypoint=client begin ... thread=Java mods
morizen-java: entrypoint=client end ... thread=Java mods
morizen-java: shutdown end
```

#### Phase 1 real-mod trials

`py -3 tools/java_mod_bridge.py real-mods` ran all five hash-pinned 26.3 artifacts from
`tools/benchmark-deps/mods.lock.json`, each in a fresh process/home and without a Minecraft jar.
These are diagnostic results, not compatibility claims:

| Artifact | Result | First observed gap |
|---|---|---|
| Sodium 0.9.2 (+ pinned Fabric API) | unsupported | Loader/Mixin initialization reaches missing game runtime dependencies (`org.slf4j.LoggerFactory`); many `net.minecraft.*` targets also have no Phase 2 shim |
| Lithium 0.26.2 | unsupported | required Mixin target `HoglinSpecificSensor` is absent |
| FerriteCore 9.0.0 | partial load only | process reaches Bootstrap because it has no entrypoint, but its `net.minecraft.*` Mixin targets are absent and its plugin also lacks Log4j; none of its functionality ran |
| Fabric API 0.161.0+26.3 | unsupported | missing Guava (`MapMaker`) and then absent game classes/shims |
| ImmediatelyFast 1.17.1 | unsupported | missing `net.minecraft.server.packs.resources.PreparableReloadListener` |

The machine has no user-installed 26.3 client/runtime library set, so Phase 1 could not add the
official third-party game libraries (Guava, SLF4J, Log4j) to these trials. `LaunchConfig` accepts
verified `game_libraries` separately and never accepts the Minecraft client jar. The trials did
confirm that released class tweakers call the readable namespace `official`; the initial Phase 0
`named` label was corrected above.

#### Phase 1 commands actually run

| Command | Result |
|---|---|
| `cargo check -p morizen-compat -p morizen-jvm --all-targets` | passed |
| `cargo test -p morizen-compat -p morizen-jvm --lib --bins --tests --examples` | 4 passed |
| `cargo test -p morizen-compat -p morizen-jvm --doc` | passed (no doctests) |
| `py -3 -m unittest tests.test_java_mod_bridge tests.test_compat_ledger tests.test_lab.PublicationTests` | 8 passed |
| `py -3 -m unittest discover -s tests` | 429 run, 12 skipped; bridge tests passed; one existing root-`README.md` parity-heading failure and one unrelated Python 3.14/Windows cp1252 error in `test_port_publish` |
| `py -3 tools/compat_ledger.py --check` | passed, 8/83 weighted members |
| `py -3 tools/java_mod_bridge.py test` | passed provider reflection contract, empty `main`/`client` demo, report, shutdown and expected failing-mod exception translation |
| `py -3 tools/java_mod_bridge.py real-mods` | completed five isolated trials; results above |
| `cargo check -p horizon-client --no-default-features` | blocked in existing dependency `ring`: bundled `gcc.exe` has no `cc1` on this Windows GNU toolchain, before `horizon-client` was checked |
| `cargo check -p horizon-client --no-default-features --features java-mods` | same `ring`/`cc1` environment blocker before client code |
| `cargo clippy -p morizen-compat -p morizen-jvm --all-targets -- -D warnings` | not run: this installed Rust toolchain has no Clippy component |
| `py -3 tools/parity.py --changed-files <phase-1-files> --list` | 361 harnesses selected (root workspace manifest changed); not run on this blocked client toolchain |
| `py -3 tools/pr_ready.py --quick --base 89f447c07` | timed out after 20 minutes in content-root tests; before timeout public tree and rustfmt passed, the existing root-README test failed, Clippy was unavailable, and workspace tests/doctests hit the same `ring`/`cc1` blocker |

To reproduce the headless demo by hand, run `py -3 tools/java_mod_bridge.py prepare`; it prints the
fresh local config path. Then run
`cargo run -p morizen-jvm --example empty_fabric -- <that-run-config.json>`. The one-command gate is
`py -3 tools/java_mod_bridge.py test`. Starting the full native client additionally requires a
working platform C toolchain and setting `MORIZEN_JAVA_MOD_CONFIG` to that config while building with
`--features java-mods`. There is deliberately no launcher UI before Phase 2.

### Phase 2 result — generated shims and native registries

`compat-java/shim-surface.json` tracks 12 classes and their exact descriptors. `tools/gen_compat.py`
checks every non-extension member against the user's SHA-1-pinned 26.3 jar while skipping all Code
attributes, emits sources only under `.local/`, and compiles `morizen-shims.jar`. The local check
matched all 12 classes against client SHA-1 `e877b6a07acd633fb3bb475002175cec036e7b87`.

The real Fabric test mod `morizen_registry` uses `Registry.register`, `BuiltInRegistries`, `Block`,
`BlockItem`, `Item` and their Properties classes. It registered two blocks and two item forms. Java
wrote one atomic namespace transaction after entrypoints; Rust validated it and generated a normal
content mod containing declarations, blockstates, models, deterministic PNG textures and
translations. The ordinary content pipeline assigns block/item/network/command IDs and, only in a
`java-mods` build, adds the generated entries to the category and search creative tabs. Foreign
namespaces, duplicates, invalid templates, non-finite properties and transactions over 4096 entries
are refused before any content is applied.

The test output ended with:

```text
morizen-java: registry block morizen_registry:ruby_block
morizen-java: registry item morizen_registry:ruby_block (block item)
morizen-java: registry item morizen_registry:ruby
morizen-java: registry block morizen_registry:ruby_lamp
morizen-java: registry transaction .../morizen_registry.json
morizen-java: content=.../morizen_registry id=morizen_registry blocks=2 items=1
morizen-java: Phase 2 registry transaction and content package passed
```

Prepared 26.3 resources now exist locally, including `VanillaCreativeTabsExporter`; no Mojang file
or generated shim is tracked. The client checks with and without `java-mods` pass on the installed
MinGW toolchain. The actual offscreen world proof and feature-off executable hash are recorded in the
final command table below rather than inferred from package generation.

### Phase 3 result — lifecycle/event batches

`JavaModDispatcher` is a cloneable channel endpoint with no JNI state. It sends validated batches to
the sole JVM owner thread and waits at most `event_timeout_ms` (250 ms by default). The Java side
freezes callback registration at `start`, requires globally increasing sequence numbers, runs every
callback on thread `Java mods`, caps a batch at 256 events/actions and 2 MiB, disables a failing mod's
callbacks, and dispatches `stop` exactly once before the VM is destroyed. Game seams now emit client
and server ticks, LAN join/leave, block interaction and commands; returned chat, command, cancel,
set-block and give-item actions are applied only after Java returns.

`lifecycle` plus `lifecycle-failing` proved: start → ticks → join → interaction → command → leave →
stop ordering; two events in one JNI call; one expected callback exception disabling only that mod;
and dispatch refusal after stop. No `JNIEnv`, `Game` borrow or pointer crosses the channel.

### Phase 4 result — handles and snapshots

`HandleTable<T>` encodes `(generation, slot + 1)` in a nonzero `u64`. Remove, slot reuse and world
clear increment generations; malformed and stale handles do not resolve. Feature-on `Game` instances
allocate world/player handles and invalidate them on world close. Canonical bounded snapshot DTOs
cover block states/properties, entities, item stacks, components and typed NBT, rejecting non-finite
numbers, invalid IDs, excessive counts and NBT deeper than 32 levels.

The lifecycle integration sent Java a canonical world snapshot containing a generated ruby lamp,
player entity, ruby stack and `minecraft:custom_data`; Java validated its handles/data and returned a
give-item action. Rust then invalidated the player and stack handles and proved both stale lookups
failed. This is the supported snapshot/transport slice; broad vanilla world/entity methods remain
`partial` in the ledger.

### Phase 5 result — high-frequency transport slice

The same bounded action protocol has typed payload, screen, render, sound, recipe and loot event/action
kinds. The integration test returned one action for each low-frequency kind, then sent the maximum
256 render events in one JNI call and received one bounded empty action batch. The transport and call
budget are supported; actual Fabric networking, screen widgets, render graph hooks, sound instances,
recipe mutation and loot APIs remain explicitly `partial`, not credited as complete compatibility.

### Phase 6 result — transformed shim

`morizen_mixin_head` has a required Mixin config targeting the generated
`Block.defaultBlockState()` method. Knot transformed that ordinary shim body and a real
`@Inject(at = @At("HEAD"))` callback ran before the mod's Fabric entrypoint completed:

```text
morizen-java: mixin-head head
morizen-java: mixin-head main
morizen-java: Phase 6 transformed-shim HEAD callback passed
```

Only this HEAD callback pattern receives ledger credit. Redirects, local/constant/ordinal-sensitive
injections and arbitrary overwrites remain unsupported because generated shims intentionally contain
no Mojang implementation bytecode.

### Phase 7 result — Quilt, and Forge-family boundary

Quilt Loader 0.30.2, quilt-json5 1.0.4+final and quilt-config 1.3.3 are hash pinned in
`compat-java/quilt.lock.json`. A distinct service-loaded `MorizenQuiltGameProvider` advertises the
official namespace, mounts the same bootstrap/shims, and runs under the same Rust JVM owner. A real
`quilt.mod.json` artifact was discovered as a Quilt mod and its `morizen` entrypoint ran on `Java
mods`; the compatibility report correctly names loader `quilt@0.30.2`.

Forge-family support cannot reuse either Knot provider:

- **NeoForge:** its release metadata on 2026-11-02 ends at `26.1.2.126`; there is no NeoForge build
  for Minecraft 26.3 to pin or test. Morizen therefore refuses to invent a 26.3 contract.
- **Forge:** Forge `26.3-66.1.3` exists, but its ModLauncher launch service, FML mod-discovery
  language providers, event buses, access transformers and patched-game bootstrap are not a
  `GameProvider`. Starting the published client launch target would start/patch Mojang's Java game,
  which this bridge forbids.

The required Forge design is a Morizen ModLauncher launch service whose target is a committed
bootstrap class, with a separate FML compatibility jar for `@Mod`, deferred registries and event bus
events. It must mount generated shims as the game layer, translate access transformers against those
shims, and use the existing batch/action ABI. NeoForge needs its own independently pinned service and
event contract once a 26.3 release exists. Neither may masquerade as Fabric, directly invoke mod
constructors, or put the Mojang client jar on the runtime classpath. Until those loader-specific
gates exist, both entries remain `unsupported` in `compat-ledger.json`.

| Phase | Required demo and gate |
|---:|---|
| 2 | Generated local shims; one block/item appear in registries, creative inventory and world with translation/model/texture. Feature-off probe is byte-identical to base. |
| 3 | Start/stop, batched ticks, join/leave, interaction and commands; prove order, thread, bounds, exception isolation and no callback after stop. |
| 4 | Handle-backed world, state, entity, stack, NBT/components; stale handles fail and differential snapshots/actions match Java. |
| 5 | Payloads, screens/GUI/render, sound, recipes and loot; high-frequency paths pass call-budget tests. |
| 6 | One Fabric API module or Mixin pattern per change, with transformed-shim and engine tests; instruction-sensitive gaps stay reported. |
| 7 | Quilt gets provider/entrypoint tests; NeoForge/Forge each require a new bootstrap/event/mapping design before code. |

## Experimental launcher build

Phases 1-7 were demonstrated with the game alone. Players now test from the launcher. The
launcher has its own `java-mods` Cargo feature, off by default, so released launchers are
unchanged:

- Settings > Game > **Java mods config** names the bridge config. Empty means the instance's own
  `<instance home>/java-mods/config.json`.
- Play passes `MORIZEN_JAVA_MOD_CONFIG` to the game; a game built without the game-side
  `java-mods` feature ignores it.

The `java mod bridge` workflow (`.github/workflows/java-mod-bridge.yml`) builds on Windows and
uploads `morizen-windows-x64-java-mods`: `horizon-launcher.exe`, the bridge's game
(`horizon-client.exe`, `horizon-ctl.exe`), the original generators and the original
`compat-java/` sources. No generated shim, bridge jar or Mojang file is uploaded.

To test on your own machine (nothing is compiled locally):

1. `py -3 tools/java_mod_bridge.py game-config --minecraft-jar <your 26.3 jar> --mods <jar or
   folder>` builds the bridge jars from your own jar with a JDK and writes
   `.local/java-mod-bridge/in-game/config.json`. Everything stays under `.local/`.
2. Unpack the downloaded kit and run `py -3 setup.py` from it. The script copies your newest
   prepared launcher version to `morizen-java-mods`, puts the kit's game binaries in it, copies
   the config to the game home's `java-mods/`, selects the version and starts the launcher.
   `--no-start` stops before starting it; `--tag` names the version.
3. Press Play, then `give @s morizen_registry:ruby` and
   `setblock ~ ~ ~ morizen_registry:ruby_lamp`: they come from a Java mod's registry calls.

The kit needs an already prepared Morizen version, because preparation uses the exporters built
with the pinned vanilla jars, which a hosted runner cannot build and no artifact may carry. The
prepared version is copied, never modified.

## Test layout

Test mods under `compat-java/test-mods/` begin with Phase 1's `empty` and `failing`; later phases add
`registry`, `lifecycle`, `world`, `network-ui` and one `mixin-*` per injection/access pattern.

Each runner uses fresh `HORIZON_HOME`, `XDG_*` and launcher homes under the approved temp folder.
No real player data is written. Feature-off tests run first, feature-on tests second. Optional
vanilla comparison runs use the player's own prepared jar and never publish it or generated shims.

Each phase reports exact commands/counts, demo log, feature-off/on results, scratch-home manual steps,
and 3-5 pinned real mods as `loaded`, `partial`, `unsupported` or `loader failure` with the first gap.

## Security limits

An in-process JVM is not a sandbox. A Java mod can use files, network, processes, reflection,
`Unsafe`, native libraries and `System.exit`; a JNI/JVM/native-library fault can terminate or corrupt
the Rust process. Java's removed SecurityManager cannot provide the isolation Morizen's old
WebAssembly runtime did. The launcher must eventually show an explicit native-code trust warning,
but launcher work is forbidden until Phase 2 passes.

The bridge guarantees only that unsupported methods throw, Rust panics do not cross JNI, ordinary
Java exceptions are reported, handles/actions are validated and bounded, and no JVM call occurs
while the engine is mutably borrowed or holds a world lock.

If process isolation becomes a requirement, an out-of-process JVM with IPC is the defensible design,
but that is a different architecture and is not silently substituted here.

## Main risks and open decisions

1. **Mixin bytecode mismatch:** Phase 6 translates selected patterns, not arbitrary Mixins.
2. **Generated-shim distribution:** public shipping needs a separate legal decision.
3. **Provider instability:** pin 0.19.5 and reflect-test its internal interface.
4. **Fabric API breadth:** 44 modules, 59 Mixin configs and 1,125 named game class references.
5. **Threading/deadlocks:** queue all work; Java never holds `Game`.
6. **Registry timing:** finish before `load_resources`; later changes require full reload.
7. **Client/server ownership:** one JVM session per process with explicit logical sides.
8. **Shutdown:** non-daemon mod threads require a deadline and report.
9. **Native crash risk:** unavoidable in-process; never describe this as a sandbox.

## Phase 0 reproduction record

Evidence gathered on Windows with JDK `25.0.5.0.2`:

```text
python tools/runtime.py-equivalent fabric_libraries()
  -> downloaded/verified Fabric Loader 0.19.5, Mixin 0.8.7 and ASM 9.10.1 under .local/
javap -p net.fabricmc.loader.impl.game.GameProvider
  -> provider method contract recorded above
javap -p net.fabricmc.loader.impl.launch.knot.KnotClassDelegate
javap -p org.spongepowered.asm.mixin.transformer.IMixinTransformer
  -> Knot pre/post-Mixin transformation boundary recorded above
signature-only scan of tools/benchmark-deps/mods.lock.json artifacts
  -> named/intermediary and API-area tables recorded above
```

Sources used as documentation/API evidence:

- [Fabric Loader documentation](https://docs.fabricmc.net/develop/loader/) and its [custom-application/GameProvider guide](https://wiki.fabricmc.net/documentation:fabric_loader)
- [Mixin environment documentation](https://github.com/SpongePowered/Mixin/wiki/Introduction-to-Mixins---The-Mixin-Environment)
- [JDK 25 JNI Invocation API](https://docs.oracle.com/en/java/javase/25/docs/specs/jni/invocation.html)
- [`jni` crate 0.22.4](https://docs.rs/crate/jni/0.22.4), feature `invocation`
- local repository evidence named throughout this document

No Fabric/Mixin mod or loader source was read, translated or copied. Only published documentation,
metadata, hashes, service resources and class/member signatures were inspected.

### Phase 0 checks actually run

| Command | Result |
|---|---|
| `py -3 tools/check_public_tree.py` | passed |
| `py -3 -m unittest tests.test_runtime tests.test_class_index tests.test_lab` | 29 passed |
| `cargo check -p horizon-core` | passed |
| `cargo test -p horizon-core --lib` | 108 passed, 2 ignored, 1 failed: `jmath::tests::matches_hotspot_on_aarch64` differed by one bit on Windows x86_64 GNU |
| `cargo test -p horizon-core --lib -- --skip jmath::tests::matches_hotspot_on_aarch64` | 108 passed, 2 ignored, 1 filtered |
| `cargo check -p horizon-client --no-default-features` | blocked before checking the client: `ring` could not find `gcc.exe` in this Windows GNU toolchain |
| `py -3 tools/pr_ready.py --quick --base main` | public tree passed; no Rust crate was selected; the repository-wide Python suite ran 423 tests with 12 skipped and failed one pre-existing check, `test_readme_percentage_matches_table`, because the root `README.md` has no parity heading |

Phase 0 had no runtime mod demo: its deliverable was this design and reproducible provider/mapping
evidence. It was committed before Phase 1 implementation began.

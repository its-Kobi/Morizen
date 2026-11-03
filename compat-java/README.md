# Morizen Java compatibility layer

This project contains original shim/bootstrap code for the experimental Java-mod bridge. It is
compiled locally with the user's JDK 25 by `tools/java_mod_bridge.py`; compiled classes and JARs go
under `.local/java-mod-bridge/` and are never committed.

- `src/main/java/dev/morizen/compat/provider/` is placed on the embedded VM's parent class path and
  supplies Fabric Loader's custom `GameProvider` service.
- The other `src/main` classes are packaged separately and loaded through Knot's transforming target
  class loader.
- `src/quilt/` contains the independently compiled Quilt 0.30.2 provider/bootstrap; its public
  runtime dependencies and hashes are pinned in `quilt.lock.json`.
- `test-mods/` contains purpose-built gates: empty/failing entrypoints, registry content, lifecycle
  batches and isolation, a transformed-shim Mixin HEAD callback, and native Quilt metadata.
- `shim-surface.json` is the reviewed 26.3 member surface. `tools/gen_compat.py` validates it against
  the user's jar before generating anything.

No `net.minecraft.*` source belongs here. Signature-only generated shims are private local output
under `.local/java-mod-bridge/generated-src/`.

# Morizen Java compatibility layer

This project contains original shim/bootstrap code for the experimental Java-mod bridge. It is
compiled locally with the user's JDK 25 by `tools/java_mod_bridge.py`; compiled classes and JARs go
under `.local/java-mod-bridge/` and are never committed.

- `src/main/java/dev/morizen/compat/provider/` is placed on the embedded VM's parent class path and
  supplies Fabric Loader's custom `GameProvider` service.
- The other `src/main` classes are packaged separately and loaded through Knot's transforming target
  class loader.
- `test-mods/empty/` is the Phase 1 Fabric mod. Its `main` and `client` entrypoints write a marker
  used by the headless bridge test.

No `net.minecraft.*` source belongs here. Signature-only generated shims are private local output
under `.local/java-mod-bridge/generated-src/`.

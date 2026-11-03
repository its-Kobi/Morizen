package dev.morizen.compat.quilt;

/** Morizen's loader-neutral Quilt entrypoint used while QSL modules are ported. */
@FunctionalInterface
public interface QuiltInitializer {
    void onInitialize();
}

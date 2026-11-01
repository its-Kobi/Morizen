package dev.morizen.testmod;

import net.fabricmc.api.ModInitializer;

/** Deterministic proof that a Java initializer exception becomes a Rust error. */
public final class FailingMod implements ModInitializer {
    @Override
    public void onInitialize() {
        throw new IllegalStateException("expected Phase 1 entrypoint failure");
    }
}

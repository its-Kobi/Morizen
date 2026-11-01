package dev.morizen.compat;

/** Stable home for the small JNI ABI; registration begins in Phase 2. */
public final class NativeBridge {
    private NativeBridge() {}

    public static native void reportUnsupported(String modId, String member, String phase);
}

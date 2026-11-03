package dev.morizen.compat;

import java.nio.file.Path;
import dev.morizen.compat.registry.MorizenRegistries;

/** Stable home for the small JNI ABI and the registry transaction. */
public final class NativeBridge {
    private NativeBridge() {}

    public static native void reportUnsupported(String modId, String member, String phase);

    /**
     * Writes what Java mods registered during their entrypoints. The Rust host
     * reads the directory only after Fabric's initializers have returned, so a
     * mod can never observe a half-applied registry.
     */
    public static void commitRegistry() {
        String directory = System.getProperty("morizen.registryDir");
        if (directory == null || directory.isBlank()) {
            return;
        }
        try {
            MorizenRegistries.commit(Path.of(directory));
        } catch (RuntimeException error) {
            throw error;
        } catch (Exception error) {
            throw new IllegalStateException(
                    "Could not write the Java mod registry transaction to " + directory, error);
        }
    }
}

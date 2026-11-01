package dev.morizen.testmod;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.api.ModInitializer;
import net.fabricmc.loader.api.FabricLoader;

/** Phase 1 proof that Fabric creates and invokes real mod entrypoints. */
public final class EmptyMod implements ModInitializer, ClientModInitializer {
    @Override
    public void onInitialize() {
        record("main");
    }

    @Override
    public void onInitializeClient() {
        record("client");
    }

    private static synchronized void record(String entrypoint) {
        String version = FabricLoader.getInstance()
                .getModContainer("morizen_empty")
                .orElseThrow()
                .getMetadata()
                .getVersion()
                .getFriendlyString();
        String thread = Thread.currentThread().getName();
        System.out.println("morizen-java: entrypoint=" + entrypoint + " begin mod=morizen_empty@"
                + version + " thread=" + thread);
        Path marker = Path.of(System.getProperty("morizen.marker"));
        try {
            Files.writeString(
                    marker,
                    entrypoint + "\n",
                    StandardCharsets.UTF_8,
                    StandardOpenOption.CREATE,
                    StandardOpenOption.APPEND);
        } catch (IOException error) {
            throw new IllegalStateException("Could not write Java-mod test marker " + marker, error);
        }
        System.out.println("morizen-java: entrypoint=" + entrypoint + " end mod=morizen_empty@"
                + version + " thread=" + thread);
    }
}

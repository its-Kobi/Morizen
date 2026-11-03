package dev.morizen.testmod;

import dev.morizen.compat.quilt.QuiltInitializer;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

/** Native quilt.mod.json entrypoint invoked by the embedded Quilt Loader. */
public final class QuiltMod implements QuiltInitializer {
    @Override
    public void onInitialize() {
        Path marker = Path.of(System.getProperty("morizen.marker"));
        try {
            Files.writeString(marker, "quilt\n", StandardCharsets.UTF_8);
        } catch (IOException error) {
            throw new IllegalStateException("Could not write Quilt marker " + marker, error);
        }
        System.out.println("morizen-java: quilt entrypoint=morizen thread="
                + Thread.currentThread().getName());
    }
}

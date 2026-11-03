package dev.morizen.testmod;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import net.fabricmc.api.ModInitializer;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;

/** Proves Mixin transforms the generated shim's ordinary method body. */
public final class MixinHeadMod implements ModInitializer {
    @Override
    public void onInitialize() {
        Block block = new Block(BlockBehaviour.Properties.of());
        if (block.defaultBlockState().getBlock() != block) {
            throw new IllegalStateException("generated BlockState lost its Block");
        }
        record("main");
    }

    public static synchronized void record(String line) {
        Path marker = Path.of(System.getProperty("morizen.marker"));
        try {
            Files.writeString(
                    marker,
                    line + "\n",
                    StandardCharsets.UTF_8,
                    StandardOpenOption.CREATE,
                    StandardOpenOption.APPEND);
        } catch (IOException error) {
            throw new IllegalStateException("Could not write Mixin marker " + marker, error);
        }
        System.out.println("morizen-java: mixin-head " + line);
    }
}

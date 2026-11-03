package dev.morizen.testmod;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import net.fabricmc.api.ModInitializer;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;

/**
 * Phase 2 proof: a real Fabric mod compiled only against generated shims
 * registers blocks and items through the vanilla API, and the native engine
 * receives one transaction.
 */
public final class RegistryMod implements ModInitializer {
    public static final Identifier RUBY_BLOCK_ID =
            Identifier.fromNamespaceAndPath("morizen_registry", "ruby_block");
    public static final Block RUBY_BLOCK = Registry.register(
            BuiltInRegistries.BLOCK,
            RUBY_BLOCK_ID,
            new Block(BlockBehaviour.Properties.of()
                    .strength(4.0F)
                    .lightLevel(state -> 3)
                    .requiresCorrectToolForDrops()));
    public static final Item RUBY_BLOCK_ITEM = Registry.register(
            BuiltInRegistries.ITEM,
            RUBY_BLOCK_ID,
            new BlockItem(RUBY_BLOCK, new Item.Properties()));
    public static final Item RUBY = Registry.register(
            BuiltInRegistries.ITEM,
            Identifier.fromNamespaceAndPath("morizen_registry", "ruby"),
            new Item(new Item.Properties().stacksTo(16)));
    public static final Block RUBY_LAMP = Registry.register(
            BuiltInRegistries.BLOCK,
            Identifier.fromNamespaceAndPath("morizen_registry", "ruby_lamp"),
            new Block(BlockBehaviour.Properties.of()
                    .morizenTemplate("minecraft:glowstone")
                    .strength(0.3F)
                    .lightLevel(state -> 15)));

    @Override
    public void onInitialize() {
        check("block", BuiltInRegistries.BLOCK, RUBY_BLOCK, RUBY_BLOCK_ID);
        check("block-item", BuiltInRegistries.ITEM, RUBY_BLOCK_ITEM, RUBY_BLOCK_ID);
        check(
                "item",
                BuiltInRegistries.ITEM,
                RUBY,
                Identifier.fromNamespaceAndPath("morizen_registry", "ruby"));
        record("block morizen_registry:ruby_block");
        record("block morizen_registry:ruby_lamp");
        record("item morizen_registry:ruby_block");
        record("item morizen_registry:ruby");
        System.out.println("morizen-java: registry entrypoints complete");
    }

    private static <T> void check(String kind, Registry<T> registry, T value, Identifier id) {
        if (!registry.containsKey(id) || registry.getValue(id) != value) {
            throw new IllegalStateException("Registry lost " + kind + " " + id);
        }
        if (!id.equals(registry.getKey(value))) {
            throw new IllegalStateException("Registry key mismatch for " + kind + " " + id);
        }
        if (registry.getId(value) < 0) {
            throw new IllegalStateException("Registry id missing for " + kind + " " + id);
        }
    }

    private static synchronized void record(String line) {
        Path marker = Path.of(System.getProperty("morizen.marker"));
        try {
            Files.writeString(
                    marker,
                    line + "\n",
                    StandardCharsets.UTF_8,
                    StandardOpenOption.CREATE,
                    StandardOpenOption.APPEND);
        } catch (IOException error) {
            throw new IllegalStateException("Could not write Java-mod test marker " + marker, error);
        }
        System.out.println("morizen-java: " + line);
    }
}

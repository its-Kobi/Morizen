package dev.morizen.compat.registry;

import dev.morizen.compat.UnsupportedCompatException;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import net.minecraft.core.Registry;
import net.minecraft.resources.Identifier;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;

/**
 * Collects {@code Registry.register} calls made by Java mods and writes one
 * JSON transaction per namespace for the Rust host.
 *
 * <p>This is deliberately not a second live registry: Java only records the
 * declaration, and the native engine builds the real block/item tables from
 * the transaction through the ordinary content-mod pipeline.
 */
public final class MorizenRegistries {
    public static final BridgeRegistry<Block> BLOCK = new BridgeRegistry<>("block");
    public static final BridgeRegistry<Item> ITEM = new BridgeRegistry<>("item");

    private static final Map<Block, Identifier> BLOCK_IDS = new IdentityHashMap<>();
    private static final Map<String, BlockRecord> BLOCKS = new LinkedHashMap<>();
    private static final Map<String, ItemRecord> ITEMS = new LinkedHashMap<>();

    private MorizenRegistries() {}

    public static <V, T extends V> T register(Registry<V> registry, Identifier id, T entry) {
        if (id == null || entry == null) {
            throw new NullPointerException("Registry.register needs an id and an entry");
        }
        String key = id.toString();
        if (registry == BLOCK) {
            if (!(entry instanceof Block block)) {
                throw new UnsupportedCompatException(
                        "Registry.register(BLOCK, " + key + ", " + entry.getClass().getName() + ")");
            }
            recordBlock(key, id, block);
            return entry;
        }
        if (registry == ITEM) {
            if (entry instanceof BlockItem blockItem) {
                recordBlockItem(key, blockItem);
                return entry;
            }
            if (entry instanceof Item item) {
                recordItem(key, item);
                return entry;
            }
            throw new UnsupportedCompatException(
                    "Registry.register(ITEM, " + key + ", " + entry.getClass().getName() + ")");
        }
        throw new UnsupportedCompatException("Registry.register(" + registry + ")");
    }

    private static void recordBlock(String key, Identifier id, Block block) {
        BLOCK.register(id, block);
        BlockRecord record = new BlockRecord();
        record.id = key;
        BlockBehaviour.Properties properties = block.properties();
        if (properties != null) {
            record.template = properties.morizenTemplateName();
            record.strength = properties.morizenStrength();
            record.light = properties.morizenLight();
            record.friction = properties.morizenFriction();
            record.requiresTool = properties.morizenRequiresTool();
        }
        BLOCKS.put(key, record);
        BLOCK_IDS.put(block, id);
        System.out.println("morizen-java: registry block " + key);
    }

    private static void recordBlockItem(String key, BlockItem item) {
        ITEM.register(Identifier.parse(key), item);
        Identifier blockId = item.getBlock() == null ? null : BLOCK_IDS.get(item.getBlock());
        if (blockId == null) {
            throw new UnsupportedCompatException(
                    "BlockItem " + key + " was registered before its Block");
        }
        BlockRecord record = BLOCKS.get(blockId.toString());
        if (record == null) {
            throw new UnsupportedCompatException(
                    "BlockItem " + key + " has no block declaration " + blockId);
        }
        record.item = true;
        System.out.println("morizen-java: registry item " + key + " (block item)");
    }

    private static void recordItem(String key, Item item) {
        ITEM.register(Identifier.parse(key), item);
        ItemRecord record = new ItemRecord();
        record.id = key;
        Item.Properties properties = item.morizenProperties();
        if (properties != null) {
            record.template = properties.morizenItemTemplateName();
            record.maxStackSize = properties.morizenMaxStackSize();
        }
        ITEMS.put(key, record);
        System.out.println("morizen-java: registry item " + key);
    }

    /** Writes one transaction file per namespace, atomically. */
    public static void commit(Path directory) throws IOException {
        if (BLOCKS.isEmpty() && ITEMS.isEmpty()) {
            return;
        }
        Set<String> namespaces = new LinkedHashSet<>();
        for (String id : BLOCKS.keySet()) {
            namespaces.add(namespaceOf(id));
        }
        for (String id : ITEMS.keySet()) {
            namespaces.add(namespaceOf(id));
        }
        Files.createDirectories(directory);
        for (String namespace : namespaces) {
            List<BlockRecord> blocks = new ArrayList<>();
            for (BlockRecord record : BLOCKS.values()) {
                if (namespaceOf(record.id).equals(namespace)) {
                    blocks.add(record);
                }
            }
            List<ItemRecord> items = new ArrayList<>();
            for (ItemRecord record : ITEMS.values()) {
                if (namespaceOf(record.id).equals(namespace)) {
                    items.add(record);
                }
            }
            Path target = directory.resolve(namespace + ".json");
            Path temporary = directory.resolve(namespace + ".json.tmp");
            Files.writeString(temporary, transaction(namespace, blocks, items), StandardCharsets.UTF_8);
            try {
                Files.move(
                        temporary,
                        target,
                        StandardCopyOption.REPLACE_EXISTING,
                        StandardCopyOption.ATOMIC_MOVE);
            } catch (AtomicMoveNotSupportedException fallback) {
                Files.move(temporary, target, StandardCopyOption.REPLACE_EXISTING);
            }
            System.out.println("morizen-java: registry transaction " + target);
        }
    }

    private static String namespaceOf(String id) {
        int colon = id.indexOf(':');
        String namespace = colon < 0 ? "minecraft" : id.substring(0, colon);
        if (namespace.isEmpty()
                || namespace.length() > 64
                || namespace.chars().anyMatch(character ->
                        !(character >= 'a' && character <= 'z')
                                && !(character >= '0' && character <= '9')
                                && character != '_')) {
            throw new UnsupportedCompatException(
                    "registry namespace " + namespace + " is not a content-mod id");
        }
        return namespace;
    }

    private static String transaction(
            String namespace, List<BlockRecord> blocks, List<ItemRecord> items) {
        StringBuilder json = new StringBuilder(256 + blocks.size() * 160 + items.size() * 96);
        json.append("{\"schema\":1,\"namespace\":");
        quote(json, namespace);
        json.append(",\"blocks\":[");
        for (int index = 0; index < blocks.size(); index++) {
            if (index > 0) {
                json.append(',');
            }
            BlockRecord block = blocks.get(index);
            json.append("{\"id\":");
            quote(json, block.id);
            json.append(",\"template\":");
            quote(json, block.template);
            json.append(",\"strength\":").append(number(block.strength));
            json.append(",\"light\":").append(Math.max(0, Math.min(15, block.light)));
            json.append(",\"friction\":").append(number(block.friction));
            json.append(",\"requires_tool\":").append(block.requiresTool);
            json.append(",\"item\":").append(block.item).append('}');
        }
        json.append("],\"items\":[");
        for (int index = 0; index < items.size(); index++) {
            if (index > 0) {
                json.append(',');
            }
            ItemRecord item = items.get(index);
            json.append("{\"id\":");
            quote(json, item.id);
            json.append(",\"template\":");
            quote(json, item.template);
            json.append(",\"max_stack_size\":")
                    .append(Math.max(1, Math.min(99, item.maxStackSize)))
                    .append('}');
        }
        json.append("]}");
        return json.toString();
    }

    private static String number(float value) {
        if (!Float.isFinite(value) || value < 0.0F) {
            return "0.0";
        }
        return Float.toString(Math.min(value, 1_000_000.0F));
    }

    private static void quote(StringBuilder json, String value) {
        json.append('"');
        for (int index = 0; index < value.length(); index++) {
            char character = value.charAt(index);
            switch (character) {
                case '"' -> json.append("\\\"");
                case '\\' -> json.append("\\\\");
                case '\n' -> json.append("\\n");
                case '\r' -> json.append("\\r");
                case '\t' -> json.append("\\t");
                default -> {
                    if (character < 0x20) {
                        json.append(String.format("\\u%04x", (int) character));
                    } else {
                        json.append(character);
                    }
                }
            }
        }
        json.append('"');
    }

    private static final class BlockRecord {
        String id;
        String template = "minecraft:stone";
        float strength = 1.0F;
        int light;
        float friction = 0.6F;
        boolean requiresTool;
        boolean item;
    }

    private static final class ItemRecord {
        String id;
        String template = "minecraft:paper";
        int maxStackSize = 64;
    }
}

package dev.morizen.testmod;

import dev.morizen.compat.events.BridgeAction;
import dev.morizen.compat.events.BridgeEvent;
import dev.morizen.compat.events.BridgeEvents;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.List;
import net.fabricmc.api.ModInitializer;

/** Phase 3 proof for ordered, batched lifecycle and interaction callbacks. */
public final class LifecycleMod implements ModInitializer {
    @Override
    public void onInitialize() {
        BridgeEvents.register("morizen_lifecycle", LifecycleMod::event);
    }

    private static List<BridgeAction> event(BridgeEvent event) {
        if (!event.kind().equals("render")) {
            record("ok " + event.sequence() + " " + event.kind() + " "
                    + Thread.currentThread().getName());
        }
        return switch (event.kind()) {
            case "block_interact" -> List.of(BridgeAction.setBlock(
                    event, event.x(), event.y() + 1, event.z(), "morizen_registry:ruby_lamp"));
            case "command" -> {
                if (!event.text().equals("bridge ping")) {
                    throw new IllegalStateException("unexpected command " + event.text());
                }
                yield List.of(
                        BridgeAction.chat(event, "bridge pong"), BridgeAction.cancel(event));
            }
            case "world_snapshot" -> {
                if (event.worldHandle() == 0
                        || event.playerHandle() == 0
                        || !event.text().contains("\"morizen_registry:ruby_lamp\"")
                        || !event.text().contains("\"minecraft:custom_data\"")) {
                    throw new IllegalStateException("world snapshot lost handles, state or components");
                }
                yield List.of(BridgeAction.giveItem(event, "morizen_registry:ruby", 2));
            }
            case "network_payload" ->
                    List.of(BridgeAction.customPayload(event, "morizen:test/010203"));
            case "screen" -> List.of(BridgeAction.openScreen(event, "morizen:test"));
            case "sound" -> List.of(BridgeAction.playSound(event, "minecraft:block.note_block.pling"));
            case "recipe" -> List.of(BridgeAction.giveItem(event, "morizen_registry:ruby", 1));
            case "loot" -> List.of(BridgeAction.giveItem(event, "morizen_registry:ruby", 3));
            default -> List.of();
        };
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
            throw new IllegalStateException("Could not write lifecycle marker " + marker, error);
        }
        System.out.println("morizen-java: lifecycle " + line);
    }
}

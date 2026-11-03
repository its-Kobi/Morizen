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

/** Throws once so the harness proves another mod continues and this one does not. */
public final class LifecycleFailingMod implements ModInitializer {
    @Override
    public void onInitialize() {
        BridgeEvents.register("morizen_lifecycle_failing", LifecycleFailingMod::event);
    }

    private static List<BridgeAction> event(BridgeEvent event) {
        record("fail " + event.sequence() + " " + event.kind());
        if (event.kind().equals("player_join")) {
            throw new IllegalStateException("expected isolated lifecycle failure");
        }
        return List.of();
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
    }
}

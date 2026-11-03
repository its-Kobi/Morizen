package dev.morizen.compat.events;

import java.nio.charset.StandardCharsets;

/** A bounded request Java returns for the native engine to apply after callbacks. */
public final class BridgeAction {
    static final int MAX_TEXT_BYTES = 4096;

    private final long eventSequence;
    private final String kind;
    private final int[] position;
    private final long value;
    private final String text;

    private BridgeAction(
            long eventSequence, String kind, int[] position, long value, String text) {
        this.eventSequence = eventSequence;
        this.kind = kind;
        this.position = position == null ? null : position.clone();
        this.value = value;
        this.text = text == null ? "" : text;
        if (this.text.getBytes(StandardCharsets.UTF_8).length > MAX_TEXT_BYTES) {
            throw new IllegalArgumentException("Bridge action text exceeds 4096 UTF-8 bytes");
        }
    }

    public static BridgeAction chat(BridgeEvent event, String text) {
        return text(event, "chat", text);
    }

    public static BridgeAction command(BridgeEvent event, String command) {
        return text(event, "command", command);
    }

    public static BridgeAction cancel(BridgeEvent event) {
        return text(event, "cancel", "");
    }

    public static BridgeAction playSound(BridgeEvent event, String sound) {
        return text(event, "play_sound", sound);
    }

    public static BridgeAction openScreen(BridgeEvent event, String screen) {
        return text(event, "open_screen", screen);
    }

    public static BridgeAction customPayload(BridgeEvent event, String channelAndHexPayload) {
        return text(event, "custom_payload", channelAndHexPayload);
    }

    public static BridgeAction giveItem(BridgeEvent event, String item, int count) {
        return new BridgeAction(event.sequence(), "give_item", null, count, item);
    }

    public static BridgeAction setBlock(BridgeEvent event, int x, int y, int z, String block) {
        return new BridgeAction(event.sequence(), "set_block", new int[] {x, y, z}, 0, block);
    }

    static BridgeAction error(BridgeEvent event, String text) {
        return text(event, "error", text);
    }

    private static BridgeAction text(BridgeEvent event, String kind, String text) {
        if (event == null) {
            throw new NullPointerException("Bridge action needs its source event");
        }
        return new BridgeAction(event.sequence(), kind, null, 0, text);
    }

    long eventSequence() {
        return eventSequence;
    }

    String kind() {
        return kind;
    }

    int[] position() {
        return position == null ? null : position.clone();
    }

    long value() {
        return value;
    }

    String text() {
        return text;
    }
}

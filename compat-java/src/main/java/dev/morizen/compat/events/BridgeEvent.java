package dev.morizen.compat.events;

/** One immutable native-engine event in a batched Java callback. */
public final class BridgeEvent {
    private final long sequence;
    private final String kind;
    private final long tick;
    private final long worldHandle;
    private final long playerHandle;
    private final int[] position;
    private final String text;

    BridgeEvent(
            long sequence,
            String kind,
            long tick,
            long worldHandle,
            long playerHandle,
            int[] position,
            String text) {
        this.sequence = sequence;
        this.kind = kind;
        this.tick = tick;
        this.worldHandle = worldHandle;
        this.playerHandle = playerHandle;
        this.position = position == null ? null : position.clone();
        this.text = text;
    }

    public long sequence() {
        return sequence;
    }

    public String kind() {
        return kind;
    }

    public long tick() {
        return tick;
    }

    public long worldHandle() {
        return worldHandle;
    }

    public long playerHandle() {
        return playerHandle;
    }

    public boolean hasPosition() {
        return position != null;
    }

    public int x() {
        return coordinate(0);
    }

    public int y() {
        return coordinate(1);
    }

    public int z() {
        return coordinate(2);
    }

    public String text() {
        return text;
    }

    private int coordinate(int index) {
        if (position == null) {
            throw new IllegalStateException("Event " + kind + " has no block position");
        }
        return position[index];
    }
}

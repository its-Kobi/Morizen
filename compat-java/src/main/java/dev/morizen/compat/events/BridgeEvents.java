package dev.morizen.compat.events;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * The lifecycle/event batch boundary. Registration happens in Fabric
 * entrypoints; dispatch happens only on Morizen's dedicated "Java mods"
 * thread. A callback failure disables that mod and does not stop later mods.
 */
public final class BridgeEvents {
    public static final int MAX_EVENTS = 256;
    public static final int MAX_ACTIONS = 256;

    @FunctionalInterface
    public interface Callback {
        List<BridgeAction> handle(BridgeEvent event) throws Exception;
    }

    private record Registration(String modId, Callback callback) {}

    private static final List<Registration> CALLBACKS = new ArrayList<>();
    private static final Set<String> DISABLED = new HashSet<>();
    private static boolean started;
    private static boolean stopped;
    private static long lastSequence = -1;

    private BridgeEvents() {}

    public static synchronized void register(String modId, Callback callback) {
        if (started) {
            throw new IllegalStateException("Bridge event callbacks must register during entrypoints");
        }
        if (modId == null || modId.isBlank() || callback == null) {
            throw new IllegalArgumentException("Bridge event callback needs a mod id and callback");
        }
        CALLBACKS.add(new Registration(modId, callback));
    }

    public static synchronized boolean hasCallbacks() {
        return !CALLBACKS.isEmpty();
    }

    /** Called reflectively by the provider for one coarse JNI batch. */
    public static synchronized String dispatch(String wire) {
        if (!Thread.currentThread().getName().equals("Java mods")) {
            throw new IllegalStateException("Bridge events ran off the Java mods thread");
        }
        if (stopped) {
            throw new IllegalStateException("Bridge events were dispatched after stop");
        }
        List<BridgeEvent> events = parseEvents(wire);
        List<BridgeAction> actions = new ArrayList<>();
        for (BridgeEvent event : events) {
            if (event.sequence() <= lastSequence) {
                throw new IllegalArgumentException("Bridge event sequences did not increase");
            }
            if (!started && !event.kind().equals("start")) {
                throw new IllegalStateException("The first bridge event must be start");
            }
            lastSequence = event.sequence();
            started = true;
            for (Registration registration : CALLBACKS) {
                if (DISABLED.contains(registration.modId())) {
                    continue;
                }
                try {
                    List<BridgeAction> returned = registration.callback().handle(event);
                    if (returned != null) {
                        for (BridgeAction action : returned) {
                            if (action == null) {
                                throw new IllegalArgumentException("callback returned a null action");
                            }
                            if (action.eventSequence() != event.sequence()) {
                                throw new IllegalArgumentException(
                                        "callback returned an action for another event");
                            }
                            if (actions.size() == MAX_ACTIONS) {
                                throw new IllegalStateException("bridge action batch exceeds 256 actions");
                            }
                            actions.add(action);
                        }
                    }
                } catch (Throwable error) {
                    DISABLED.add(registration.modId());
                    String message = registration.modId() + ": " + error.getClass().getName()
                            + (error.getMessage() == null ? "" : ": " + error.getMessage());
                    System.err.println("morizen-java: disabled callbacks for " + message);
                    actions.add(BridgeAction.error(event, message));
                }
            }
            if (event.kind().equals("stop")) {
                stopped = true;
            }
        }
        System.out.println(
                "morizen-java: event-batch events=" + events.size() + " actions=" + actions.size());
        return writeActions(actions);
    }

    private static List<BridgeEvent> parseEvents(String wire) {
        if (wire == null || wire.length() > 2 * 1024 * 1024) {
            throw new IllegalArgumentException("Bridge event wire is absent or too large");
        }
        String[] lines = wire.split("\\n", -1);
        if (lines.length == 0 || !lines[0].equals("MORIZEN_EVENTS\t1")) {
            throw new IllegalArgumentException("Unsupported bridge event wire header");
        }
        List<BridgeEvent> events = new ArrayList<>();
        for (int line = 1; line < lines.length; line++) {
            if (lines[line].isEmpty()) {
                continue;
            }
            String[] fields = lines[line].split("\\t", -1);
            if (fields.length != 10 || !fields[0].equals("E")) {
                throw new IllegalArgumentException("Malformed bridge event row " + line);
            }
            int[] position = null;
            if (!(fields[6].equals("_") && fields[7].equals("_") && fields[8].equals("_"))) {
                position = new int[] {
                    Integer.parseInt(fields[6]),
                    Integer.parseInt(fields[7]),
                    Integer.parseInt(fields[8])
                };
            }
            events.add(new BridgeEvent(
                    Long.parseUnsignedLong(fields[1]),
                    fields[2],
                    Long.parseUnsignedLong(fields[3]),
                    Long.parseUnsignedLong(fields[4]),
                    Long.parseUnsignedLong(fields[5]),
                    position,
                    decodeHex(fields[9])));
            if (events.size() > MAX_EVENTS) {
                throw new IllegalArgumentException("Bridge event batch exceeds 256 events");
            }
        }
        return events;
    }

    private static String writeActions(List<BridgeAction> actions) {
        StringBuilder wire = new StringBuilder("MORIZEN_ACTIONS\t1\n");
        for (BridgeAction action : actions) {
            int[] position = action.position();
            wire.append("A\t").append(Long.toUnsignedString(action.eventSequence()))
                    .append('\t').append(action.kind()).append('\t');
            if (position == null) {
                wire.append("_\t_\t_");
            } else {
                wire.append(position[0]).append('\t').append(position[1]).append('\t')
                        .append(position[2]);
            }
            wire.append('\t').append(action.value()).append('\t')
                    .append(encodeHex(action.text())).append('\n');
        }
        if (wire.length() > 2 * 1024 * 1024) {
            throw new IllegalStateException("Bridge action wire is too large");
        }
        return wire.toString();
    }

    private static String encodeHex(String text) {
        byte[] bytes = text.getBytes(StandardCharsets.UTF_8);
        char[] encoded = new char[bytes.length * 2];
        char[] digits = "0123456789abcdef".toCharArray();
        for (int index = 0; index < bytes.length; index++) {
            int value = bytes[index] & 0xff;
            encoded[index * 2] = digits[value >>> 4];
            encoded[index * 2 + 1] = digits[value & 0x0f];
        }
        return new String(encoded);
    }

    private static String decodeHex(String text) {
        if ((text.length() & 1) != 0) {
            throw new IllegalArgumentException("Bridge text has odd-length hex");
        }
        byte[] bytes = new byte[text.length() / 2];
        for (int index = 0; index < bytes.length; index++) {
            int high = Character.digit(text.charAt(index * 2), 16);
            int low = Character.digit(text.charAt(index * 2 + 1), 16);
            if (high < 0 || low < 0) {
                throw new IllegalArgumentException("Bridge text contains non-hex data");
            }
            bytes[index] = (byte) ((high << 4) | low);
        }
        String decoded = new String(bytes, StandardCharsets.UTF_8);
        if (decoded.getBytes(StandardCharsets.UTF_8).length > BridgeAction.MAX_TEXT_BYTES) {
            throw new IllegalArgumentException("Bridge event text exceeds 4096 UTF-8 bytes");
        }
        return decoded;
    }
}

package dev.morizen.compat;

/** A clear, attributable failure when a generated shim member is not supported yet. */
public final class UnsupportedCompatException extends UnsupportedOperationException {
    public UnsupportedCompatException(String member) {
        super(member);
    }
}

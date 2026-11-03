package dev.morizen.compat.registry;

import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import net.minecraft.core.DefaultedRegistry;
import net.minecraft.resources.Identifier;

/**
 * The in-memory registry object the generated {@code BuiltInRegistries} fields
 * point at. It keeps insertion order and identity lookups, which is all the
 * registration transaction needs; the native engine owns the real registry.
 */
public final class BridgeRegistry<T> implements DefaultedRegistry<T> {
    private final String kind;
    private final Map<Identifier, T> entries = new LinkedHashMap<>();
    private final Map<T, Identifier> ids = new IdentityHashMap<>();
    private T defaultValue;

    BridgeRegistry(String kind) {
        this.kind = kind;
    }

    public String morizenKind() {
        return kind;
    }

    public T register(Identifier id, T value) {
        if (id == null || value == null) {
            throw new NullPointerException("Registry entries need an id and a value");
        }
        if (entries.putIfAbsent(id, value) != null) {
            throw new IllegalStateException("Duplicate registry entry " + id);
        }
        ids.put(value, id);
        if (defaultValue == null) {
            defaultValue = value;
        }
        return value;
    }

    @Override
    public Identifier getDefaultKey() {
        if (defaultValue == null) {
            return null;
        }
        return ids.get(defaultValue);
    }

    @Override
    public Identifier getKey(T value) {
        return ids.get(value);
    }

    @Override
    public T getValue(Identifier id) {
        return entries.get(id);
    }

    @Override
    public int getId(T value) {
        int id = 0;
        for (T entry : entries.values()) {
            if (entry == value) {
                return id;
            }
            id++;
        }
        return -1;
    }

    @Override
    public boolean containsKey(Identifier id) {
        return entries.containsKey(id);
    }

    @Override
    public Set<Identifier> keySet() {
        return Collections.unmodifiableSet(entries.keySet());
    }
}

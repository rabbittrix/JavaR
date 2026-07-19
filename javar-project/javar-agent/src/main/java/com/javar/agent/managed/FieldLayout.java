package com.javar.agent.managed;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Per-class off-heap layout: field name → byte offset within a Rust region.
 */
public final class FieldLayout {

    private final String className;
    private final Map<String, FieldSlot> slots;
    private final int totalSize;

    public FieldLayout(String className, Map<String, FieldSlot> slots, int totalSize) {
        this.className = className;
        this.slots = Collections.unmodifiableMap(new LinkedHashMap<String, FieldSlot>(slots));
        this.totalSize = totalSize;
    }

    public String className() {
        return className;
    }

    public int totalSize() {
        return totalSize;
    }

    public FieldSlot slot(String fieldName) {
        return slots.get(fieldName);
    }

    public Map<String, FieldSlot> slots() {
        return slots;
    }

    public static final class FieldSlot {
        public final String name;
        public final String descriptor;
        public final int offset;
        public final int size;

        public FieldSlot(String name, String descriptor, int offset, int size) {
            this.name = name;
            this.descriptor = descriptor;
            this.offset = offset;
            this.size = size;
        }
    }

    /** JVM type descriptor → aligned size in bytes. */
    public static int sizeOf(String descriptor) {
        if (descriptor == null || descriptor.isEmpty()) {
            return 0;
        }
        switch (descriptor.charAt(0)) {
            case 'Z':
            case 'B':
                return 1;
            case 'C':
            case 'S':
                return 2;
            case 'I':
            case 'F':
                return 4;
            case 'J':
            case 'D':
                return 8;
            default:
                return 0; // reference — not off-heaped
        }
    }

    public static int align(int offset, int alignment) {
        int mask = alignment - 1;
        return (offset + mask) & ~mask;
    }
}

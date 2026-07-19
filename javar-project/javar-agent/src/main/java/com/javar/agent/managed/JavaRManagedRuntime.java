package com.javar.agent.managed;

import com.javar.agent.JavaRAgent;
import com.javar.agent.memory.OffHeapBridge;
import com.javar.agent.memory.OffHeapBridgeFactory;

import java.lang.ref.ReferenceQueue;
import java.lang.ref.WeakReference;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;
import java.util.logging.Logger;

/**
 * Runtime backing {@link JavaRManaged}: allocates Rust regions and exposes
 * typed get/put used by rewritten bytecode.
 */
public final class JavaRManagedRuntime {

    private static final Logger LOG = Logger.getLogger(JavaRManagedRuntime.class.getName());

    /** Synthetic field name injected into managed shells. */
    public static final String REGION_FIELD = "__javar_region";
    public static final String REGION_DESC = "J";

    private static final Map<String, FieldLayout> LAYOUTS = new ConcurrentHashMap<String, FieldLayout>();

    private static final ReferenceQueue<Object> QUEUE = new ReferenceQueue<Object>();
    private static final Map<WeakRef, Long> WEAK_REGIONS = new ConcurrentHashMap<WeakRef, Long>();
    private static final AtomicLong BYTES_OFF_HEAP = new AtomicLong();
    private static final AtomicLong REGION_COUNT = new AtomicLong();

    private static volatile OffHeapBridge bridge;
    private static volatile boolean cleanerStarted;

    private JavaRManagedRuntime() {
    }

    public static void registerLayout(FieldLayout layout) {
        LAYOUTS.put(layout.className(), layout);
        LOG.fine("Registered off-heap layout for " + layout.className()
                + " (" + layout.totalSize() + " bytes)");
    }

    public static FieldLayout layout(String className) {
        return LAYOUTS.get(className);
    }

    public static long managedBytes() {
        OffHeapBridge b = bridge();
        long nativeBytes = 0L;
        if (b != null) {
            try {
                nativeBytes = b.managedBytes();
            } catch (Throwable ignored) {
                // fall through
            }
        }
        return Math.max(nativeBytes, BYTES_OFF_HEAP.get());
    }

    public static long regionCount() {
        return REGION_COUNT.get();
    }

    /** Estimated GC savings: bytes kept out of the Java heap for managed shells. */
    public static long gcSavingsBytes() {
        return BYTES_OFF_HEAP.get();
    }

    /**
     * Ensure {@code shell} has an off-heap region; returns region id.
     * Invoked from rewritten constructors / first field access.
     * <p>
     * Region ids are stored only on the synthetic {@code __javar_region} field —
     * never in an identityHashCode map (those collide under high churn / after GC).
     */
    public static long ensureRegion(Object shell, String className) {
        if (shell == null) {
            return 0L;
        }
        long existing = readRegionField(shell);
        if (existing != 0L) {
            return existing;
        }

        FieldLayout layout = LAYOUTS.get(className);
        int size = layout != null ? layout.totalSize() : 64;
        if (size <= 0) {
            size = 8;
        }

        OffHeapBridge b = bridge();
        long id = 0L;
        if (b != null) {
            id = b.allocate(size, 8);
        }
        if (id == 0L) {
            // Fallback: direct ByteBuffer still off-heap (Java heap untouched for payload).
            id = FallbackArena.allocate(size);
        }

        writeRegionField(shell, id);
        BYTES_OFF_HEAP.addAndGet(size);
        REGION_COUNT.incrementAndGet();
        track(shell, id, size);
        return id;
    }

    public static int getInt(Object shell, String className, int offset) {
        return buffer(shell, className).getInt(offset);
    }

    public static void putInt(Object shell, String className, int offset, int value) {
        buffer(shell, className).putInt(offset, value);
    }

    /**
     * PUTFIELD-shaped setter: stack was {@code obj, value} then {@code className, offset}
     * → descriptor {@code (Ljava/lang/Object;ILjava/lang/String;I)V}.
     */
    public static void putIntPf(Object shell, int value, String className, int offset) {
        putInt(shell, className, offset, value);
    }

    public static long getLong(Object shell, String className, int offset) {
        return buffer(shell, className).getLong(offset);
    }

    public static void putLong(Object shell, String className, int offset, long value) {
        buffer(shell, className).putLong(offset, value);
    }

    /** PUTFIELD-shaped: {@code (Ljava/lang/Object;JLjava/lang/String;I)V}. */
    public static void putLongPf(Object shell, long value, String className, int offset) {
        putLong(shell, className, offset, value);
    }

    public static boolean getBoolean(Object shell, String className, int offset) {
        return buffer(shell, className).get(offset) != 0;
    }

    public static void putBoolean(Object shell, String className, int offset, boolean value) {
        buffer(shell, className).put(offset, (byte) (value ? 1 : 0));
    }

    public static void putBooleanPf(Object shell, boolean value, String className, int offset) {
        putBoolean(shell, className, offset, value);
    }

    public static byte getByte(Object shell, String className, int offset) {
        return buffer(shell, className).get(offset);
    }

    public static void putByte(Object shell, String className, int offset, byte value) {
        buffer(shell, className).put(offset, value);
    }

    public static void putBytePf(Object shell, byte value, String className, int offset) {
        putByte(shell, className, offset, value);
    }

    public static short getShort(Object shell, String className, int offset) {
        return buffer(shell, className).getShort(offset);
    }

    public static void putShort(Object shell, String className, int offset, short value) {
        buffer(shell, className).putShort(offset, value);
    }

    public static void putShortPf(Object shell, short value, String className, int offset) {
        putShort(shell, className, offset, value);
    }

    public static char getChar(Object shell, String className, int offset) {
        return buffer(shell, className).getChar(offset);
    }

    public static void putChar(Object shell, String className, int offset, char value) {
        buffer(shell, className).putChar(offset, value);
    }

    public static void putCharPf(Object shell, char value, String className, int offset) {
        putChar(shell, className, offset, value);
    }

    public static float getFloat(Object shell, String className, int offset) {
        return buffer(shell, className).getFloat(offset);
    }

    public static void putFloat(Object shell, String className, int offset, float value) {
        buffer(shell, className).putFloat(offset, value);
    }

    public static void putFloatPf(Object shell, float value, String className, int offset) {
        putFloat(shell, className, offset, value);
    }

    public static double getDouble(Object shell, String className, int offset) {
        return buffer(shell, className).getDouble(offset);
    }

    public static void putDouble(Object shell, String className, int offset, double value) {
        buffer(shell, className).putDouble(offset, value);
    }

    public static void putDoublePf(Object shell, double value, String className, int offset) {
        putDouble(shell, className, offset, value);
    }

    private static ByteBuffer buffer(Object shell, String className) {
        long id = ensureRegion(shell, className);
        if (id >= FallbackArena.MIN_ID) {
            return FallbackArena.buffer(id).order(ByteOrder.LITTLE_ENDIAN);
        }
        OffHeapBridge b = bridge();
        if (b != null && id > 0L) {
            ByteBuffer buf = b.asByteBuffer(id);
            if (buf != null) {
                return buf.order(ByteOrder.LITTLE_ENDIAN);
            }
            // Native region disappeared (freed) — force reallocate once.
            writeRegionField(shell, 0L);
            id = ensureRegion(shell, className);
            if (id >= FallbackArena.MIN_ID) {
                return FallbackArena.buffer(id).order(ByteOrder.LITTLE_ENDIAN);
            }
            buf = b.asByteBuffer(id);
            if (buf != null) {
                return buf.order(ByteOrder.LITTLE_ENDIAN);
            }
        }
        throw new IllegalStateException(
                "off-heap region unavailable for " + className + " (id=" + id + ")");
    }

    private static OffHeapBridge bridge() {
        OffHeapBridge local = bridge;
        if (local == null) {
            try {
                local = JavaRAgent.getOffHeap();
            } catch (Throwable t) {
                local = OffHeapBridgeFactory.get();
            }
            bridge = local;
        }
        return local;
    }

    private static long readRegionField(Object shell) {
        try {
            java.lang.reflect.Field f = shell.getClass().getDeclaredField(REGION_FIELD);
            f.setAccessible(true);
            return f.getLong(shell);
        } catch (Throwable t) {
            return 0L;
        }
    }

    private static void writeRegionField(Object shell, long id) {
        try {
            java.lang.reflect.Field f = shell.getClass().getDeclaredField(REGION_FIELD);
            f.setAccessible(true);
            f.setLong(shell, id);
        } catch (Throwable t) {
            // Field may be missing before transform completes; caller retries on access.
        }
    }

    private static void track(Object shell, long regionId, int size) {
        startCleaner();
        WEAK_REGIONS.put(new WeakRef(shell, QUEUE, regionId, size), regionId);
    }

    private static synchronized void startCleaner() {
        if (cleanerStarted) {
            return;
        }
        cleanerStarted = true;
        Thread t = new Thread(new Runnable() {
            @Override
            public void run() {
                for (; ; ) {
                    try {
                        WeakRef ref = (WeakRef) QUEUE.remove();
                        WEAK_REGIONS.remove(ref);
                        freeRegion(ref.regionId, ref.size);
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        return;
                    }
                }
            }
        }, "javar-managed-cleaner");
        t.setDaemon(true);
        t.start();
    }

    private static void freeRegion(long id, int size) {
        if (id == 0L) {
            return;
        }
        OffHeapBridge b = bridge();
        if (b != null && id < FallbackArena.MIN_ID) {
            b.free(id);
        } else {
            FallbackArena.free(id);
        }
        BYTES_OFF_HEAP.addAndGet(-size);
        REGION_COUNT.decrementAndGet();
    }

    private static final class WeakRef extends WeakReference<Object> {
        final long regionId;
        final int size;

        WeakRef(Object referent, ReferenceQueue<Object> q, long regionId, int size) {
            super(referent, q);
            this.regionId = regionId;
            this.size = size;
        }
    }

    /**
     * Off-heap fallback when native {@code javar_core} is not loaded — still
     * uses direct buffers (outside the Java heap).
     */
    static final class FallbackArena {
        static final long MIN_ID = 1L << 62;
        private static final AtomicLong NEXT = new AtomicLong(MIN_ID + 1);
        private static final Map<Long, ByteBuffer> BUFS = new ConcurrentHashMap<Long, ByteBuffer>();

        static long allocate(int size) {
            long id = NEXT.getAndIncrement();
            ByteBuffer buf = ByteBuffer.allocateDirect(size).order(ByteOrder.LITTLE_ENDIAN);
            BUFS.put(id, buf);
            return id;
        }

        static ByteBuffer buffer(long id) {
            ByteBuffer buf = BUFS.get(id);
            if (buf == null) {
                throw new IllegalStateException("unknown fallback region " + id);
            }
            return buf;
        }

        static void free(long id) {
            BUFS.remove(id);
        }
    }
}

package com.javar.agent.memory;

import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Selects Project Panama (Java 22+) or JNI fallback for zero-copy off-heap access.
 */
public final class OffHeapBridgeFactory {

    private static final Logger LOG = Logger.getLogger(OffHeapBridgeFactory.class.getName());
    private static final Object LOCK = new Object();
    private static volatile OffHeapBridge INSTANCE;

    private OffHeapBridgeFactory() {
    }

    public static OffHeapBridge get() {
        OffHeapBridge local = INSTANCE;
        if (local != null) {
            return local;
        }
        synchronized (LOCK) {
            if (INSTANCE == null) {
                INSTANCE = create();
            }
            return INSTANCE;
        }
    }

    /** True when a native-backed bridge is available. */
    public static boolean isNativeAvailable() {
        OffHeapBridge b = get();
        return !(b instanceof NoopOffHeapBridge);
    }

    static OffHeapBridge create() {
        if (!NativeLibraryLoader.tryLoad()) {
            LOG.warning("Using no-op off-heap bridge (native library missing)");
            return new NoopOffHeapBridge();
        }

        int feature = javaFeatureVersion();
        if (feature >= 22) {
            OffHeapBridge panama = tryPanama();
            if (panama != null) {
                LOG.info("JavaR off-heap bridge: Project Panama (Java " + feature + ")");
                return panama;
            }
            LOG.log(Level.INFO, "Panama bridge unavailable; falling back to JNI");
        } else {
            LOG.info("JavaR off-heap bridge: JNI (Java " + feature + "; Panama requires 22+)");
        }

        try {
            return new JniOffHeapBridge();
        } catch (UnsatisfiedLinkError e) {
            LOG.log(Level.WARNING, "JNI off-heap bridge failed", e);
            return new NoopOffHeapBridge();
        }
    }

    private static OffHeapBridge tryPanama() {
        try {
            Class<?> cls = Class.forName("com.javar.agent.memory.PanamaOffHeapBridge");
            Object instance = cls.getDeclaredConstructor().newInstance();
            return (OffHeapBridge) instance;
        } catch (ClassNotFoundException e) {
            // Multi-release classes absent (agent built on JDK < 22).
            return null;
        } catch (Throwable t) {
            LOG.log(Level.WARNING, "Failed to initialize PanamaOffHeapBridge", t);
            return null;
        }
    }

    /**
     * Java 8-compatible feature detection.
     * {@code java.specification.version} is {@code 1.8} on 8, {@code 11}/{@code 22} later.
     */
    static int javaFeatureVersion() {
        String v = System.getProperty("java.specification.version", "1.8");
        if (v.startsWith("1.")) {
            try {
                return Integer.parseInt(v.substring(2));
            } catch (NumberFormatException e) {
                return 8;
            }
        }
        try {
            int dot = v.indexOf('.');
            String major = dot < 0 ? v : v.substring(0, dot);
            return Integer.parseInt(major);
        } catch (NumberFormatException e) {
            return 8;
        }
    }
}

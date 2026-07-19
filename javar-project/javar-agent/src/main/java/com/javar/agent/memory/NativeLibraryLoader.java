package com.javar.agent.memory;

import java.io.File;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Loads {@code javar_core} ({@code javar_core.dll} / {@code libjavar_core.so|dylib}).
 * <p>
 * Override path with {@code -Djavar.native.path=/abs/path/to/lib} or
 * {@code JAVAR_NATIVE_PATH}.
 */
public final class NativeLibraryLoader {

    private static final Logger LOG = Logger.getLogger(NativeLibraryLoader.class.getName());
    private static final Object LOCK = new Object();
    private static volatile boolean loaded;
    private static volatile String loadError;

    private NativeLibraryLoader() {
    }

    public static boolean isLoaded() {
        return loaded;
    }

    public static String loadError() {
        return loadError;
    }

    public static void load() {
        if (loaded) {
            return;
        }
        synchronized (LOCK) {
            if (loaded) {
                return;
            }
            try {
                String explicit = System.getProperty("javar.native.path");
                if (explicit == null || explicit.isEmpty()) {
                    explicit = System.getenv("JAVAR_NATIVE_PATH");
                }
                if (explicit != null && !explicit.isEmpty()) {
                    System.load(new File(explicit).getAbsolutePath());
                    LOG.info("Loaded JavaR native library from " + explicit);
                } else {
                    System.loadLibrary("javar_core");
                    LOG.info("Loaded JavaR native library via java.library.path (javar_core)");
                }
                loaded = true;
                loadError = null;
            } catch (UnsatisfiedLinkError e) {
                loadError = e.getMessage();
                LOG.log(Level.WARNING,
                        "javar_core native library not loaded — off-heap bridge unavailable. "
                                + "Set -Djavar.native.path or java.library.path. Cause: "
                                + e.getMessage());
                throw e;
            }
        }
    }

    /** Attempt load without throwing; returns success. */
    public static boolean tryLoad() {
        try {
            load();
            return true;
        } catch (UnsatisfiedLinkError e) {
            return false;
        }
    }
}

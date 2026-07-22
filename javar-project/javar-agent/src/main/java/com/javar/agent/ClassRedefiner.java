package com.javar.agent;

import com.javar.agent.managed.ManagedClassTransformer;
import com.javar.agent.shadow.ShadowClassManager;

import java.lang.instrument.ClassDefinition;
import java.lang.instrument.Instrumentation;
import java.lang.instrument.UnmodifiableClassException;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Applies bytecode via {@link Instrumentation#redefineClasses}, falling back to
 * {@link ShadowClassManager} when the JVM rejects structural changes.
 */
public final class ClassRedefiner {

    private static final Logger LOG = Logger.getLogger(ClassRedefiner.class.getName());

    private final Instrumentation instrumentation;
    private final ShadowClassManager shadows;
    /** Last successfully applied bytecode per binary class name. */
    private final Map<String, byte[]> lastApplied = new ConcurrentHashMap<String, byte[]>();
    /** Previous version for instant rollback. */
    private final Map<String, byte[]> previous = new ConcurrentHashMap<String, byte[]>();

    public ClassRedefiner(Instrumentation instrumentation, ShadowClassManager shadows) {
        this.instrumentation = instrumentation;
        this.shadows = shadows;
    }

    public synchronized RedefineResult redefine(String className, byte[] bytecode) {
        return redefine(className, bytecode, false, 0, null);
    }

    public synchronized RedefineResult redefineStructural(
            String className, String shadowName, int version, byte[] bytecode) {
        return redefine(className, bytecode, true, version, shadowName);
    }

    private RedefineResult redefine(
            String className,
            byte[] bytecode,
            boolean forceShadow,
            int version,
            String shadowName) {
        if (className == null || bytecode == null || bytecode.length == 0) {
            return RedefineResult.fail("invalid redefine request");
        }

        // Match class-load rewrite so @JavaRManaged schema stays HotSwap-compatible.
        byte[] prepared = ManagedClassTransformer.prepareForHotReload(className, bytecode);

        // Reject IDE Java-23 (etc.) bytecode before HotSwap — surface SYNC ERROR immediately.
        Integer classMajor = classFileMajor(prepared);
        Integer runtimeMajor = runtimeJavaMajor();
        if (classMajor != null && runtimeMajor != null) {
            int bytecodeJava = classMajor.intValue() - 44;
            if (classMajor.intValue() >= 52 && bytecodeJava > runtimeMajor.intValue()) {
                return RedefineResult.versionMismatch(
                        "class major " + classMajor + " (Java " + bytecodeJava
                                + ") > runtime Java " + runtimeMajor);
            }
        }

        byte[] prior = lastApplied.get(className);
        if (prior != null) {
            previous.put(className, prior);
        }

        if (forceShadow) {
            String shadow = shadowName != null
                    ? shadowName
                    : className + "$JavaR_v" + Math.max(version, 1);
            ShadowClassManager.InstallResult installed =
                    shadows.install(className, shadow, version, prepared);
            if (installed.success) {
                lastApplied.put(className, prepared);
                return RedefineResult.ok(installed.message);
            }
            return RedefineResult.fail(installed.message);
        }

        Class<?> target = findLoadedClass(className);
        if (target == null) {
            target = tryLoadClass(className);
        }
        if (target == null) {
            lastApplied.put(className, prepared);
            LOG.info("Class not loaded yet; cached bytecode for " + className);
            if (looksLikeUserAppJvm()) {
                // Surface in Reload history (Pending) so edits to cold classes still show up.
                return RedefineResult.pending("cached (not loaded): " + className);
            }
            // Tooling JVM (Metals / language servers) — do not pollute history.
            return RedefineResult.cached("cached (not loaded): " + className);
        }

        if (!instrumentation.isModifiableClass(target)) {
            return RedefineResult.fail("class not modifiable: " + className);
        }

        try {
            instrumentation.redefineClasses(new ClassDefinition(target, prepared));
            lastApplied.put(className, prepared);
            LOG.info("Redefined " + className + " (" + prepared.length + " bytes)");
            return RedefineResult.ok("redefined: " + className);
        } catch (ClassNotFoundException e) {
            return RedefineResult.fail("class not found: " + className);
        } catch (UnmodifiableClassException e) {
            return RedefineResult.fail("unmodifiable: " + className);
        } catch (UnsupportedOperationException e) {
            LOG.log(Level.WARNING,
                    "HotSwap rejected structural change for " + className
                            + " — installing shadow class", e);
            String shadow = className + "$JavaR_v" + System.currentTimeMillis();
            ShadowClassManager.InstallResult installed =
                    shadows.install(className, shadow, 1, prepared);
            if (installed.success) {
                lastApplied.put(className, prepared);
                return RedefineResult.ok(installed.message);
            }
            return RedefineResult.fail("structural fallback failed: " + installed.message);
        } catch (UnsupportedClassVersionError e) {
            LOG.log(Level.SEVERE, "bytecode version mismatch for " + className, e);
            return RedefineResult.versionMismatch(
                    "UnsupportedClassVersionError: " + e.getMessage());
        } catch (Throwable e) {
            // Catch Errors too so the agent socket always returns a frame (no silent hang-up).
            String msg = e.getMessage() != null ? e.getMessage() : e.getClass().getSimpleName();
            if (isVersionMismatch(e, msg)) {
                LOG.log(Level.SEVERE, "bytecode version mismatch for " + className, e);
                return RedefineResult.versionMismatch(msg);
            }
            LOG.log(Level.SEVERE, "redefine failed for " + className, e);
            return RedefineResult.fail(msg);
        }
    }

    private static boolean isVersionMismatch(Throwable e, String msg) {
        if (e instanceof UnsupportedClassVersionError) {
            return true;
        }
        String m = msg == null ? "" : msg.toLowerCase();
        String name = e.getClass().getName().toLowerCase();
        return m.contains("unsupportedclassversion")
                || m.contains("class file version")
                || m.contains("compiled by a more recent version")
                || name.contains("unsupportedclassversion");
    }

    public synchronized RedefineResult rollback(String className) {
        byte[] prior = previous.remove(className);
        if (prior == null) {
            return RedefineResult.fail("no rollback snapshot for " + className);
        }
        return redefine(className, prior);
    }

    private static Integer classFileMajor(byte[] bytes) {
        if (bytes == null || bytes.length < 8) {
            return null;
        }
        if ((bytes[0] & 0xFF) != 0xCA
                || (bytes[1] & 0xFF) != 0xFE
                || (bytes[2] & 0xFF) != 0xBA
                || (bytes[3] & 0xFF) != 0xBE) {
            return null;
        }
        return Integer.valueOf(((bytes[6] & 0xFF) << 8) | (bytes[7] & 0xFF));
    }

    private static Integer runtimeJavaMajor() {
        String v = System.getProperty("java.specification.version", "");
        if (v.startsWith("1.")) {
            v = v.substring(2);
        }
        try {
            return Integer.valueOf(Integer.parseInt(v.split("\\.")[0]));
        } catch (Exception e) {
            return null;
        }
    }

    private Class<?> findLoadedClass(String className) {
        Class<?>[] loaded = instrumentation.getAllLoadedClasses();
        for (Class<?> c : loaded) {
            if (className.equals(c.getName())) {
                return c;
            }
        }
        return null;
    }

    /** Load via an application ClassLoader (Spring Boot) so first-edit redefine can HotSwap. */
    private Class<?> tryLoadClass(String className) {
        Class<?>[] loaded = instrumentation.getAllLoadedClasses();
        for (Class<?> c : loaded) {
            String n = c.getName();
            if (!(n.startsWith("com.javar.demo")
                    || n.contains("springframework.boot")
                    || (n.endsWith("Application") && !n.toLowerCase().contains("language")))) {
                continue;
            }
            ClassLoader cl = c.getClassLoader();
            if (cl == null) {
                continue;
            }
            try {
                Class<?> found = Class.forName(className, false, cl);
                LOG.info("Loaded " + className + " via " + cl.getClass().getName() + " for hot-reload");
                return found;
            } catch (ClassNotFoundException ignored) {
                // try next loader
            } catch (Throwable t) {
                LOG.log(Level.FINE, "tryLoadClass via " + cl + " failed", t);
            }
        }
        try {
            return Class.forName(className);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private boolean looksLikeUserAppJvm() {
        for (Class<?> c : instrumentation.getAllLoadedClasses()) {
            String n = c.getName();
            if (n.startsWith("com.javar.demo")) {
                return true;
            }
            if (n.endsWith("Application")
                    && !n.toLowerCase().contains("language")
                    && !n.contains("Launcher")) {
                return true;
            }
        }
        return false;
    }

    public static final class RedefineResult {
        public final boolean success;
        /** True when the class was not loaded; bytecode cached only — not a live reload. */
        public final boolean cachedOnly;
        /** True when cached for later — still counted in Reload history as Pending. */
        public final boolean pendingHistory;
        /** True when IDE bytecode is newer than the running JVM. */
        public final boolean versionMismatch;
        public final String message;

        private RedefineResult(
                boolean success,
                boolean cachedOnly,
                boolean pendingHistory,
                boolean versionMismatch,
                String message) {
            this.success = success;
            this.cachedOnly = cachedOnly;
            this.pendingHistory = pendingHistory;
            this.versionMismatch = versionMismatch;
            this.message = message;
        }

        public static RedefineResult ok(String message) {
            return new RedefineResult(true, false, false, false, message);
        }

        public static RedefineResult cached(String message) {
            return new RedefineResult(true, true, false, false, message);
        }

        public static RedefineResult pending(String message) {
            return new RedefineResult(true, true, true, false, message);
        }

        public static RedefineResult versionMismatch(String message) {
            return new RedefineResult(false, false, false, true, message);
        }

        public static RedefineResult fail(String message) {
            return new RedefineResult(false, false, false, false, message);
        }
    }
}

package com.javar.agent;

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

        byte[] prior = lastApplied.get(className);
        if (prior != null) {
            previous.put(className, prior);
        }

        if (forceShadow) {
            String shadow = shadowName != null
                    ? shadowName
                    : className + "$JavaR_v" + Math.max(version, 1);
            ShadowClassManager.InstallResult installed =
                    shadows.install(className, shadow, version, bytecode);
            if (installed.success) {
                lastApplied.put(className, bytecode);
                return RedefineResult.ok(installed.message);
            }
            return RedefineResult.fail(installed.message);
        }

        Class<?> target = findLoadedClass(className);
        if (target == null) {
            lastApplied.put(className, bytecode);
            LOG.info("Class not loaded yet; cached bytecode for " + className);
            return RedefineResult.ok("cached (not loaded): " + className);
        }

        if (!instrumentation.isModifiableClass(target)) {
            return RedefineResult.fail("class not modifiable: " + className);
        }

        try {
            instrumentation.redefineClasses(new ClassDefinition(target, bytecode));
            lastApplied.put(className, bytecode);
            LOG.info("Redefined " + className + " (" + bytecode.length + " bytes)");
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
                    shadows.install(className, shadow, 1, bytecode);
            if (installed.success) {
                lastApplied.put(className, bytecode);
                return RedefineResult.ok(installed.message);
            }
            return RedefineResult.fail("structural fallback failed: " + installed.message);
        } catch (Exception e) {
            LOG.log(Level.SEVERE, "redefine failed for " + className, e);
            return RedefineResult.fail(e.getMessage());
        }
    }

    public synchronized RedefineResult rollback(String className) {
        byte[] prior = previous.remove(className);
        if (prior == null) {
            return RedefineResult.fail("no rollback snapshot for " + className);
        }
        return redefine(className, prior);
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

    public static final class RedefineResult {
        public final boolean success;
        public final String message;

        private RedefineResult(boolean success, String message) {
            this.success = success;
            this.message = message;
        }

        public static RedefineResult ok(String message) {
            return new RedefineResult(true, message);
        }

        public static RedefineResult fail(String message) {
            return new RedefineResult(false, message);
        }
    }
}

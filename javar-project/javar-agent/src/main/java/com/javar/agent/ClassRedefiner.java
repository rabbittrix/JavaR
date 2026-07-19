package com.javar.agent;

import java.lang.instrument.ClassDefinition;
import java.lang.instrument.Instrumentation;
import java.lang.instrument.UnmodifiableClassException;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Applies bytecode to loaded classes via {@link Instrumentation#redefineClasses}.
 * Phase 1: method-body / compatible changes.
 * Phase 1.5+: custom classloader path for structural changes (fields/methods).
 */
public final class ClassRedefiner {

    private static final Logger LOG = Logger.getLogger(ClassRedefiner.class.getName());

    private final Instrumentation instrumentation;
    /** Last successfully applied bytecode per binary class name. */
    private final Map<String, byte[]> lastApplied = new ConcurrentHashMap<String, byte[]>();
    /** Previous version for instant rollback. */
    private final Map<String, byte[]> previous = new ConcurrentHashMap<String, byte[]>();

    public ClassRedefiner(Instrumentation instrumentation) {
        this.instrumentation = instrumentation;
    }

    public synchronized RedefineResult redefine(String className, byte[] bytecode) {
        if (className == null || bytecode == null || bytecode.length == 0) {
            return RedefineResult.fail("invalid redefine request");
        }

        Class<?> target = findLoadedClass(className);
        if (target == null) {
            // Class not yet loaded — stash for future redefine / custom loader (Phase 2).
            lastApplied.put(className, bytecode);
            LOG.info("Class not loaded yet; cached bytecode for " + className);
            return RedefineResult.ok("cached (not loaded): " + className);
        }

        if (!instrumentation.isModifiableClass(target)) {
            return RedefineResult.fail("class not modifiable: " + className);
        }

        byte[] prior = lastApplied.get(className);
        if (prior != null) {
            previous.put(className, prior);
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
            // Typical for structural changes under HotSwap — escalate to custom loader later.
            LOG.log(Level.WARNING, "Structural change rejected by JVM for " + className, e);
            return RedefineResult.fail("structural change unsupported (use JavaR classloader): "
                    + e.getMessage());
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

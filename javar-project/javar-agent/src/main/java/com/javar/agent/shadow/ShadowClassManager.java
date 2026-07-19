package com.javar.agent.shadow;

import net.bytebuddy.ByteBuddy;
import net.bytebuddy.dynamic.DynamicType;
import net.bytebuddy.dynamic.loading.ClassReloadingStrategy;
import net.bytebuddy.dynamic.scaffold.TypeValidation;
import net.bytebuddy.implementation.MethodDelegation;
import net.bytebuddy.matcher.ElementMatchers;

import java.lang.instrument.Instrumentation;
import java.lang.reflect.Method;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Structural hot-swap via <strong>shadow classes</strong>.
 *
 * <h3>Bypassing JVM redefine rules</h3>
 * HotSwap forbids changing a loaded class's field/method set. JavaR instead:
 * <ol>
 *   <li>Defines a <em>new</em> type {@code Original$JavaR_vN} with the new schema
 *       ({@code ClassLoader.defineClass} — always legal for a fresh name).</li>
 *   <li>Uses ByteBuddy to rewrite only the <em>method bodies</em> of {@code Original}
 *       so they trampoline into {@link JavaRDispatcher} (schema of Original unchanged
 *       → legal {@code redefineClasses} / reloading strategy).</li>
 *   <li>Per-instance shadow twins hold new fields; compatible fields are copied.</li>
 * </ol>
 * Rust assigns {@code vN} and tracks bytecode for instant rollback.
 */
public final class ShadowClassManager {

    private static final Logger LOG = Logger.getLogger(ShadowClassManager.class.getName());

    private final Instrumentation instrumentation;
    private final Map<String, ShadowInstall> installs = new ConcurrentHashMap<String, ShadowInstall>();

    public ShadowClassManager(Instrumentation instrumentation) {
        this.instrumentation = instrumentation;
    }

    /**
     * Install a structural update.
     *
     * @param className   original binary name
     * @param shadowName  {@code Original$JavaR_vN}
     * @param version     monotonic version from Rust
     * @param bytecode    compiled bytecode of the <em>new</em> schema (still named Original)
     */
    public synchronized InstallResult install(
            String className, String shadowName, int version, byte[] bytecode) {
        try {
            Class<?> original = findLoaded(className);
            ClassLoader loader = original != null
                    ? original.getClassLoader()
                    : ClassLoader.getSystemClassLoader();
            if (loader == null) {
                loader = ClassLoader.getSystemClassLoader();
            }

            byte[] renamed = ClassRenamer.rename(bytecode, className, shadowName);
            Class<?> shadow = defineClass(loader, shadowName, renamed);
            JavaRDispatcher.activate(className, shadow);

            if (original != null) {
                installDispatcher(original);
            } else {
                LOG.info("Original " + className + " not loaded yet; shadow defined, "
                        + "dispatcher will apply when the class is loaded");
            }

            installs.put(className, new ShadowInstall(shadowName, version, shadow, bytecode));
            LOG.info("Shadow installed: " + shadowName + " (v" + version + ")");
            return InstallResult.ok("shadow: " + shadowName);
        } catch (Throwable t) {
            LOG.log(Level.SEVERE, "shadow install failed for " + className, t);
            return InstallResult.fail(t.getMessage() == null ? t.toString() : t.getMessage());
        }
    }

    /**
     * Roll back to a previous shadow / bytecode snapshot supplied by Rust.
     */
    public synchronized InstallResult rollback(
            String className, String shadowName, int version, byte[] bytecode) {
        JavaRDispatcher.deactivate(className);
        if (shadowName != null && shadowName.equals(className)) {
            return InstallResult.ok("rollback-compatible:" + className);
        }
        return install(className, shadowName, version, bytecode);
    }

    private void installDispatcher(Class<?> original) throws Exception {
        DynamicType.Unloaded<?> unloaded = new ByteBuddy()
                .with(TypeValidation.DISABLED)
                .redefine(original)
                .method(ElementMatchers.isMethod()
                        .and(ElementMatchers.not(ElementMatchers.isSynthetic()))
                        .and(ElementMatchers.not(ElementMatchers.isBridge()))
                        .and(ElementMatchers.not(ElementMatchers.isNative()))
                        .and(ElementMatchers.not(ElementMatchers.named("getClass")))
                        .and(ElementMatchers.not(ElementMatchers.isHashCode()))
                        .and(ElementMatchers.not(ElementMatchers.isEquals()))
                        .and(ElementMatchers.not(ElementMatchers.isToString())))
                .intercept(MethodDelegation.to(JavaRDispatcher.class))
                .make();

        unloaded.load(original.getClassLoader(), ClassReloadingStrategy.of(instrumentation));
        LOG.info("Dispatcher wired into " + original.getName());
    }

    private Class<?> defineClass(ClassLoader loader, String name, byte[] bytecode)
            throws Exception {
        Method define = ClassLoader.class.getDeclaredMethod(
                "defineClass", String.class, byte[].class, int.class, int.class);
        define.setAccessible(true);
        try {
            return (Class<?>) define.invoke(loader, name, bytecode, 0, bytecode.length);
        } catch (Exception e) {
            try {
                return Class.forName(name, false, loader);
            } catch (ClassNotFoundException cnf) {
                throw e;
            }
        }
    }

    private Class<?> findLoaded(String className) {
        for (Class<?> c : instrumentation.getAllLoadedClasses()) {
            if (className.equals(c.getName())) {
                return c;
            }
        }
        return null;
    }

    public ShadowInstall current(String className) {
        return installs.get(className);
    }

    public static final class ShadowInstall {
        public final String shadowName;
        public final int version;
        public final Class<?> shadowClass;
        public final byte[] bytecode;

        ShadowInstall(String shadowName, int version, Class<?> shadowClass, byte[] bytecode) {
            this.shadowName = shadowName;
            this.version = version;
            this.shadowClass = shadowClass;
            this.bytecode = bytecode;
        }
    }

    public static final class InstallResult {
        public final boolean success;
        public final String message;

        private InstallResult(boolean success, String message) {
            this.success = success;
            this.message = message;
        }

        public static InstallResult ok(String message) {
            return new InstallResult(true, message);
        }

        public static InstallResult fail(String message) {
            return new InstallResult(false, message);
        }
    }
}

package com.javar.agent.shadow;

import net.bytebuddy.implementation.bind.annotation.AllArguments;
import net.bytebuddy.implementation.bind.annotation.Origin;
import net.bytebuddy.implementation.bind.annotation.RuntimeType;
import net.bytebuddy.implementation.bind.annotation.This;

import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Runtime trampoline installed into the <em>original</em> class method bodies.
 * <p>
 * Existing instances keep type {@code Original}; calls are forwarded to the
 * current shadow twin ({@code Original$JavaR_vN}) which holds the new schema.
 */
public final class JavaRDispatcher {

    private static final Logger LOG = Logger.getLogger(JavaRDispatcher.class.getName());

    /** original binary name → active shadow Class */
    private static final Map<String, Class<?>> ACTIVE_SHADOW = new ConcurrentHashMap<String, Class<?>>();

    /** identity of original instance → shadow twin instance */
    private static final Map<Object, Object> TWINS =
            Collections.synchronizedMap(new IdentityHashMap<Object, Object>());

    private JavaRDispatcher() {
    }

    public static void activate(String originalName, Class<?> shadowClass) {
        ACTIVE_SHADOW.put(originalName, shadowClass);
        // Drop twins so they are rebuilt against the new shadow type.
        TWINS.clear();
        LOG.info("Dispatcher active: " + originalName + " → " + shadowClass.getName());
    }

    public static void deactivate(String originalName) {
        ACTIVE_SHADOW.remove(originalName);
        TWINS.clear();
    }

    public static Class<?> activeShadow(String originalName) {
        return ACTIVE_SHADOW.get(originalName);
    }

    /**
     * ByteBuddy {@link net.bytebuddy.implementation.MethodDelegation} entry.
     */
    @RuntimeType
    public static Object intercept(
            @This(optional = true) Object self,
            @Origin Method method,
            @AllArguments Object[] args) throws Throwable {
        Class<?> declaring = method.getDeclaringClass();
        String originalName = declaring.getName();
        Class<?> shadow = ACTIVE_SHADOW.get(originalName);
        if (shadow == null) {
            throw new IllegalStateException("No active JavaR shadow for " + originalName);
        }

        if (Modifier.isStatic(method.getModifiers())) {
            Method target = findMethod(shadow, method.getName(), method.getParameterTypes());
            target.setAccessible(true);
            return target.invoke(null, args);
        }

        Object twin = twinOf(self, shadow, declaring);
        Method target = findMethod(shadow, method.getName(), method.getParameterTypes());
        target.setAccessible(true);
        return target.invoke(twin, args);
    }

    private static Object twinOf(Object original, Class<?> shadow, Class<?> originalClass)
            throws Exception {
        Object existing = TWINS.get(original);
        if (existing != null && shadow.isInstance(existing)) {
            return existing;
        }
        Object twin = newInstance(shadow);
        copyCompatibleFields(original, twin, originalClass, shadow);
        TWINS.put(original, twin);
        return twin;
    }

    private static Object newInstance(Class<?> shadow) throws Exception {
        Constructor<?> ctor = shadow.getDeclaredConstructor();
        ctor.setAccessible(true);
        return ctor.newInstance();
    }

    private static void copyCompatibleFields(
            Object from, Object to, Class<?> fromType, Class<?> toType) {
        for (Field src : fromType.getDeclaredFields()) {
            if (Modifier.isStatic(src.getModifiers())) {
                continue;
            }
            try {
                Field dst = toType.getDeclaredField(src.getName());
                if (dst.getType().equals(src.getType())) {
                    src.setAccessible(true);
                    dst.setAccessible(true);
                    dst.set(to, src.get(from));
                }
            } catch (NoSuchFieldException ignored) {
                // Field removed or only exists on shadow — leave default.
            } catch (Exception e) {
                LOG.log(Level.FINE, "field copy skipped: " + src.getName(), e);
            }
        }
    }

    private static Method findMethod(Class<?> type, String name, Class<?>[] params)
            throws NoSuchMethodException {
        try {
            return type.getDeclaredMethod(name, params);
        } catch (NoSuchMethodException e) {
            return type.getMethod(name, params);
        }
    }
}

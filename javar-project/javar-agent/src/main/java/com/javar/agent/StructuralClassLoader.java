package com.javar.agent;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Phase 1.5 / Phase 2 hook: custom classloader for structural hot-reload
 * (add/remove fields and methods) when {@code redefineClasses} is insufficient.
 *
 * Not wired into the default redefine path yet — reserved for GC-elimination era.
 */
public final class StructuralClassLoader extends ClassLoader {

    private final Map<String, byte[]> pending = new ConcurrentHashMap<String, byte[]>();

    public StructuralClassLoader(ClassLoader parent) {
        super(parent);
    }

    public void stage(String className, byte[] bytecode) {
        pending.put(className, bytecode);
    }

    @Override
    protected Class<?> findClass(String name) throws ClassNotFoundException {
        byte[] bytes = pending.remove(name);
        if (bytes == null) {
            throw new ClassNotFoundException(name);
        }
        return defineClass(name, bytes, 0, bytes.length);
    }
}

package com.javar.agent.shadow;

import net.bytebuddy.jar.asm.ClassReader;
import net.bytebuddy.jar.asm.ClassVisitor;
import net.bytebuddy.jar.asm.ClassWriter;
import net.bytebuddy.jar.asm.commons.ClassRemapper;
import net.bytebuddy.jar.asm.commons.Remapper;

/**
 * Renames a class (and internal references) to a shadow binary name using ASM.
 */
public final class ClassRenamer {

    private ClassRenamer() {
    }

    /**
     * @param bytecode     original {@code .class} bytes (new schema)
     * @param oldBinary    e.g. {@code com.example.MyService}
     * @param newBinary    e.g. {@code com.example.MyService$JavaR_v2}
     */
    public static byte[] rename(byte[] bytecode, String oldBinary, String newBinary) {
        final String oldInternal = oldBinary.replace('.', '/');
        final String newInternal = newBinary.replace('.', '/');

        ClassReader reader = new ClassReader(bytecode);
        ClassWriter writer = new ClassWriter(reader, 0);
        Remapper remapper = new Remapper() {
            @Override
            public String map(String internalName) {
                if (oldInternal.equals(internalName)) {
                    return newInternal;
                }
                return internalName;
            }
        };
        ClassVisitor visitor = new ClassRemapper(writer, remapper);
        reader.accept(visitor, 0);
        return writer.toByteArray();
    }
}

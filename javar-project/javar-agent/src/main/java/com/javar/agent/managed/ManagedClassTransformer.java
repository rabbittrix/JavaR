package com.javar.agent.managed;

import net.bytebuddy.jar.asm.AnnotationVisitor;
import net.bytebuddy.jar.asm.ClassReader;
import net.bytebuddy.jar.asm.ClassVisitor;
import net.bytebuddy.jar.asm.ClassWriter;
import net.bytebuddy.jar.asm.FieldVisitor;
import net.bytebuddy.jar.asm.MethodVisitor;
import net.bytebuddy.jar.asm.Opcodes;

import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.IllegalClassFormatException;
import java.security.ProtectionDomain;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Rewrites {@code @JavaRManaged} classes so primitive {@code GETFIELD}/{@code PUTFIELD}
 * hit {@link JavaRManagedRuntime} (Rust off-heap) instead of the Java heap.
 * <p>
 * Primitive instance fields are removed from the class schema; only a synthetic
 * {@code long __javar_region} shell field remains, cutting GC pressure.
 */
public final class ManagedClassTransformer implements ClassFileTransformer {

    private static final Logger LOG = Logger.getLogger(ManagedClassTransformer.class.getName());
    private static final String ANN = "Lcom/javar/agent/managed/JavaRManaged;";
    private static final String RUNTIME = "com/javar/agent/managed/JavaRManagedRuntime";

    /**
     * Apply the same rewrite used at class-load time so hot-reload bytecode matches
     * the already-transformed JVM schema (avoids HotSwap schema mismatch).
     *
     * @return transformed bytes, or the original buffer if the class is not {@code @JavaRManaged}
     */
    public static byte[] prepareForHotReload(String binaryClassName, byte[] classfileBuffer) {
        if (binaryClassName == null || classfileBuffer == null) {
            return classfileBuffer;
        }
        String internal = binaryClassName.replace('.', '/');
        try {
            // Already prepared (e.g. IDE pushed transformed bytes) — do not double-weave.
            if (alreadyTransformed(classfileBuffer)) {
                return classfileBuffer;
            }
            byte[] out = transformManaged(internal, classfileBuffer);
            return out != null ? out : classfileBuffer;
        } catch (Throwable t) {
            LOG.log(Level.WARNING, "prepareForHotReload failed for " + binaryClassName, t);
            return classfileBuffer;
        }
    }

    @Override
    public byte[] transform(
            ClassLoader loader,
            String className,
            Class<?> classBeingRedefined,
            ProtectionDomain protectionDomain,
            byte[] classfileBuffer) throws IllegalClassFormatException {
        if (className == null || classfileBuffer == null) {
            return null;
        }
        if (className.startsWith("com/javar/agent/")
                || className.startsWith("net/bytebuddy/")
                || className.startsWith("java/")
                || className.startsWith("javax/")
                || className.startsWith("sun/")
                || className.startsWith("jdk/")) {
            return null;
        }

        // CRITICAL: redefineClasses() re-invokes transformers. ClassRedefiner already
        // ran prepareForHotReload — transforming again double-weaves constructors and
        // causes VerifyError / 500 on the next `new Trade()`.
        if (classBeingRedefined != null) {
            return null;
        }

        try {
            if (alreadyTransformed(classfileBuffer)) {
                return null;
            }
            return transformManaged(className, classfileBuffer);
        } catch (Throwable t) {
            LOG.log(Level.WARNING, "JavaRManaged transform failed for " + className, t);
            return null;
        }
    }

    /** True when bytecode already has the synthetic region field (post-transform). */
    private static boolean alreadyTransformed(byte[] classfileBuffer) {
        try {
            ClassReader reader = new ClassReader(classfileBuffer);
            RegionProbe probe = new RegionProbe();
            reader.accept(probe, ClassReader.SKIP_CODE | ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
            return probe.hasRegion;
        } catch (Throwable t) {
            return false;
        }
    }

    /** @return rewritten bytes, or {@code null} if not a managed class / no change */
    private static byte[] transformManaged(String className, byte[] classfileBuffer) {
        ClassReader probe = new ClassReader(classfileBuffer);
        AnnotationProbe ap = new AnnotationProbe();
        probe.accept(ap, ClassReader.SKIP_CODE | ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
        if (!ap.managed) {
            return null;
        }

        ClassReader reader = new ClassReader(classfileBuffer);
        ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS);
        ManagedClassVisitor visitor = new ManagedClassVisitor(writer, className);
        reader.accept(visitor, ClassReader.EXPAND_FRAMES);

        FieldLayout layout = visitor.buildLayout();
        if (layout.totalSize() > 0) {
            JavaRManagedRuntime.registerLayout(layout);
        }

        byte[] out = writer.toByteArray();
        LOG.info("JavaRManaged transformed " + className.replace('/', '.')
                + " (" + layout.slots().size() + " off-heap fields, "
                + layout.totalSize() + " bytes/instance)");
        return out;
    }

    private static final class RegionProbe extends ClassVisitor {
        boolean hasRegion;

        RegionProbe() {
            super(Opcodes.ASM9);
        }

        @Override
        public FieldVisitor visitField(
                int access, String name, String descriptor, String signature, Object value) {
            if (JavaRManagedRuntime.REGION_FIELD.equals(name)
                    && JavaRManagedRuntime.REGION_DESC.equals(descriptor)) {
                hasRegion = true;
            }
            return null;
        }
    }

    private static final class AnnotationProbe extends ClassVisitor {
        boolean managed;

        AnnotationProbe() {
            super(Opcodes.ASM9);
        }

        @Override
        public AnnotationVisitor visitAnnotation(String descriptor, boolean visible) {
            if (ANN.equals(descriptor)) {
                managed = true;
            }
            return null;
        }
    }

    private static final class ManagedClassVisitor extends ClassVisitor {
        private final String internalName;
        private final String binaryName;
        private final List<PendingField> primitiveFields = new ArrayList<PendingField>();
        private boolean hasRegionField;

        ManagedClassVisitor(ClassVisitor cv, String internalName) {
            super(Opcodes.ASM9, cv);
            this.internalName = internalName;
            this.binaryName = internalName.replace('/', '.');
        }

        @Override
        public FieldVisitor visitField(
                int access, String name, String descriptor, String signature, Object value) {
            if (JavaRManagedRuntime.REGION_FIELD.equals(name)) {
                hasRegionField = true;
                return super.visitField(access, name, descriptor, signature, value);
            }

            boolean isStatic = (access & Opcodes.ACC_STATIC) != 0;
            int size = FieldLayout.sizeOf(descriptor);
            if (!isStatic && size > 0) {
                primitiveFields.add(new PendingField(name, descriptor, size));
                return null;
            }
            return super.visitField(access, name, descriptor, signature, value);
        }

        @Override
        public MethodVisitor visitMethod(
                int access, String name, String descriptor, String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, descriptor, signature, exceptions);
            if (mv == null) {
                return null;
            }
            // Snapshot fields known so far (javac emits fields before methods).
            MethodVisitor rewriter =
                    new FieldAccessRewriter(mv, internalName, binaryName, primitiveFields);
            if ("<init>".equals(name)) {
                return new ConstructorWeaver(rewriter, internalName, binaryName);
            }
            return rewriter;
        }

        @Override
        public void visitEnd() {
            if (!hasRegionField) {
                super.visitField(
                        Opcodes.ACC_PRIVATE | Opcodes.ACC_TRANSIENT | Opcodes.ACC_SYNTHETIC,
                        JavaRManagedRuntime.REGION_FIELD,
                        JavaRManagedRuntime.REGION_DESC,
                        null,
                        null);
            }
            super.visitEnd();
        }

        FieldLayout buildLayout() {
            Map<String, FieldLayout.FieldSlot> slots = new LinkedHashMap<String, FieldLayout.FieldSlot>();
            int offset = 0;
            for (PendingField f : primitiveFields) {
                offset = FieldLayout.align(offset, Math.min(f.size, 8));
                slots.put(f.name, new FieldLayout.FieldSlot(f.name, f.descriptor, offset, f.size));
                offset += f.size;
            }
            offset = FieldLayout.align(offset, 8);
            return new FieldLayout(binaryName, slots, Math.max(offset, 8));
        }
    }

    private static final class PendingField {
        final String name;
        final String descriptor;
        final int size;

        PendingField(String name, String descriptor, int size) {
            this.name = name;
            this.descriptor = descriptor;
            this.size = size;
        }
    }

    private static final class ConstructorWeaver extends MethodVisitor {
        private final String ownerInternal;
        private final String binaryName;
        private boolean done;

        ConstructorWeaver(MethodVisitor mv, String ownerInternal, String binaryName) {
            super(Opcodes.ASM9, mv);
            this.ownerInternal = ownerInternal;
            this.binaryName = binaryName;
        }

        @Override
        public void visitMethodInsn(
                int opcode, String owner, String name, String descriptor, boolean isInterface) {
            super.visitMethodInsn(opcode, owner, name, descriptor, isInterface);
            // Only after the direct superclass <init> — never nest on other invokespecials.
            if (!done
                    && opcode == Opcodes.INVOKESPECIAL
                    && "<init>".equals(name)
                    && !ownerInternal.equals(owner)) {
                // this.__javar_region = ensureRegion(this, className);
                mv.visitVarInsn(Opcodes.ALOAD, 0);
                mv.visitVarInsn(Opcodes.ALOAD, 0);
                mv.visitLdcInsn(binaryName);
                mv.visitMethodInsn(
                        Opcodes.INVOKESTATIC,
                        RUNTIME,
                        "ensureRegion",
                        "(Ljava/lang/Object;Ljava/lang/String;)J",
                        false);
                mv.visitFieldInsn(
                        Opcodes.PUTFIELD,
                        ownerInternal,
                        JavaRManagedRuntime.REGION_FIELD,
                        JavaRManagedRuntime.REGION_DESC);
                done = true;
            }
        }
    }

    /**
     * Rewrites field access without {@code LocalVariablesSorter} (not shipped inside byte-buddy.jar).
     * PUTFIELD uses {@code *Pf} helpers whose argument order matches the operand stack.
     */
    private static final class FieldAccessRewriter extends MethodVisitor {
        private final String ownerInternal;
        private final String binaryName;
        private final Map<String, PendingField> byName = new LinkedHashMap<String, PendingField>();

        FieldAccessRewriter(
                MethodVisitor mv,
                String ownerInternal,
                String binaryName,
                List<PendingField> fields) {
            super(Opcodes.ASM9, mv);
            this.ownerInternal = ownerInternal;
            this.binaryName = binaryName;
            for (PendingField f : fields) {
                byName.put(f.name, f);
            }
        }

        @Override
        public void visitFieldInsn(int opcode, String owner, String name, String descriptor) {
            if (!ownerInternal.equals(owner) || !byName.containsKey(name)) {
                super.visitFieldInsn(opcode, owner, name, descriptor);
                return;
            }

            int offset = localOffset(name);

            if (opcode == Opcodes.GETFIELD) {
                mv.visitLdcInsn(binaryName);
                mv.visitLdcInsn(offset);
                mv.visitMethodInsn(
                        Opcodes.INVOKESTATIC,
                        RUNTIME,
                        getterName(descriptor),
                        "(Ljava/lang/Object;Ljava/lang/String;I)" + descriptor,
                        false);
                return;
            }

            if (opcode == Opcodes.PUTFIELD) {
                mv.visitLdcInsn(binaryName);
                mv.visitLdcInsn(offset);
                mv.visitMethodInsn(
                        Opcodes.INVOKESTATIC,
                        RUNTIME,
                        setterPfName(descriptor),
                        setterPfDesc(descriptor),
                        false);
                return;
            }

            super.visitFieldInsn(opcode, owner, name, descriptor);
        }

        private int localOffset(String fieldName) {
            int offset = 0;
            for (PendingField f : byName.values()) {
                offset = FieldLayout.align(offset, Math.min(f.size, 8));
                if (f.name.equals(fieldName)) {
                    return offset;
                }
                offset += f.size;
            }
            return 0;
        }

        private static String getterName(String descriptor) {
            switch (descriptor.charAt(0)) {
                case 'Z':
                    return "getBoolean";
                case 'B':
                    return "getByte";
                case 'C':
                    return "getChar";
                case 'S':
                    return "getShort";
                case 'I':
                    return "getInt";
                case 'J':
                    return "getLong";
                case 'F':
                    return "getFloat";
                case 'D':
                    return "getDouble";
                default:
                    throw new IllegalArgumentException(descriptor);
            }
        }

        private static String setterPfName(String descriptor) {
            switch (descriptor.charAt(0)) {
                case 'Z':
                    return "putBooleanPf";
                case 'B':
                    return "putBytePf";
                case 'C':
                    return "putCharPf";
                case 'S':
                    return "putShortPf";
                case 'I':
                    return "putIntPf";
                case 'J':
                    return "putLongPf";
                case 'F':
                    return "putFloatPf";
                case 'D':
                    return "putDoublePf";
                default:
                    throw new IllegalArgumentException(descriptor);
            }
        }

        private static String setterPfDesc(String descriptor) {
            return "(Ljava/lang/Object;" + descriptor + "Ljava/lang/String;I)V";
        }
    }
}

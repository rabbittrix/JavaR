package com.javar.agent.memory;

import java.nio.ByteBuffer;

/**
 * JNI fallback (Java 8–21): zero-copy via {@code NewDirectByteBuffer} into Rust regions.
 */
public final class JniOffHeapBridge implements OffHeapBridge {

    static {
        NativeLibraryLoader.load();
    }

    public JniOffHeapBridge() {
        // ensure natives linked
        nativeManagedBytes();
    }

    @Override
    public String backend() {
        return "jni";
    }

    @Override
    public long allocate(long size, long align) {
        return nativeAlloc(size, align);
    }

    @Override
    public boolean free(long regionId) {
        return nativeFree(regionId);
    }

    @Override
    public long address(long regionId) {
        return nativeAddress(regionId);
    }

    @Override
    public long size(long regionId) {
        return nativeSize(regionId);
    }

    @Override
    public ByteBuffer asByteBuffer(long regionId) {
        ByteBuffer buf = nativeAsDirectBuffer(regionId);
        if (buf == null) {
            return null;
        }
        return buf;
    }

    @Override
    public boolean write(long regionId, long offset, byte[] data) {
        return nativeWrite(regionId, offset, data);
    }

    @Override
    public long managedBytes() {
        return nativeManagedBytes();
    }

    @Override
    public int abiVersion() {
        // JNI path does not require a separate symbol; keep in sync with Rust = 1.
        return 1;
    }

    private static native long nativeAlloc(long size, long align);

    private static native boolean nativeFree(long id);

    private static native long nativeAddress(long id);

    private static native long nativeSize(long id);

    private static native long nativeManagedBytes();

    private static native ByteBuffer nativeAsDirectBuffer(long id);

    private static native boolean nativeWrite(long id, long offset, byte[] data);
}

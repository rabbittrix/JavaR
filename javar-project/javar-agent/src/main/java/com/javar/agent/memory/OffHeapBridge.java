package com.javar.agent.memory;

import java.nio.ByteBuffer;

/**
 * Zero-copy off-heap memory owned by the Rust {@code javar_core} library.
 * <p>
 * Java 22+ prefers Project Panama ({@code PanamaOffHeapBridge}); older JDKs
 * fall back to JNI DirectByteBuffer ({@code JniOffHeapBridge}).
 */
public interface OffHeapBridge {

    /** {@code "panama"} or {@code "jni"}. */
    String backend();

    /**
     * Allocate {@code size} bytes (alignment typically 8).
     *
     * @return region id, or {@code 0} on failure
     */
    long allocate(long size, long align);

    /** Release a region. Views into it must not be used afterwards. */
    boolean free(long regionId);

    /** Native address of the region (for diagnostics / advanced FFM use). */
    long address(long regionId);

    /** Byte length of the region. */
    long size(long regionId);

    /**
     * Zero-copy view of Rust memory as a direct {@link ByteBuffer}.
     * Capacity equals {@link #size(long)}; do not use after {@link #free(long)}.
     */
    ByteBuffer asByteBuffer(long regionId);

    /** Copy {@code data} into the region at {@code offset}. */
    boolean write(long regionId, long offset, byte[] data);

    /** Total bytes currently held off-heap by Rust. */
    long managedBytes();

    /** Native ABI version negotiated with {@code javar_core}. */
    int abiVersion();
}

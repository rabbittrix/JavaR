package com.javar.agent.memory;

import java.nio.ByteBuffer;

/** Placeholder when {@code javar_core} cannot be loaded. */
final class NoopOffHeapBridge implements OffHeapBridge {

    @Override
    public String backend() {
        return "noop";
    }

    @Override
    public long allocate(long size, long align) {
        return 0;
    }

    @Override
    public boolean free(long regionId) {
        return false;
    }

    @Override
    public long address(long regionId) {
        return 0;
    }

    @Override
    public long size(long regionId) {
        return 0;
    }

    @Override
    public ByteBuffer asByteBuffer(long regionId) {
        return null;
    }

    @Override
    public boolean write(long regionId, long offset, byte[] data) {
        return false;
    }

    @Override
    public long managedBytes() {
        return 0;
    }

    @Override
    public int abiVersion() {
        return 0;
    }
}

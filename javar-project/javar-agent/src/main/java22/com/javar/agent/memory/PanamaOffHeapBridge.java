package com.javar.agent.memory;

import java.lang.foreign.AddressLayout;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.nio.ByteBuffer;
import java.nio.file.Path;
import java.util.Objects;

/**
 * Project Panama / FFM bridge (Java 22+): downcalls into {@code javar_core} and
 * wraps Rust regions as zero-copy {@link MemorySegment} / {@link ByteBuffer}.
 * <p>
 * Packaged as a Multi-Release JAR entry under {@code META-INF/versions/22/}.
 */
public final class PanamaOffHeapBridge implements OffHeapBridge {

    private static final ValueLayout.OfLong C_LONG = ValueLayout.JAVA_LONG;
    private static final ValueLayout.OfInt C_INT = ValueLayout.JAVA_INT;
    /** Java 22+: {@code ValueLayout.OfAddress} was replaced by {@link AddressLayout}. */
    private static final AddressLayout C_POINTER = ValueLayout.ADDRESS;

    private final Arena arena;
    private final MethodHandle alloc;
    private final MethodHandle free;
    private final MethodHandle ptr;
    private final MethodHandle len;
    private final MethodHandle write;
    private final MethodHandle managed;
    private final MethodHandle abiVersionMh;

    public PanamaOffHeapBridge() {
        NativeLibraryLoader.load();
        this.arena = Arena.ofAuto();
        Linker linker = Linker.nativeLinker();
        SymbolLookup lookup = libraryLookup(arena);

        try {
            this.alloc = linker.downcallHandle(
                    require(lookup, "javar_mem_alloc"),
                    FunctionDescriptor.of(C_LONG, C_LONG, C_LONG));
            this.free = linker.downcallHandle(
                    require(lookup, "javar_mem_free"),
                    FunctionDescriptor.of(C_INT, C_LONG));
            this.ptr = linker.downcallHandle(
                    require(lookup, "javar_mem_ptr"),
                    FunctionDescriptor.of(C_POINTER, C_LONG));
            this.len = linker.downcallHandle(
                    require(lookup, "javar_mem_len"),
                    FunctionDescriptor.of(C_LONG, C_LONG));
            this.write = linker.downcallHandle(
                    require(lookup, "javar_mem_write"),
                    FunctionDescriptor.of(C_INT, C_LONG, C_LONG, C_POINTER, C_LONG));
            this.managed = linker.downcallHandle(
                    require(lookup, "javar_mem_managed_bytes"),
                    FunctionDescriptor.of(C_LONG));
            this.abiVersionMh = linker.downcallHandle(
                    require(lookup, "javar_mem_abi_version"),
                    FunctionDescriptor.of(ValueLayout.JAVA_INT));
        } catch (Throwable t) {
            throw new ExceptionInInitializerError(t);
        }
    }

    private static SymbolLookup libraryLookup(Arena arena) {
        String explicit = System.getProperty("javar.native.path");
        if (explicit == null || explicit.isEmpty()) {
            explicit = System.getenv("JAVAR_NATIVE_PATH");
        }
        if (explicit != null && !explicit.isEmpty()) {
            return SymbolLookup.libraryLookup(Path.of(explicit), arena);
        }
        // Already loaded via System.loadLibrary — resolve from global namespace.
        return SymbolLookup.loaderLookup().or(Linker.nativeLinker().defaultLookup());
    }

    private static MemorySegment require(SymbolLookup lookup, String name) {
        return lookup.find(name).orElseThrow(
                () -> new UnsatisfiedLinkError("missing native symbol: " + name));
    }

    @Override
    public String backend() {
        return "panama";
    }

    @Override
    public long allocate(long size, long align) {
        try {
            return (long) alloc.invokeExact(size, align <= 0 ? 8L : align);
        } catch (Throwable t) {
            throw new RuntimeException("javar_mem_alloc failed", t);
        }
    }

    @Override
    public boolean free(long regionId) {
        try {
            return ((int) free.invokeExact(regionId)) != 0;
        } catch (Throwable t) {
            throw new RuntimeException("javar_mem_free failed", t);
        }
    }

    @Override
    public long address(long regionId) {
        MemorySegment seg = asSegment(regionId);
        return seg == null ? 0L : seg.address();
    }

    @Override
    public long size(long regionId) {
        try {
            return (long) len.invokeExact(regionId);
        } catch (Throwable t) {
            throw new RuntimeException("javar_mem_len failed", t);
        }
    }

    /**
     * Zero-copy FFM view of the Rust region. Lifetime is tied to this bridge's arena;
     * do not free the region while segments/buffers derived from it are in use.
     */
    public MemorySegment asSegment(long regionId) {
        try {
            MemorySegment address = (MemorySegment) ptr.invokeExact(regionId);
            long bytes = (long) len.invokeExact(regionId);
            if (address.address() == 0L || bytes <= 0L) {
                return null;
            }
            return address.reinterpret(bytes, arena, null);
        } catch (Throwable t) {
            throw new RuntimeException("panama asSegment failed", t);
        }
    }

    @Override
    public ByteBuffer asByteBuffer(long regionId) {
        MemorySegment seg = asSegment(regionId);
        if (seg == null) {
            return null;
        }
        return seg.asByteBuffer();
    }

    @Override
    public boolean write(long regionId, long offset, byte[] data) {
        Objects.requireNonNull(data, "data");
        try (Arena scratch = Arena.ofConfined()) {
            MemorySegment src = scratch.allocateFrom(ValueLayout.JAVA_BYTE, data);
            int ok = (int) write.invokeExact(regionId, offset, src, (long) data.length);
            return ok != 0;
        } catch (Throwable t) {
            throw new RuntimeException("javar_mem_write failed", t);
        }
    }

    @Override
    public long managedBytes() {
        try {
            return (long) managed.invokeExact();
        } catch (Throwable t) {
            throw new RuntimeException("javar_mem_managed_bytes failed", t);
        }
    }

    @Override
    public int abiVersion() {
        try {
            return (int) abiVersionMh.invokeExact();
        } catch (Throwable t) {
            return 1;
        }
    }
}

package com.javar.agent;

import com.javar.agent.managed.JavaRManagedRuntime;
import com.javar.agent.memory.OffHeapBridge;

import java.lang.instrument.Instrumentation;
import java.lang.management.ManagementFactory;
import java.lang.management.MemoryMXBean;
import java.lang.management.MemoryUsage;
import java.nio.charset.StandardCharsets;

/**
 * Reports Java heap usage vs. JavaR-managed off-heap memory (Panama/JNI).
 */
public final class TelemetryReporter {

    private final Instrumentation instrumentation;
    private final MemoryMXBean memoryMXBean = ManagementFactory.getMemoryMXBean();
    private final OffHeapBridge offHeap;

    /** Manual override when native bridge is unavailable. */
    private volatile long javarManagedBytesOverride = -1L;

    public TelemetryReporter(
            Instrumentation instrumentation,
            java.util.concurrent.atomic.AtomicLong reloadCount,
            OffHeapBridge offHeap) {
        this.instrumentation = instrumentation;
        this.offHeap = offHeap;
    }

    /** Backward-compatible ctor used by tests / older call sites. */
    public TelemetryReporter(
            Instrumentation instrumentation,
            java.util.concurrent.atomic.AtomicLong reloadCount) {
        this(instrumentation, reloadCount, null);
    }

    public void setJavarManagedBytes(long bytes) {
        this.javarManagedBytesOverride = Math.max(0, bytes);
    }

    public byte[] snapshotJson(long reloadCount) {
        MemoryUsage heap = memoryMXBean.getHeapMemoryUsage();
        long loaded = instrumentation != null ? instrumentation.getAllLoadedClasses().length : 0;
        long managed = resolveManagedBytes();
        String backend = offHeap != null ? offHeap.backend() : "none";
        String json = "{"
                + "\"java_heap_used\":" + heap.getUsed() + ","
                + "\"java_heap_max\":" + heap.getMax() + ","
                + "\"javar_managed\":" + managed + ","
                + "\"gc_savings\":" + JavaRManagedRuntime.gcSavingsBytes() + ","
                + "\"managed_regions\":" + JavaRManagedRuntime.regionCount() + ","
                + "\"reload_count\":" + reloadCount + ","
                + "\"loaded_classes\":" + loaded + ","
                + "\"offheap_backend\":\"" + backend + "\""
                + "}";
        return json.getBytes(StandardCharsets.UTF_8);
    }

    private long resolveManagedBytes() {
        if (javarManagedBytesOverride >= 0) {
            return javarManagedBytesOverride;
        }
        long fromRuntime = JavaRManagedRuntime.managedBytes();
        if (offHeap != null) {
            try {
                return Math.max(fromRuntime, offHeap.managedBytes());
            } catch (Throwable ignored) {
                return fromRuntime;
            }
        }
        return fromRuntime;
    }
}

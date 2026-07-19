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
        String project = resolveProjectName();
        String json = "{"
                + "\"java_heap_used\":" + heap.getUsed() + ","
                + "\"java_heap_max\":" + heap.getMax() + ","
                + "\"javar_managed\":" + managed + ","
                + "\"gc_savings\":" + JavaRManagedRuntime.gcSavingsBytes() + ","
                + "\"managed_regions\":" + JavaRManagedRuntime.regionCount() + ","
                + "\"reload_count\":" + reloadCount + ","
                + "\"loaded_classes\":" + loaded + ","
                + "\"offheap_backend\":\"" + escapeJson(backend) + "\","
                + "\"project_name\":\"" + escapeJson(project) + "\""
                + "}";
        return json.getBytes(StandardCharsets.UTF_8);
    }

    private static String resolveProjectName() {
        String prop = System.getProperty("javar.project.name");
        if (prop != null && !prop.isEmpty()) {
            return prop;
        }
        String env = System.getenv("JAVAR_PROJECT_NAME");
        if (env != null && !env.isEmpty()) {
            return env;
        }
        // Fall back to main class simple name when available.
        for (StackTraceElement el : Thread.currentThread().getStackTrace()) {
            // ignore
        }
        String cmd = System.getProperty("sun.java.command", "");
        if (!cmd.isEmpty()) {
            String first = cmd.split("\\s+")[0];
            if (first.endsWith(".jar")) {
                int slash = Math.max(first.lastIndexOf('/'), first.lastIndexOf('\\'));
                return slash >= 0 ? first.substring(slash + 1) : first;
            }
            int dot = first.lastIndexOf('.');
            return dot >= 0 ? first.substring(dot + 1) : first;
        }
        return "java-app";
    }

    private static String escapeJson(String s) {
        if (s == null) {
            return "";
        }
        return s.replace("\\", "\\\\").replace("\"", "\\\"");
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

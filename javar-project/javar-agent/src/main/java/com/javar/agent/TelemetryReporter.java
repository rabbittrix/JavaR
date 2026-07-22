package com.javar.agent;

import com.javar.agent.managed.JavaRManagedRuntime;
import com.javar.agent.memory.OffHeapBridge;

import java.lang.instrument.Instrumentation;
import java.lang.management.ManagementFactory;
import java.lang.management.MemoryMXBean;
import java.lang.management.MemoryUsage;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.Deque;

/**
 * Reports Java heap usage vs. JavaR-managed off-heap memory (Panama/JNI),
 * plus a ring buffer of hot-reload events for the Control Center.
 */
public final class TelemetryReporter {

    /** Ring buffer size — Dashboard receives these immediately on late connect. */
    private static final int HISTORY_CAP = 64;

    private final Instrumentation instrumentation;
    private final MemoryMXBean memoryMXBean = ManagementFactory.getMemoryMXBean();
    private final OffHeapBridge offHeap;
    private final Deque<ReloadEvent> history = new ArrayDeque<ReloadEvent>();

    /** Manual override when native bridge is unavailable. */
    private volatile long javarManagedBytesOverride = -1L;

    /** Sticky Control Center alert (e.g. IDE Java 23 vs runtime Java 21). */
    private volatile String syncAlert = "";

    public TelemetryReporter(
            Instrumentation instrumentation,
            java.util.concurrent.atomic.AtomicLong reloadCount,
            OffHeapBridge offHeap) {
        this.instrumentation = instrumentation;
        this.offHeap = offHeap;
    }

    public TelemetryReporter(
            Instrumentation instrumentation,
            java.util.concurrent.atomic.AtomicLong reloadCount) {
        this(instrumentation, reloadCount, null);
    }

    public void setJavarManagedBytes(long bytes) {
        this.javarManagedBytesOverride = Math.max(0, bytes);
    }

    public void setSyncAlert(String message) {
        this.syncAlert = message == null ? "" : message;
    }

    public void clearSyncAlert() {
        this.syncAlert = "";
    }

    public String getSyncAlert() {
        return syncAlert;
    }

    public synchronized void recordReload(String className, String changeType, int version) {
        recordReload(className, changeType, version, System.currentTimeMillis());
    }

    public synchronized void recordReload(String className, String changeType, int version, long tsMs) {
        history.addFirst(new ReloadEvent(
                tsMs,
                className == null ? "?" : className,
                changeType == null ? "Body" : changeType,
                version));
        while (history.size() > HISTORY_CAP) {
            history.removeLast();
        }
    }

    public byte[] snapshotJson(long reloadCount) {
        MemoryUsage heap = memoryMXBean.getHeapMemoryUsage();
        long loaded = instrumentation != null ? instrumentation.getAllLoadedClasses().length : 0;
        long managed = resolveManagedBytes();
        String backend = offHeap != null ? offHeap.backend() : "none";
        String project = resolveProjectName();
        String histJson = historyJson();
        long pid = AgentRegistry.currentPid();
        String cmd = System.getProperty("sun.java.command", "");
        if (cmd.length() > 240) {
            cmd = cmd.substring(0, 237) + "...";
        }
        // MemoryMXBean#getMax can be -1 ("undefined") — never emit negatives (breaks Rust u64 JSON).
        long heapUsed = Math.max(0L, heap.getUsed());
        long heapMax = heap.getMax();
        if (heapMax < 0L) {
            heapMax = Math.max(heap.getCommitted(), heapUsed);
        }
        if (heapMax < heapUsed) {
            heapMax = heapUsed;
        }
        String json = "{"
                + "\"java_heap_used\":" + heapUsed + ","
                + "\"java_heap_max\":" + heapMax + ","
                + "\"javar_managed\":" + Math.max(0L, managed) + ","
                + "\"gc_savings\":" + Math.max(0L, JavaRManagedRuntime.gcSavingsBytes()) + ","
                + "\"managed_regions\":" + Math.max(0L, JavaRManagedRuntime.regionCount()) + ","
                + "\"reload_count\":" + Math.max(0L, reloadCount) + ","
                + "\"loaded_classes\":" + Math.max(0L, loaded) + ","
                + "\"offheap_backend\":\"" + escapeJson(backend) + "\","
                + "\"project_name\":\"" + escapeJson(project) + "\","
                + "\"pid\":" + pid + ","
                + "\"jvm_cmd\":\"" + escapeJson(cmd) + "\","
                + "\"sync_alert\":\"" + escapeJson(syncAlert) + "\","
                + "\"reload_history\":" + histJson
                + "}";
        return json.getBytes(StandardCharsets.UTF_8);
    }

    private synchronized String historyJson() {
        StringBuilder sb = new StringBuilder();
        sb.append('[');
        boolean first = true;
        for (ReloadEvent e : history) {
            if (!first) {
                sb.append(',');
            }
            first = false;
            sb.append('{')
                    .append("\"ts\":").append(e.tsMs).append(',')
                    .append("\"class_name\":\"").append(escapeJson(e.className)).append("\",")
                    .append("\"change_type\":\"").append(escapeJson(e.changeType)).append("\",")
                    .append("\"version\":").append(e.version)
                    .append('}');
        }
        sb.append(']');
        return sb.toString();
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

    static final class ReloadEvent {
        final long tsMs;
        final String className;
        final String changeType;
        final int version;

        ReloadEvent(long tsMs, String className, String changeType, int version) {
            this.tsMs = tsMs;
            this.className = className;
            this.changeType = changeType;
            this.version = version;
        }
    }
}

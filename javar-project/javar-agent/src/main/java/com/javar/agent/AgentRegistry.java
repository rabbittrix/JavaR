package com.javar.agent;

import java.io.IOException;
import java.lang.management.ManagementFactory;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Registers a live JavaR agent under {@code ~/.javar/agents/<pid>.json}
 * so the Dashboard can discover and switch between multiple JVMs.
 * <p>
 * Schema: {@code {"pid":123,"name":"ProjectName","port":19223,"cmd":"..."}}
 *
 * @author Roberto de Souza (rabbittrix@hotmail.com)
 */
public final class AgentRegistry {

    private static final Logger LOG = Logger.getLogger(AgentRegistry.class.getName());
    private static final AtomicBoolean HOOK_INSTALLED = new AtomicBoolean(false);
    private static volatile long registeredPid = -1L;

    private AgentRegistry() {
    }

    public static Path agentsDir() {
        String override = System.getenv("JAVAR_HOME");
        Path root;
        if (override != null && !override.isEmpty()) {
            root = Paths.get(override);
        } else {
            root = Paths.get(System.getProperty("user.home", "."), ".javar");
        }
        return root.resolve("agents");
    }

    public static long currentPid() {
        try {
            String name = ManagementFactory.getRuntimeMXBean().getName();
            int at = name.indexOf('@');
            if (at > 0) {
                return Long.parseLong(name.substring(0, at));
            }
        } catch (Exception ignored) {
            // fall through
        }
        return 0L;
    }

    /**
     * Write {@code ~/.javar/agents/[PID].json} and install a one-shot shutdown hook
     * that deletes the file when the JVM exits.
     */
    public static void register(int port, String projectName) {
        try {
            Path dir = agentsDir();
            Files.createDirectories(dir);
            long pid = currentPid();
            Path file = dir.resolve(pid + ".json");
            String name = resolveName(projectName);
            String cmd = System.getProperty("sun.java.command", "");
            if (cmd.length() > 400) {
                cmd = cmd.substring(0, 397) + "...";
            }
            String cwd = System.getProperty("user.dir", "");
            String launchedBy = System.getProperty("javar.launched.by", "");
            if (launchedBy == null || launchedBy.isEmpty()) {
                String env = System.getenv("JAVAR_LAUNCHED_BY");
                launchedBy = env != null ? env : "";
            }
            long startedMs = System.currentTimeMillis();
            // Always overwrite [PID].json with the successful bind port (incl. migrations).
            String json = "{"
                    + "\"pid\":" + pid + ","
                    + "\"name\":\"" + escape(name) + "\","
                    + "\"port\":" + port + ","
                    + "\"cwd\":\"" + escape(cwd) + "\","
                    + "\"launched_by\":\"" + escape(launchedBy) + "\","
                    + "\"started_ms\":" + startedMs + ","
                    + "\"cmd\":\"" + escape(cmd) + "\""
                    + "}\n";
            Files.write(file, json.getBytes(StandardCharsets.UTF_8));
            registeredPid = pid;
            ensureShutdownHook();
            LOG.info("JavaR agent registered at " + file
                    + " (port=" + port + ", name=" + name
                    + ", launched_by=" + launchedBy + ", cwd=" + cwd + ")");
        } catch (Exception e) {
            LOG.log(Level.WARNING, "Could not register JavaR agent", e);
        }
    }

    public static void unregister() {
        long pid = registeredPid;
        if (pid < 0) {
            pid = currentPid();
        }
        unregister(pid);
        registeredPid = -1L;
    }

    public static void unregister(long pid) {
        if (pid <= 0) {
            return;
        }
        try {
            Path file = agentsDir().resolve(pid + ".json");
            Files.deleteIfExists(file);
        } catch (IOException ignored) {
            // best-effort
        }
    }

    private static void ensureShutdownHook() {
        if (!HOOK_INSTALLED.compareAndSet(false, true)) {
            return;
        }
        Runtime.getRuntime().addShutdownHook(new Thread(new Runnable() {
            @Override
            public void run() {
                unregister();
            }
        }, "javar-agent-unreg"));
    }

    /** Prefer -Djavar.project.name, then env, then a short form of sun.java.command. */
    static String resolveName(String projectName) {
        if (projectName != null && !projectName.isEmpty()) {
            return projectName;
        }
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

    private static String escape(String s) {
        if (s == null) {
            return "";
        }
        return s.replace("\\", "\\\\")
                .replace("\"", "\\\"")
                .replace("\n", " ")
                .replace("\r", " ");
    }
}

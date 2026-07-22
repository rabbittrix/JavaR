package com.javar.agent;

import com.javar.agent.managed.ManagedClassTransformer;
import com.javar.agent.memory.OffHeapBridge;
import com.javar.agent.memory.OffHeapBridgeFactory;
import com.javar.agent.shadow.ShadowClassManager;

import java.lang.instrument.Instrumentation;
import java.util.concurrent.atomic.AtomicLong;
import java.util.logging.Logger;

/**
 * JavaR Agent — attaches via {@code -javaagent} or dynamic attach.
 * Performs class redefinition and exposes a local socket for javar-core.
 * <p>
 * Off-heap zero-copy: Project Panama on Java 22+, JNI DirectByteBuffer otherwise.
 *
 * Supports Java 8 through latest LTS (21+) and Java 22+ Panama.
 *
 * @author Roberto de Souza (rabbittrix@hotmail.com)
 */
public final class JavaRAgent {

    private static final Logger LOG = Logger.getLogger(JavaRAgent.class.getName());

    private static volatile Instrumentation instrumentation;
    private static volatile AgentSocketServer socketServer;
    private static volatile ClassRedefiner redefiner;
    private static volatile ShadowClassManager shadowManager;
    private static volatile OffHeapBridge offHeap;
    private static final AtomicLong RELOAD_COUNT = new AtomicLong();

    private JavaRAgent() {
    }

    /** JVM startup hook: {@code -javaagent:javar-agent.jar[=port=19222]} */
    public static void premain(String args, Instrumentation inst) {
        if (isBuildOrIdeLauncherJvm()) {
            LOG.info("JavaR agent skipped (launcher JVM: " + launcherCommandHint() + ")");
            return;
        }
        start(args, inst, false);
    }

    /** Dynamic attach hook. */
    public static void agentmain(String args, Instrumentation inst) {
        if (isBuildOrIdeLauncherJvm()) {
            LOG.info("JavaR agent skipped (launcher JVM: " + launcherCommandHint() + ")");
            return;
        }
        start(args, inst, true);
    }

    /**
     * When {@code JAVA_TOOL_OPTIONS} injects the agent globally, Maven / Surefire /
     * IntelliJ / VS Code / Eclipse language-server JVMs also see it. Those processes
     * must not bind the agent port — leave it free for the real application JVM
     * (e.g. Spring Boot forked by {@code mvn spring-boot:run} or Run from any IDE).
     */
    static boolean isBuildOrIdeLauncherJvm() {
        String cmd = System.getProperty("sun.java.command", "");
        if (cmd.isEmpty()) {
            return false;
        }
        String lower = cmd.toLowerCase();

        // Maven plexus parent (spring-boot:run forks the real app) — never bind here.
        if (lower.contains("plexus.classworlds.launcher")
                || lower.contains("org.codehaus.plexus")
                || (lower.contains("spring-boot:run") && !lower.contains("application"))) {
            return true;
        }

        // Never skip a real Spring Boot / user application JVM.
        if (lower.contains("org.springframework.boot")
                || lower.contains("springapplication")
                || (lower.contains(".jar") && lower.contains("spring") && !lower.contains("spring-boot:run"))) {
            // Fat-jar / Boot loader — keep agent. (Still skip if it's clearly an IDE jar.)
            if (!lower.contains("equinox")
                    && !lower.contains("eclipse")
                    && !lower.contains("language-server")
                    && !lower.contains("languageserver")) {
                return false;
            }
        }
        // Main class style apps (com.foo.DemoApplication) — keep.
        if (lower.contains("application")
                && !lower.contains("launcher")
                && !lower.contains("language")) {
            return false;
        }

        return lower.contains("maven")
                || lower.contains("surefire")
                || lower.contains("intellij")
                || lower.contains("idea64")
                || lower.contains("ls.delegate.bat")
                || lower.contains("xmlserverlauncher")
                || lower.contains("org.eclipse.lemminx")
                || lower.contains("org.eclipse.equinox")
                || lower.contains("equinox.launcher")
                || lower.contains("eclipse.jdt")
                || lower.contains("jdt.ls")
                || lower.contains("jdt_ws")
                || lower.contains("redhat.java")
                || lower.contains("languageserver")
                || lower.contains("language-server")
                || lower.contains("language server")
                || lower.contains("kotlin-language-server")
                || lower.contains("gradle-language-server")
                || lower.contains("spring-boot-language-server")
                || lower.contains("scala.meta.metals")
                || lower.contains("metals.main")
                || lower.contains("bloop.bloopserver")
                || lower.contains("bloopserver")
                || lower.contains("bloop.Server")
                || lower.contains("scala.cli")
                || lower.contains("plexus.classworlds.launcher")
                || lower.contains("org.codehaus.plexus");
    }

    private static String launcherCommandHint() {
        String cmd = System.getProperty("sun.java.command", "");
        if (cmd.length() > 120) {
            return cmd.substring(0, 117) + "...";
        }
        return cmd;
    }

    private static void start(String args, Instrumentation inst, boolean dynamic) {
        instrumentation = inst;
        AgentOptions options = AgentOptions.parse(args);

        shadowManager = new ShadowClassManager(inst);
        redefiner = new ClassRedefiner(inst, shadowManager);
        offHeap = OffHeapBridgeFactory.get();
        // MUST run before any @JavaRManaged class is loaded / transformed.
        com.javar.agent.managed.JavaRManagedRuntime.bootstrap(offHeap);
        TelemetryReporter telemetry = new TelemetryReporter(inst, RELOAD_COUNT, offHeap);

        // Transparent @JavaRManaged off-heap mapping (GETFIELD/PUTFIELD → Rust).
        inst.addTransformer(new ManagedClassTransformer(), true);

        // Never abort the host JVM if the telemetry port is busy — app must keep running.
        try {
            socketServer = new AgentSocketServer(options.port, redefiner, telemetry, RELOAD_COUNT);
            int bound = socketServer.startOrFallback(options.port);
            if (bound > 0) {
                String projectName = System.getProperty("javar.project.name", "");
                AgentRegistry.register(bound, projectName);
                LOG.info(String.format(
                        "JavaR agent ready (dynamic=%s, port=%d, redefine=%s, retransform=%s, offheap=%s, abi=%d, managed=on)",
                        dynamic,
                        bound,
                        inst.isRedefineClassesSupported(),
                        inst.isRetransformClassesSupported(),
                        offHeap.backend(),
                        offHeap.abiVersion()));
            }
            // bound < 0 → WARNING already logged; transformers still active for redefine via JNI/later attach
        } catch (Exception e) {
            LOG.warning(
                    "WARNING: JavaR Agent server could not start (port busy), "
                            + "but the application will continue running without telemetry. "
                            + "(" + e.getMessage() + ")");
        }
    }

    public static Instrumentation getInstrumentation() {
        return instrumentation;
    }

    public static ClassRedefiner getRedefiner() {
        return redefiner;
    }

    /** Zero-copy off-heap bridge (Panama or JNI). */
    public static OffHeapBridge getOffHeap() {
        OffHeapBridge local = offHeap;
        if (local == null) {
            local = OffHeapBridgeFactory.get();
            offHeap = local;
        }
        return local;
    }

    /** Structural hot-reload shadow manager. */
    public static ShadowClassManager getShadowManager() {
        return shadowManager;
    }

    /**
     * Optional JNI entrypoint for in-process frames from javar-core (feature jni-bridge).
     */
    public static void onNativeFrame(byte[] frame) {
        if (socketServer != null) {
            socketServer.handleFrameBytes(frame);
        }
    }

    static final class AgentOptions {
        final int port;

        AgentOptions(int port) {
            this.port = port;
        }

        static AgentOptions parse(String args) {
            int port = 19222;
            if (args != null && !args.isEmpty()) {
                for (String part : args.split(",")) {
                    String[] kv = part.split("=", 2);
                    if (kv.length == 2 && "port".equalsIgnoreCase(kv[0].trim())) {
                        port = Integer.parseInt(kv[1].trim());
                    }
                }
            }
            String env = System.getenv("JAVAR_AGENT_PORT");
            if (env != null && !env.isEmpty()) {
                port = Integer.parseInt(env);
            }
            return new AgentOptions(port);
        }
    }
}

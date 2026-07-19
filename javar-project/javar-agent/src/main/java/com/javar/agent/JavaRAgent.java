package com.javar.agent;

import com.javar.agent.managed.ManagedClassTransformer;
import com.javar.agent.memory.OffHeapBridge;
import com.javar.agent.memory.OffHeapBridgeFactory;
import com.javar.agent.shadow.ShadowClassManager;

import java.lang.instrument.Instrumentation;
import java.util.concurrent.atomic.AtomicLong;
import java.util.logging.Level;
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
        start(args, inst, false);
    }

    /** Dynamic attach hook. */
    public static void agentmain(String args, Instrumentation inst) {
        start(args, inst, true);
    }

    private static void start(String args, Instrumentation inst, boolean dynamic) {
        instrumentation = inst;
        AgentOptions options = AgentOptions.parse(args);

        shadowManager = new ShadowClassManager(inst);
        redefiner = new ClassRedefiner(inst, shadowManager);
        offHeap = OffHeapBridgeFactory.get();
        TelemetryReporter telemetry = new TelemetryReporter(inst, RELOAD_COUNT, offHeap);

        // Transparent @JavaRManaged off-heap mapping (GETFIELD/PUTFIELD → Rust).
        inst.addTransformer(new ManagedClassTransformer(), true);

        try {
            socketServer = new AgentSocketServer(options.port, redefiner, telemetry, RELOAD_COUNT);
            socketServer.start();
            LOG.info(String.format(
                    "JavaR agent ready (dynamic=%s, port=%d, redefine=%s, retransform=%s, offheap=%s, abi=%d, managed=on)",
                    dynamic,
                    options.port,
                    inst.isRedefineClassesSupported(),
                    inst.isRetransformClassesSupported(),
                    offHeap.backend(),
                    offHeap.abiVersion()));
        } catch (Exception e) {
            LOG.log(Level.SEVERE, "Failed to start JavaR agent socket", e);
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

package com.javar.agent;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Local TCP server that receives framed messages from javar-core / IDE clients.
 */
public final class AgentSocketServer {

    private static final Logger LOG = Logger.getLogger(AgentSocketServer.class.getName());
    private static final int MAGIC = 0x4A415652; // JAVR
    private static final byte VERSION = 1;

    private static final byte KIND_PING = 1;
    private static final byte KIND_PONG = 2;
    private static final byte KIND_STATUS = 3;
    private static final byte KIND_ERROR = 4;
    private static final byte KIND_REDEFINE = 5;
    private static final byte KIND_ROLLBACK = 6;
    private static final byte KIND_TELEMETRY = 7;
    private static final byte KIND_HOT_DEPLOY = 8;
    private static final byte KIND_STRUCTURAL = 9;

    private final int port;
    private final ClassRedefiner redefiner;
    private final TelemetryReporter telemetry;
    private final AtomicLong reloadCount;
    private final AtomicBoolean running = new AtomicBoolean(false);
    private final ExecutorService pool = Executors.newCachedThreadPool(r -> {
        Thread t = new Thread(r, "javar-agent-io");
        t.setDaemon(true);
        return t;
    });

    private ServerSocket serverSocket;

    public AgentSocketServer(
            int port,
            ClassRedefiner redefiner,
            TelemetryReporter telemetry,
            AtomicLong reloadCount) {
        this.port = port;
        this.redefiner = redefiner;
        this.telemetry = telemetry;
        this.reloadCount = reloadCount;
    }

    public void start() throws IOException {
        if (!running.compareAndSet(false, true)) {
            return;
        }
        serverSocket = new ServerSocket(port, 50, InetAddress.getByName("127.0.0.1"));
        pool.execute(this::acceptLoop);
        LOG.info("JavaR agent listening on 127.0.0.1:" + port);
    }

    public void stop() {
        running.set(false);
        try {
            if (serverSocket != null) {
                serverSocket.close();
            }
        } catch (IOException ignored) {
            // ignore
        }
        pool.shutdownNow();
    }

    private void acceptLoop() {
        while (running.get()) {
            try {
                Socket client = serverSocket.accept();
                pool.execute(() -> handleClient(client));
            } catch (IOException e) {
                if (running.get()) {
                    LOG.log(Level.WARNING, "accept failed", e);
                }
            }
        }
    }

    private void handleClient(Socket socket) {
        try {
            socket.setTcpNoDelay(true);
            InputStream in = socket.getInputStream();
            OutputStream out = socket.getOutputStream();
            while (running.get() && !socket.isClosed()) {
                byte[] frame = readFrame(in);
                if (frame == null) {
                    break;
                }
                byte[] response = dispatchFrame(frame);
                if (response != null) {
                    out.write(response);
                    out.flush();
                }
            }
        } catch (IOException e) {
            LOG.log(Level.FINE, "client disconnected", e);
        } finally {
            try {
                socket.close();
            } catch (IOException ignored) {
                // ignore
            }
        }
    }

    /** Entry used by JNI {@link JavaRAgent#onNativeFrame(byte[])}. */
    public void handleFrameBytes(byte[] frame) {
        dispatchFrame(frame);
    }

    private byte[] dispatchFrame(byte[] frame) {
        if (frame.length < 10) {
            return null;
        }
        ByteBuffer buf = ByteBuffer.wrap(frame).order(ByteOrder.LITTLE_ENDIAN);
        int magic = buf.getInt();
        if (magic != MAGIC) {
            LOG.warning("invalid magic: " + Integer.toHexString(magic));
            return null;
        }
        byte version = buf.get();
        if (version != VERSION) {
            LOG.warning("unsupported protocol version: " + version);
            return null;
        }
        byte kind = buf.get();
        int payloadLen = buf.getInt();
        if (payloadLen < 0 || 10 + payloadLen > frame.length) {
            return null;
        }
        byte[] payload = new byte[payloadLen];
        buf.get(payload);

        switch (kind) {
            case KIND_PING:
                return encodeFrame(KIND_PONG, new byte[0]);
            case KIND_REDEFINE:
                return handleRedefine(payload, false);
            case KIND_STRUCTURAL:
                return handleRedefine(payload, true);
            case KIND_HOT_DEPLOY:
                // IDE nudge — core file watcher performs compile/redefine; ack only.
                return encodeFrame(KIND_STATUS, jsonStatus("hot_deploy", "accepted"));
            case KIND_ROLLBACK:
                return handleRollback(payload);
            case KIND_TELEMETRY:
                return encodeFrame(KIND_TELEMETRY, telemetry.snapshotJson(reloadCount.get()));
            case KIND_STATUS:
                return encodeFrame(KIND_STATUS, jsonStatus("ok", "agent alive"));
            default:
                return encodeFrame(KIND_ERROR, jsonStatus("error", "unknown kind " + kind));
        }
    }

    private byte[] handleRedefine(byte[] payload, boolean structuralHint) {
        try {
            if (payload.length < 4) {
                return encodeFrame(KIND_ERROR, jsonStatus("error", "truncated redefine"));
            }
            ByteBuffer buf = ByteBuffer.wrap(payload).order(ByteOrder.LITTLE_ENDIAN);
            int headerLen = buf.getInt();
            if (headerLen < 0 || 4 + headerLen > payload.length) {
                return encodeFrame(KIND_ERROR, jsonStatus("error", "bad redefine header"));
            }
            String headerJson = new String(payload, 4, headerLen, StandardCharsets.UTF_8);
            String className = JsonMini.stringField(headerJson, "class_name");
            int bytecodeLen = JsonMini.intField(headerJson, "bytecode_len");
            int start = 4 + headerLen;
            int end = start + bytecodeLen;
            if (className == null || end > payload.length) {
                return encodeFrame(KIND_ERROR, jsonStatus("error", "invalid redefine payload"));
            }
            byte[] bytecode = new byte[bytecodeLen];
            System.arraycopy(payload, start, bytecode, 0, bytecodeLen);

            boolean structural = structuralHint
                    || "true".equalsIgnoreCase(JsonMini.stringField(headerJson, "structural"));
            // JSON boolean may appear without quotes — also check raw fragment.
            if (!structural && headerJson.contains("\"structural\":true")) {
                structural = true;
            }

            ClassRedefiner.RedefineResult result;
            if (structural) {
                String shadowName = JsonMini.stringField(headerJson, "shadow_name");
                int version = JsonMini.intField(headerJson, "version");
                if (version < 0) {
                    version = 1;
                }
                if (shadowName == null || shadowName.isEmpty()) {
                    shadowName = className + "$JavaR_v" + version;
                }
                result = redefiner.redefineStructural(className, shadowName, version, bytecode);
            } else {
                result = redefiner.redefine(className, bytecode);
            }

            if (result.success) {
                reloadCount.incrementAndGet();
                String state = structural ? "shadow" : "redefined";
                String changeType = structural ? "Structural" : "Body";
                int ver = structural ? JsonMini.intField(headerJson, "version") : 0;
                if (ver < 0) {
                    ver = (int) reloadCount.get();
                }
                telemetry.recordReload(className, changeType, ver);
                return encodeFrame(KIND_STATUS, jsonStatus(state, result.message));
            }
            return encodeFrame(KIND_ERROR, jsonStatus("error", result.message));
        } catch (Exception e) {
            return encodeFrame(KIND_ERROR, jsonStatus("error", e.getMessage()));
        }
    }

    private byte[] handleRollback(byte[] payload) {
        String json = new String(payload, StandardCharsets.UTF_8);
        String className = JsonMini.stringField(json, "detail");
        if (className == null) {
            className = JsonMini.stringField(json, "class_name");
        }
        if (className == null) {
            return encodeFrame(KIND_ERROR, jsonStatus("error", "missing class name"));
        }
        ClassRedefiner.RedefineResult result = redefiner.rollback(className);
        byte kind = result.success ? KIND_STATUS : KIND_ERROR;
        return encodeFrame(kind, jsonStatus(result.success ? "rollback" : "error", result.message));
    }

    private static byte[] readFrame(InputStream in) throws IOException {
        byte[] header = readFully(in, 10);
        if (header == null) {
            return null;
        }
        ByteBuffer buf = ByteBuffer.wrap(header).order(ByteOrder.LITTLE_ENDIAN);
        buf.getInt(); // magic
        buf.get();    // version
        buf.get();    // kind
        int payloadLen = buf.getInt();
        if (payloadLen < 0 || payloadLen > 64 * 1024 * 1024) {
            throw new IOException("payload too large: " + payloadLen);
        }
        byte[] payload = readFully(in, payloadLen);
        if (payload == null) {
            return null;
        }
        byte[] frame = new byte[10 + payloadLen];
        System.arraycopy(header, 0, frame, 0, 10);
        System.arraycopy(payload, 0, frame, 10, payloadLen);
        return frame;
    }

    private static byte[] readFully(InputStream in, int len) throws IOException {
        if (len == 0) {
            return new byte[0];
        }
        byte[] buf = new byte[len];
        int off = 0;
        while (off < len) {
            int n = in.read(buf, off, len - off);
            if (n < 0) {
                return off == 0 ? null : null;
            }
            off += n;
        }
        return buf;
    }

    private static byte[] encodeFrame(byte kind, byte[] payload) {
        ByteBuffer buf = ByteBuffer.allocate(10 + payload.length).order(ByteOrder.LITTLE_ENDIAN);
        buf.putInt(MAGIC);
        buf.put(VERSION);
        buf.put(kind);
        buf.putInt(payload.length);
        buf.put(payload);
        return buf.array();
    }

    private static byte[] jsonStatus(String state, String detail) {
        String json = "{\"state\":\"" + escape(state) + "\",\"detail\":\"" + escape(detail) + "\"}";
        return json.getBytes(StandardCharsets.UTF_8);
    }

    private static String escape(String s) {
        if (s == null) {
            return "";
        }
        return s.replace("\\", "\\\\").replace("\"", "\\\"");
    }

    /** Minimal JSON field extractor — avoids external deps on Java 8. */
    static final class JsonMini {
        private JsonMini() {
        }

        static String stringField(String json, String key) {
            String needle = "\"" + key + "\"";
            int i = json.indexOf(needle);
            if (i < 0) {
                return null;
            }
            int colon = json.indexOf(':', i + needle.length());
            if (colon < 0) {
                return null;
            }
            int q1 = json.indexOf('"', colon + 1);
            if (q1 < 0) {
                return null;
            }
            int q2 = json.indexOf('"', q1 + 1);
            if (q2 < 0) {
                return null;
            }
            return json.substring(q1 + 1, q2);
        }

        static int intField(String json, String key) {
            String needle = "\"" + key + "\"";
            int i = json.indexOf(needle);
            if (i < 0) {
                return -1;
            }
            int colon = json.indexOf(':', i + needle.length());
            if (colon < 0) {
                return -1;
            }
            int start = colon + 1;
            while (start < json.length() && Character.isWhitespace(json.charAt(start))) {
                start++;
            }
            int end = start;
            while (end < json.length() && (Character.isDigit(json.charAt(end)))) {
                end++;
            }
            if (start == end) {
                return -1;
            }
            return Integer.parseInt(json.substring(start, end));
        }
    }
}

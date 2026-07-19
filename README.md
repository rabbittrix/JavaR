# JavaR

**High-Performance Java Accelerator & Hot-Reload Engine**

JavaR is a hybrid system where a **Rust core** runs as a sidecar/agent beside the JVM to deliver:

1. **Structural Hot-Reloading** — change fields, methods, and classes without restarting the JVM  
2. **Off-Heap Memory Management** — keep heavy data structures in Rust and bypass Java GC pauses (Phase 2)  
3. **Instant Rollback** — state-tracking to revert failed code changes in milliseconds  

**Author:** Roberto de Souza (`rabbittrix@hotmail.com`)

---

## Logo

A modern, “addictive” mark: a stylized **R** fused with a Duke-inspired silhouette, neon orange → rust-red, on a dark stage — suggesting speed, hot metal, and the Java ↔ Rust bond.

<p align="center">
  <img src="javar-project/docs/assets/javar-logo.svg" alt="JavaR logo" width="220" />
</p>

---

## Architecture

```text
┌─────────────────┐     watch .java/.class      ┌──────────────────┐
│   javar-core    │ ───────────────────────────▶│  compile (javac) │
│  (Rust sidecar) │                             └────────┬─────────┘
└────────┬────────┘                                      │ bytecode
         │ TCP / JNI (zero-copy frames)                  ▼
         ▼                                      ┌──────────────────┐
┌─────────────────┐     redefineClasses         │  javar-agent     │
│  IDE / javar    │◀── telemetry ───────────────│  (Java Agent)    │
│  VS Code ext    │                             └──────────────────┘
└─────────────────┘                                      │
                                                         ▼
                                                ┌──────────────────┐
                                                │      JVM         │
                                                └──────────────────┘
```

| Component | Role |
|-----------|------|
| `javar-core` | File watching (`notify`), compile orchestration, rollback store, off-heap scaffold |
| `javar-agent` | `java.lang.instrument` agent — `redefineClasses`, socket server, telemetry |
| `javar-cli` | IDE-agnostic CLI (`javar init`, `run`, `status`) |
| `javar-vscode` | VS Code extension — Hot Deploy button + memory telemetry |

**Java support:** 8 → latest LTS (21+)  
**Platforms:** Linux, Windows, macOS  
**IDE-agnostic:** core is a CLI/agent usable from IntelliJ, Eclipse, or VS Code

---

## Raise the project

### Prerequisites

- Rust (`rustup`)
- JDK 8+ (JDK 22+ recommended for Project Panama)
- Maven 3.8+ (for `javar-agent`)

### From source (this repo)

```powershell
cd javar-project

cargo build --release -p javar-cli -p javar-core
cd javar-agent
mvn -DskipTests package
cd ..

# Optional: install CLI onto PATH
cargo install --path javar-cli
```

### One-liner (Unix / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/rabbittrix/javar/main/javar-project/scripts/install.sh | sh
```

Then add `~/.javar/bin` to your `PATH`.

### Windows (PowerShell)

```powershell
# from repo root:
.\javar-project\scripts\install.ps1
```

---

## Quick start

```powershell
cd javar-project

# 1. Scaffold
javar init ..\demo-app
cd ..\demo-app

# 2. Start your JVM with the agent (flags printed by `javar run --flags-only`)
javar run --flags-only
# java -javaagent:...\javar-agent-0.1.0.jar=port=19222 -cp ... com.example.HelloJavaR

# 3. In another terminal — start the Rust sidecar
javar run

# 4. Probe health / telemetry
javar status
```

Edit a `.java` file, save, and JavaR compiles + pushes bytecode to the agent.

Native library (off-heap Panama/JNI), when needed:

```powershell
# After: cargo build -p javar-core  (from javar-project/)
$env:JAVAR_NATIVE_PATH = "$PWD\target\release\javar_core.dll"
```

---

## Commands

| Command | Description |
|---------|-------------|
| `javar init [path]` | Create `javar.toml` and a sample `HelloJavaR` app |
| `javar run [path]` | Print JVM inject flags and start `javar-core` watching the project |
| `javar run --flags-only` | Only print `-javaagent` / env flags |
| `javar run --port 19222` | Choose agent port |
| `javar status` | Ping the agent and print heap vs JavaR-managed memory |

Environment variables:

| Variable | Default | Meaning |
|----------|---------|---------|
| `JAVAR_AGENT_ADDR` | `127.0.0.1:19222` | Core → agent address |
| `JAVAR_AGENT_PORT` | `19222` | Agent listen port |
| `JAVAR_NATIVE_PATH` | — | Absolute path to `javar_core` shared library |

---

## Hot-reload flow

1. **Watch** — `javar-core` debounces `notify` events on `.java` / `.class`  
2. **Compile** — sources go through background `javac`; `.class` files are mmap’d  
3. **Frame** — bytecode is sent in a compact binary frame (header + payload, no extra concat on the write path)  
4. **Redefine** — agent calls `Instrumentation.redefineClasses`  
5. **Rollback** — previous bytecode is snapshotted; failed changes can be reverted in milliseconds  

Structural changes (new fields/methods) that HotSwap rejects will use a custom classloader path (`StructuralClassLoader`) in a later milestone.

---

## VS Code extension

```powershell
cd javar-project\javar-vscode
npm install
npm run compile
# F5 in VS Code, or: npx vsce package
```

Features:

- Connects to the local JavaR agent socket (`javar.coreHost` / `javar.corePort`)
- **JavaR: Hot Deploy** command + editor title flame button
- **Memory Telemetry** view — Java Heap vs JavaR Managed Memory
- Status bar live counters

---

## Workspace layout

```text
Project_JavaR/                 ← repo root (this README)
├── README.md
└── javar-project/             ← Cargo workspace + agent + VS Code ext
    ├── Cargo.toml
    ├── javar-core/            ← Rust sidecar (watcher, bridge, protocol, memory)
    ├── javar-cli/             ← `javar` binary
    ├── javar-agent/           ← Java Instrumentation agent (Maven)
    ├── javar-vscode/          ← VS Code extension
    ├── scripts/               ← install.sh / install.ps1
    └── docs/assets/           ← logo SVG
```

### Off-heap zero-copy bridge (Panama / JNI)

Rust owns off-heap regions (`javar_mem_*` C ABI in `javar-project/javar-core/include/javar_mem.h`). The JVM attaches without copying:

| JDK | Backend | Mechanism |
|-----|---------|-----------|
| **22+** | Project Panama | `Linker.downcallHandle` + `MemorySegment.ofAddress(…).reinterpret(…)` |
| **8–21** | JNI fallback | `NewDirectByteBuffer` over the same Rust pointer |

```java
OffHeapBridge mem = JavaRAgent.getOffHeap(); // or OffHeapBridgeFactory.get()
long id = mem.allocate(1 << 20, 8);
ByteBuffer view = mem.asByteBuffer(id);      // zero-copy
// Java 22+: ((PanamaOffHeapBridge) mem).asSegment(id)
mem.free(id);
```

Load the native library with `-Djavar.native.path=/path/to/javar_core.dll` (or `libjavar_core.so` / `.dylib`), or put it on `java.library.path`. Build the agent on JDK 22+ to include the Multi-Release Panama classes (`META-INF/versions/22/`).

---

## Protocol (summary)

Little-endian frames:

```text
[u32 magic=JAVR][u8 version=1][u8 kind][u32 payload_len][payload...]
```

Kinds: `Ping`, `Pong`, `Status`, `Error`, `Redefine`, `Rollback`, `Telemetry`, `HotDeploy`.

---

## Author

**Roberto de Souza**  
Email: [rabbittrix@hotmail.com](mailto:rabbittrix@hotmail.com)

---

## License

Apache-2.0

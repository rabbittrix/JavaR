<!-- include BANNER.md branding -->

```text
     ██╗ █████╗ ██╗   ██╗ █████╗ ██████╗
     ██║██╔══██╗██║   ██║██╔══██╗██╔══██╗
     ██║███████║██║   ██║███████║██████╔╝
██   ██║██╔══██║╚██╗ ██╔╝██╔══██║██╔══██╗
╚█████╔╝██║  ██║ ╚████╔╝ ██║  ██║██║  ██║
 ╚════╝ ╚═╝  ╚═╝  ╚═══╝  ╚═╝  ╚═╝╚═╝  ╚═╝
```

<p align="center">

![JavaR logo](docs/assets/logo.png)

</p>

<p align="center">
  <b>Zero-Restart Java</b> — structural hot-reload · Rust off-heap · invisible GC bypass<br/>
  by <b>Roberto de Souza</b> (<a href="mailto:rabbittrix@hotmail.com">rabbittrix@hotmail.com</a>)
</p>

---

# JavaR

**High-Performance Java Accelerator & Hot-Reload Engine**

JavaR pairs a **Rust sidecar** with a **Java agent** so you can change running code and keep heavy data out of the GC — without restarting the JVM.

1. **Structural Hot-Reloading** — add fields/methods via shadow classes (`Original$JavaR_vN`)  
2. **Off-Heap Memory** — `@JavaRManaged` stores primitives in Rust (`javar_mem_*`)  
3. **Instant Rollback** — version stack for failed reloads  
4. **Control Center** — `javar dashboard` (ratatui TUI)

---

## Getting Started

### Prerequisites

- Rust (`rustup`), JDK 8+ (22+ for Panama), Maven 3.8+

### Build

```powershell
cd javar-project
cargo build --release -p javar-cli -p javar-core
cd javar-agent
mvn -DskipTests package
cd ..
cargo install --path javar-cli
```

### Highlight: `@JavaRManaged`

Annotate a class — the agent rewrites primitive field access to Rust off-heap memory. The Java object stays a tiny shell.

```java
import com.javar.agent.managed.JavaRManaged;

@JavaRManaged
public class SensorReading {
    private int temperature; // off-heap (Rust)
    private long timestamp;  // off-heap (Rust)
    private String label;    // reference stays on-heap
}
```

```powershell
# Smart run — detects pom.xml / build.gradle, target/classes or build/classes,
# finds a public static void main, injects -javaagent + native lib path:
javar run

# Override main / classpath after --
javar run . -- com.example.HelloJavaR
javar run -- -cp app.jar Main

# Sidecar only (no JVM), or print the resolved java line:
javar run --watch-only
javar run --flags-only

# Control Center
javar dashboard
```

| Command | Purpose |
|---------|---------|
| `javar init` | Scaffold project + `javar.toml` |
| `javar run [PATH]` | Smart-detect build, classes, main; start core + JVM |
| `javar run [PATH] -- [java args…]` | Same, with explicit `java` args after `--` |
| `javar run --watch-only` | Start javar-core only (IDE / cockpit) |
| `javar run --agent <jar>` | Override agent JAR path |
| `javar status` | One-shot telemetry |
| `javar dashboard` | Live TUI (heap vs off-heap, shadows, GC, logs) |

**Smart run** looks for `pom.xml` / `build.gradle(.kts)`, prefers `target/classes` (Maven) or `build/classes/java/main` (Gradle), resolves `javar-agent.jar` and `javar_core` native lib (`JAVAR_NATIVE_PATH` / `-Djavar.native.path`), and discovers a main class if you omit one.

---

## Architecture

```text
┌─────────────┐  watch .java/.class   ┌──────────────┐
│ javar-core  │ ───────────────────▶  │ javac / mmap │
│  (Rust)     │                       └──────┬───────┘
└──────┬──────┘                              │
       │ schema diff (compatible vs structural)
       ▼
┌─────────────┐   redefine / Structural(9)   ┌────────────────┐
│ javar-cli   │◀────── telemetry ────────────│  javar-agent   │
│ dashboard   │                              │ ByteBuddy+ASM  │
└─────────────┘                              └────────┬───────┘
                                                      ▼
                                               ┌─────────────┐
                                               │     JVM     │
                                               └─────────────┘
```

### Shadow-class bypass (structural HotSwap)

The JVM forbids changing a *loaded* class’s field/method set. JavaR does not fight that rule:

1. **Rust** detects a structural schema change and assigns `Original$JavaR_vN`  
2. **Agent** defines a **new** class with the new schema (always legal)  
3. **ByteBuddy** rewrites only method *bodies* on `Original` → `JavaRDispatcher` (schema frozen → HotSwap-legal)  
4. Live instances keep type `Original`; each gets a shadow twin for new state  

```text
Caller → Original.foo()  ──dispatch──▶  Original$JavaR_v2.foo()
              │                              │
         frozen schema                 new fields/methods
```

### Off-heap / Panama

| JDK | Backend |
|-----|---------|
| 22+ | Project Panama FFM |
| 8–21 | JNI `NewDirectByteBuffer` |

Native lib: `-Djavar.native.path=` / `JAVAR_NATIVE_PATH`.

---

## Control Center (TUI)

```bash
javar dashboard --addr 127.0.0.1:19222
# keys: q quit · ←/→ tabs · 1–4 jump · r refresh
```

- **Performance** — JVM heap vs JavaR off-heap chart; **sysinfo** JVM process table  
- **Hot-Reload** — shadow/reload history + estimated restart time saved  
- **GC Metrics** — `@JavaRManaged` regions & bytes kept off-heap  
- **Logs** — live bytecode injection feed  

---

## VS Code Cockpit

```bash
cd vscode-javar
npm install && npm run compile
# F5 to launch Extension Development Host
```

- Status bar: `JavaR: Active · Heap … · Off-heap …`  
- **JavaR: Force Re-sync** — save + HotDeploy nudge  
- Auto-finds `javar` on `PATH` and can `javar run` the workspace  
- Sidebar: off-heap region summary  

Icon: `docs/assets/icon.png` · Logo: `docs/assets/logo.svg` · Banner: [`BANNER.md`](BANNER.md)

---

## CI / Releases

[`.github/workflows/build.yml`](.github/workflows/build.yml) builds Linux, Windows, and macOS (arm64 + x86_64 cross-compile on `macos-14`). Tag `v*` to publish zips with CLI + native lib + agent JAR.

---

## Vision

> **Zero-Restart Java** — keep the JVM warm, move weight into Rust, and ship feedback loops measured in milliseconds, not minutes.

**Author:** Roberto de Souza · `rabbittrix@hotmail.com`  
**License:** Apache-2.0

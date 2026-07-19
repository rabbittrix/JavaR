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
4. **Control Center** — `javar dashboard` / `javar tui` (ratatui)

---

## Getting Started

### Prerequisites

- Rust (`rustup`), JDK 8+ (22+ for Panama), Maven 3.8+

### Build & install

```powershell
cd javar-project
cargo build --release -p javar-cli -p javar-core
cd javar-agent
mvn -DskipTests package
cd ..
cargo install --path javar-cli
```

Release zips from CI also ship `bin/javar`, `lib/*javar_core*`, and `agent/javar-agent.jar`.

### First project

```powershell
javar init my-app
cd my-app
# compile your sources (mvn compile / gradle classes / javac …)
javar run
```

`javar run` detects the build layout, injects the agent + native library, finds a `main`, and starts the JVM.

---

## CLI commands

```text
javar <COMMAND>

Commands:
  init       Scaffold a JavaR-enabled project (config + sample layout)
  run        Smart-launch java with agent + native lib (and start the sidecar)
  status     Probe agent socket and print telemetry
  dashboard  Open the Control Center (ratatui TUI)
  tui        Alias for dashboard
```

### `javar init [PATH]`

Scaffold `javar.toml` and a sample `HelloJavaR` class. Default path: `.`

```bash
javar init
javar init ./demo
```

### `javar run [OPTIONS] [PATH] [-- <ARGS>...]`

Smart run (default path `.`):

1. Detects `pom.xml` or `build.gradle` / `build.gradle.kts`
2. Finds compiled classes (`target/classes` or `build/classes/java/main`)
3. Resolves `javar-agent.jar` (absolute `-javaagent`) and `javar_core` native lib
4. Discovers a `public static void main` if you omit one
5. Starts **javar-core** + **java**

```bash
# Full smart launch
javar run
javar run ./my-app

# Explicit main / classpath after --
javar run -- com.example.HelloJavaR
javar run . -- -cp target/classes com.example.HelloJavaR
javar run -- -cp app.jar Main

# Override agent JAR / port
javar run --agent /path/to/javar-agent.jar --port 19222
javar run . --agent ./lib/javar-agent.jar -- com.example.Main

# Sidecar only (no JVM) — used by the VS Code Cockpit
javar run --watch-only
javar run ./my-app --watch-only --port 19222

# Print resolved flags / java line only
javar run --flags-only
javar run --flags-only -- com.example.Main

# Launch java without spawning javar-core
javar run --no-core -- com.example.Main
```

| Option | Meaning |
|--------|---------|
| `--agent <JAR>` | Explicit agent JAR (skips auto-discovery) |
| `--port <N>` | Agent listen port (default `19222`) |
| `--flags-only` | Print inject flag / equivalent `java` line; do not start processes |
| `--no-core` | Do not spawn javar-core |
| `--watch-only` | Start sidecar only; do not auto-launch a JVM |
| `-- <ARGS>…` | Forwarded to `java` after `-javaagent` (and smart `-cp` / main if missing) |

**Agent discovery** (when `--agent` is omitted): `JAVAR_AGENT_JAR` → `../javar-agent/target/*javar-agent*.jar` → workspace `javar-agent/target/` → Maven `package` fallback.

**Native library:** `JAVAR_NATIVE_PATH` or auto-find `javar_core.dll` / `libjavar_core.so|.dylib` under `target/release|debug`, then set `-Djavar.native.path` and `-Djava.library.path`.

### `javar status [--addr HOST:PORT]`

One-shot ping + telemetry from the agent. Default: `127.0.0.1:19222`

```bash
javar status
javar status --addr 127.0.0.1:19222
```

### `javar dashboard` / `javar tui`

Live Control Center. Default: `127.0.0.1:19222`

```bash
javar dashboard
javar dashboard --addr 127.0.0.1:19222
javar tui
```

Keys: `q` quit · `←`/`→` tabs · `1`–`4` jump · `r` refresh

---

## Highlight: `@JavaRManaged`

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

Native lib: `-Djavar.native.path=` / `JAVAR_NATIVE_PATH` (also set automatically by `javar run`).

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

Extension: **JavaR Cockpit** (`jrsf.javar`) in [`vscode-javar/`](vscode-javar/)

```bash
cd vscode-javar
npm install && npm run compile
# F5 to launch Extension Development Host
```

| Cockpit action | CLI equivalent |
|----------------|----------------|
| Auto-start / **Start CLI / Core** | `javar run <workspace> --watch-only --port <N>` |
| **Open Control Center (TUI)** | `javar dashboard --addr 127.0.0.1:<N>` |
| **Connect Agent** / status bar | agent TCP on port `19222` (same as `javar status`) |
| **Force Re-sync** | save + HotDeploy nudge to the agent |

Icon: `docs/assets/icon.png` · Logo: `docs/assets/logo.svg` · Banner: [`BANNER.md`](BANNER.md)

---

## Environment variables

| Variable | Purpose |
|----------|---------|
| `JAVAR_AGENT_JAR` | Override path to `javar-agent.jar` |
| `JAVAR_AGENT_ADDR` | Agent address (`127.0.0.1:19222`) |
| `JAVAR_NATIVE_PATH` | Absolute path to `javar_core` shared library |

---

## CI / Releases

[`.github/workflows/build.yml`](.github/workflows/build.yml) builds Linux, Windows, and macOS (arm64 + x86_64 cross-compile on `macos-14`). Tag `v*` to publish zips with CLI + native lib + agent JAR.

---

## Vision

> **Zero-Restart Java** — keep the JVM warm, move weight into Rust, and ship feedback loops measured in milliseconds, not minutes.

**Author:** Roberto de Souza · `rabbittrix@hotmail.com`  
**License:** Apache-2.0

# JavaR Cockpit

<p align="center">
  <img src="https://raw.githubusercontent.com/rabbittrix/JavaR/main/vscode-javar/media/logo.png" alt="JavaR" width="160" />
</p>

**Zero-Restart Java** — hot-reload, off-heap telemetry, and `javar` CLI integration for Visual Studio Code / Cursor.

Author: **Roberto de Souza** (`rabbittrix@hotmail.com`)  
Publisher: **JRSF** · Identifier: `jrsf.javar`

---

## What this extension does

| Feature | Description |
|---------|-------------|
| **Status bar** | Shows `JavaR: Active` plus JVM heap vs off-heap memory |
| **Force Re-sync** | Saves the current file and nudges the agent to hot-deploy |
| **Start CLI / Core** | Runs `javar run <workspace> --watch-only` (auto-start optional) |
| **Control Center** | Opens a terminal with `javar dashboard` (ratatui TUI) |
| **Off-Heap Regions** | Sidebar view of managed region / memory stats |

JavaR itself is a **Rust sidecar + Java agent**. This extension is the VS Code cockpit that talks to the agent on `127.0.0.1:19222` and launches the CLI.

---

## Prerequisites

1. A Java / Maven / **Spring Boot** project
2. The **JavaR CLI** — installed automatically by the extension when missing  
   (or set `javar.cliPath` / use the installers below)

### Install the CLI

**Automatic (recommended):** open a Java workspace and accept **Install** when prompted,  
or run **JavaR: Install / Repair CLI** from the Command Palette.

**Manual — Windows (PowerShell):**

```powershell
iwr https://javar.dev/install.ps1 | iex
```

**Manual — Linux / macOS:**

```bash
curl -fsSL https://javar.dev/install.sh | sh
```

`javar setup` extracts the embedded agent + native lib, bootstraps **Maven** into  
`~/.javar/tools` when needed, and adds `~/.javar/bin` (with an `mvn` shim) to your PATH.

---

## Quick start

### 1. Run your app with the JavaR CLI

`javar run` extracts the embedded agent if needed, detects Maven / Gradle / **Spring Boot**,  
auto-installs Maven when missing, builds if needed, and launches with `-javaagent`.  
Spring Boot apps prefer `java -jar target/*-*.jar`. The **project name** appears in the  
status bar and Control Center.

```bash
# Full smart launch (agent + native + -cp + discovered Main)
javar run
javar run ./my-app

# Build the Java project first (PowerShell-safe — no &&)
javar build

# Explicit main / classpath after --
javar run -- com.example.HelloJavaR
javar run . -- -cp target/classes com.example.HelloJavaR
javar run -- -cp app.jar Main

# Override agent / port
javar run --agent /path/to/javar-agent.jar --port 19222 -- com.example.Main

# Print the resolved java line without starting processes
javar run --flags-only

# Sidecar only (no JVM) — what the Cockpit auto-start uses
javar run --watch-only
```

| CLI command | Purpose |
|-------------|---------|
| `javar init [PATH]` | Scaffold project + `javar.toml` + sample main |
| `javar setup` | Extract embedded agent/native to `~/.javar/bin` + PATH |
| `javar build [PATH]` | Maven/Gradle package (PowerShell-safe, no `&&`) |
| `javar run [PATH]` | Smart-detect build, classes, main; start core + JVM |
| `javar run [PATH] -- [java args…]` | Same, with explicit `java` args after `--` |
| `javar run --watch-only` | Start javar-core only (IDE / cockpit) |
| `javar run --flags-only` | Print inject flag / equivalent `java` line |
| `javar run --no-core` | Launch java without spawning javar-core |
| `javar run --agent <jar>` | Override agent JAR path |
| `javar run --port <N>` | Agent listen port (default `19222`) |
| `javar status [--addr HOST:PORT]` | One-shot ping + telemetry |
| `javar dashboard` / `javar tui` | Live Control Center TUI |

Optional native off-heap library (auto-resolved by `javar run` when possible):

```bash
# Windows
set JAVAR_NATIVE_PATH=C:\path\to\javar_core.dll

# Linux / macOS
export JAVAR_NATIVE_PATH=/path/to/libjavar_core.so   # or .dylib
```

### 2. Open the project in VS Code / Cursor

- Install **JavaR Cockpit** (`jrsf.javar`)
- Open a folder that contains Java sources (and ideally `javar.toml`)
- With `javar.autoStart` enabled (default), the extension starts:

  ```bash
  javar run <workspace> --watch-only --port 19222
  ```

  Start your app separately with `javar run` (smart launch) in a terminal, or attach `-javaagent` yourself.

### 3. Use the Cockpit

| Action | How |
|--------|-----|
| See live memory | Look at the **status bar** (bottom left) |
| Hot deploy current file | Command Palette → **JavaR: Force Re-sync** (or the sync icon on `.java` editors) |
| Start watcher manually | **JavaR: Start CLI / Core** → `javar run … --watch-only` |
| Connect telemetry only | **JavaR: Connect Agent** |
| Full TUI dashboard | **JavaR: Open Control Center (TUI)** → `javar dashboard` |
| Off-heap summary | Activity bar → **JavaR** → Off-Heap Regions |

---

## Extension commands

| Command | ID | CLI / behavior |
|---------|-----|----------------|
| JavaR: Force Re-sync | `javar.forceResync` | Save file + HotDeploy nudge to agent |
| JavaR: Start CLI / Core | `javar.startCli` | `javar run <folder> --watch-only --port <N>` |
| JavaR: Open Control Center (TUI) | `javar.openDashboard` | `javar dashboard --addr 127.0.0.1:<N>` |
| JavaR: Connect Agent | `javar.connect` | Open TCP telemetry to agent |
| JavaR: Install / Repair CLI | `javar.installCli` | Download CLI into `~/.javar/bin` + setup |

---

## Settings

Open **Settings → Extensions → JavaR**:

| Setting | Default | Meaning |
|---------|---------|---------|
| `javar.cliPath` | `javar` | Path to the CLI binary |
| `javar.agentHost` | `127.0.0.1` | Agent TCP host |
| `javar.agentPort` | `19222` | Agent TCP port |
| `javar.autoStart` | `true` | Auto-run `javar run --watch-only` when a workspace opens |
| `javar.autoInstallCli` | `true` | Offer to install the CLI when it is missing |

---

## Transparent off-heap (`@JavaRManaged`)

Annotate classes so primitive fields live in Rust-managed memory (less GC pressure):

```java
import com.javar.agent.managed.JavaRManaged;

@JavaRManaged
public class SensorReading {
    private int temperature; // stored off-heap
    private long timestamp;  // stored off-heap
    private String label;    // references stay on-heap
}
```

Requires the **javar-agent** on the JVM (`javar run` injects it). The status bar / sidebar then show off-heap usage.

---

## Structural hot-reload (shadow classes)

When you add fields/methods, JavaR does **not** fight JVM HotSwap limits:

1. Defines `YourClass$JavaR_vN` (new class — always allowed)
2. Proxies existing method bodies to the shadow via ByteBuddy
3. Keeps your instances typed as the original class

Edit → save → **Force Re-sync** (or let the file watcher pick it up).

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Status bar: **Offline** | Run `javar run` (or attach `-javaagent`) and ensure port `19222` is free |
| CLI not found | Install `javar` or set `javar.cliPath` to the full path |
| No off-heap numbers | Set `JAVAR_NATIVE_PATH` / let `javar run` resolve `javar_core` |
| Force Re-sync does nothing | Confirm agent is up (`javar status`) and core is watching (`javar run --watch-only`) |

```bash
javar status --addr 127.0.0.1:19222
javar dashboard --addr 127.0.0.1:19222
javar run --flags-only
```

---

## More documentation

Full project docs, architecture, and CI builds:

**https://github.com/rabbittrix/JavaR**

---

## License

Apache-2.0  

© Roberto de Souza (`rabbittrix@hotmail.com`)

# JavaR Cockpit

<p align="center">
  <img src="media/logo.png" alt="JavaR" width="160" />
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
| **Start CLI / Core** | Runs `javar run` on your workspace (auto-start optional) |
| **Control Center** | Opens a terminal with `javar dashboard` (ratatui TUI) |
| **Off-Heap Regions** | Sidebar view of managed region / memory stats |

JavaR itself is a **Rust sidecar + Java agent**. This extension is the VS Code cockpit that talks to the agent on `127.0.0.1:19222` and launches the CLI.

---

## Prerequisites

1. **JavaR CLI** on your `PATH` (or set `javar.cliPath`)
2. **javar-agent** attached to your JVM (`-javaagent:...`)
3. A Java project (optionally with `javar.toml`)

### Install the CLI (from source)

```bash
git clone https://github.com/rabbittrix/JavaR.git
cd JavaR/javar-project
cargo install --path javar-cli
```

Release zips from GitHub Actions also include `bin/javar`, `lib/*javar_core*`, and `agent/javar-agent.jar`.

---

## Quick start

### 1. Run your app with the JavaR agent

```bash
java -javaagent:/path/to/javar-agent.jar=port=19222 -cp <classpath> com.example.Main
```

Optional native off-heap library:

```bash
# Windows
set JAVAR_NATIVE_PATH=C:\path\to\javar_core.dll

# Linux / macOS
export JAVAR_NATIVE_PATH=/path/to/libjavar_core.so   # or .dylib
```

### 2. Open the project in VS Code / Cursor

- Install **JavaR Cockpit** (`jrsf.javar`)
- Open a folder that contains Java sources (and ideally `javar.toml`)
- With `javar.autoStart` enabled (default), the extension starts `javar run` and connects to the agent

### 3. Use the Cockpit

| Action | How |
|--------|-----|
| See live memory | Look at the **status bar** (bottom left) |
| Hot deploy current file | Command Palette → **JavaR: Force Re-sync** (or the sync icon on `.java` editors) |
| Start watcher manually | **JavaR: Start CLI / Core** |
| Connect telemetry only | **JavaR: Connect Agent** |
| Full TUI dashboard | **JavaR: Open Control Center (TUI)** |
| Off-heap summary | Activity bar → **JavaR** → Off-Heap Regions |

---

## Commands

| Command | ID |
|---------|-----|
| JavaR: Force Re-sync | `javar.forceResync` |
| JavaR: Start CLI / Core | `javar.startCli` |
| JavaR: Open Control Center (TUI) | `javar.openDashboard` |
| JavaR: Connect Agent | `javar.connect` |

---

## Settings

Open **Settings → Extensions → JavaR**:

| Setting | Default | Meaning |
|---------|---------|---------|
| `javar.cliPath` | `javar` | Path to the CLI binary |
| `javar.agentHost` | `127.0.0.1` | Agent TCP host |
| `javar.agentPort` | `19222` | Agent TCP port |
| `javar.autoStart` | `true` | Auto-run `javar run` when a workspace opens |

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

Requires the **javar-agent** JAR on the JVM. The status bar / sidebar then show off-heap usage.

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
| Status bar: **Offline** | Start JVM with `-javaagent` and ensure port `19222` is free |
| CLI not found | Install `javar` or set `javar.cliPath` to the full path |
| No off-heap numbers | Set `JAVAR_NATIVE_PATH` / load `javar_core` native lib |
| Force Re-sync does nothing | Confirm agent is up (`javar status`) and core is watching the project |

```bash
javar status --addr 127.0.0.1:19222
javar dashboard
```

---

## More documentation

Full project docs, architecture, and CI builds:

**https://github.com/rabbittrix/JavaR**

---

## License

Apache-2.0  

© Roberto de Souza (`rabbittrix@hotmail.com`)

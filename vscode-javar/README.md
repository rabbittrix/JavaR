# JavaR Cockpit

<p align="center">
  <img src="https://raw.githubusercontent.com/rabbittrix/JavaR/main/vscode-javar/media/logo.png" alt="JavaR" width="160" />
</p>

**Zero-Restart Java** — sidecar watcher, off-heap telemetry, and Force Re-sync for Visual Studio Code / Cursor.

Author: **Roberto de Souza** (`rabbittrix@hotmail.com`)  
Publisher: **JRSF** · Identifier: `jrsf.javar` · Version: **0.1.8**

---

## What this extension does

| Feature | Description |
|---------|-------------|
| **Status bar** | `JavaR: Active` + project name + JVM heap vs off-heap |
| **Force Re-sync** | Saves the current file and nudges the agent to hot-deploy |
| **Start CLI / Core** | Starts **`javar run --watch-only`** only (never launches your app) |
| **Control Center** | Opens `javar dashboard` (reload history table) |
| **Off-Heap Regions** | Sidebar: project, regions, managed bytes |
| **Install / Repair CLI** | Downloads CLI into `~/.javar/bin` when missing |

The Cockpit is **not** an app launcher. Run your Maven / Spring Boot / IDE Run configuration as usual — ideally after:

```bash
javar enable --global
```

so every JVM picks up the agent via `JAVA_TOOL_OPTIONS`.

---

## Prerequisites

1. A Java / Maven / **Spring Boot** workspace  
2. **JavaR CLI** — auto-offered on activation, or install manually:

**Windows:**

```powershell
iwr https://javar.dev/install.ps1 | iex
javar setup
javar enable --global
```

**Linux / macOS:**

```bash
curl -fsSL https://javar.dev/install.sh | sh
javar setup
javar enable --global
```

Then **restart the IDE** so `JAVA_TOOL_OPTIONS` is visible to Run/Debug.

---

## Quick start

### 1. Invisible agent (recommended)

```bash
javar setup
javar enable --global
# restart Cursor / VS Code
```

Run the app with your normal workflow (`mvn spring-boot:run`, IDE Run, etc.).

### 2. Open the project in VS Code / Cursor

- Install **JavaR Cockpit** (`jrsf.javar`) or `javar-0.1.8.vsix`
- Open a folder with `pom.xml` / Gradle / Java sources
- With `javar.autoStart` (default), the extension starts:

  ```bash
  javar run <workspace> --watch-only --port 19222
  ```

### 3. Use the Cockpit

| Action | How |
|--------|-----|
| See live memory / project | Status bar (bottom left) |
| Hot deploy current file | **JavaR: Force Re-sync** (or sync icon on `.java` editors) |
| Start watcher | **JavaR: Start CLI / Core** → watch-only sidecar |
| Control Center TUI | **JavaR: Open Control Center** → reload history |
| Repair CLI | **JavaR: Install / Repair CLI** |

---

## Extension commands

| Command | ID | Behavior |
|---------|-----|----------|
| JavaR: Force Re-sync | `javar.forceResync` | Save + HotDeploy nudge |
| JavaR: Start CLI / Core | `javar.startCli` | `javar run … --watch-only` |
| JavaR: Open Control Center (TUI) | `javar.openDashboard` | `javar dashboard` |
| JavaR: Connect Agent | `javar.connect` | Poll telemetry on port 19222 |
| JavaR: Install / Repair CLI | `javar.installCli` | Install into `~/.javar/bin` |

---

## Settings

| Setting | Default | Meaning |
|---------|---------|---------|
| `javar.cliPath` | `javar` | Path to the CLI binary |
| `javar.agentHost` | `127.0.0.1` | Agent TCP host |
| `javar.agentPort` | `19222` | Agent TCP port |
| `javar.autoStart` | `true` | Auto-start watch-only sidecar |
| `javar.autoInstallCli` | `true` | Offer CLI install when missing |

---

## Useful CLI commands

| Command | Purpose |
|---------|---------|
| `javar enable --global` | `JAVA_TOOL_OPTIONS` for every JVM |
| `javar disable --global` | Remove agent from `JAVA_TOOL_OPTIONS` |
| `javar tools install` | Optional Maven bootstrap (never auto-prompted) |
| `javar build` | Explicit Maven/Gradle package |
| `javar run --watch-only` | Sidecar only |
| `javar dashboard` | TUI with reload history (time · class · change · version) |
| `javar status` | One-shot agent ping + telemetry |

`javar run` **never** prompts to build. If classes are missing it warns and watches passively.

---

## Transparent off-heap (`@JavaRManaged`)

```java
import com.javar.agent.managed.JavaRManaged;

@JavaRManaged
public class SensorReading {
    private int temperature; // off-heap
    private long timestamp;
    private String label;    // on-heap reference
}
```

Requires the agent on the JVM (`javar enable --global` or an explicit `-javaagent`).

---

## Structural hot-reload

1. Defines `YourClass$JavaR_vN` (shadow class)
2. Proxies method bodies via ByteBuddy
3. Keeps instances typed as the original class

Edit → save → **Force Re-sync** (or let the sidecar watcher pick it up).

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Status bar: **Watching (agent offline)** | Start the app with agent loaded (`javar enable --global` + restart IDE), or run `javar run --watch-only` and ensure something listens on `19222` |
| CLI not found | **JavaR: Install / Repair CLI** or set `javar.cliPath` |
| No off-heap numbers | Confirm native lib at `~/.javar/bin/javar_core.*` (`javar setup`) |
| Force Re-sync does nothing | `javar status` — agent must be up |
| Spring Boot plugin errors | Use `spring-boot-maven-plugin` (not `maven-spring-boot-plugin`) |

```bash
javar status --addr 127.0.0.1:19222
javar dashboard --addr 127.0.0.1:19222
echo $env:JAVA_TOOL_OPTIONS   # PowerShell: should show -javaagent:…javar-agent.jar
```

---

## More documentation

**https://github.com/rabbittrix/JavaR**

---

## License

Apache-2.0  

© Roberto de Souza (`rabbittrix@hotmail.com`)

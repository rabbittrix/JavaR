# JavaR Cockpit

<p align="center">
  <img src="https://raw.githubusercontent.com/rabbittrix/JavaR/main/vscode-javar/media/logo.png" alt="JavaR" width="160" />
</p>

**Zero-Restart Java** — workspace agent injection, sidecar watcher, off-heap telemetry, and Force Re-sync for Visual Studio Code / Cursor.

Author: **Roberto de Souza** (`rabbittrix@hotmail.com`)  
Publisher: **JRSF** · Identifier: `jrsf.javar` · Version: **0.1.11**

---

## What this extension does

| Feature | Description |
|---------|-------------|
| **Workspace inject** | Adds `-javaagent` to **this folder’s** Run/Debug `vmArgs`, integrated terminal env, and `.mvn/maven.config` for Spring Boot. **Never** sets user-global `JAVA_TOOL_OPTIONS`. |
| **Status bar** | `JavaR: Active` + project + port + heap / hist |
| **Force Re-sync** | Saves the file and retargets the sidecar to the live agent |
| **Start CLI / Core** | Starts **watch-only** sidecar (never launches your app) |
| **Control Center** | Opens `javar dashboard` |
| **Configure Workspace** | Re-apply Run / Maven / Spring Boot injection |
| **Install / Repair CLI** | Installs CLI into `~/.javar/bin` when missing |

After install, use **standard** Maven or Spring Boot commands (or IDE **Run Java**) — you do **not** need a special “JavaR Run” action.

---

## Prerequisites

1. A Java / Maven / Spring Boot workspace (`pom.xml` or Gradle)  
2. **JavaR CLI** (auto-offered on activation):

```powershell
# Windows
iwr https://javar.dev/install.ps1 | iex
javar setup
javar disable --global   # clear legacy global injection if present
```

```bash
# Linux / macOS
curl -fsSL https://javar.dev/install.sh | sh
javar setup
javar disable --global
```

---

## Quick start

### 1. Install the extension

```bash
code --install-extension javar-0.1.11.vsix
```

Or: Extensions → **Install from VSIX…** → `vscode-javar/javar-0.1.11.vsix`

### 2. Open your project

Open a folder with `pom.xml` / Gradle / Java sources. With defaults (`javar.injectWorkspace` + `javar.autoStart`):

1. Workspace settings get the agent flags  
2. `.mvn/maven.config` gets Spring Boot agent lines (if `pom.xml` exists)  
3. Watch-only sidecar starts  
4. Status bar polls `~/.javar/agents` for your app  

**Open a new integrated terminal** once so `JAVA_TOOL_OPTIONS` / `MAVEN_OPTS` apply.

CLI equivalent:

```bash
javar inject
```

### 3. Run as usual (Maven or Spring Boot)

```bash
# Spring Boot (agent via .mvn/maven.config after inject)
mvn -q -DskipTests package
mvn spring-boot:run

# Plain Maven exec
mvn -q exec:java -Dexec.mainClass=com.example.App

# Or use IDE "Run Java" / Debug / Spring Boot Dashboard
```

If the Control Center shows OFFLINE / “(awaiting reload)”, the agent is not on your app JVM (often Bloop held `:19222`). Run `javar inject`, restart the app, then `javar dashboard` again.

### 4. Hot-reload

Edit a `.java` file → Save. Sidecar compiles (`javac --release 21`) and redefines. Check:

- Status bar `hist N`  
- **JavaR: Open Control Center** → Hot-Reload history  

---

## Extension commands

| Command | ID | Behavior |
|---------|-----|----------|
| JavaR: Configure Workspace | `javar.configureWorkspace` | Inject agent into workspace Run/Debug + terminal + `.mvn/maven.config` |
| JavaR: Force Re-sync | `javar.forceResync` | Save + retarget sidecar |
| JavaR: Start CLI / Core | `javar.startCli` | `javar-core` / `javar run --watch-only` |
| JavaR: Open Control Center (TUI) | `javar.openDashboard` | `javar dashboard` |
| JavaR: Connect Agent | `javar.connect` | Poll telemetry |
| JavaR: Install / Repair CLI | `javar.installCli` | Install into `~/.javar/bin` |

---

## Settings

| Setting | Default | Meaning |
|---------|---------|---------|
| `javar.cliPath` | `javar` | Path to the CLI binary |
| `javar.agentHost` | `127.0.0.1` | Agent TCP host |
| `javar.agentPort` | `19222` | Preferred agent port (may auto-bump if busy / tooling) |
| `javar.autoStart` | `true` | Auto-start watch-only sidecar |
| `javar.autoInstallCli` | `true` | Offer CLI install when missing |
| `javar.injectWorkspace` | `true` | Auto workspace agent injection on activate |

### What gets written (workspace only)

- `java.debug.settings.vmArgs` → `-javaagent:…/javar-agent.jar=port=N -Djavar.native.path=…`  
- `terminal.integrated.env.*` → `JAVA_TOOL_OPTIONS`, `MAVEN_OPTS`, `JAVAR_AGENT_ADDR`, `JAVAR_PINNED_ADDR`, `JAVAR_AGENT_PORT`  
- `.mvn/maven.config` (Maven projects) → only these lines (no `#` comments):

```text
-Dspring-boot.run.agents=C:/Users/…/.javar/bin/javar-agent.jar
-Dspring-boot.run.jvmArguments=-Djavar.agent.port=19223
```

No changes to the Windows user Environment registry.

---

## Useful CLI commands

| Command | Purpose |
|---------|---------|
| `javar setup` | Extract agent + native lib; clean leftover global env |
| `javar inject` | Write `.vscode/settings.json` + `.mvn/maven.config` |
| `javar disable --global` | Strip legacy `JAVA_TOOL_OPTIONS` from the Registry |
| `javar run` | Explicit launch + pinned watcher (no IDE) |
| `javar run --watch-only` | Sidecar only |
| `javar build` | Maven/Gradle package |
| `javar dashboard` | TUI (auto-starts watcher; ignores Bloop / IDE agents) |
| `javar status` | Agent ping + telemetry |

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

Requires the agent on the **app** JVM (workspace inject, `javar run`, or explicit `-javaagent`).

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| Status bar: **Watching (agent offline)** | Start the app via Run Java / `mvn spring-boot:run`; or `javar run` |
| `Unrecognized maven.config entries: [#, BEGIN, …]` | Remove `#` comment lines from `.mvn/maven.config` — run **Configure Workspace** / `javar inject` again (0.1.11+) |
| Maven/Spring app has no agent | **JavaR: Configure Workspace**, then restart the app; confirm `javar setup` |
| Hot-reload hist=0 | Sidecar must be running (**Start CLI / Core**); Force Re-sync retargets port |
| Dashboard stuck “(awaiting reload)” on Bloop | Update CLI (`javar setup`); dashboard ignores tooling JVMs — restart Spring Boot after inject |
| 500 after reload | Update agent (`javar setup`) — double-transform on `@JavaRManaged` is fixed |
| Legacy global env noise | `javar disable --global` and restart the IDE |
| Wrong process / port | Dashboard `p` to switch; Cockpit prefers `*Application` / `javar-run` / `vscode` |

```bash
javar status --addr 127.0.0.1:19223
javar dashboard
```

---

## Package this VSIX

```bash
cd vscode-javar
npm install
npm run compile
npx @vscode/vsce package --no-dependencies -o javar-0.1.11.vsix
# → javar-0.1.11.vsix
```

---

## More documentation

**https://github.com/rabbittrix/JavaR**

---

## License

Apache-2.0  

© Roberto de Souza (`rabbittrix@hotmail.com`)

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
  <b>Zero-Restart Java</b> — invisible agent via <code>JAVA_TOOL_OPTIONS</code> · Spring Boot · Rust off-heap<br/>
  by <b>Roberto de Souza</b> (<a href="mailto:rabbittrix@hotmail.com">rabbittrix@hotmail.com</a>)
</p>

---

# JavaR

**One binary. Zero friction.** Enable JavaR once — every IDE and CLI JVM loads the agent. No need to wrap your app with `javar run`.

## Quick start (recommended)

```powershell
# 1) Install CLI
iwr https://javar.dev/install.ps1 | iex          # Windows
# curl -fsSL https://javar.dev/install.sh | sh  # Linux / macOS

# 2) Extract agent + native lib, then enable invisible mode
javar setup
javar enable --global

# 3) Restart your IDE / terminal, then run your app as usual
mvn spring-boot:run
# or:  java -jar target/my-app.jar
# or:  IDE Run / Debug

# 4) Telemetry (optional)
javar dashboard
```

`javar enable --global` sets the **user** environment variable:

```text
JAVA_TOOL_OPTIONS=-javaagent:%USERPROFILE%/.javar/bin/javar-agent.jar=port=19222 -Djavar.native.path=%USERPROFILE%/.javar/bin/javar_core.dll
```

(Linux/macOS uses `~/.javar/bin/…` with forward slashes.)

Disable later:

```bash
javar disable --global
```

---

## Install mirrors

| Platform | Command |
|----------|---------|
| Windows | `irm https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.ps1 \| iex` |
| Unix | `curl -fsSL https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.sh \| sh` |

Scripts in-repo: [`scripts/install.ps1`](scripts/install.ps1) · [`scripts/install.sh`](scripts/install.sh)

---

## CLI reference

| Command | Purpose |
|---------|---------|
| `javar setup` | Extract embedded agent JAR + native lib to `~/.javar/bin`, install CLI + sidecar, update PATH |
| `javar enable --global` | Inject agent via user `JAVA_TOOL_OPTIONS` (IDE-agnostic) |
| `javar disable --global` | Remove JavaR flags from `JAVA_TOOL_OPTIONS` |
| `javar tools install` | **Optional** — bootstrap Apache Maven under `~/.javar/tools` + `mvn` shim (never auto-prompted) |
| `javar build [PATH]` | Explicit Maven `clean package` / Gradle `build` |
| `javar run [PATH]` | Sidecar + optional JVM launch; **never prompts to build** — warns and watches if classes are missing |
| `javar run --watch-only` | Sidecar / file watcher only (what the VS Code Cockpit starts) |
| `javar run --flags-only` | Print the resolved `-javaagent` / `java` line |
| `javar run -- com.example.App` | Launch with discovered/explicit main (if classes exist) |
| `javar status` | Ping agent + print telemetry |
| `javar dashboard` / `javar tui` | Control Center TUI (project name, heap vs off-heap, **reload history**) |
| `javar init [PATH]` | Scaffold `javar.toml` + sample main |

### Passive `javar run`

- Does **not** ask `Build now? (Y/n)`
- Does **not** auto-install Maven (use `javar tools install` if you need it)
- If `target/classes` / Boot jar is missing → warning + **passive watcher** (exit 0 path for sidecar)
- Prefer invisible mode for day-to-day app launches

---

## Spring Boot

Use the correct plugin id: `spring-boot-maven-plugin` (not `maven-spring-boot-plugin`).

```bash
javar enable --global
mvn -DskipTests package
mvn spring-boot:run
# other terminal:
javar dashboard
```

JavaR detects Spring Boot from `pom.xml`, prefers `<start-class>` / `<mainClass>`, and can launch a fat jar when you explicitly use `javar run` after `mvn package`.

---

## Off-heap (`@JavaRManaged`)

```java
import com.javar.agent.managed.JavaRManaged;

@JavaRManaged
public class SensorReading {
    private int temperature; // off-heap
    private long timestamp;
    private String label;    // on-heap reference
}
```

---

## What you get

| Feature | Detail |
|---------|--------|
| Invisible agent | `JAVA_TOOL_OPTIONS` → IntelliJ, Eclipse, VS Code, `mvn`, `java` |
| Embedded assets | Agent JAR + native lib inside the CLI; extracted to `~/.javar/bin` |
| Structural hot-reload | Shadow classes `Original$JavaR_vN` — no JVM restart |
| Off-heap memory | `@JavaRManaged` primitives live in Rust (Panama / JNI) |
| Control Center | Reload history: **time · class · change · version** |
| VS Code Cockpit | Extension `jrsf.javar` — sidecar + telemetry + Force Re-sync (**never launches your app**) |

---

## VS Code / Cursor

Install **JavaR Cockpit** (`jrsf.javar`) from the Marketplace, or:

```bash
code --install-extension vscode-javar/javar-0.1.8.vsix
```

The extension only:

1. Ensures the CLI is installed (optional prompt)
2. Starts `javar run --watch-only` (sidecar)
3. Polls agent telemetry on `127.0.0.1:19222`
4. Provides **Force Re-sync**

Run the app via your IDE or `mvn` after `javar enable --global`.

---

## Dev build (contributors)

```bash
cd javar-project
cargo build --release -p javar-core
cargo build --release -p javar-cli   # build.rs embeds agent via internal Maven
./target/release/javar setup
./target/release/javar enable --global
```

---

**Author / owner:** Roberto de Souza · `rabbittrix@hotmail.com`  
**License:** Apache-2.0  
**Repo:** https://github.com/rabbittrix/JavaR

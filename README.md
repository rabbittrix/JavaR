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
  <b>Zero-Restart Java</b> — self-bootstrapping CLI · structural hot-reload · Rust off-heap<br/>
  by <b>Roberto de Souza</b> (<a href="mailto:rabbittrix@hotmail.com">rabbittrix@hotmail.com</a>)
</p>

---

# JavaR

**One binary. Zero config.** The `javar` CLI embeds the Java agent and native library, installs itself on your PATH, and smart-launches your app.

## Install

**Windows (PowerShell):**

```powershell
iwr https://javar.dev/install.ps1 | iex
```

**Linux / macOS:**

```bash
curl -fsSL https://javar.dev/install.sh | sh
```

GitHub raw mirrors (same scripts in this repo):

```powershell
# Windows
irm https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.ps1 | iex
```

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.sh | sh
```

| Script | Path |
|--------|------|
| Windows | [`scripts/install.ps1`](scripts/install.ps1) |
| Unix | [`scripts/install.sh`](scripts/install.sh) |

The installer:

1. Downloads the latest GitHub release zip for your OS (or builds from source if none)
2. Installs into `~/.javar/bin` (`%USERPROFILE%\.javar\bin` on Windows)
3. Runs `javar setup` — extracts embedded agent/native assets and adds the dir to your PATH

Optional: set `JAVAR_REPO=owner/name` to install from a fork.

## Run

```bash
# In any Maven / Gradle / javar.toml project:
javar run
```

That’s it. JavaR will:

1. Extract the **embedded** agent JAR + native lib to `~/.javar/bin/` (if missing)  
2. Detect `pom.xml` / `build.gradle` and offer to **build** if classes are missing (`javar build`)  
3. Find a `public static void main`  
4. Start the sidecar + JVM with absolute `-javaagent` and native path already set  

```bash
javar setup                 # extract embedded agent/native + PATH
javar build                 # Maven/Gradle package (PowerShell-safe, no &&)
javar run                   # smart launch with embedded -javaagent
javar run --watch-only      # sidecar only (IDE cockpit)
javar run -- com.example.App
javar dashboard             # Control Center TUI
javar status
```

## What you get

| Feature | Detail |
|---------|--------|
| Structural hot-reload | Shadow classes `Original$JavaR_vN` — no JVM restart |
| Off-heap memory | `@JavaRManaged` primitives live in Rust |
| Control Center | `javar dashboard` (heap vs off-heap, shadows, GC) |
| VS Code Cockpit | Extension `jrsf.javar` — telemetry + Force Re-sync |

```java
import com.javar.agent.managed.JavaRManaged;

@JavaRManaged
public class SensorReading {
    private int temperature; // off-heap
    private long timestamp;
    private String label;    // on-heap reference
}
```

## Dev build (contributors)

```bash
cd javar-project
cargo build --release -p javar-core
# build.rs auto-runs: mvn -f javar-agent/pom.xml clean package -DskipTests
# (no shell && — works in PowerShell)
cargo build --release -p javar-cli
./target/release/javar setup
```

Requires Maven on `PATH`. Emergency compile without agent: `JAVAR_SKIP_AGENT_EMBED=1`.

---

**Author / owner:** Roberto de Souza · `rabbittrix@hotmail.com`  
**License:** Apache-2.0  
**Repo:** https://github.com/rabbittrix/JavaR

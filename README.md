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
  <b>Zero-Restart Java</b> — self-bootstrapping CLI · Spring Boot · Maven auto-install · Rust off-heap<br/>
  by <b>Roberto de Souza</b> (<a href="mailto:rabbittrix@hotmail.com">rabbittrix@hotmail.com</a>)
</p>

---

# JavaR

**One binary. Zero config.** The `javar` CLI embeds the Java agent and native library, installs itself (and Maven if needed) on your PATH, and smart-launches Maven, Gradle, and **Spring Boot** apps.

## Install

**Windows (PowerShell):**

```powershell
iwr https://javar.dev/install.ps1 | iex
```

**Linux / macOS:**

```bash
curl -fsSL https://javar.dev/install.sh | sh
```

GitHub raw mirrors:

```powershell
irm https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.ps1 | iex
```

```bash
curl -fsSL https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.sh | sh
```

The installer:

1. Places the CLI in `~/.javar/bin` and runs `javar setup`
2. Extracts the **embedded** agent JAR + native lib
3. Bootstraps **Apache Maven** into `~/.javar/tools` when missing and adds a `mvn` shim on PATH
4. Installs the `javar-core` sidecar for file watching

**VS Code / Cursor:** install the **JavaR Cockpit** extension (`jrsf.javar`). It offers **automatic CLI install** if `javar` is not on PATH.

## Run

```bash
# In any Maven / Gradle / Spring Boot / javar.toml project:
javar run
```

JavaR will:

1. Extract the embedded agent + native lib to `~/.javar/bin/` if needed  
2. Detect Maven / Gradle / **Spring Boot** and offer to build (`javar build`)  
3. Auto-install Maven into `~/.javar/tools` when the project is Maven and `mvn` is missing  
4. Launch Spring Boot via the executable fat jar (`java -jar`) when present  
5. Show the **project name** on `javar dashboard` and in the VS Code status bar  

```bash
javar setup                 # agent + native + Maven shim + PATH
javar build                 # Maven/Gradle package (Spring Boot fat jar)
javar run                   # smart launch with embedded -javaagent
javar run --watch-only      # sidecar only (IDE cockpit)
javar run -- com.example.App
javar dashboard             # Control Center TUI (shows project name)
javar status
```

## Spring Boot

```bash
cd my-spring-app
javar run          # builds if needed, then java -javaagent:… -jar target/*.jar
```

- Detects `spring-boot` in `pom.xml`
- Prefers `<start-class>` / `<mainClass>`
- Uses the packaged Boot jar when available; otherwise `target/classes` + Maven runtime classpath

## What you get

| Feature | Detail |
|---------|--------|
| Structural hot-reload | Shadow classes `Original$JavaR_vN` — no JVM restart |
| Off-heap memory | `@JavaRManaged` primitives live in Rust |
| Maven auto-install | Downloaded under `~/.javar/tools`, shim in `~/.javar/bin` |
| Spring Boot | Fat-jar launch + dependency classpath fallback |
| Control Center | `javar dashboard` — project name, heap vs off-heap |
| VS Code Cockpit | Extension `jrsf.javar` — auto CLI install + telemetry |

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
cargo build --release -p javar-cli   # build.rs embeds agent via internal Maven
./target/release/javar setup
```

---

**Author / owner:** Roberto de Souza · `rabbittrix@hotmail.com`  
**License:** Apache-2.0  
**Repo:** https://github.com/rabbittrix/JavaR

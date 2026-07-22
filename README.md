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
  <b>Zero-Restart Java</b> — workspace agent · Maven · Spring Boot · Rust off-heap<br/>
  by <b>Roberto de Souza</b> (<a href="mailto:rabbittrix@hotmail.com">rabbittrix@hotmail.com</a>)
</p>

---

# JavaR

Hot-reload Java without restarting the JVM. A **Rust sidecar** watches sources, compiles with `--release 21`, and pushes bytecode to a **Java agent**. Optional **`@JavaRManaged`** keeps primitives off-heap.

**Global `JAVA_TOOL_OPTIONS` injection is removed** (it conflicted with IDE language servers). Prefer:

1. **VS Code / Cursor Cockpit** — workspace-scoped agent for Run Java / `mvn` / Spring Boot  
2. **`javar inject`** — write project inject files (including `.mvn/maven.config`)  
3. **`javar run`** — explicit CLI launch + pinned watcher  

---

## Quick start

### A) VS Code / Cursor (recommended for Maven & Spring Boot)

```powershell
# 1) Install CLI once
iwr https://javar.dev/install.ps1 | iex          # Windows
# curl -fsSL https://javar.dev/install.sh | sh  # Linux / macOS
javar setup
javar disable --global   # clear any leftover global JAVA_TOOL_OPTIONS

# 2) Install extension
code --install-extension vscode-javar/javar-0.1.11.vsix
# or: Cursor → Extensions → Install from VSIX…
```

Open your **Maven** or **Spring Boot** folder. On activate the Cockpit:

- Injects `-javaagent` into **workspace** `java.debug.settings.vmArgs` (Run / Debug)  
- Sets **integrated terminal** `JAVA_TOOL_OPTIONS` / `MAVEN_OPTS` (this workspace only)  
- Writes **`.mvn/maven.config`** (`spring-boot.run.agents` + port) so `mvn spring-boot:run` works from any terminal  
- Starts the **watch-only sidecar**  

Then use your normal workflow — **no `javar run` required**:

```bash
mvn -q -DskipTests compile
mvn spring-boot:run
# or: Run Java / Debug from the IDE
# or: mvn exec:java …
```

Open a **new** terminal after first activate so env injection applies. Monitor: **JavaR: Open Control Center** or `javar dashboard`.

CLI equivalent (no extension):

```bash
cd your-spring-project
javar inject
mvn spring-boot:run
# other terminal:
javar dashboard
```

### B) Explicit CLI (`javar run`)

```bash
cd your-maven-or-spring-project
javar run
# other terminal:
javar dashboard
```

`javar run` injects `-javaagent` + native path on **that process only**, pins the watcher to its port, and registers `~/.javar/agents/<pid>.json`.

---

## Install mirrors

| Platform | Command |
|----------|---------|
| Windows | `irm https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.ps1 \| iex` |
| Unix | `curl -fsSL https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.sh \| sh` |

In-repo: [`scripts/install.ps1`](scripts/install.ps1) · [`scripts/install.sh`](scripts/install.sh)

---

## CLI reference

| Command | Purpose |
|---------|---------|
| `javar setup` | Extract agent JAR + native lib to `~/.javar/bin`, install CLI + sidecar, update PATH. **Never** sets global agent env; strips leftovers. |
| `javar inject [PATH]` | Write workspace inject: `.vscode/settings.json` + `.mvn/maven.config` (no `#` comments — Maven rejects them) |
| `javar disable --global` | Emergency cleanup: remove JavaR from user/machine `JAVA_TOOL_OPTIONS` / `JAVAR_NATIVE_PATH` |
| `javar enable --global` | **Removed** — only cleans leftovers and points you to `javar inject` / `javar run` / the Cockpit |
| `javar run [PATH]` | Explicit pinned sidecar + launch JVM with `-javaagent` (strips inherited `JAVA_TOOL_OPTIONS`) |
| `javar run --watch-only` | Sidecar only (what the Cockpit starts) |
| `javar run --flags-only` | Print resolved `-javaagent` / `java` line |
| `javar build [PATH]` | Maven `clean package` / Gradle `build` |
| `javar tools install` | Optional Maven under `~/.javar/tools` (never auto-prompted) |
| `javar status` | Ping agent + telemetry JSON |
| `javar dashboard` / `javar tui` | Control Center — skips Bloop/IDE noise; auto-connects when your app registers |
| `javar init [PATH]` | Scaffold `javar.toml` + sample main |
| `javar uninstall` | Cleanup env + delete `~/.javar` |

### Plain Maven vs Spring Boot

| Workflow | How the agent loads |
|----------|---------------------|
| Cockpit / `javar inject` + `mvn spring-boot:run` | `.mvn/maven.config` → `spring-boot.run.agents` (+ port via `jvmArguments`) |
| Cockpit + Run Java / Debug | `java.debug.settings.vmArgs` |
| Cockpit + integrated terminal `mvn` / `exec:java` | Terminal `JAVA_TOOL_OPTIONS` / `MAVEN_OPTS` |
| No IDE | `javar run` (or pass `-javaagent` yourself) |

`.mvn/maven.config` must contain **only** Maven CLI args (one per line). Do **not** put `#` comments there — older Maven versions treat them as unrecognized options.

Use the Spring plugin id: `spring-boot-maven-plugin` (not `maven-spring-boot-plugin`).

---

## Hot-reload notes

- Sidecar watches `src/**/*.java`, compiles with isolated `javac --release 21`, then redefines.  
- Dashboard / Cockpit keep a **pinned** watcher on the live **app** agent port (never Bloop / Metals).  
- `@JavaRManaged` classes are prepared for HotSwap once — **no double-transform** (avoids 500 after reload).  
- Prefer method-body constants for demos (e.g. `defaultCurrency()`) so string changes show on the next request.

```java
import com.javar.agent.managed.JavaRManaged;

@JavaRManaged
public class Trade {
    private long id;
    private double price;
    private String currency;

    public Trade(long id, double price) {
        this.id = id;
        this.price = price;
        this.currency = defaultCurrency(); // edit + Save → hot-reload
    }

    private static String defaultCurrency() { return "EUR"; }
}
```

---

## What you get

| Feature | Detail |
|---------|--------|
| Workspace agent | Cockpit / `javar inject` — per-folder Run/Debug + terminal + `.mvn/maven.config` |
| Explicit CLI | `javar run` for isolated launches |
| Embedded assets | Agent JAR + native lib inside the CLI → `~/.javar/bin` |
| Structural hot-reload | Shadow classes `Original$JavaR_vN` when needed |
| Off-heap | `@JavaRManaged` primitives in Rust (Panama / JNI) |
| Control Center | Reload history: **time · class · change · version** |
| VS Code Cockpit | `jrsf.javar` **0.1.11** — inject + sidecar + Force Re-sync |

---

## VS Code / Cursor

See [`vscode-javar/README.md`](vscode-javar/README.md).

```bash
code --install-extension vscode-javar/javar-0.1.11.vsix
```

---

## Dev build (contributors)

```bash
cd javar-project
cargo build --release -p javar-core
cargo build --release -p javar-cli   # build.rs embeds agent via Maven
./target/release/javar setup
# optional: package extension
cd ../vscode-javar && npm install && npm run package
# → javar-0.1.11.vsix
```

---

**Author / owner:** Roberto de Souza · `rabbittrix@hotmail.com`  
**License:** Apache-2.0  
**Repo:** https://github.com/rabbittrix/JavaR

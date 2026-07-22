# JavaR project (Rust workspace + Java agent)

This directory is the **source tree** for the CLI, sidecar, and agent.

| Path | Role |
|------|------|
| `javar-core/` | Rust sidecar — watch `src/**/*.java`, `javac --release 21`, redefine |
| `javar-cli/` | `javar` binary — `run`, `inject`, `dashboard`, `setup`, … |
| `javar-agent/` | Java agent JAR (`-javaagent`) |

End-user docs and install instructions live in the repo root:

→ **[../README.md](../README.md)**  
→ **[../vscode-javar/README.md](../vscode-javar/README.md)**

## Build

```bash
cd javar-project
cargo build --release -p javar-core
cargo build --release -p javar-cli
# agent is built by javar-cli build.rs (Maven) or:
cd javar-agent && mvn -q -DskipTests package
```

Install local binaries:

```bash
./target/release/javar setup
```

## Useful local commands

```bash
# Per-project inject for mvn spring-boot:run / Run Java (no global env)
javar inject /path/to/spring-app

# Explicit launch
javar run /path/to/spring-app

# Monitor (skips Bloop / IDE JVMs)
javar dashboard
```

`javar inject` writes:

- `.vscode/settings.json` — `java.debug.settings.vmArgs` + terminal env  
- `.mvn/maven.config` — **only** `-Dspring-boot.run.agents=…` and `-Dspring-boot.run.jvmArguments=…` (no `#` comments)

**Author:** Roberto de Souza · `rabbittrix@hotmail.com`

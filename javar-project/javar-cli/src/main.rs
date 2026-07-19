//! JavaR CLI — orchestrate build, agent injection, Control Center TUI.

mod dashboard;
mod smart_run;

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use clap::{Parser, Subcommand};
use javar_core::protocol::{Frame, Message, MessageKind};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "javar",
    version,
    about = "JavaR — high-performance Java hot-reload engine",
    long_about = "IDE-agnostic CLI for the JavaR Rust core + Java agent sidecar."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Scaffold a JavaR-enabled project (config + sample layout).
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Start the sidecar and smart-launch `java` with the agent + native lib.
    ///
    /// Detects Maven/Gradle, `target/classes` or `build/classes`, finds a main class,
    /// and injects `-javaagent` / `-Djavar.native.path`. Extra java args go after `--`:
    /// `javar run` · `javar run . -- -cp app.jar Main`
    Run {
        /// Project directory (optional; defaults to `.`).
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        /// Explicit path to javar-agent.jar (auto-discovers if omitted).
        #[arg(long)]
        agent: Option<PathBuf>,
        /// Agent listen port
        #[arg(long, default_value_t = 19222)]
        port: u16,
        /// Only print resolved flags / launch line; do not start processes
        #[arg(long)]
        flags_only: bool,
        /// Do not spawn javar-core (useful when only launching java)
        #[arg(long)]
        no_core: bool,
        /// Only start the watcher/sidecar — do not auto-launch a JVM
        #[arg(long)]
        watch_only: bool,
        /// Arguments for `java` — everything after `--`
        /// (e.g. `javar run . -- -cp app.jar Main`).
        #[arg(last = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Probe agent socket and print telemetry / status.
    Status {
        #[arg(long, default_value = "127.0.0.1:19222")]
        addr: String,
    },
    /// Open the JavaR Control Center (ratatui dashboard).
    Dashboard {
        #[arg(long, default_value = "127.0.0.1:19222")]
        addr: String,
    },
    /// Alias for `dashboard`.
    Tui {
        #[arg(long, default_value = "127.0.0.1:19222")]
        addr: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let is_tui = matches!(cli.command, Commands::Dashboard { .. } | Commands::Tui { .. });
    if !is_tui {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .init();
    }

    match cli.command {
        Commands::Init { path } => cmd_init(&path),
        Commands::Run {
            path,
            agent,
            port,
            flags_only,
            no_core,
            watch_only,
            args,
        } => {
            let path = path.unwrap_or_else(|| PathBuf::from("."));
            cmd_run(
                &path,
                agent.as_deref(),
                port,
                flags_only,
                no_core,
                watch_only,
                &args,
            )
        }
        Commands::Status { addr } => cmd_status(&addr),
        Commands::Dashboard { addr } | Commands::Tui { addr } => dashboard::run_dashboard(addr),
    }
}

fn cmd_init(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let config = path.join("javar.toml");
    if !config.exists() {
        fs::write(
            &config,
            r#"# JavaR project configuration
# Author: Roberto de Souza <rabbittrix@hotmail.com>

[project]
name = "app"

[watch]
paths = ["src", "target/classes"]
debounce_ms = 120

[agent]
port = 19222

[build]
source_roots = ["src/main/java", "src"]
output_dir = "target/classes"
"#,
        )?;
        info!(path = %config.display(), "wrote javar.toml");
    } else {
        warn!("javar.toml already exists");
    }

    let src = path.join("src/main/java/com/example");
    fs::create_dir_all(&src)?;
    let sample = src.join("HelloJavaR.java");
    if !sample.exists() {
        fs::write(
            &sample,
            r#"package com.example;

/** Sample app — edit me and watch JavaR hot-reload. */
public class HelloJavaR {
    public static void main(String[] args) throws Exception {
        int n = 0;
        while (true) {
            System.out.println("Hello from JavaR #" + n);
            n++;
            Thread.sleep(2000);
        }
    }
}
"#,
        )?;
    }

    println!("JavaR project initialized at {}", path.display());
    println!("Next: javar run   (or: javar run . -- com.example.HelloJavaR)");
    Ok(())
}

fn cmd_run(
    path: &Path,
    agent: Option<&Path>,
    port: u16,
    flags_only: bool,
    no_core: bool,
    watch_only: bool,
    args: &[String],
) -> Result<()> {
    let agent_jar = resolve_agent_jar(path, agent)?;
    let agent_abs = absolute_path(&agent_jar)?;
    let agent_flag = format!("-javaagent:{}=port={}", agent_abs.display(), port);

    let project = smart_run::SmartProject::discover(path);
    let native = smart_run::resolve_native_library(path);

    println!("# JavaR smart run — {}", smart_run::describe_project(&project));
    println!("# JavaR agent: {}", agent_abs.display());
    if let Some(ref n) = native {
        println!("# Native lib:  {}", n.display());
    } else {
        println!("# Native lib:  (not found — set JAVAR_NATIVE_PATH)");
    }
    println!("# Inject flag:");
    println!("{}", agent_flag);
    println!();
    println!("JAVAR_AGENT_ADDR=127.0.0.1:{}", port);

    let want_java = !watch_only
        && ( !args.is_empty()
            || smart_run::can_smart_launch(&project) );

    let java_argv = if want_java {
        match smart_run::build_java_launch_args(&project, args, native.as_deref()) {
            Ok(v) => Some(v),
            Err(e) if args.is_empty() && !watch_only => {
                warn!("smart java launch skipped: {e:#}");
                None
            }
            Err(e) => return Err(e),
        }
    } else {
        None
    };

    if flags_only {
        if let Some(ref argv) = java_argv {
            print!("# Equivalent java launch:\njava {}", agent_flag);
            for a in argv {
                print!(" {}", shell_escape(a));
            }
            println!();
        }
        return Ok(());
    }

    let addr = format!("127.0.0.1:{port}");
    let mut core_child = None;

    if !no_core {
        info!("starting javar-core sidecar");
        core_child = Some(spawn_core(path, &addr)?);
    }

    if let Some(java_argv) = java_argv {
        info!(
            ?java_argv,
            agent = %agent_abs.display(),
            "launching java with JavaR agent"
        );
        let mut cmd = Command::new("java");
        cmd.arg(&agent_flag)
            .args(&java_argv)
            .env("JAVAR_AGENT_ADDR", &addr)
            .current_dir(&project.root)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        if let Some(ref n) = native {
            cmd.env("JAVAR_NATIVE_PATH", n);
        }

        let status = cmd.status().context("spawn java")?;

        if let Some(mut child) = core_child {
            let _ = child.kill();
            let _ = child.wait();
        }

        if !status.success() {
            bail!("java exited with {status}");
        }
        return Ok(());
    }

    // Watcher / sidecar only.
    if let Some(mut child) = core_child {
        let status = child.wait().context("wait javar-core")?;
        if !status.success() {
            bail!("javar-core exited with {status}");
        }
    }

    Ok(())
}

fn spawn_core(path: &Path, addr: &str) -> Result<std::process::Child> {
    if let Some(bin) = resolve_core_bin(path) {
        return Command::new(bin)
            .arg(path)
            .env("JAVAR_AGENT_ADDR", addr)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .context("spawn javar-core");
    }

    Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "javar-core",
            "--",
            path.to_str().unwrap_or("."),
        ])
        .current_dir(workspace_root(path))
        .env("JAVAR_AGENT_ADDR", addr)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("cargo run javar-core")
}

fn shell_escape(s: &str) -> String {
    if s.chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\''))
    {
        format!("\"{}\"", s.replace('\"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Resolve `path` to an absolute filesystem path.
fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
    }
    let abs = std::env::current_dir()
        .context("current_dir")?
        .join(path);
    Ok(abs.canonicalize().unwrap_or(abs))
}

fn cmd_status(addr: &str) -> Result<()> {
    let mut stream =
        TcpStream::connect(addr).with_context(|| format!("connect to agent at {addr}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;

    let ping = Frame::encode(&Message::ping());
    stream.write_all(&ping.header)?;
    stream.write_all(&ping.payload)?;
    stream.flush()?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).context("read pong")?;
    let (msg, _) = Frame::decode(&buf[..n]).context("decode pong")?;
    println!("agent: connected ({addr})");
    println!("ping:  {:?}", msg.kind);

    let tel = Frame::encode(&Message {
        kind: MessageKind::Telemetry,
        body: Bytes::new(),
    });
    stream.write_all(&tel.header)?;
    stream.write_all(&tel.payload)?;
    stream.flush()?;

    let n = stream.read(&mut buf).context("read telemetry")?;
    let (msg, _) = Frame::decode(&buf[..n]).context("decode telemetry")?;
    if msg.kind == MessageKind::Telemetry {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&msg.body) {
            println!("telemetry:");
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            println!("telemetry: {}", String::from_utf8_lossy(&msg.body));
        }
    } else {
        println!("status kind: {:?}", msg.kind);
    }
    Ok(())
}

/// Resolve the agent JAR.
/// Prefer `--agent`, then `../javar-agent/target/*.jar`, then workspace/project paths.
fn resolve_agent_jar(project: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if p.exists() {
            return Ok(p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        }
        bail!("agent JAR not found: {}", p.display());
    }

    if let Ok(env) = std::env::var("JAVAR_AGENT_JAR") {
        let p = PathBuf::from(&env);
        if p.exists() {
            return Ok(p.canonicalize().unwrap_or(p));
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // Relative to CWD: ../javar-agent/target/
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    candidates.push(cwd.join("../javar-agent/target"));
    candidates.push(project.join("../javar-agent/target"));
    candidates.push(workspace_root(project).join("javar-agent/target"));
    candidates.push(project.join("javar-agent/target"));
    candidates.push(project.join("lib"));
    candidates.push(cwd.join("lib"));

    for dir in &candidates {
        if let Some(jar) = find_agent_jar_in_dir(dir) {
            info!(path = %jar.display(), "resolved javar-agent jar");
            return Ok(jar.canonicalize().unwrap_or(jar));
        }
    }

    // Named fallbacks
    let named = [
        workspace_root(project).join("javar-agent/target/javar-agent-0.1.0.jar"),
        cwd.join("../javar-agent/target/javar-agent-0.1.0.jar"),
        project.join("lib/javar-agent.jar"),
    ];
    for c in &named {
        if c.exists() {
            return Ok(c.canonicalize().unwrap_or_else(|_| c.clone()));
        }
    }

    // Try Maven build once
    let agent_dir = workspace_root(project).join("javar-agent");
    if agent_dir.join("pom.xml").exists() {
        info!("building javar-agent with Maven");
        let mvn = if cfg!(windows) { "mvn.cmd" } else { "mvn" };
        let status = Command::new(mvn)
            .args(["-q", "-DskipTests", "package"])
            .current_dir(&agent_dir)
            .status();
        if let Ok(st) = status {
            if st.success() {
                if let Some(jar) = find_agent_jar_in_dir(&agent_dir.join("target")) {
                    return Ok(jar);
                }
            }
        }
    }

    bail!(
        "javar-agent jar not found. Looked in ../javar-agent/target/ and workspace. \
         Build with: cd javar-agent && mvn package  (or pass --agent <path>)"
    )
}

fn find_agent_jar_in_dir(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let mut jars: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("jar")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| {
                        let lower = n.to_ascii_lowercase();
                        lower.contains("javar-agent") && !lower.contains("sources") && !lower.contains("javadoc")
                    })
                    .unwrap_or(false)
        })
        .collect();

    // Prefer non-original / shaded artifact names; sort for stability.
    jars.sort();
    // Prefer exact versioned name if present.
    if let Some(preferred) = jars.iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("javar-agent-") && n.ends_with(".jar"))
            .unwrap_or(false)
    }) {
        return Some(preferred.clone());
    }
    jars.pop()
}

fn resolve_core_bin(project: &Path) -> Option<PathBuf> {
    let root = workspace_root(project);
    [
        root.join("target/release/javar-core.exe"),
        root.join("target/release/javar-core"),
        root.join("target/debug/javar-core.exe"),
        root.join("target/debug/javar-core"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

pub(crate) fn workspace_root(hint: &Path) -> PathBuf {
    let hint = hint.canonicalize().unwrap_or_else(|_| hint.to_path_buf());
    if hint.join("Cargo.toml").exists() && hint.join("javar-core").exists() {
        return hint;
    }
    if let Some(parent) = hint.parent() {
        if parent.join("Cargo.toml").exists() && parent.join("javar-core").exists() {
            return parent.to_path_buf();
        }
    }
    let mut cur = hint.as_path();
    loop {
        if cur.join("Cargo.toml").exists() && cur.join("javar-core").is_dir() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }
    PathBuf::from(".")
}

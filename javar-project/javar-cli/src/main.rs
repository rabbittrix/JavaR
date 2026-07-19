//! JavaR CLI — orchestrate build, agent injection, Control Center TUI.

mod dashboard;

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
    /// Build agent/core (if needed), start watching, and print JVM inject flags.
    Run {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Agent listen port
        #[arg(long, default_value_t = 19222)]
        port: u16,
        /// Skip spawning javar-core (print flags only)
        #[arg(long)]
        flags_only: bool,
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
            port,
            flags_only,
        } => cmd_run(&path, port, flags_only),
        Commands::Status { addr } => cmd_status(&addr),
        Commands::Dashboard { addr } | Commands::Tui { addr } => {
            dashboard::run_dashboard(addr)
        }
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
    println!("Next: javar run");
    Ok(())
}

fn cmd_run(path: &Path, port: u16, flags_only: bool) -> Result<()> {
    let agent_jar = resolve_agent_jar(path)?;
    let java_opts = format!("-javaagent:{}=port={}", agent_jar.display(), port);

    println!("# Inject the agent into your JVM:");
    println!("export JAVA_TOOL_OPTIONS='{}'", java_opts);
    println!("# or:");
    println!("java {} -cp <classpath> com.example.Main", java_opts);
    println!();
    println!("JAVAR_AGENT_ADDR=127.0.0.1:{}", port);

    if flags_only {
        return Ok(());
    }

    let core_bin = resolve_core_bin(path);
    let addr = format!("127.0.0.1:{port}");

    info!(?core_bin, "starting javar-core sidecar");
    match core_bin {
        Some(bin) => {
            let status = Command::new(bin)
                .arg(path)
                .env("JAVAR_AGENT_ADDR", &addr)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .context("spawn javar-core")?;
            if !status.success() {
                bail!("javar-core exited with {status}");
            }
        }
        None => {
            let status = Command::new("cargo")
                .args([
                    "run",
                    "-p",
                    "javar-core",
                    "--",
                    path.to_str().unwrap_or("."),
                ])
                .current_dir(workspace_root(path))
                .env("JAVAR_AGENT_ADDR", &addr)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .context("cargo run javar-core")?;
            if !status.success() {
                bail!("javar-core exited with {status}");
            }
        }
    }
    Ok(())
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

fn resolve_agent_jar(project: &Path) -> Result<PathBuf> {
    let candidates = [
        workspace_root(project).join("javar-agent/target/javar-agent-0.1.0.jar"),
        project.join("javar-agent/target/javar-agent-0.1.0.jar"),
        project.join("lib/javar-agent.jar"),
    ];
    for c in &candidates {
        if c.exists() {
            return Ok(c.canonicalize().unwrap_or_else(|_| c.clone()));
        }
    }
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
                let jar = agent_dir.join("target/javar-agent-0.1.0.jar");
                if jar.exists() {
                    return Ok(jar);
                }
            }
        }
    }
    bail!("javar-agent jar not found. Build with: cd javar-agent && mvn package")
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

fn workspace_root(hint: &Path) -> PathBuf {
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

//! JavaR CLI — self-bootstrapping orchestrator + Control Center TUI.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>

mod dashboard;
mod embed;
mod global_mode;
mod maven;
mod setup;
mod smart_build;
mod smart_run;
mod style;
mod tools_cmd;

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
use tracing::warn;
use tracing_subscriber::EnvFilter;

/// Embedded agent JAR — produced by `build.rs` via internal Maven before compile.
/// Path is relative to this file (`javar-cli/src/main.rs`).
pub(crate) const AGENT_BYTES: &[u8] =
    include_bytes!("../../javar-agent/target/javar-agent.jar");

#[derive(Parser, Debug)]
#[command(
    name = "javar",
    version,
    about = "JavaR — Zero-Restart Java (self-bootstrapping CLI)",
    long_about = "Single-binary CLI: embeds agent JAR + native lib, smart-builds Maven/Gradle projects, and hot-reloads Java without JVM restart."
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
    /// Install runtime assets to ~/.javar and add the CLI to PATH.
    Setup,
    /// Build the current Java project (Maven `clean package` or Gradle `build`).
    Build {
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },
    /// Start the sidecar and smart-launch `java` with the agent + native lib.
    ///
    /// Detects Maven/Gradle, builds if needed, finds a main class, and injects
    /// `-javaagent` / `-Djavar.native.path` from local or embedded assets.
    Run {
        /// Project directory (optional; defaults to `.`).
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        /// Explicit path to javar-agent.jar (auto-discovers / extracts if omitted).
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
        /// Skip the interactive “build now?” prompt
        #[arg(long)]
        yes: bool,
        /// Arguments for `java` — everything after `--`
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
    /// Make JavaR invisible: inject agent via JAVA_TOOL_OPTIONS for every JVM.
    Enable {
        /// Apply to the user environment (all IDEs / `mvn` / `java`).
        #[arg(long)]
        global: bool,
    },
    /// Remove JavaR from JAVA_TOOL_OPTIONS.
    Disable {
        #[arg(long)]
        global: bool,
    },
    /// Optional tool bootstrap (Maven under ~/.javar/tools). Never auto-run.
    Tools {
        #[command(subcommand)]
        action: ToolsCmd,
    },
}

#[derive(Subcommand, Debug)]
enum ToolsCmd {
    /// Install / refresh Maven shim (only when you ask).
    Install {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let is_tui = matches!(cli.command, Commands::Dashboard { .. } | Commands::Tui { .. });
    if !is_tui {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
            )
            .init();
    }

    // Quiet diagnostic: prove this binary embeds the agent (catches stale PATH copies).
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        eprintln!(
            "javar {} (embedded agent: {} bytes)",
            env!("CARGO_PKG_VERSION"),
            AGENT_BYTES.len()
        );
    }

    match cli.command {
        Commands::Init { path } => cmd_init(&path),
        Commands::Setup => setup::cmd_setup(),
        Commands::Build { path } => smart_build::cmd_build(&path),
        Commands::Run {
            path,
            agent,
            port,
            flags_only,
            no_core,
            watch_only,
            yes,
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
                yes,
                &args,
            )
        }
        Commands::Status { addr } => cmd_status(&addr),
        Commands::Dashboard { addr } | Commands::Tui { addr } => dashboard::run_dashboard(addr),
        Commands::Enable { global } => {
            if !global {
                bail!("use:  javar enable --global");
            }
            global_mode::cmd_enable_global()
        }
        Commands::Disable { global } => {
            if !global {
                bail!("use:  javar disable --global");
            }
            global_mode::cmd_disable_global()
        }
        Commands::Tools { action } => match action {
            ToolsCmd::Install { path } => tools_cmd::cmd_tools_install(&path),
        },
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
        style::ok(format!("wrote {}", config.display()));
    } else {
        style::warn_line("javar.toml already exists");
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

    style::header("JavaR project ready");
    style::ok(format!("Initialized at {}", path.display()));
    style::info_line("Next: javar run");
    Ok(())
}

fn cmd_run(
    path: &Path,
    agent: Option<&Path>,
    port: u16,
    flags_only: bool,
    no_core: bool,
    watch_only: bool,
    _auto_yes: bool,
    args: &[String],
) -> Result<()> {
    style::banner_line("JavaR smart run");

    // NEVER fail with "jar not found" — write AGENT_BYTES to ~/.javar/bin if needed.
    let agent_abs = embed::ensure_agent_jar(agent)?;
    let agent_flag = format!("-javaagent:{}=port={}", agent_abs.display(), port);
    let native = embed::resolve_or_extract_native(path);

    let mut project = smart_run::SmartProject::discover(path);
    // Passive: never prompt to build — warn and continue as watcher if needed.
    project = smart_build::note_missing_artifacts(&project);

    let project_name = smart_run::project_display_name(&project.root);
    style::info_line(smart_run::describe_project(&project));
    style::ok(format!("Agent  {}", agent_abs.display()));
    if let Some(ref n) = native {
        style::ok(format!("Native {}", n.display()));
    } else {
        style::warn_line("Native lib not found — set JAVAR_NATIVE_PATH for off-heap");
    }
    style::info_line(&agent_flag);
    style::info_line(format!("JAVAR_AGENT_ADDR=127.0.0.1:{port}"));

    let can_launch = !watch_only && (!args.is_empty() || smart_run::can_smart_launch(&project));

    let java_argv = if can_launch {
        match smart_run::build_java_launch_args(&project, args, native.as_deref()) {
            Ok(mut v) => {
                v.insert(0, format!("-Djavar.project.name={project_name}"));
                Some(v)
            }
            Err(e) => {
                // Passive: do not fail — fall back to sidecar watcher.
                style::warn_line(format!(
                    "Cannot launch JVM yet ({e:#}). Starting passive watcher — \
                     build with your IDE/`mvn package`, or enable global mode: \
                     javar enable --global"
                ));
                None
            }
        }
    } else {
        if watch_only {
            style::info_line("Watch-only mode — sidecar will track file changes.");
        }
        None
    };

    if flags_only {
        if let Some(ref argv) = java_argv {
            print!("java {}", agent_flag);
            for a in argv {
                print!(" {}", shell_escape(a));
            }
            println!();
        } else {
            println!("{agent_flag}");
        }
        return Ok(());
    }

    let addr = format!("127.0.0.1:{port}");
    let mut core_child = None;

    if !no_core {
        style::banner_line("Starting javar-core sidecar");
        core_child = spawn_core(path, &addr)?;
    }

    if let Some(java_argv) = java_argv {
        style::banner_line("Launching JVM with JavaR agent");
        let mut cmd = Command::new("java");
        cmd.arg(&agent_flag)
            .args(&java_argv)
            .env("JAVAR_AGENT_ADDR", &addr)
            .env("JAVAR_PROJECT_NAME", &project_name)
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

    // Passive watcher: keep sidecar alive until Ctrl+C.
    if let Some(mut child) = core_child {
        style::ok("Passive watcher running — Ctrl+C to stop");
        style::info_line("Tip: javar enable --global  → every IDE/JVM loads the agent");
        let status = child.wait().context("wait javar-core")?;
        if !status.success() {
            bail!("javar-core exited with {status}");
        }
    } else {
        style::warn_line("No sidecar and no JVM launch — nothing to do.");
    }

    Ok(())
}

fn spawn_core(path: &Path, addr: &str) -> Result<Option<std::process::Child>> {
    if let Some(bin) = resolve_core_bin(path) {
        let child = Command::new(&bin)
            .arg(path)
            .env("JAVAR_AGENT_ADDR", addr)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawn javar-core ({})", bin.display()))?;
        return Ok(Some(child));
    }

    let root = workspace_root(path);
    // Only use `cargo run` when we're inside the JavaR source tree.
    if root.join("Cargo.toml").is_file() && root.join("javar-core").is_dir() {
        let child = Command::new("cargo")
            .args([
                "run",
                "-q",
                "-p",
                "javar-core",
                "--",
                path.to_str().unwrap_or("."),
            ])
            .current_dir(&root)
            .env("JAVAR_AGENT_ADDR", addr)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .context("cargo run javar-core")?;
        return Ok(Some(child));
    }

    style::warn_line(
        "javar-core sidecar not installed — file watching disabled \
         (JVM agent still runs). Fix: rebuild + `javar setup`",
    );
    Ok(None)
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
    style::ok(format!("agent connected ({addr})"));
    style::info_line(format!("ping: {:?}", msg.kind));

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
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            println!("{}", String::from_utf8_lossy(&msg.body));
        }
    } else {
        style::info_line(format!("status kind: {:?}", msg.kind));
    }
    Ok(())
}

fn resolve_core_bin(project: &Path) -> Option<PathBuf> {
    let root = workspace_root(project);
    let home_bin = embed::javar_bin_dir();
    let beside_cli = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mut candidates = vec![
        home_bin.join("javar-core.exe"),
        home_bin.join("javar-core"),
        root.join("target/release/javar-core.exe"),
        root.join("target/release/javar-core"),
        root.join("target/debug/javar-core.exe"),
        root.join("target/debug/javar-core"),
    ];
    if let Some(dir) = beside_cli {
        candidates.push(dir.join("javar-core.exe"));
        candidates.push(dir.join("javar-core"));
    }
    candidates.into_iter().find(|p| p.is_file())
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

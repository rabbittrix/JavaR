//! Resolve the live JavaR agent port for the current project from `~/.javar/agents/*.json`.
//! Skips IDE / Maven tooling JVMs that steal 19222+.

use serde_json::Value;
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::info;

pub const PORT_RANGE_START: u16 = 19222;
pub const PORT_RANGE_END: u16 = 19242;

/// Prefer a live registry agent that matches `project_root` (not IDE / Maven parent).
pub fn resolve_agent_addr(preferred: &str, project_root: &Path) -> String {
    let folder = project_root
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let root_s = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
        .replace('\\', "/");

    let mut candidates: Vec<(i32, String)> = Vec::new();

    if let Some(dir) = agents_dir() {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let port = v.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16;
                if !(PORT_RANGE_START..=PORT_RANGE_END).contains(&port) {
                    continue;
                }
                let name = v
                    .get("name")
                    .or_else(|| v.get("project_name"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                let cmd = v
                    .get("cmd")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                let cwd = v
                    .get("cwd")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .replace('\\', "/");
                if is_tooling(&name, &cmd) {
                    continue;
                }
                if !port_open(port) {
                    continue;
                }
                let addr = format!("127.0.0.1:{port}");
                candidates.push((score_agent(&name, &cmd, &cwd, &folder, &root_s), addr));
            }
        }
    }

    for port in PORT_RANGE_START..=PORT_RANGE_END {
        if !port_open(port) {
            continue;
        }
        let addr = format!("127.0.0.1:{port}");
        if candidates.iter().any(|(_, a)| a == &addr) {
            continue;
        }
        let mut score = 5;
        if addr == preferred {
            score += 2;
        }
        candidates.push((score, addr));
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    if let Some((score, addr)) = candidates.first() {
        info!(%addr, score, folder = %folder, "selected JavaR agent for hot-reload");
        return addr.clone();
    }
    preferred.to_string()
}

fn score_agent(name: &str, cmd: &str, cwd: &str, folder: &str, root: &str) -> i32 {
    let mut score = 40;
    if name.ends_with("application") || cmd.contains("application") {
        score += 200;
    }
    if name.contains("spring") || cmd.contains("springframework") {
        score += 120;
    }
    if name.contains("demo") || cmd.contains("demo") {
        score += 80;
    }
    if !folder.is_empty() && (name.contains(folder) || cmd.contains(folder)) {
        score += 150;
    }
    // Strong match: agent registered from this project directory.
    if !root.is_empty() && !cwd.is_empty() && (cwd == root || cwd.contains(folder)) {
        score += 220;
    }
    if cmd.contains(".jar") && !cmd.contains("language-server") {
        score += 40;
    }
    if name == "launcher" || cmd.contains("plexus") || cmd.contains("spring-boot:run") {
        score -= 250;
    }
    score
}

fn is_tooling(name: &str, cmd: &str) -> bool {
    [
        "eclipse",
        "equinox",
        "redhat.java",
        "jdt",
        "lemminx",
        "xmlserver",
        "languageserver",
        "language-server",
        "metals",
        "bloop",
        "bloopserver",
        "scala.cli",
        "plexus",
        "classworlds.launcher",
        "surefire",
    ]
    .iter()
    .any(|m| name.contains(m) || cmd.contains(m))
        || name == "launcher"
        || (cmd.contains("spring-boot:run") && !cmd.contains("application"))
}

pub fn agents_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("JAVAR_HOME") {
        return Some(PathBuf::from(p).join("agents"));
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    Some(home.join(".javar").join("agents"))
}

fn port_open(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok()
}

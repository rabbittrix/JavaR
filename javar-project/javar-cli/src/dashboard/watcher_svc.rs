//! Keep a pinned `javar-core` sidecar alive for the Dashboard's target agent.
//! Without this, edits never compile/redefine (hist stays 0).

use crate::embed;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tracing::{info, warn};

static WATCHER: Mutex<Option<WatcherState>> = Mutex::new(None);

struct WatcherState {
    child: Child,
    addr: String,
    project: PathBuf,
}

/// Ensure a sidecar is watching `project` and sending bytecode only to `addr`.
pub fn ensure_pinned_watcher(project: &Path, addr: &str) -> Option<String> {
    if addr.trim().is_empty() || !addr.contains(':') {
        return None;
    }
    if !project.is_dir() {
        return None;
    }
    let src = project.join("src");
    if !src.is_dir() && !project.join("src/main/java").is_dir() {
        return None;
    }

    let mut guard = WATCHER.lock().ok()?;
    if let Some(st) = guard.as_mut() {
        // Still alive and targeting the same agent+project?
        if st.addr == addr && st.project == project {
            match st.child.try_wait() {
                Ok(None) => return Some(format!("watcher ok → {addr}")),
                Ok(Some(status)) => {
                    warn!(?status, "javar-core exited; restarting");
                }
                Err(e) => warn!(?e, "javar-core status check failed; restarting"),
            }
        } else {
            let _ = st.child.kill();
            let _ = st.child.wait();
        }
        *guard = None;
    }

    let bin = resolve_core_bin()?;
    let child = Command::new(&bin)
        .arg(project)
        .env("JAVAR_AGENT_ADDR", addr)
        .env("JAVAR_PINNED_ADDR", addr)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    info!(
        bin = %bin.display(),
        project = %project.display(),
        %addr,
        "dashboard started pinned javar-core watcher"
    );
    *guard = Some(WatcherState {
        child,
        addr: addr.to_string(),
        project: project.to_path_buf(),
    });
    Some(format!("started watcher → {addr} ({})", project.display()))
}

pub fn shutdown_watcher() {
    if let Ok(mut guard) = WATCHER.lock() {
        if let Some(mut st) = guard.take() {
            let _ = st.child.kill();
            let _ = st.child.wait();
        }
    }
}

fn resolve_core_bin() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "javar-core.exe"
    } else {
        "javar-core"
    };
    let mut candidates = vec![embed::javar_bin_dir().join(name)];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(name));
        }
    }
    candidates.push(PathBuf::from("target/release").join(name));
    candidates.push(PathBuf::from("../target/release").join(name));
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    // Last resort: look next to a cargo install.
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let p = PathBuf::from(home).join(".javar").join("bin").join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    return None;
}

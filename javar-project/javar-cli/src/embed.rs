//! Embedded agent JAR + native library — force-extract to `~/.javar/bin/` when missing.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>

use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[allow(clippy::all)]
mod assets {
    include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));
}

use assets::{EMBEDDED_NATIVE, HAS_EMBEDDED_AGENT, HAS_EMBEDDED_NATIVE, NATIVE_NAME};

/// Embedded agent — built by `build.rs` into this stable path before compile.
/// Relative to `javar-cli/src/embed.rs` → `javar-project/javar-agent/target/javar-agent.jar`.
const AGENT_JAR: &[u8] = include_bytes!("../../javar-agent/target/javar-agent.jar");

pub fn javar_home() -> PathBuf {
    if let Ok(p) = std::env::var("JAVAR_HOME") {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".javar")
}

pub fn javar_bin_dir() -> PathBuf {
    javar_home().join("bin")
}

fn agent_cache_path() -> PathBuf {
    javar_bin_dir().join("javar-agent.jar")
}

fn native_cache_path() -> PathBuf {
    javar_bin_dir().join(NATIVE_NAME)
}

fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))
}

fn write_with_progress(dest: &Path, bytes: &[u8], label: &str) -> Result<()> {
    ensure_dir(dest.parent().unwrap())?;
    let pb = ProgressBar::new(bytes.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.magenta} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes}",
        )
        .unwrap()
        .progress_chars("█▓░"),
    );
    pb.set_message(format!("Extracting {label}"));
    pb.enable_steady_tick(Duration::from_millis(80));
    fs::write(dest, bytes).with_context(|| format!("write {}", dest.display()))?;
    pb.finish_with_message(format!("Extracted {}", dest.display()));
    Ok(())
}

/// Force-write embedded agent bytes to `~/.javar/bin/javar-agent.jar`.
pub fn force_extract_agent() -> Result<PathBuf> {
    if AGENT_JAR.is_empty() || !HAS_EMBEDDED_AGENT {
        bail!(
            "no embedded javar-agent.jar in this binary.\n\
             Rebuild javar-cli with Maven on PATH (build.rs runs mvn package automatically),\n\
             or set JAVAR_AGENT_JAR. Tip:  javar build"
        );
    }
    let dest = agent_cache_path();
    let needs_write = !dest.is_file()
        || fs::metadata(&dest)
            .map(|m| m.len() as usize != AGENT_JAR.len())
            .unwrap_or(true);
    if needs_write {
        write_with_progress(&dest, AGENT_JAR, "javar-agent.jar")?;
    }
    Ok(dest.canonicalize().unwrap_or(dest))
}

/// Force-write embedded native lib when present.
pub fn force_extract_native() -> Option<PathBuf> {
    if !HAS_EMBEDDED_NATIVE || EMBEDDED_NATIVE.is_empty() {
        return None;
    }
    let dest = native_cache_path();
    let needs_write = !dest.is_file()
        || fs::metadata(&dest)
            .map(|m| m.len() as usize != EMBEDDED_NATIVE.len())
            .unwrap_or(true);
    if needs_write {
        write_with_progress(&dest, EMBEDDED_NATIVE, NATIVE_NAME).ok()?;
    }
    Some(dest.canonicalize().unwrap_or(dest))
}

/// Resolve agent: overrides → force-extract embedded to `~/.javar/bin` → local last.
pub fn resolve_or_extract_agent(project: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Ok(p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        }
        bail!("agent JAR not found: {}", p.display());
    }

    if let Ok(env) = std::env::var("JAVAR_AGENT_JAR") {
        let p = PathBuf::from(&env);
        if p.is_file() {
            return Ok(p.canonicalize().unwrap_or(p));
        }
    }

    if !AGENT_JAR.is_empty() {
        return force_extract_agent();
    }

    if let Some(local) = find_local_agent(project) {
        let dest = agent_cache_path();
        let _ = ensure_dir(dest.parent().unwrap());
        let _ = fs::copy(&local, &dest);
        return Ok(dest.canonicalize().unwrap_or(local));
    }

    let cached = agent_cache_path();
    if cached.is_file() {
        return Ok(cached.canonicalize().unwrap_or(cached));
    }

    bail!(
        "javar-agent.jar not found.\n\
         Rebuild javar-cli (build.rs embeds the agent) or set JAVAR_AGENT_JAR.\n\
         Then run:  javar setup"
    )
}

/// Resolve native: env → force-extract → local/dev.
pub fn resolve_or_extract_native(project: &Path) -> Option<PathBuf> {
    if let Ok(env) = std::env::var("JAVAR_NATIVE_PATH") {
        let p = PathBuf::from(env);
        if p.is_file() {
            return Some(p.canonicalize().unwrap_or(p));
        }
    }

    if let Some(p) = force_extract_native() {
        return Some(p);
    }

    if let Some(local) = find_local_native(project) {
        let dest = native_cache_path();
        let _ = ensure_dir(dest.parent().unwrap());
        let _ = fs::copy(&local, &dest);
        return Some(local);
    }

    let cached = native_cache_path();
    if cached.is_file() {
        return Some(cached.canonicalize().unwrap_or(cached));
    }
    None
}

fn find_local_agent(project: &Path) -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let candidates = [
        cwd.join("../javar-agent/target/javar-agent.jar"),
        project.join("../javar-agent/target/javar-agent.jar"),
        crate::workspace_root(project).join("javar-agent/target/javar-agent.jar"),
        crate::workspace_root(project).join("javar-agent/target/javar-agent-0.1.0.jar"),
        project.join("lib/javar-agent.jar"),
        cwd.join("lib/javar-agent.jar"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn find_local_native(project: &Path) -> Option<PathBuf> {
    let root = crate::workspace_root(project);
    let names = [
        NATIVE_NAME,
        "javar_core.dll",
        "libjavar_core.so",
        "libjavar_core.dylib",
    ];
    let dirs = [
        root.join("target/release"),
        root.join("target/debug"),
        root.join("lib"),
        project.join("lib"),
        javar_bin_dir(),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from(".")),
    ];
    for dir in dirs {
        for name in names {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p.canonicalize().unwrap_or(p));
            }
        }
    }
    None
}

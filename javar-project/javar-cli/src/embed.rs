//! Runtime extract of embedded agent / native lib to `~/.javar/bin/`.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>
//!
//! Agent resolution NEVER fails with "jar not found" / "cd … && mvn package".
//! It always materializes [`crate::AGENT_BYTES`] under `~/.javar/bin/javar-agent.jar`.

use crate::style;
use crate::AGENT_BYTES;
use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[allow(clippy::all)]
mod assets {
    include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));
}

use assets::{EMBEDDED_NATIVE, HAS_EMBEDDED_NATIVE, NATIVE_NAME};

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

/// Absolute path used for `-javaagent:` (always under `~/.javar/bin/`).
pub fn agent_jar_path() -> PathBuf {
    javar_bin_dir().join("javar-agent.jar")
}

fn native_cache_path() -> PathBuf {
    javar_bin_dir().join(NATIVE_NAME)
}

fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))
}

/// Resolve the agent JAR for `-javaagent`.
///
/// - Optional `--agent` / `JAVAR_AGENT_JAR` override if the file exists.
/// - Otherwise: if `~/.javar/bin/javar-agent.jar` is missing, write `AGENT_BYTES` immediately.
/// - Returns the absolute path. Never suggests `cd … && mvn package`.
pub fn ensure_agent_jar(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Ok(absolute(p));
        }
        style::warn_line(format!(
            "--agent path missing ({}), extracting embedded version...",
            p.display()
        ));
    }

    if let Ok(env) = std::env::var("JAVAR_AGENT_JAR") {
        let p = PathBuf::from(&env);
        if p.is_file() {
            return Ok(absolute(&p));
        }
        style::warn_line("JAVAR_AGENT_JAR path missing, extracting embedded version...");
    }

    materialize_embedded_agent()
}

/// Write [`AGENT_BYTES`] to `~/.javar/bin/javar-agent.jar` when missing/stale/empty.
pub fn materialize_embedded_agent() -> Result<PathBuf> {
    let dest = agent_jar_path();
    ensure_dir(dest.parent().unwrap())?;

    if AGENT_BYTES.is_empty() {
        // Should not happen for release builds — still avoid the old "mvn package" message.
        bail!(
            "this javar binary was built without an embedded agent (0 bytes).\n\
             Reinstall with a release build:  cargo install --path javar-cli --force\n\
             or run:  {} setup",
            std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "javar".into())
        );
    }

    let on_disk = dest.is_file()
        .then(|| fs::metadata(&dest).ok().map(|m| m.len() as usize))
        .flatten();

    match on_disk {
        Some(n) if n == AGENT_BYTES.len() && n > 0 => {
            // Already good.
        }
        Some(_) => {
            style::info_line("Updating embedded agent in ~/.javar/bin...");
            fs::write(&dest, AGENT_BYTES)
                .with_context(|| format!("write {}", dest.display()))?;
            style::ok(format!("Agent → {}", dest.display()));
        }
        None => {
            style::warn_line("Agent not found, extracting embedded version...");
            fs::write(&dest, AGENT_BYTES)
                .with_context(|| format!("write {}", dest.display()))?;
            style::ok(format!("Agent → {}", dest.display()));
        }
    }

    Ok(absolute(&dest))
}

fn absolute(p: &Path) -> PathBuf {
    let raw = p.canonicalize().unwrap_or_else(|_| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(p)
        }
    });
    // Strip Windows `\\?\` so `-javaagent:` paths work with java.exe.
    strip_extended_prefix(raw)
}

fn strip_extended_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        p
    }
}

pub fn force_extract_native() -> Option<PathBuf> {
    if !HAS_EMBEDDED_NATIVE || EMBEDDED_NATIVE.is_empty() {
        return None;
    }
    let dest = native_cache_path();
    let _ = ensure_dir(dest.parent()?);
    let needs_write = !dest.is_file()
        || fs::metadata(&dest)
            .map(|m| m.len() as usize != EMBEDDED_NATIVE.len())
            .unwrap_or(true);
    if needs_write {
        let pb = ProgressBar::new(EMBEDDED_NATIVE.len() as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.magenta} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes}",
            )
            .unwrap()
            .progress_chars("█▓░"),
        );
        pb.set_message(format!("Extracting {NATIVE_NAME}"));
        pb.enable_steady_tick(Duration::from_millis(80));
        fs::write(&dest, EMBEDDED_NATIVE).ok()?;
        pb.finish_with_message(format!("Extracted {}", dest.display()));
    }
    Some(absolute(&dest))
}

pub fn resolve_or_extract_native(project: &Path) -> Option<PathBuf> {
    if let Ok(env) = std::env::var("JAVAR_NATIVE_PATH") {
        let p = PathBuf::from(env);
        if p.is_file() {
            return Some(absolute(&p));
        }
    }

    if let Some(p) = force_extract_native() {
        return Some(p);
    }

    if let Some(local) = find_local_native(project) {
        let dest = native_cache_path();
        let _ = ensure_dir(dest.parent().unwrap());
        let _ = fs::copy(&local, &dest);
        return Some(absolute(&local));
    }

    let cached = native_cache_path();
    if cached.is_file() {
        return Some(absolute(&cached));
    }
    None
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
                return Some(p);
            }
        }
    }
    None
}

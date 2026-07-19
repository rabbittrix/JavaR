//! Embedded agent JAR + native library — extract to `~/.javar/bin/` when needed.

use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[allow(clippy::all)]
mod assets {
    include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));
}

use assets::{
    EMBEDDED_AGENT, EMBEDDED_NATIVE, HAS_EMBEDDED_AGENT, HAS_EMBEDDED_NATIVE, NATIVE_NAME,
};

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

fn stamp_path(kind: &str) -> PathBuf {
    javar_bin_dir().join(format!(".{kind}.sha256"))
}

fn content_stamp(bytes: &[u8]) -> String {
    let n = bytes.len();
    let head = bytes.get(..16.min(n)).unwrap_or(&[]);
    let tail = if n > 16 {
        &bytes[n.saturating_sub(16)..]
    } else {
        &[]
    };
    format!("{n}:{:02x?}:{:02x?}", head, tail)
}

fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))
}

fn extract_if_needed(kind: &str, dest: &Path, bytes: &[u8], label: &str) -> Result<PathBuf> {
    if bytes.is_empty() {
        bail!(
            "no embedded {kind} in this javar binary. \
             Rebuild with agent/native present, or set JAVAR_AGENT_JAR / JAVAR_NATIVE_PATH. \
             Dev: mvn -DskipTests package in javar-agent, then cargo build -p javar-core --release"
        );
    }

    ensure_dir(&javar_bin_dir())?;
    let stamp = content_stamp(bytes);
    let stamp_file = stamp_path(kind);

    let up_to_date = dest.is_file()
        && fs::read_to_string(&stamp_file)
            .map(|s| s.trim() == stamp)
            .unwrap_or(false)
        && fs::metadata(dest)
            .map(|m| m.len() as usize == bytes.len())
            .unwrap_or(false);

    if up_to_date {
        return Ok(dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf()));
    }

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

    let tmp = dest.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        const CHUNK: usize = 64 * 1024;
        let mut written = 0usize;
        while written < bytes.len() {
            let end = (written + CHUNK).min(bytes.len());
            f.write_all(&bytes[written..end])?;
            written = end;
            pb.set_position(written as u64);
        }
        f.flush()?;
    }
    fs::rename(&tmp, dest).with_context(|| format!("install {}", dest.display()))?;
    fs::write(&stamp_file, &stamp)?;
    pb.finish_with_message(format!("Extracted {label}"));

    Ok(dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf()))
}

/// Prefer local/dev paths; otherwise extract the embedded agent to `~/.javar/bin/`.
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

    if let Some(local) = find_local_agent(project) {
        return Ok(local);
    }

    let cached = agent_cache_path();
    if cached.is_file() && HAS_EMBEDDED_AGENT && !EMBEDDED_AGENT.is_empty() {
        let stamp = content_stamp(EMBEDDED_AGENT);
        if fs::read_to_string(stamp_path("agent"))
            .map(|s| s.trim() == stamp)
            .unwrap_or(false)
        {
            return Ok(cached.canonicalize().unwrap_or(cached));
        }
    }

    if HAS_EMBEDDED_AGENT && !EMBEDDED_AGENT.is_empty() {
        return extract_if_needed("agent", &cached, EMBEDDED_AGENT, "javar-agent.jar");
    }

    if cached.is_file() {
        return Ok(cached.canonicalize().unwrap_or(cached));
    }

    bail!(
        "javar-agent.jar not found.\n\
         This binary was built without an embedded agent, and no local/dev jar was found.\n\
         Fix: install a release build and run `javar setup`, or set JAVAR_AGENT_JAR.\n\
         Dev: cd javar-agent && mvn -DskipTests package"
    )
}

/// Prefer local/dev native lib; otherwise extract embedded lib to `~/.javar/bin/`.
pub fn resolve_or_extract_native(project: &Path) -> Option<PathBuf> {
    if let Ok(env) = std::env::var("JAVAR_NATIVE_PATH") {
        let p = PathBuf::from(env);
        if p.is_file() {
            return Some(p.canonicalize().unwrap_or(p));
        }
    }

    if let Some(local) = find_local_native(project) {
        return Some(local);
    }

    let cached = native_cache_path();
    if HAS_EMBEDDED_NATIVE && !EMBEDDED_NATIVE.is_empty() {
        return extract_if_needed("native", &cached, EMBEDDED_NATIVE, NATIVE_NAME).ok();
    }

    if cached.is_file() {
        return Some(cached.canonicalize().unwrap_or(cached));
    }
    None
}

fn find_local_agent(project: &Path) -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let dirs = [
        cwd.join("../javar-agent/target"),
        project.join("../javar-agent/target"),
        crate::workspace_root(project).join("javar-agent/target"),
        project.join("javar-agent/target"),
        project.join("lib"),
        cwd.join("lib"),
        cwd.join("agent"),
    ];
    for dir in dirs {
        if let Some(jar) = pick_agent_jar(&dir) {
            return Some(jar.canonicalize().unwrap_or(jar));
        }
    }
    let named = [
        crate::workspace_root(project).join("javar-agent/target/javar-agent-0.1.0.jar"),
        cwd.join("../javar-agent/target/javar-agent-0.1.0.jar"),
        project.join("lib/javar-agent.jar"),
    ];
    named.into_iter().find(|p| p.is_file())
}

fn pick_agent_jar(dir: &Path) -> Option<PathBuf> {
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
                        lower.contains("javar-agent")
                            && !lower.contains("sources")
                            && !lower.contains("javadoc")
                            && !lower.contains("original")
                    })
                    .unwrap_or(false)
        })
        .collect();
    jars.sort();
    jars.pop()
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

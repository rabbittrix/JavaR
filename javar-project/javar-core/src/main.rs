//! JavaR Core sidecar binary — watches sources and drives the Java Agent.

use anyhow::{Context, Result};
use javar_core::bridge::{BridgeConfig, SocketBridge};
use javar_core::{resolve_agent_addr, CoreConfig, JavaRCore};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let project_root = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let preferred = std::env::var("JAVAR_AGENT_ADDR").unwrap_or_else(|_| "127.0.0.1:19222".into());
    let agent_addr = resolve_agent_addr(&preferred, &project_root);
    if agent_addr != preferred {
        info!(
            preferred = %preferred,
            resolved = %agent_addr,
            "redirected sidecar to live user-app agent port"
        );
    }

    let mut config = CoreConfig {
        project_root: project_root.clone(),
        agent_addr: agent_addr.clone(),
        ..CoreConfig::default()
    };

    // Internal compiler: watch sources ONLY — never wait on IDE `target/classes`.
    config.watch_paths = vec![
        project_root.join("src"),
        project_root.join("src/main/java"),
    ];
    config.source_roots = discover_source_roots(&project_root);
    config.output_dir = project_root.join("target/classes");
    config.classpath = discover_classpath(&project_root);
    // Sub-100ms debounce so Save → redefine can land under 500ms end-to-end.
    config.debounce_ms = 40;

    let release = javar_core::resolve_compiler_release();
    info!(
        sources = ?config.source_roots,
        out = %config.output_dir.display(),
        agent = %agent_addr,
        release,
        "isolated incremental javac on .java save (--release + javar-agent.jar on -cp)"
    );

    let bridge = Arc::new(
        SocketBridge::connect(BridgeConfig {
            addr: agent_addr,
            reconnect: true,
        })
        .await
        .context("connect to JavaR agent")?,
    );

    let core = JavaRCore::new(config, bridge);
    core.run().await
}

fn discover_source_roots(root: &Path) -> Vec<PathBuf> {
    let candidates = [
        root.join("src/main/java"),
        root.join("src"),
        root.join("java"),
    ];
    let found: Vec<_> = candidates.into_iter().filter(|p| p.is_dir()).collect();
    if found.is_empty() {
        vec![root.join("src")]
    } else {
        found
    }
}

fn discover_classpath(root: &Path) -> Vec<PathBuf> {
    let mut cp = Vec::new();
    let classes = root.join("target/classes");
    if classes.is_dir() {
        cp.push(classes);
    }
    for dir in [
        root.join("target/dependency"),
        root.join("target/lib"),
    ] {
        push_jars(&dir, &mut cp);
    }
    // Fat / Boot jars in target/ (excluding sources/javadoc).
    if let Ok(entries) = fs::read_dir(root.join("target")) {
        for e in entries.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if p.extension().and_then(|x| x.to_str()) == Some("jar")
                && !name.contains("sources")
                && !name.contains("javadoc")
                && !name.contains("original")
            {
                cp.push(p);
            }
        }
    }
    if let Some(home) = dirs_home() {
        let agent = home.join(".javar").join("bin").join("javar-agent.jar");
        if agent.is_file() {
            cp.push(agent);
        }
    }
    let cp_file = root.join("target/classpath.txt");
    if let Ok(text) = fs::read_to_string(cp_file) {
        for part in text.split([';', '\n', '\r']) {
            let p = PathBuf::from(part.trim());
            if p.is_file() && !cp.iter().any(|x| x == &p) {
                cp.push(p);
            }
        }
    }
    cp
}

fn push_jars(dir: &Path, cp: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("jar") {
                cp.push(p);
            }
        }
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

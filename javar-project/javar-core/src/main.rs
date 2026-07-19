//! JavaR Core sidecar binary — watches sources and drives the Java Agent.

use anyhow::{Context, Result};
use javar_core::bridge::{BridgeConfig, SocketBridge};
use javar_core::{CoreConfig, JavaRCore};
use std::path::PathBuf;
use std::sync::Arc;
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

    let agent_addr = std::env::var("JAVAR_AGENT_ADDR").unwrap_or_else(|_| "127.0.0.1:19222".into());

    let mut config = CoreConfig {
        project_root: project_root.clone(),
        agent_addr: agent_addr.clone(),
        ..CoreConfig::default()
    };

    // Resolve watch / source paths relative to project root.
    config.watch_paths = vec![
        project_root.join("src"),
        project_root.join("target/classes"),
    ];
    config.source_roots = discover_source_roots(&project_root);
    config.output_dir = project_root.join("target/classes");

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

fn discover_source_roots(root: &std::path::Path) -> Vec<PathBuf> {
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

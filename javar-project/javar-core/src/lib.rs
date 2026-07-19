//! JavaR Core — high-performance sidecar for JVM hot-reload and off-heap memory.
//!
//! Architecture:
//! - [`watcher`] observes `.java` / `.class` changes with debounced notify events
//! - [`compiler`] triggers background `javac` / incremental builds
//! - [`bridge`] delivers bytecode to the Java Agent (socket or JNI)
//! - [`protocol`] defines a zero-copy-friendly binary framing layer
//! - [`memory`] Phase-2 scaffold for off-heap managed regions
//! - [`rollback`] tracks class versions for instant revert

pub mod bridge;
pub mod compiler;
pub mod memory;
pub mod protocol;
pub mod rollback;
pub mod watcher;

pub use bridge::{AgentBridge, BridgeConfig};
pub use compiler::{CompileRequest, Compiler};
pub use protocol::{Frame, Message, MessageKind};
pub use rollback::RollbackStore;
pub use watcher::{WatchConfig, WatchEvent, Watcher};

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

/// Shared runtime configuration for the JavaR sidecar.
#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub project_root: PathBuf,
    pub watch_paths: Vec<PathBuf>,
    pub classpath: Vec<PathBuf>,
    pub source_roots: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub agent_addr: String,
    pub debounce_ms: u64,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            watch_paths: vec![PathBuf::from("src")],
            classpath: Vec::new(),
            source_roots: vec![PathBuf::from("src/main/java")],
            output_dir: PathBuf::from("target/classes"),
            agent_addr: "127.0.0.1:19222".into(),
            debounce_ms: 120,
        }
    }
}

/// Orchestrates watch → compile → redefine → rollback.
pub struct JavaRCore {
    config: CoreConfig,
    bridge: Arc<dyn AgentBridge>,
    compiler: Compiler,
    rollback: Arc<RollbackStore>,
}

impl JavaRCore {
    pub fn new(config: CoreConfig, bridge: Arc<dyn AgentBridge>) -> Self {
        let compiler = Compiler::new(
            config.source_roots.clone(),
            config.classpath.clone(),
            config.output_dir.clone(),
        );
        Self {
            config,
            bridge,
            compiler,
            rollback: Arc::new(RollbackStore::new()),
        }
    }

    /// Run the sidecar until cancelled.
    pub async fn run(self) -> Result<()> {
        info!(
            root = %self.config.project_root.display(),
            agent = %self.config.agent_addr,
            "JavaR core starting"
        );

        let watch_cfg = WatchConfig {
            paths: self.config.watch_paths.clone(),
            debounce_ms: self.config.debounce_ms,
        };

        let mut watcher = Watcher::start(watch_cfg)?;
        let bridge = self.bridge.clone();
        let compiler = self.compiler;
        let rollback = self.rollback.clone();

        // Announce readiness to agent / IDE clients.
        bridge
            .send(Message::status("ready", "JavaR core online"))
            .await?;

        while let Some(event) = watcher.next().await {
            if let Err(err) = handle_event(&event, &compiler, &bridge, &rollback).await {
                error!(?err, path = %event.path.display(), "hot-reload cycle failed");
                let _ = bridge
                    .send(Message::error(format!(
                        "reload failed for {}: {err}",
                        event.path.display()
                    )))
                    .await;
            }
        }

        Ok(())
    }

    pub fn rollback_store(&self) -> Arc<RollbackStore> {
        self.rollback.clone()
    }
}

async fn handle_event(
    event: &WatchEvent,
    compiler: &Compiler,
    bridge: &Arc<dyn AgentBridge>,
    rollback: &Arc<RollbackStore>,
) -> Result<()> {
    info!(path = %event.path.display(), kind = ?event.kind, "change detected");

    let artifact = match event.kind {
        watcher::ChangeKind::JavaSource => {
            let req = CompileRequest::from_source(&event.path);
            compiler.compile_async(req).await?
        }
        watcher::ChangeKind::ClassFile => {
            // Zero-copy mmap of the .class payload when possible.
            compiler.load_class_bytes(&event.path)?
        }
        watcher::ChangeKind::Other => return Ok(()),
    };

    // Snapshot previous bytecode for instant rollback.
    rollback.snapshot(&artifact.class_name, &artifact.bytecode);

    bridge
        .send(Message::redefine(
            artifact.class_name.clone(),
            artifact.bytecode.clone(),
        ))
        .await?;

    info!(
        class = %artifact.class_name,
        bytes = artifact.bytecode.len(),
        "bytecode sent to agent"
    );

    Ok(())
}

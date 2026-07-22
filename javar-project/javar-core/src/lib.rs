//! JavaR Core — high-performance sidecar for JVM hot-reload and off-heap memory.
//!
//! Architecture:
//! - [`watcher`] observes `.java` / `.class` changes with debounced notify events
//! - [`compiler`] triggers background `javac` / incremental builds
//! - [`bridge`] delivers bytecode to the Java Agent (socket or JNI)
//! - [`protocol`] defines a zero-copy-friendly binary framing layer
//! - [`memory`] Phase-2 scaffold for off-heap managed regions
//! - [`rollback`] tracks class versions for instant revert

pub mod agent_resolve;
pub mod bridge;
pub mod classfile;
pub mod compiler;
pub mod memory;
pub mod protocol;
pub mod rollback;
pub mod shadow;
pub mod watcher;

pub use agent_resolve::resolve_agent_addr;
pub use bridge::{AgentBridge, BridgeConfig};
pub use classfile::{ChangeKind as SchemaChangeKind, ClassSchema};
pub use compiler::{resolve_compiler_release, CompileRequest, Compiler};
pub use protocol::{Frame, Message, MessageKind};
pub use rollback::RollbackStore;
pub use shadow::{ShadowRegistry, ShadowVersion};
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

/// Orchestrates watch → compile → redefine/shadow → rollback.
pub struct JavaRCore {
    config: CoreConfig,
    bridge: Arc<dyn AgentBridge>,
    compiler: Compiler,
    rollback: Arc<RollbackStore>,
    shadows: Arc<ShadowRegistry>,
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
            shadows: Arc::new(ShadowRegistry::new()),
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
        let shadows = self.shadows.clone();
        let project_root = self.config.project_root.clone();
        let preferred = self.config.agent_addr.clone();

        // Announce readiness to agent / IDE clients.
        bridge
            .send(Message::status("ready", "JavaR core online"))
            .await?;

        while let Some(event) = watcher.next().await {
            if let Err(err) = handle_event(
                &event,
                &compiler,
                &bridge,
                &rollback,
                &shadows,
                &project_root,
                &preferred,
            )
            .await
            {
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

    pub fn shadow_registry(&self) -> Arc<ShadowRegistry> {
        self.shadows.clone()
    }
}

async fn handle_event(
    event: &WatchEvent,
    compiler: &Compiler,
    bridge: &Arc<dyn AgentBridge>,
    rollback: &Arc<RollbackStore>,
    shadows: &Arc<ShadowRegistry>,
    project_root: &std::path::Path,
    preferred: &str,
) -> Result<()> {
    // Only `.java` saves drive the pipeline. IDE writes to `target/classes` are ignored.
    if !matches!(event.kind, watcher::ChangeKind::JavaSource) {
        return Ok(());
    }

    info!(
        "[WATCHER] Change detected in {}",
        event.path.display()
    );

    // Force re-compile — never push stale IDE bytecode.
    info!(
        "[WATCHER] Force re-compile via javac --release {}",
        compiler.release()
    );
    let artifact = compiler
        .compile_async(CompileRequest::from_source(&event.path))
        .await?;

    // Pin to the `javar run` process when set; otherwise resolve by project.
    let target = if let Ok(pinned) = std::env::var("JAVAR_PINNED_ADDR") {
        if !pinned.is_empty() {
            pinned
        } else {
            resolve_agent_addr(preferred, project_root)
        }
    } else {
        resolve_agent_addr(preferred, project_root)
    };
    bridge.retarget(target.clone()).await?;
    info!(agent = %target, "[WATCHER] Sending bytecode to pinned/project agent");

    rollback.snapshot(&artifact.class_name, &artifact.bytecode);

    let (change, version) =
        shadows.prepare(&artifact.class_name, artifact.bytecode.clone())?;

    let msg = match change {
        classfile::ChangeKind::Structural => {
            info!(
                class = %artifact.class_name,
                shadow = %version.shadow_name,
                v = version.version,
                "structural change → shadow class path"
            );
            Message::structural(
                version.class_name.clone(),
                version.shadow_name.clone(),
                version.version,
                version.bytecode.clone(),
            )
        }
        classfile::ChangeKind::Compatible => {
            Message::redefine(version.class_name.clone(), version.bytecode.clone())
        }
    };

    bridge.send(msg).await?;

    info!(
        class = %artifact.class_name,
        bytes = artifact.bytecode.len(),
        ?change,
        agent = %target,
        "bytecode sent to agent"
    );

    Ok(())
}

//! Debounced filesystem watcher optimized for `.java` / `.class` hot-reload.

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver};
use notify::{EventKind, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct WatchConfig {
    pub paths: Vec<PathBuf>,
    pub debounce_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    JavaSource,
    ClassFile,
    Other,
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

impl WatchEvent {
    pub fn from_path(path: PathBuf) -> Self {
        let kind = match path.extension().and_then(|e| e.to_str()) {
            Some("java") => ChangeKind::JavaSource,
            Some("class") => ChangeKind::ClassFile,
            _ => ChangeKind::Other,
        };
        Self { path, kind }
    }
}

/// Async-friendly wrapper around `notify` with mini-debouncer.
pub struct Watcher {
    rx: Receiver<WatchEvent>,
    /// Kept alive so the OS watch handles are not dropped.
    _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
}

impl Watcher {
    pub fn start(config: WatchConfig) -> Result<Self> {
        let (tx, rx) = unbounded::<WatchEvent>();

        let mut debouncer = new_debouncer(
            Duration::from_millis(config.debounce_ms),
            move |res: Result<Vec<notify_debouncer_mini::DebouncedEvent>, _>| {
                match res {
                    Ok(events) => {
                        for ev in events {
                            if matches!(ev.kind, DebouncedEventKind::Any) {
                                let we = WatchEvent::from_path(ev.path);
                                if we.kind != ChangeKind::Other {
                                    let _ = tx.send(we);
                                }
                            }
                        }
                    }
                    Err(err) => warn!(?err, "watch error"),
                }
            },
        )
        .context("create debouncer")?;

        for path in &config.paths {
            if path.exists() {
                debouncer
                    .watcher()
                    .watch(path, RecursiveMode::Recursive)
                    .with_context(|| format!("watch {}", path.display()))?;
                debug!(path = %path.display(), "watching");
            } else {
                warn!(path = %path.display(), "watch path missing, skipping");
            }
        }

        Ok(Self {
            rx,
            _debouncer: debouncer,
        })
    }

    /// Await the next relevant change (non-blocking poll bridged into async).
    pub async fn next(&mut self) -> Option<WatchEvent> {
        loop {
            match self.rx.try_recv() {
                Ok(ev) => return Some(ev),
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    tokio::time::sleep(Duration::from_millis(16)).await;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => return None,
            }
        }
    }
}

/// Filter helper used by tests / CLI status.
pub fn is_reloadable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("java") | Some("class")
    )
}

/// Map raw notify event kinds for diagnostics.
pub fn describe_kind(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::Create(_) => "create",
        EventKind::Modify(_) => "modify",
        EventKind::Remove(_) => "remove",
        EventKind::Any => "any",
        EventKind::Access(_) => "access",
        EventKind::Other => "other",
    }
}

//! Instant rollback store — keeps previous class bytecode for failed reloads.

use bytes::Bytes;
use dashmap::DashMap;
use std::collections::VecDeque;
use tracing::info;

const DEFAULT_DEPTH: usize = 8;

#[derive(Debug, Default)]
pub struct RollbackStore {
    /// class_name → ring buffer of prior bytecode versions (oldest at front).
    versions: DashMap<String, VecDeque<Bytes>>,
    depth: usize,
}

impl RollbackStore {
    pub fn new() -> Self {
        Self {
            versions: DashMap::new(),
            depth: DEFAULT_DEPTH,
        }
    }

    pub fn with_depth(depth: usize) -> Self {
        Self {
            versions: DashMap::new(),
            depth: depth.max(1),
        }
    }

    /// Snapshot current-on-disk / last-known bytecode before applying a new version.
    pub fn snapshot(&self, class_name: &str, bytecode: &Bytes) {
        let mut q = self
            .versions
            .entry(class_name.to_string())
            .or_insert_with(VecDeque::new);
        q.push_back(bytecode.clone());
        while q.len() > self.depth {
            q.pop_front();
        }
    }

    /// Pop the most recent snapshot for a class (instant rollback candidate).
    pub fn pop(&self, class_name: &str) -> Option<Bytes> {
        let mut entry = self.versions.get_mut(class_name)?;
        let bytes = entry.pop_back()?;
        info!(%class_name, bytes = bytes.len(), "rollback snapshot restored");
        Some(bytes)
    }

    pub fn peek(&self, class_name: &str) -> Option<Bytes> {
        self.versions
            .get(class_name)
            .and_then(|q| q.back().cloned())
    }

    pub fn tracked_classes(&self) -> Vec<String> {
        self.versions.iter().map(|e| e.key().clone()).collect()
    }
}

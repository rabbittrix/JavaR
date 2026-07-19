//! Shadow-class version registry — Rust-driven structural hot-reload tracking.
//!
//! # Why shadows?
//! `Instrumentation.redefineClasses` may only replace method *bodies*. Adding or
//! removing fields/methods is rejected by the JVM. JavaR bypasses that by:
//!
//! 1. Defining a **new** class `Original$JavaR_vN` (always allowed — new name).
//! 2. Keeping the original class's schema frozen.
//! 3. Rewriting original method bodies (HotSwap-legal) to dispatch into the shadow.
//! 4. Storing per-instance twins / side-car state for new fields.
//!
//! This module assigns version numbers and remembers bytecode for instant rollback.

use crate::classfile::{shadow_binary_name, ChangeKind, ClassSchema};
use bytes::Bytes;
use dashmap::DashMap;
use tracing::info;

#[derive(Debug, Clone)]
pub struct ShadowVersion {
    pub class_name: String,
    pub shadow_name: String,
    pub version: u32,
    pub structural: bool,
    pub bytecode: Bytes,
}

#[derive(Debug, Default)]
struct ClassHistory {
    next_version: u32,
    /// Last known schema (from the most recently applied bytecode).
    schema: Option<ClassSchema>,
    /// Stack of installed shadows / redefine snapshots (newest at back).
    stack: Vec<ShadowVersion>,
}

/// Process-wide tracker used by the sidecar before sending frames to the agent.
#[derive(Debug, Default)]
pub struct ShadowRegistry {
    classes: DashMap<String, ClassHistory>,
}

impl ShadowRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Classify a newly compiled artifact and record a version entry.
    pub fn prepare(
        &self,
        class_name: &str,
        bytecode: Bytes,
    ) -> anyhow::Result<(ChangeKind, ShadowVersion)> {
        let schema = ClassSchema::parse(&bytecode)?;
        let mut entry = self.classes.entry(class_name.to_string()).or_default();

        let change = match &entry.schema {
            Some(prev) => schema.classify_against(prev),
            None => ChangeKind::Compatible,
        };

        entry.next_version = entry.next_version.saturating_add(1);
        let version = entry.next_version;
        let structural = change == ChangeKind::Structural;
        let shadow_name = if structural {
            shadow_binary_name(class_name, version)
        } else {
            class_name.to_string()
        };

        let record = ShadowVersion {
            class_name: class_name.to_string(),
            shadow_name: shadow_name.clone(),
            version,
            structural,
            bytecode,
        };

        entry.schema = Some(schema);
        entry.stack.push(record.clone());
        // Keep last 8 versions for rollback.
        if entry.stack.len() > 8 {
            entry.stack.remove(0);
        }

        info!(
            %class_name,
            %shadow_name,
            version,
            ?change,
            "shadow registry prepared reload"
        );

        Ok((change, record))
    }

    /// Pop the current version and return the previous one (instant rollback).
    pub fn rollback(&self, class_name: &str) -> Option<ShadowVersion> {
        let mut entry = self.classes.get_mut(class_name)?;
        // Drop current.
        let _ = entry.stack.pop()?;
        let prev = entry.stack.last()?.clone();
        if let Ok(schema) = ClassSchema::parse(&prev.bytecode) {
            entry.schema = Some(schema);
        }
        info!(
            %class_name,
            shadow = %prev.shadow_name,
            version = prev.version,
            "shadow rollback selected"
        );
        Some(prev)
    }

    pub fn current(&self, class_name: &str) -> Option<ShadowVersion> {
        self.classes
            .get(class_name)
            .and_then(|e| e.stack.last().cloned())
    }

    pub fn tracked(&self) -> Vec<String> {
        self.classes.iter().map(|e| e.key().clone()).collect()
    }
}

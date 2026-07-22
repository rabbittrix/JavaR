//! JavaR Control Center — real-time TUI dashboard (ratatui).
//!
//! Process picker (`p`): reads `~/.javar/agents/*.json`, shows a centered modal,
//! and reconnects telemetry to the selected port.

mod agent;
mod app;
mod discover;
mod java_proc;
mod ui;
mod watcher_svc;

pub use app::run_dashboard;

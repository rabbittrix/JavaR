//! Optional tool bootstrap (`javar tools install`) — never auto-prompted.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>

use crate::maven;
use crate::style;
use anyhow::Result;
use std::path::Path;

pub fn cmd_tools_install(project: &Path) -> Result<()> {
    style::header("javar tools install");
    style::info_line("Installing / refreshing optional build tools (Maven shim)…");
    match maven::ensure_maven_installed(project) {
        Ok(p) => style::ok(format!("Maven → {}", p.display())),
        Err(e) => style::warn_line(format!("Maven: {e:#}")),
    }
    style::ok("Done — Maven is only installed when you run this command.");
    Ok(())
}

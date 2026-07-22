//! Legacy global injection cleanup.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>
//!
//! Global `JAVA_TOOL_OPTIONS` mode is **removed**. JavaR injects only via
//! `javar run`. This module thoroughly strips leftover env vars from the
//! Windows Registry / shell rc files.

use crate::embed;
use crate::style;
use anyhow::Result;
#[cfg(not(windows))]
use anyhow::Context;
use std::env;
use std::fs;

const MARKER_START: &str = "-javaagent:";
const ENV_KEY: &str = "JAVA_TOOL_OPTIONS";
const NATIVE_ENV_KEY: &str = "JAVAR_NATIVE_PATH";

/// Global mode is retired — clean leftovers and point users at `javar run`.
pub fn cmd_enable_global() -> Result<()> {
    style::header("javar enable --global (removed)");
    style::warn_line(
        "Global JAVA_TOOL_OPTIONS injection is no longer supported — it conflicts with IDE JVMs.",
    );
    style::info_line("Use the explicit entry point instead:");
    style::info_line("  javar run          # launch app + agent + watcher");
    style::info_line("  javar dashboard    # monitor that process");
    style::banner_line("Cleaning any leftover global injection…");
    clear_javar_env_thorough()?;
    Ok(())
}

/// Thoroughly remove JavaR from `JAVA_TOOL_OPTIONS` / `JAVAR_NATIVE_PATH`
/// (Windows Registry user + machine, process env, Unix shell rc).
pub fn cmd_disable_global() -> Result<()> {
    style::header("javar disable --global");
    clear_javar_env_thorough()?;
    style::info_line("Restart open IDEs / terminals so they drop stale env.");
    style::info_line("From now on:  javar run   (per-project injection only)");
    Ok(())
}

/// Called from `javar setup` — never sets global agent env; only cleans leftovers.
pub fn ensure_no_global_injection() -> Result<()> {
    clear_javar_env_thorough()
}

/// Disable global injection, then delete `~/.javar` entirely.
pub fn cmd_uninstall() -> Result<()> {
    style::header("javar uninstall");
    let _ = clear_javar_env_thorough();
    let home = embed::javar_home();
    if home.is_dir() {
        match remove_dir_best_effort(&home) {
            Ok(()) => style::ok(format!("Deleted {}", home.display())),
            Err(e) => style::warn_line(format!(
                "Could not fully delete {} ({e:#}). Close running JavaR processes and retry.",
                home.display()
            )),
        }
    }
    println!("JavaR has been completely removed from your system.");
    Ok(())
}

fn clear_javar_env_thorough() -> Result<()> {
    // 1) Process environment (this shell).
    scrub_process_env();

    // 2) Persistent user / machine stores.
    #[cfg(windows)]
    {
        clear_windows_registry()?;
    }
    #[cfg(not(windows))]
    {
        clear_unix_shell_rc()?;
    }

    // 3) Legacy helper file from old enable --global.
    let env_file = embed::javar_home().join("env.sh");
    if env_file.is_file() {
        let _ = fs::remove_file(&env_file);
        style::ok(format!("Removed {}", env_file.display()));
    }

    Ok(())
}

fn scrub_process_env() {
    if let Ok(cur) = env::var(ENV_KEY) {
        let next = strip_javar_tool_options(&cur);
        if next.trim().is_empty() {
            env::remove_var(ENV_KEY);
            style::ok(format!("Cleared process {ENV_KEY}"));
        } else if next != cur {
            env::set_var(ENV_KEY, &next);
            style::ok(format!("Stripped JavaR from process {ENV_KEY}"));
        }
    }
    if env::var_os(NATIVE_ENV_KEY).is_some() {
        env::remove_var(NATIVE_ENV_KEY);
        style::ok(format!("Cleared process {NATIVE_ENV_KEY}"));
    }
}

#[cfg(windows)]
fn clear_windows_registry() -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    // User environment (HKCU\Environment) — primary.
    scrub_reg_key(
        RegKey::predef(HKEY_CURRENT_USER),
        "Environment",
        "user (HKCU)",
    )?;

    // Machine environment (HKLM\SYSTEM\...\Environment) — best-effort (needs admin).
    let hklm_path = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    match RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(hklm_path, KEY_READ | KEY_WRITE)
    {
        Ok(_) => {
            scrub_reg_key(
                RegKey::predef(HKEY_LOCAL_MACHINE),
                hklm_path,
                "machine (HKLM)",
            )?;
        }
        Err(_) => {
            // Read-only probe: still report if JavaR residue exists.
            if let Ok(env) =
                RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(hklm_path, KEY_READ)
            {
                let jto: String = env.get_value(ENV_KEY).unwrap_or_default();
                let native: String = env.get_value(NATIVE_ENV_KEY).unwrap_or_default();
                if jto.contains("javar-agent") || !native.is_empty() {
                    style::warn_line(
                        "Machine (HKLM) still has JavaR env — re-run as Administrator to clear it.",
                    );
                }
            }
        }
    }

    broadcast_env_change();
    Ok(())
}

#[cfg(windows)]
fn scrub_reg_key(root: winreg::RegKey, subkey: &str, label: &str) -> Result<()> {
    use winreg::enums::*;

    let env = match root.open_subkey_with_flags(subkey, KEY_READ | KEY_WRITE) {
        Ok(k) => k,
        Err(e) => {
            style::warn_line(format!("Could not open {label} Environment: {e}"));
            return Ok(());
        }
    };

    let current: String = env.get_value(ENV_KEY).unwrap_or_default();
    if current.is_empty() {
        style::info_line(format!("{label} {ENV_KEY}: (already empty)"));
    } else {
        let next = strip_javar_tool_options(&current);
        if next.trim().is_empty() {
            let _ = env.delete_value(ENV_KEY);
            style::ok(format!("Removed {label} {ENV_KEY}"));
        } else if next != current {
            env.set_value(ENV_KEY, &next)?;
            style::ok(format!("Stripped JavaR from {label} {ENV_KEY}"));
        } else {
            style::info_line(format!("{label} {ENV_KEY}: no JavaR tokens"));
        }
    }

    match env.delete_value(NATIVE_ENV_KEY) {
        Ok(()) => style::ok(format!("Removed {label} {NATIVE_ENV_KEY}")),
        Err(_) => style::info_line(format!("{label} {NATIVE_ENV_KEY}: (already empty)")),
    }

    for key in ["JAVAR_AGENT_ADDR", "JAVAR_PROJECT_NAME"] {
        let _ = env.delete_value(key);
    }
    Ok(())
}

#[cfg(windows)]
fn broadcast_env_change() {
    use std::process::Command;
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Add-Type -Namespace Win32 -Name NativeMethods -MemberDefinition '[DllImport(\"user32.dll\", SetLastError=true, CharSet=CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);' -ErrorAction SilentlyContinue; $r=[UIntPtr]::Zero; [void][Win32.NativeMethods]::SendMessageTimeout([IntPtr]0xffff,0x1A,[UIntPtr]::Zero,'Environment',2,5000,[ref]$r)",
        ])
        .status();
}

#[cfg(not(windows))]
fn clear_unix_shell_rc() -> Result<()> {
    let home = dirs::home_dir().context("home dir")?;
    let marker = "# JavaR JAVA_TOOL_OPTIONS (javar enable --global)";
    for rc_name in [".zshrc", ".bashrc", ".profile"] {
        let rc = home.join(rc_name);
        if !rc.is_file() {
            continue;
        }
        let content = fs::read_to_string(&rc).unwrap_or_default();
        if !content.contains(marker)
            && !content.contains("javar-agent")
            && !content.contains("export JAVA_TOOL_OPTIONS=")
            && !content.contains("export JAVAR_NATIVE_PATH=")
        {
            continue;
        }
        let mut out = String::new();
        let mut skip = false;
        for l in content.lines() {
            if l.contains(marker) {
                skip = true;
                continue;
            }
            if skip && (l.starts_with("export JAVA_TOOL_OPTIONS=") || l.trim().is_empty()) {
                if l.starts_with("export JAVA_TOOL_OPTIONS=") {
                    skip = false;
                }
                continue;
            }
            skip = false;
            if l.starts_with("export JAVA_TOOL_OPTIONS=") && l.contains("javar-agent") {
                continue;
            }
            if l.starts_with("export JAVAR_NATIVE_PATH=") {
                continue;
            }
            out.push_str(l);
            out.push('\n');
        }
        fs::write(&rc, out)?;
        style::ok(format!("Removed JavaR env from {}", rc.display()));
    }
    Ok(())
}

fn remove_dir_best_effort(dir: &std::path::Path) -> Result<()> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = remove_dir_best_effort(&path);
                let _ = fs::remove_dir(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
    fs::remove_dir_all(dir).or_else(|_| {
        if dir.exists() {
            Err(anyhow::anyhow!("directory still present (files may be locked)"))
        } else {
            Ok(())
        }
    })
}

pub fn strip_javar_tool_options(existing: &str) -> String {
    existing
        .split_whitespace()
        .filter(|tok| {
            let t = *tok;
            if t.starts_with(MARKER_START) && t.to_lowercase().contains("javar") {
                return false;
            }
            if t.starts_with("-Djavar.") {
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_agent_and_native() {
        let raw = "-Xmx1g -javaagent:C:/Users/x/.javar/bin/javar-agent.jar=port=19222 -Djavar.native.path=C:/x/javar_core.dll -ea";
        assert_eq!(strip_javar_tool_options(raw), "-Xmx1g -ea");
    }
}

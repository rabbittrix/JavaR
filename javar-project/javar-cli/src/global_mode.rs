//! Global invisible mode — inject JavaR via `JAVA_TOOL_OPTIONS`.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>
//!
//! `javar enable --global` makes ANY JVM (IDE, `mvn`, `java`, Gradle) load the
//! embedded agent without `javar run`.

use crate::embed;
use crate::style;
use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;

const MARKER_START: &str = "-javaagent:";
const ENV_KEY: &str = "JAVA_TOOL_OPTIONS";

/// Build the canonical `JAVA_TOOL_OPTIONS` fragment for this machine.
pub fn tool_options_value() -> Result<String> {
    let agent = embed::ensure_agent_jar(None)?;
    let _ = embed::force_extract_native();
    let native = embed::javar_bin_dir().join(if cfg!(windows) {
        "javar_core.dll"
    } else if cfg!(target_os = "macos") {
        "libjavar_core.dylib"
    } else {
        "libjavar_core.so"
    });

    let agent_s = forward_slashes(&agent);
    let native_s = forward_slashes(&native);

    // Keep paths with forward slashes — JAVA_TOOL_OPTIONS is happier that way on Windows.
    let mut opts = format!("-javaagent:{agent_s}=port=19222 -Djavar.native.path={native_s}");
    let _ = &mut opts;
    Ok(opts)
}

fn forward_slashes(p: &std::path::Path) -> String {
    let s = p
        .canonicalize()
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    s.strip_prefix("//?/")
        .or_else(|| s.strip_prefix(r"\\?\"))
        .unwrap_or(&s)
        .replace('\\', "/")
}

/// Enable JavaR for all JVMs via user `JAVA_TOOL_OPTIONS`.
pub fn cmd_enable_global() -> Result<()> {
    style::header("javar enable --global");
    let fragment = tool_options_value()?;
    let current = get_user_env(ENV_KEY).unwrap_or_default();
    let next = merge_tool_options(&current, &fragment);
    set_user_env(ENV_KEY, &next)?;
    // Also set for this process so immediate `java` works in the same shell.
    env::set_var(ENV_KEY, &next);
    style::ok(format!("{ENV_KEY} = {next}"));
    style::info_line("Any new JVM (IDE / mvn / java) will load the JavaR agent.");
    style::info_line("Restart open IDEs / terminals so they pick up the variable.");
    style::info_line("Disable later with:  javar disable --global");
    Ok(())
}

/// Remove JavaR agent flags from user `JAVA_TOOL_OPTIONS`.
pub fn cmd_disable_global() -> Result<()> {
    style::header("javar disable --global");
    let current = get_user_env(ENV_KEY).unwrap_or_default();
    let next = strip_javar_tool_options(&current);
    if next.trim().is_empty() {
        remove_user_env(ENV_KEY)?;
        env::remove_var(ENV_KEY);
        style::ok(format!("Cleared user {ENV_KEY}"));
    } else {
        set_user_env(ENV_KEY, &next)?;
        env::set_var(ENV_KEY, &next);
        style::ok(format!("{ENV_KEY} = {next}"));
    }
    style::info_line("Restart open IDEs / terminals to drop the agent.");
    Ok(())
}

pub fn merge_tool_options(existing: &str, fragment: &str) -> String {
    let cleaned = strip_javar_tool_options(existing);
    if cleaned.trim().is_empty() {
        fragment.to_string()
    } else {
        format!("{} {}", cleaned.trim(), fragment.trim())
    }
}

pub fn strip_javar_tool_options(existing: &str) -> String {
    // Drop tokens that belong to JavaR (javaagent pointing at javar-agent, -Djavar.*).
    existing
        .split_whitespace()
        .filter(|tok| {
            let t = *tok;
            if t.starts_with(MARKER_START) && t.contains("javar-agent") {
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

#[cfg(windows)]
fn get_user_env(key: &str) -> Result<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let env = hkcu
        .open_subkey_with_flags("Environment", KEY_READ)
        .context("open HKCU\\Environment")?;
    Ok(env.get_value(key).unwrap_or_default())
}

#[cfg(windows)]
fn set_user_env(key: &str, value: &str) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu.create_subkey("Environment")?;
    env.set_value(key, &value.to_string())?;
    broadcast_env_change();
    Ok(())
}

#[cfg(windows)]
fn remove_user_env(key: &str) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let env = hkcu.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
    let _ = env.delete_value(key);
    broadcast_env_change();
    Ok(())
}

#[cfg(windows)]
fn broadcast_env_change() {
    use std::process::Command;
    // Notify Explorer / apps that Environment changed (WM_SETTINGCHANGE).
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Add-Type -Namespace Win32 -Name NativeMethods -MemberDefinition '[DllImport(\"user32.dll\", SetLastError=true, CharSet=CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);' -ErrorAction SilentlyContinue; $r=[UIntPtr]::Zero; [void][Win32.NativeMethods]::SendMessageTimeout([IntPtr]0xffff,0x1A,[UIntPtr]::Zero,'Environment',2,5000,[ref]$r)",
        ])
        .status();
}

#[cfg(not(windows))]
fn get_user_env(key: &str) -> Result<String> {
    Ok(env::var(key).unwrap_or_default())
}

#[cfg(not(windows))]
fn set_user_env(key: &str, value: &str) -> Result<()> {
    use std::io::Write;
    let home = dirs::home_dir().context("home dir")?;
    let marker = "# JavaR JAVA_TOOL_OPTIONS (javar enable --global)";
    let line = format!("export {key}=\"{value}\"");
    let rc = if env::var("SHELL").unwrap_or_default().contains("zsh") {
        home.join(".zshrc")
    } else {
        home.join(".bashrc")
    };
    let content = std::fs::read_to_string(&rc).unwrap_or_default();
    let mut lines: Vec<&str> = content.lines().collect();
    // Remove previous JavaR block
    let mut out = String::new();
    let mut skip = false;
    for l in lines.drain(..) {
        if l.contains(marker) {
            skip = true;
            continue;
        }
        if skip {
            if l.starts_with("export JAVA_TOOL_OPTIONS=") {
                skip = false;
                continue;
            }
            skip = false;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(marker);
    out.push('\n');
    out.push_str(&line);
    out.push('\n');
    std::fs::write(&rc, out).with_context(|| format!("write {}", rc.display()))?;
    // Persist a copy under ~/.javar for non-interactive shells.
    let env_file = embed::javar_home().join("env.sh");
    let _ = std::fs::create_dir_all(embed::javar_home());
    let mut f = std::fs::File::create(&env_file)?;
    writeln!(f, "{marker}")?;
    writeln!(f, "{line}")?;
    style::info_line(format!("Also wrote {}", env_file.display()));
    style::info_line(format!("Run: source {}", rc.display()));
    Ok(())
}

#[cfg(not(windows))]
fn remove_user_env(_key: &str) -> Result<()> {
    use std::io::Write;
    let home = dirs::home_dir().context("home dir")?;
    let marker = "# JavaR JAVA_TOOL_OPTIONS (javar enable --global)";
    for rc_name in [".zshrc", ".bashrc", ".profile"] {
        let rc = home.join(rc_name);
        if !rc.is_file() {
            continue;
        }
        let content = std::fs::read_to_string(&rc).unwrap_or_default();
        if !content.contains(marker) {
            continue;
        }
        let mut out = String::new();
        let mut skip = false;
        for l in content.lines() {
            if l.contains(marker) {
                skip = true;
                continue;
            }
            if skip && l.starts_with("export JAVA_TOOL_OPTIONS=") {
                skip = false;
                continue;
            }
            out.push_str(l);
            out.push('\n');
        }
        std::fs::write(&rc, out)?;
        style::ok(format!("Removed JavaR block from {}", rc.display()));
    }
    let env_file = embed::javar_home().join("env.sh");
    if env_file.is_file() {
        let _ = std::fs::remove_file(&env_file);
    }
    Ok(())
}

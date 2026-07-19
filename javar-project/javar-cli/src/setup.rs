//! `javar setup` — extract runtime assets and add CLI to PATH.

use crate::embed;
use crate::style;
use anyhow::{Context, Result};
#[cfg(not(windows))]
use anyhow::bail;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn cmd_setup() -> Result<()> {
    style::header("JavaR setup");
    style::banner_line(format!("OS: {} / {}", env::consts::OS, env::consts::ARCH));

    // If ~/.javar/bin/javar-agent.jar is missing, write AGENT_BYTES immediately.
    match embed::ensure_agent_jar(None) {
        Ok(agent) => style::ok(format!("Agent → {}", agent.display())),
        Err(e) => style::warn_line(format!("Agent: {e:#}")),
    }
    match embed::force_extract_native() {
        Some(n) => style::ok(format!("Native → {}", n.display())),
        None => {
            let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match embed::resolve_or_extract_native(&cwd) {
                Some(n) => style::ok(format!("Native → {}", n.display())),
                None => style::warn_line(
                    "Native lib not embedded/found — rebuild javar-core then javar-cli",
                ),
            }
        }
    }

    let exe = env::current_exe().context("current_exe")?;
    style::info_line(format!("Binary: {}", exe.display()));

    install_binary_copy(&exe)?;
    install_core_sidecar()?;
    prepend_user_path(&embed::javar_bin_dir())?;

    // Maven for app builds: bootstrap under ~/.javar/tools + shim on PATH.
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match crate::maven::ensure_maven_installed(&cwd) {
        Ok(p) => style::ok(format!("Maven → {}", p.display())),
        Err(e) => style::warn_line(format!("Maven bootstrap: {e:#}")),
    }

    check_tool("java", &["-version"]);
    check_tool_maven();

    style::ok(format!("JavaR home: {}", embed::javar_home().display()));
    style::ok("Setup complete — open a new terminal, then try: javar run");
    Ok(())
}

fn install_core_sidecar() -> Result<()> {
    let dest_name = if cfg!(windows) {
        "javar-core.exe"
    } else {
        "javar-core"
    };
    let dest = embed::javar_bin_dir().join(dest_name);
    let _ = fs::create_dir_all(embed::javar_bin_dir());

    let candidates = [
        env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(dest_name))),
        env::current_dir().ok().map(|d| {
            d.join("target")
                .join("release")
                .join(dest_name)
        }),
        // Typical layout when run from javar-project/
        Some(PathBuf::from("target/release").join(dest_name)),
        Some(PathBuf::from("../target/release").join(dest_name)),
    ];

    for src in candidates.into_iter().flatten() {
        if !src.is_file() {
            continue;
        }
        match fs::copy(&src, &dest) {
            Ok(_) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = fs::metadata(&dest) {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o755);
                        let _ = fs::set_permissions(&dest, perms);
                    }
                }
                style::ok(format!("Sidecar → {}", dest.display()));
                return Ok(());
            }
            Err(e) => style::warn_line(format!("Could not copy sidecar from {}: {e}", src.display())),
        }
    }

    if dest.is_file() {
        style::info_line(format!("Sidecar already at {}", dest.display()));
    } else {
        style::warn_line(
            "javar-core sidecar not found — build with: cargo build --release -p javar-core",
        );
    }
    Ok(())
}

fn install_binary_copy(exe: &Path) -> Result<()> {
    let dest_name = if cfg!(windows) { "javar.exe" } else { "javar" };
    let mut targets = vec![embed::javar_bin_dir().join(dest_name)];

    // Overwrite stale `cargo install` copies that often win on PATH.
    if let Ok(home) = env::var("USERPROFILE").or_else(|_| env::var("HOME")) {
        targets.push(PathBuf::from(home).join(".cargo").join("bin").join(dest_name));
    }
    if let Ok(cargo_home) = env::var("CARGO_HOME") {
        targets.push(PathBuf::from(cargo_home).join("bin").join(dest_name));
    }

    let exe_canon = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    for dest in targets {
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if exe_canon == dest.canonicalize().unwrap_or_default() {
            style::info_line(format!("CLI already at {}", dest.display()));
            continue;
        }
        // Only replace existing cargo-bin javar, or always write ~/.javar/bin.
        let is_javar_home = dest.starts_with(embed::javar_bin_dir());
        if !is_javar_home && !dest.is_file() {
            continue;
        }
        match fs::copy(exe, &dest) {
            Ok(_) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = fs::metadata(&dest) {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o755);
                        let _ = fs::set_permissions(&dest, perms);
                    }
                }
                style::ok(format!("Installed CLI → {}", dest.display()));
            }
            Err(e) => style::warn_line(format!("Could not install to {}: {e}", dest.display())),
        }
    }
    Ok(())
}

/// Prepend a directory to the user PATH (Windows registry / shell rc).
pub(crate) fn prepend_user_path(bin_dir: &Path) -> Result<()> {
    let bin = bin_dir
        .canonicalize()
        .unwrap_or_else(|_| bin_dir.to_path_buf());
    let bin_str = bin.to_string_lossy();
    // Windows canonicalize may yield `\\?\C:\…` which breaks PATH / tools.
    let bin_str = bin_str
        .strip_prefix(r"\\?\")
        .unwrap_or(&bin_str)
        .to_string();

    #[cfg(windows)]
    {
        add_to_path_windows(&bin_str)?;
    }
    #[cfg(not(windows))]
    {
        add_to_path_unix(&bin_str)?;
    }
    Ok(())
}

#[cfg(windows)]
fn add_to_path_windows(bin: &str) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let env = hkcu.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
    let current: String = env.get_value("Path").unwrap_or_default();
    let parts: Vec<&str> = current.split(';').filter(|s| !s.is_empty()).collect();
    if parts.iter().any(|p| p.eq_ignore_ascii_case(bin)) {
        style::info_line("User PATH already contains ~/.javar/bin");
        return Ok(());
    }
    // Prepend so ~/.javar/bin wins over a stale ~/.cargo/bin/javar.
    let rest: Vec<&str> = parts
        .into_iter()
        .filter(|p| !p.eq_ignore_ascii_case(bin))
        .collect();
    let new_path = std::iter::once(bin)
        .chain(rest)
        .collect::<Vec<_>>()
        .join(";");
    env.set_value("Path", &new_path)?;
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "[Environment]::SetEnvironmentVariable('Path',[Environment]::GetEnvironmentVariable('Path','User'),'User')",
        ])
        .status();
    style::ok(format!("Prepended to user PATH: {bin}"));
    style::info_line("Restart the terminal for PATH to apply");
    Ok(())
}

#[cfg(not(windows))]
fn add_to_path_unix(bin: &str) -> Result<()> {
    use std::io::Write;

    let home = dirs::home_dir().context("home dir")?;
    let marker = "# JavaR PATH (javar setup)";
    let line = format!("export PATH=\"{bin}:$PATH\"");
    let candidates = [
        home.join(".zshrc"),
        home.join(".bashrc"),
        home.join(".profile"),
    ];
    let shell = env::var("SHELL").unwrap_or_default();
    let preferred = if shell.contains("zsh") {
        home.join(".zshrc")
    } else if shell.contains("bash") {
        home.join(".bashrc")
    } else {
        home.join(".profile")
    };

    let mut targets = vec![preferred];
    for c in candidates {
        if !targets.contains(&c) {
            targets.push(c);
        }
    }

    for rc in targets {
        if !rc.exists() && rc != home.join(".profile") {
            continue;
        }
        let content = fs::read_to_string(&rc).unwrap_or_default();
        if content.contains(marker) || content.contains(bin) {
            style::info_line(format!("PATH already configured in {}", rc.display()));
            return Ok(());
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&rc)
            .with_context(|| format!("open {}", rc.display()))?;
        writeln!(f)?;
        writeln!(f, "{marker}")?;
        writeln!(f, "{line}")?;
        style::ok(format!("Appended PATH export to {}", rc.display()));
        style::info_line(format!("Run: source {}", rc.display()));
        return Ok(());
    }

    bail!("could not find a shell rc file to update PATH");
}

fn check_tool(name: &str, args: &[&str]) {
    match which::which(name) {
        Ok(p) => {
            let _ = Command::new(&p).args(args).output();
            style::ok(format!("{name} found ({})", p.display()));
        }
        Err(_) => style::warn_line(format!("{name} not found on PATH — install a JDK")),
    }
}

fn check_tool_maven() {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match crate::maven::resolve_mvn(&cwd) {
        Ok(p) => style::ok(format!("maven found ({})", p.display())),
        Err(_) => style::warn_line(
            "maven not found — `javar run` / `javar build` will locate or bootstrap it",
        ),
    }
}

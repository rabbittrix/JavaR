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

    // Force-extract embedded agent + native into ~/.javar/bin.
    match embed::force_extract_agent() {
        Ok(agent) => style::ok(format!("Agent → {}", agent.display())),
        Err(e) => {
            let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match embed::resolve_or_extract_agent(&cwd, None) {
                Ok(agent) => style::ok(format!("Agent → {}", agent.display())),
                Err(_) => style::warn_line(format!("Agent: {e:#}")),
            }
        }
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
    add_to_path(&embed::javar_bin_dir())?;

    check_tool("java", &["-version"]);
    check_tool_maven();

    style::ok(format!("JavaR home: {}", embed::javar_home().display()));
    style::ok("Setup complete — open a new terminal, then try: javar run");
    Ok(())
}

fn install_binary_copy(exe: &Path) -> Result<()> {
    let dest_dir = embed::javar_bin_dir();
    fs::create_dir_all(&dest_dir)?;
    let dest_name = if cfg!(windows) { "javar.exe" } else { "javar" };
    let dest = dest_dir.join(dest_name);
    if exe.canonicalize().ok().as_ref() == dest.canonicalize().ok().as_ref() {
        style::info_line("CLI already installed in ~/.javar/bin");
        return Ok(());
    }
    fs::copy(exe, &dest).with_context(|| format!("copy {} → {}", exe.display(), dest.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms)?;
    }
    style::ok(format!("Installed CLI → {}", dest.display()));
    Ok(())
}

fn add_to_path(bin_dir: &Path) -> Result<()> {
    let bin = bin_dir
        .canonicalize()
        .unwrap_or_else(|_| bin_dir.to_path_buf());
    let bin_str = bin.display().to_string();

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
    let new_path = if current.is_empty() {
        bin.to_string()
    } else if current.ends_with(';') {
        format!("{current}{bin}")
    } else {
        format!("{current};{bin}")
    };
    env.set_value("Path", &new_path)?;
    // Broadcast WM_SETTINGCHANGE so new shells pick it up (best-effort).
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "[Environment]::SetEnvironmentVariable('Path',[Environment]::GetEnvironmentVariable('Path','User'),'User')",
        ])
        .status();
    style::ok(format!("Added to user PATH: {bin}"));
    style::info_line("Restart the terminal (or log out/in) for PATH to apply");
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
    let mvn = if which::which("mvn").is_ok() {
        Some("mvn")
    } else if cfg!(windows) && which::which("mvn.cmd").is_ok() {
        Some("mvn.cmd")
    } else {
        None
    };
    match mvn {
        Some(m) => {
            if let Ok(p) = which::which(m) {
                style::ok(format!("maven found ({})", p.display()));
            }
        }
        None => style::warn_line(
            "maven not found — optional for app builds; `javar run` will prompt when needed",
        ),
    }
}

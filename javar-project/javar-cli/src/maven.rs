//! Locate or bootstrap Apache Maven for `javar build` / smart run.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>

use crate::embed;
use crate::style;
use anyhow::{bail, Context, Result};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MAVEN_VERSION: &str = "3.9.6";

/// Run Maven with a resolved JDK (`JAVA_HOME` + PATH) so misconfigured env still works.
/// Always pins `-Dmaven.compiler.release` when a target can be detected.
pub fn run_maven(project_root: &Path, args: &[&str]) -> Result<()> {
    if let Some(release) = crate::version_sync::compiler_release_target(project_root)
        .or_else(crate::version_sync::runtime_java_major)
    {
        // Avoid duplicating -Dmaven.compiler.release if caller already set it.
        let already = args.iter().any(|a| a.starts_with("-Dmaven.compiler.release="));
        if already {
            return run_maven_with_java(project_root, Some(release), args);
        }
        return run_maven_aligned(project_root, release, args);
    }
    let prefer = preferred_java_major(project_root);
    run_maven_with_java(project_root, prefer, args)
}

/// Force `-Dmaven.compiler.release` (and source/target) to `release`, using a matching JDK.
pub fn run_maven_aligned(project_root: &Path, release: u32, goals: &[&str]) -> Result<()> {
    let mut args: Vec<String> = Vec::new();
    args.push(format!("-Dmaven.compiler.release={release}"));
    args.push(format!("-Dmaven.compiler.source={release}"));
    args.push(format!("-Dmaven.compiler.target={release}"));
    args.push(format!("-Djava.version={release}"));
    for g in goals {
        // Skip duplicate release flags from callers that already included them.
        if g.starts_with("-Dmaven.compiler.") || g.starts_with("-Djava.version=") {
            continue;
        }
        args.push((*g).to_string());
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    style::info_line(format!(
        "Force compatibility: -Dmaven.compiler.release={release}"
    ));
    run_maven_with_java(project_root, Some(release), &refs)
}

fn run_maven_with_java(project_root: &Path, prefer_major: Option<u32>, args: &[&str]) -> Result<()> {
    // Prefer existing Maven; do not auto-download during normal builds.
    let mvn = resolve_mvn_no_bootstrap(project_root)
        .or_else(|_| resolve_mvn(project_root))?;
    let java_home = resolve_java_home(prefer_major)?;
    style::info_line(format!("Maven {}", mvn.display()));
    style::info_line(format!("JAVA_HOME={}", java_home.display()));

    let mut cmd = maven_command(&mvn, args);
    cmd.current_dir(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    apply_java_env(&mut cmd, &java_home);

    let status = cmd
        .status()
        .with_context(|| format!("failed to spawn Maven at {}", mvn.display()))?;
    if !status.success() {
        bail!(
            "Maven failed ({status}).\nMaven: {}\nJAVA_HOME: {}",
            mvn.display(),
            java_home.display()
        );
    }
    Ok(())
}

fn maven_command(mvn: &Path, args: &[&str]) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        let mut full = vec![
            "/C".to_string(),
            native_path(mvn),
        ];
        full.extend(args.iter().map(|s| (*s).to_string()));
        c.args(full);
        c
    } else {
        let mut c = Command::new(mvn);
        c.args(args);
        c
    }
}

fn apply_java_env(cmd: &mut Command, java_home: &Path) {
    cmd.env("JAVA_HOME", java_home);
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut new_path = OsString::from(java_home.join("bin"));
    new_path.push(sep);
    new_path.push(env::var_os("PATH").unwrap_or_default());
    cmd.env("PATH", new_path);
}

/// Ensure Maven is available for a Maven project: resolve, bootstrap if needed,
/// install `mvn` shim into `~/.javar/bin` (already on PATH after `javar setup`).
pub fn ensure_maven_installed(project_root: &Path) -> Result<PathBuf> {
    let had_path = find_on_path().is_some();
    let mvn = resolve_mvn(project_root)?;
    let _ = install_mvn_shim(&mvn);
    // First-time / bootstrapped installs: expose Maven `bin` on the user PATH.
    if !had_path {
        if let Some(bin_dir) = mvn.parent() {
            let _ = crate::setup::prepend_user_path(bin_dir);
        }
        let _ = crate::setup::prepend_user_path(&embed::javar_bin_dir());
    }
    Ok(mvn)
}

/// Absolute path to `mvn` / `mvn.cmd` without downloading anything.
pub fn resolve_mvn_no_bootstrap(project_root: &Path) -> Result<PathBuf> {
    if let Some(p) = find_on_path() {
        return Ok(p);
    }
    if let Some(p) = find_wrapper(project_root) {
        return Ok(p);
    }
    if let Some(p) = find_system_install() {
        return Ok(p);
    }
    if let Some(p) = find_bootstrapped() {
        return Ok(p);
    }
    bail!("Maven not found on PATH (install Maven or run: javar tools install)")
}

/// Absolute path to `mvn` / `mvn.cmd` — may bootstrap under `~/.javar/tools`.
pub fn resolve_mvn(project_root: &Path) -> Result<PathBuf> {
    if let Ok(p) = resolve_mvn_no_bootstrap(project_root) {
        return Ok(p);
    }

    style::warn_line("Maven not on PATH — installing Apache Maven into ~/.javar/tools…");
    let mvn = bootstrap_maven()?;
    let _ = install_mvn_shim(&mvn);
    Ok(mvn)
}

/// Write `~/.javar/bin/mvn(.cmd)` that forwards to the real Maven binary.
fn install_mvn_shim(real_mvn: &Path) -> Result<()> {
    let bin = embed::javar_bin_dir();
    fs::create_dir_all(&bin)?;
    let real = native_path(real_mvn);

    if cfg!(windows) {
        let shim = bin.join("mvn.cmd");
        let body = format!(
            "@echo off\r\n\
             REM JavaR Maven shim — forwards to bootstrapped / system Maven\r\n\
             \"{real}\" %*\r\n"
        );
        fs::write(&shim, body).with_context(|| format!("write {}", shim.display()))?;
        // Also a bare `mvn` for Git Bash / some tools.
        let bash_shim = bin.join("mvn");
        let bash_body = format!("#!/usr/bin/env sh\nexec \"{real}\" \"$@\"\n");
        let _ = fs::write(&bash_shim, bash_body);
    } else {
        let shim = bin.join("mvn");
        let body = format!("#!/usr/bin/env sh\nexec \"{real}\" \"$@\"\n");
        fs::write(&shim, body).with_context(|| format!("write {}", shim.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&shim)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&shim, perms)?;
        }
    }
    style::ok(format!("Maven shim → {}/mvn", bin.display()));
    Ok(())
}

fn find_on_path() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["mvn.cmd", "mvn"]
    } else {
        &["mvn"]
    };
    for name in names {
        if let Ok(p) = which::which(name) {
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn find_wrapper(root: &Path) -> Option<PathBuf> {
    let names = if cfg!(windows) {
        ["mvnw.cmd", "mvnw"]
    } else {
        ["mvnw", "mvnw.cmd"]
    };
    for name in names {
        let p = root.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// MAVEN_HOME (root or …/bin), common install dirs, scoop, etc.
fn find_system_install() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(m2) = std::env::var("MAVEN_HOME") {
        candidates.extend(maven_bins_under(PathBuf::from(m2.trim())));
    }
    if let Ok(m2) = std::env::var("M2_HOME") {
        candidates.extend(maven_bins_under(PathBuf::from(m2.trim())));
    }

    if cfg!(windows) {
        if let Ok(home) = std::env::var("USERPROFILE") {
            candidates.push(
                PathBuf::from(&home).join("scoop\\apps\\maven\\current\\bin\\mvn.cmd"),
            );
        }
        candidates.extend([
            PathBuf::from(r"C:\maven\bin\mvn.cmd"),
            PathBuf::from(r"C:\apache-maven\bin\mvn.cmd"),
            PathBuf::from(r"C:\Program Files\Apache\maven\bin\mvn.cmd"),
            PathBuf::from(r"C:\Program Files\Maven\bin\mvn.cmd"),
            PathBuf::from(r"C:\tools\apache-maven\bin\mvn.cmd"),
        ]);
    } else {
        candidates.extend([
            PathBuf::from("/usr/local/bin/mvn"),
            PathBuf::from("/opt/homebrew/bin/mvn"),
            PathBuf::from("/usr/bin/mvn"),
        ]);
    }

    candidates.into_iter().find(|p| p.is_file())
}

/// Accept either `MAVEN_HOME=C:\maven` or the common misconfig `MAVEN_HOME=C:\maven\bin`.
fn maven_bins_under(home: PathBuf) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        out.push(home.join("bin").join("mvn.cmd"));
        out.push(home.join("mvn.cmd"));
        out.push(home.join("bin").join("mvn"));
        out.push(home.join("mvn"));
    } else {
        out.push(home.join("bin").join("mvn"));
        out.push(home.join("mvn"));
    }
    out
}

fn tools_dir() -> PathBuf {
    embed::javar_home().join("tools")
}

fn bootstrapped_mvn() -> PathBuf {
    let home = tools_dir().join(format!("apache-maven-{MAVEN_VERSION}"));
    if cfg!(windows) {
        home.join("bin").join("mvn.cmd")
    } else {
        home.join("bin").join("mvn")
    }
}

fn find_bootstrapped() -> Option<PathBuf> {
    let p = bootstrapped_mvn();
    p.is_file().then_some(p)
}

fn bootstrap_maven() -> Result<PathBuf> {
    let tools = tools_dir();
    let mvn = bootstrapped_mvn();
    if mvn.is_file() {
        return Ok(mvn);
    }

    fs::create_dir_all(&tools).with_context(|| format!("create {}", tools.display()))?;
    let zip_name = format!("apache-maven-{MAVEN_VERSION}-bin.zip");
    let zip_path = tools.join(&zip_name);
    let url = format!(
        "https://archive.apache.org/dist/maven/maven-3/{MAVEN_VERSION}/binaries/{zip_name}"
    );

    style::info_line(format!("Downloading Apache Maven {MAVEN_VERSION}…"));
    download_file(&url, &zip_path)?;
    style::info_line("Extracting Maven…");
    extract_zip(&zip_path, &tools)?;

    if !mvn.is_file() {
        bail!(
            "Maven bootstrap failed — expected {} after extract",
            mvn.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&mvn)?.permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(&mvn, perms);
    }

    style::ok(format!("Maven ready → {}", mvn.display()));
    Ok(mvn)
}

fn native_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\")
        .unwrap_or(&s)
        .replace('/', if cfg!(windows) { "\\" } else { "/" })
}

fn download_file(url: &str, dest: &Path) -> Result<()> {
    if dest.is_file() && fs::metadata(dest).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        return Ok(());
    }
    let tmp = dest.with_extension("download");
    let _ = fs::remove_file(&tmp);
    let tmp_s = native_path(&tmp);

    let curl_ok = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o", &tmp_s, url])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !curl_ok {
        let ps = format!(
            "Invoke-WebRequest -Uri '{url}' -OutFile '{tmp_s}' -UseBasicParsing"
        );
        let status = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .status()
            .context("spawn powershell for Maven download")?;
        if !status.success() {
            bail!("Failed to download Maven from {url}");
        }
    }

    if let Err(e) = fs::rename(&tmp, dest) {
        fs::copy(&tmp, dest).with_context(|| format!("copy maven zip after rename fail: {e}"))?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(())
}

/// Find a real JDK home (`bin/javac` exists). Fixes broken/orphaned `JAVA_HOME`.
/// When `prefer_major` is set (e.g. 21 from pom), prefer a matching JDK over a newer one.
pub fn resolve_java_home(prefer_major: Option<u32>) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(jh) = env::var("JAVA_HOME") {
        candidates.push(PathBuf::from(jh.trim()));
    }
    if let Some(from_java) = java_home_from_running_java() {
        candidates.push(from_java);
    }
    if cfg!(windows) {
        if let Ok(rd) = fs::read_dir(r"C:\Program Files\Java") {
            for e in rd.flatten() {
                candidates.push(e.path());
            }
        }
        for root in [
            r"C:\Program Files\Eclipse Adoptium",
            r"C:\Program Files\Microsoft",
            r"C:\Program Files\Amazon Corretto",
            r"C:\Program Files\AdoptOpenJDK",
            r"C:\Program Files\Zulu",
        ] {
            if let Ok(rd) = fs::read_dir(root) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.to_ascii_lowercase().contains("jdk"))
                        .unwrap_or(false)
                    {
                        candidates.push(p);
                    }
                }
            }
        }
    } else {
        candidates.extend([
            PathBuf::from("/usr/lib/jvm/default-java"),
            PathBuf::from("/usr/lib/jvm/default"),
        ]);
    }

    let mut valid: Vec<PathBuf> = Vec::new();
    for c in &candidates {
        if is_valid_jdk(c) {
            valid.push(c.clone());
            continue;
        }
        // Common misconfig: JAVA_HOME points at …\bin
        if let Some(parent) = c.parent() {
            if is_valid_jdk(parent) {
                valid.push(parent.to_path_buf());
            }
        }
    }

    if let Some(major) = prefer_major {
        if let Some(match_) = valid.iter().find(|p| jdk_major(p) == Some(major)) {
            return Ok(match_.clone());
        }
    }

    if let Some(first) = valid.into_iter().next() {
        return Ok(first);
    }

    bail!(
        "No valid JDK found for Maven (need a directory with bin/javac).\n\
         Install a JDK 17+ and/or set JAVA_HOME to that directory."
    );
}

pub fn preferred_java_major(project_root: &Path) -> Option<u32> {
    let pom = project_root.join("pom.xml");
    let text = fs::read_to_string(pom).ok()?;
    for key in [
        "maven.compiler.release",
        "maven.compiler.source",
        "java.version",
    ] {
        let open = format!("<{key}>");
        let close = format!("</{key}>");
        if let Some(i) = text.find(&open) {
            let rest = &text[i + open.len()..];
            if let Some(j) = rest.find(&close) {
                if let Ok(n) = rest[..j].trim().parse::<u32>() {
                    return Some(n);
                }
            }
        }
    }
    // <release>21</release> inside compiler plugin
    if let Some(i) = text.find("<release>") {
        let rest = &text[i + "<release>".len()..];
        if let Some(j) = rest.find("</release>") {
            if let Ok(n) = rest[..j].trim().parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

fn jdk_major(home: &Path) -> Option<u32> {
    let name = home.file_name()?.to_str()?.to_ascii_lowercase();
    // jdk-21, jdk-21.0.2, temurin-21.0.1-jdk, ...
    for part in name.split(['-', '_', '.']) {
        if let Ok(n) = part.parse::<u32>() {
            if (8..=25).contains(&n) {
                return Some(n);
            }
        }
    }
    // Fallback: ask javac -version
    let javac = if cfg!(windows) {
        home.join("bin").join("javac.exe")
    } else {
        home.join("bin").join("javac")
    };
    let out = Command::new(javac).arg("-version").output().ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // "javac 21.0.2" / "javac 1.8.0_392"
    let ver = text.split_whitespace().nth(1)?;
    if let Some(rest) = ver.strip_prefix("1.") {
        return rest.split('.').next()?.parse().ok();
    }
    ver.split('.').next()?.parse().ok()
}

fn is_valid_jdk(home: &Path) -> bool {
    let javac = if cfg!(windows) {
        home.join("bin").join("javac.exe")
    } else {
        home.join("bin").join("javac")
    };
    javac.is_file()
}

fn java_home_from_running_java() -> Option<PathBuf> {
    let out = Command::new("java")
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("java.home = ") {
            let home = PathBuf::from(rest.trim());
            if is_valid_jdk(&home) {
                return Some(home);
            }
            if let Some(parent) = home.parent() {
                if is_valid_jdk(parent) {
                    return Some(parent.to_path_buf());
                }
            }
            return Some(home);
        }
    }
    None
}

fn extract_zip(zip: &Path, dest_dir: &Path) -> Result<()> {
    let zip_s = native_path(zip);
    let dest_s = native_path(dest_dir);

    let tar_ok = Command::new("tar")
        .args(["-xf", &zip_s, "-C", &dest_s])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if tar_ok {
        return Ok(());
    }

    if cfg!(windows) {
        let ps = format!(
            "Expand-Archive -LiteralPath '{zip_s}' -DestinationPath '{dest_s}' -Force"
        );
        let status = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .status()
            .context("spawn Expand-Archive")?;
        if !status.success() {
            bail!("Failed to extract {zip_s}");
        }
    } else {
        let status = Command::new("unzip")
            .args(["-qo", &zip_s, "-d", &dest_s])
            .status()
            .context("spawn unzip")?;
        if !status.success() {
            bail!("Failed to extract {zip_s}");
        }
    }
    Ok(())
}

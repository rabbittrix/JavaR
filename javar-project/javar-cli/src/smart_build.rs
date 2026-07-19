//! Smart project build — prompt and run Maven/Gradle when classes are missing.

use crate::smart_run::{BuildSystem, SmartProject};
use crate::style;
use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// If this is a Maven/Gradle project without usable classes, offer to build.
pub fn ensure_project_built(project: &SmartProject, assume_yes: bool) -> Result<SmartProject> {
    let needs_build = match project.classes_dir.as_ref() {
        None => true,
        Some(dir) => !dir_has_classes(dir),
    };

    if !needs_build {
        return Ok(SmartProject::discover(&project.root));
    }

    match project.build {
        BuildSystem::Maven => {
            if !project.root.join("pom.xml").is_file() {
                return Ok(SmartProject::discover(&project.root));
            }
            if !confirm(
                "Maven project not built (no target/classes). Build now? (Y/n) ",
                assume_yes,
            )? {
                style::warn_line("Skipping build — javar run may fail without classes.");
                return Ok(SmartProject::discover(&project.root));
            }
            run_maven_compile(&project.root)?;
        }
        BuildSystem::Gradle => {
            if !confirm(
                "Gradle project not built (no build/classes). Build now? (Y/n) ",
                assume_yes,
            )? {
                style::warn_line("Skipping build — javar run may fail without classes.");
                return Ok(SmartProject::discover(&project.root));
            }
            run_gradle_classes(&project.root)?;
        }
        BuildSystem::Unknown => {}
    }

    Ok(SmartProject::discover(&project.root))
}

fn dir_has_classes(dir: &Path) -> bool {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("class"))
}

fn confirm(prompt: &str, assume_yes: bool) -> Result<bool> {
    use std::io::IsTerminal;
    if assume_yes {
        style::info_line("Building project (--yes)");
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        style::info_line("Non-interactive stdin — building project");
        return Ok(true);
    }
    eprint!("{}", style::accent(prompt));
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let t = line.trim().to_ascii_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes")
}

fn run_maven_compile(root: &Path) -> Result<()> {
    let mvn = resolve_mvn()?;
    style::banner_line(format!("Building with {} -DskipTests compile", mvn));
    let pb = spinner("mvn compile");
    let status = Command::new(&mvn)
        .args(["-q", "-DskipTests", "compile"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to spawn `{mvn}` — is Maven on PATH?"))?;
    pb.finish_and_clear();
    if !status.success() {
        bail!("Maven compile failed ({status})");
    }
    style::ok("Maven compile finished");
    Ok(())
}

fn run_gradle_classes(root: &Path) -> Result<()> {
    let gradle = resolve_gradle(root)?;
    style::banner_line(format!("Building with {} classes", gradle));
    let pb = spinner("gradle classes");
    let status = Command::new(&gradle)
        .arg("classes")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to spawn `{gradle}`"))?;
    pb.finish_and_clear();
    if !status.success() {
        bail!("Gradle classes failed ({status})");
    }
    style::ok("Gradle classes finished");
    Ok(())
}

fn resolve_mvn() -> Result<String> {
    if which::which("mvn").is_ok() {
        return Ok("mvn".into());
    }
    if cfg!(windows) && which::which("mvn.cmd").is_ok() {
        return Ok("mvn.cmd".into());
    }
    bail!("Maven not found on PATH (need `mvn`)");
}

fn resolve_gradle(root: &Path) -> Result<String> {
    let wrapper = if cfg!(windows) {
        root.join("gradlew.bat")
    } else {
        root.join("gradlew")
    };
    if wrapper.is_file() {
        return Ok(wrapper.display().to_string());
    }
    if which::which("gradle").is_ok() {
        return Ok("gradle".into());
    }
    bail!("Gradle not found (no gradlew / gradle on PATH)");
}

fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.magenta} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

//! Smart project build — prompt and run Maven/Gradle when classes are missing.
//! All process spawns use `std::process::Command` directly (PowerShell-safe, no `&&`).

use crate::smart_run::{BuildSystem, SmartProject};
use crate::style;
use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// `javar build` — package/compile the project in `root` (Maven or Gradle).
pub fn cmd_build(root: &Path) -> Result<()> {
    let project = SmartProject::discover(root);
    style::header("javar build");
    style::info_line(crate::smart_run::describe_project(&project));

    match project.build {
        BuildSystem::Maven => {
            if !project.root.join("pom.xml").is_file() {
                bail!("no pom.xml in {}", project.root.display());
            }
            run_maven_package(&project.root)?;
        }
        BuildSystem::Gradle => {
            run_gradle_build(&project.root)?;
        }
        BuildSystem::Unknown => {
            if project.root.join("pom.xml").is_file() {
                run_maven_package(&project.root)?;
            } else if project.root.join("build.gradle").is_file()
                || project.root.join("build.gradle.kts").is_file()
            {
                run_gradle_build(&project.root)?;
            } else {
                bail!(
                    "not a Maven/Gradle project (no pom.xml / build.gradle).\n\
                     Tip: run this inside your Java app directory."
                );
            }
        }
    }
    style::ok("Build finished");
    Ok(())
}

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
            run_maven_package(&project.root)?;
        }
        BuildSystem::Gradle => {
            if !confirm(
                "Gradle project not built (no build/classes). Build now? (Y/n) ",
                assume_yes,
            )? {
                style::warn_line("Skipping build — run  javar build  later.");
                return Ok(SmartProject::discover(&project.root));
            }
            run_gradle_build(&project.root)?;
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

/// `mvn -DskipTests clean package` as separate argv (no shell).
fn run_maven_package(root: &Path) -> Result<()> {
    let mvn = resolve_mvn()?;
    style::banner_line(format!("{mvn} -DskipTests clean package"));
    let pb = spinner("mvn package");
    // Single Maven invocation — goals are argv, never `clean && package` in a shell.
    let status = Command::new(&mvn)
        .args(["-B", "-DskipTests", "clean", "package"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| {
            format!("failed to spawn `{mvn}` — install Maven and ensure it is on PATH")
        })?;
    pb.finish_and_clear();
    if !status.success() {
        bail!("Maven package failed ({status}). Fix the project, then run:  javar build");
    }
    style::ok("Maven package finished");
    Ok(())
}

fn run_gradle_build(root: &Path) -> Result<()> {
    let gradle = resolve_gradle(root)?;
    style::banner_line(format!("{gradle} build -x test"));
    let pb = spinner("gradle build");
    let status = Command::new(&gradle)
        .args(["build", "-x", "test"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to spawn `{gradle}`"))?;
    pb.finish_and_clear();
    if !status.success() {
        bail!("Gradle build failed ({status}). Fix the project, then run:  javar build");
    }
    style::ok("Gradle build finished");
    Ok(())
}

fn resolve_mvn() -> Result<String> {
    if cfg!(windows) {
        if which::which("mvn.cmd").is_ok() {
            return Ok("mvn.cmd".into());
        }
    }
    if which::which("mvn").is_ok() {
        return Ok("mvn".into());
    }
    bail!("Maven not found on PATH. Install Maven, then run:  javar build");
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

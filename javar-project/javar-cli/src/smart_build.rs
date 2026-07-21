//! Smart project build — explicit `javar build` only (no interactive prompts).
//! All process spawns use `std::process::Command` directly (PowerShell-safe, no `&&`).

use crate::smart_run::{BuildSystem, SmartProject};
use crate::style;
use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
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
            // Use PATH / system Maven only — no silent download here.
            // For bootstrap: `javar tools install`.
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

/// Passive check: warn if classes / Boot jar are missing — never prompt, never fail.
/// Returns the rediscovered project. Callers should fall back to watch-only.
pub fn note_missing_artifacts(project: &SmartProject) -> SmartProject {
    let missing_classes = match project.classes_dir.as_ref() {
        None => true,
        Some(dir) => !dir_has_classes(dir),
    };
    let spring =
        project.build == BuildSystem::Maven && crate::smart_run::is_spring_boot(&project.root);
    let missing_boot_jar =
        spring && crate::smart_run::find_spring_boot_jar(&project.root).is_none();

    if missing_boot_jar {
        style::warn_line(
            "Spring Boot executable jar not found — run your usual `mvn package` \
             (or `javar build`). JavaR will watch passively.",
        );
    } else if missing_classes {
        style::warn_line(
            "No compiled classes yet — run `mvn package` / `gradle build` \
             (or `javar build`). JavaR will watch passively.",
        );
    }

    SmartProject::discover(&project.root)
}

fn dir_has_classes(dir: &Path) -> bool {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("class"))
}

/// `mvn -DskipTests clean package` as separate argv (no shell).
fn run_maven_package(root: &Path) -> Result<()> {
    style::banner_line("Building project via Maven");
    let pb = spinner("Maven package");
    // Prefer PATH Maven; only bootstrap when explicitly requested via tools install.
    let result = if crate::maven::resolve_mvn_no_bootstrap(root).is_ok() {
        crate::maven::run_maven(root, &["-B", "-DskipTests", "clean", "package"])
    } else {
        style::warn_line(
            "Maven not on PATH — run `javar tools install` once, or install Maven yourself.",
        );
        Err(anyhow::anyhow!("Maven not found on PATH"))
    };
    pb.finish_and_clear();
    result?;
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

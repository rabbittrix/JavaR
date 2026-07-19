//! Smart `javar run` — detect Maven/Gradle layout, classpath, main class, native lib.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildSystem {
    Maven,
    Gradle,
    Unknown,
}

#[derive(Debug)]
pub struct SmartProject {
    pub root: PathBuf,
    pub build: BuildSystem,
    pub classes_dir: Option<PathBuf>,
    pub source_roots: Vec<PathBuf>,
}

impl SmartProject {
    pub fn discover(root: &Path) -> Self {
        let root = root
            .canonicalize()
            .unwrap_or_else(|_| root.to_path_buf());

        let build = detect_build_system(&root);
        let source_roots = discover_source_roots(&root);
        let classes_dir = discover_classes_dir(&root, build);

        if let Some(ref c) = classes_dir {
            info!(path = %c.display(), ?build, "found compiled classes");
        } else {
            warn!(?build, "no compiled classes dir (target/classes or build/classes)");
        }

        Self {
            root,
            build,
            classes_dir,
            source_roots,
        }
    }
}

fn detect_build_system(root: &Path) -> BuildSystem {
    if root.join("pom.xml").is_file() {
        info!("detected Maven project (pom.xml)");
        BuildSystem::Maven
    } else if root.join("build.gradle").is_file() || root.join("build.gradle.kts").is_file() {
        info!("detected Gradle project (build.gradle)");
        BuildSystem::Gradle
    } else {
        BuildSystem::Unknown
    }
}

fn discover_source_roots(root: &Path) -> Vec<PathBuf> {
    let candidates = [
        root.join("src/main/java"),
        root.join("src"),
        root.join("java"),
    ];
    candidates.into_iter().filter(|p| p.is_dir()).collect()
}

fn discover_classes_dir(root: &Path, build: BuildSystem) -> Option<PathBuf> {
    let maven = [root.join("target/classes")];
    let gradle = [
        root.join("build/classes/java/main"),
        root.join("build/classes/java"),
        root.join("build/classes"),
    ];
    let generic = [
        root.join("target/classes"),
        root.join("build/classes/java/main"),
        root.join("build/classes"),
        root.join("out/production/classes"),
    ];

    let order: Vec<PathBuf> = match build {
        BuildSystem::Maven => maven
            .into_iter()
            .chain(gradle)
            .chain(generic)
            .collect(),
        BuildSystem::Gradle => gradle
            .into_iter()
            .chain(maven)
            .chain(generic)
            .collect(),
        BuildSystem::Unknown => generic.to_vec(),
    };

    let mut seen = std::collections::HashSet::new();
    let mut empty_fallback: Option<PathBuf> = None;
    for dir in order {
        if !seen.insert(dir.clone()) {
            continue;
        }
        if !dir.is_dir() {
            continue;
        }
        if dir_has_class_files(&dir) {
            return Some(dir.canonicalize().unwrap_or(dir));
        }
        if empty_fallback.is_none() {
            empty_fallback = Some(dir.canonicalize().unwrap_or(dir));
        }
    }
    empty_fallback
}

fn dir_has_class_files(dir: &Path) -> bool {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("class"))
}

/// Find a FQCN with `public static void main`, preferring compiled classes and common names.
pub fn find_main_class(project: &SmartProject) -> Option<String> {
    if let Some(from_pom) = read_main_class_from_pom(&project.root) {
        info!(main = %from_pom, "main class from pom.xml");
        return Some(from_pom);
    }

    let mut candidates: Vec<(i32, String, PathBuf)> = Vec::new();

    for src_root in &project.source_roots {
        for entry in WalkDir::new(src_root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("java"))
        {
            let path = entry.path();
            let Ok(text) = fs::read_to_string(path) else {
                continue;
            };
            if !source_has_main(&text) {
                continue;
            }
            let Some(fqcn) = java_path_to_fqcn(src_root, path) else {
                continue;
            };
            let score = main_class_score(&fqcn, project.classes_dir.as_deref());
            candidates.push((score, fqcn, path.to_path_buf()));
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    if let Some((_, fqcn, path)) = candidates.first() {
        info!(main = %fqcn, source = %path.display(), "discovered main class");
        Some(fqcn.clone())
    } else {
        None
    }
}

fn main_class_score(fqcn: &str, classes: Option<&Path>) -> i32 {
    let simple = fqcn.rsplit('.').next().unwrap_or(fqcn);
    let mut score = 0;
    if simple.eq_ignore_ascii_case("Main") {
        score += 100;
    } else if simple.ends_with("Main") || simple.ends_with("App") || simple.ends_with("Application")
    {
        score += 50;
    } else if simple.contains("Hello") {
        score += 20;
    }
    if let Some(classes) = classes {
        let class_file = classes.join(fqcn.replace('.', "/") + ".class");
        if class_file.is_file() {
            score += 200;
        }
    }
    score
}

/// Rough but practical detection of a JVM entry point in source.
fn source_has_main(source: &str) -> bool {
    let stripped = strip_java_comments(source);
    let compact: String = stripped
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    // publicstaticvoidmain(  — covers String[] / String... / throws variants
    compact.contains("publicstaticvoidmain(")
}

fn strip_java_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut in_line = false;
    let mut in_block = false;
    let mut in_str = false;
    let mut in_char = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let next = bytes.get(i + 1).map(|&b| b as char);
        if in_line {
            if c == '\n' {
                in_line = false;
                out.push(c);
            }
            i += 1;
            continue;
        }
        if in_block {
            if c == '*' && next == Some('/') {
                in_block = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if in_char {
            out.push(c);
            if c == '\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if c == '\'' {
                in_char = false;
            }
            i += 1;
            continue;
        }
        if c == '/' && next == Some('/') {
            in_line = true;
            i += 2;
            continue;
        }
        if c == '/' && next == Some('*') {
            in_block = true;
            i += 2;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '\'' {
            in_char = true;
            out.push(c);
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn java_path_to_fqcn(src_root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(src_root).ok()?;
    let mut parts: Vec<String> = rel
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let last = parts.last_mut()?;
    if !last.ends_with(".java") {
        return None;
    }
    last.truncate(last.len() - 5);
    if parts.iter().any(|p| p.is_empty() || p.contains('-')) {
        return None;
    }
    Some(parts.join("."))
}

fn read_main_class_from_pom(root: &Path) -> Option<String> {
    let pom = root.join("pom.xml");
    let text = fs::read_to_string(pom).ok()?;
    // <mainClass>com.example.App</mainClass>
    let key = "<mainClass>";
    let start = text.find(key)? + key.len();
    let end = text[start..].find("</mainClass>")? + start;
    let name = text[start..end].trim();
    if name.is_empty() || name.contains('<') {
        return None;
    }
    Some(name.to_string())
}

/// True if `args` already set a classpath (`-cp` / `-classpath` / `--class-path`).
pub fn args_have_classpath(args: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-cp" || a == "-classpath" || a == "--class-path" {
            return true;
        }
        if a.starts_with("-cp=") || a.starts_with("-classpath=") {
            return true;
        }
        // jar mode
        if a == "-jar" {
            return true;
        }
        i += 1;
    }
    false
}

/// Heuristic: last non-option token that looks like a Java FQCN / simple class name.
pub fn args_have_main_class(args: &[String]) -> bool {
    args.iter().rev().any(|a| looks_like_main_class(a))
}

fn looks_like_main_class(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') {
        return false;
    }
    // path-like classpath entries
    if s.contains('/') || s.contains('\\') || s.ends_with(".jar") || s.ends_with(".class") {
        return false;
    }
    let mut parts = s.split('.');
    let first = parts.next().unwrap_or("");
    if first.is_empty() {
        return false;
    }
    // At least one identifier; typically starts with uppercase for class
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.')
        && s.split('.').all(|p| {
            !p.is_empty()
                && p.chars()
                    .next()
                    .map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
                    .unwrap_or(false)
        })
}

/// Build the `java` argv after `-javaagent:...` (classpath, native props, main, …).
pub fn build_java_launch_args(
    project: &SmartProject,
    user_args: &[String],
    native: Option<&Path>,
) -> Result<Vec<String>> {
    let mut out = Vec::new();

    if let Some(native) = native {
        let abs = native
            .canonicalize()
            .unwrap_or_else(|_| native.to_path_buf());
        out.push(format!("-Djavar.native.path={}", abs.display()));
        if let Some(dir) = abs.parent() {
            out.push(format!("-Djava.library.path={}", dir.display()));
        }
    }

    let mut rest = user_args.to_vec();

    if !args_have_classpath(&rest) {
        let classes = project
            .classes_dir
            .clone()
            .context(
                "no compiled classes found (expected target/classes or build/classes/java/main). \
                 Build the project first (mvn compile / gradle classes).",
            )?;
        out.push("-cp".into());
        out.push(classes.display().to_string());
    }

    if !args_have_main_class(&rest) {
        let main = find_main_class(project).context(
            "no main class provided and none found with `public static void main`. \
             Pass one after `--`, e.g. `javar run -- com.example.App`",
        )?;
        // If user passed program args only (unlikely without main), keep them after main.
        if rest.is_empty() {
            rest.push(main);
        } else if rest.iter().all(|a| !looks_like_main_class(a)) {
            // Insert main before trailing program args that aren't options…
            // Prefer: treat entire rest as program args after discovered main.
            let mut with_main = vec![main];
            with_main.append(&mut rest);
            rest = with_main;
        }
    }

    out.extend(rest);
    Ok(out)
}

/// Whether this directory looks like a Java app we can auto-launch.
pub fn can_smart_launch(project: &SmartProject) -> bool {
    project.classes_dir.is_some()
        && (project.build != BuildSystem::Unknown
            || !project.source_roots.is_empty()
            || project.root.join("javar.toml").is_file())
}

pub fn describe_project(project: &SmartProject) -> String {
    let build = match project.build {
        BuildSystem::Maven => "Maven",
        BuildSystem::Gradle => "Gradle",
        BuildSystem::Unknown => "unknown",
    };
    let classes = project
        .classes_dir
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(none)".into());
    format!("build={build}, classes={classes}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_main_signature() {
        assert!(source_has_main(
            "public class A { public static void main(String[] args) {} }"
        ));
        assert!(source_has_main(
            "public static void main( String args[] ) throws Exception {}"
        ));
        assert!(!source_has_main(
            "// public static void main(String[] args)\nclass A {}"
        ));
    }

    #[test]
    fn fqcn_from_path() {
        let root = PathBuf::from("/proj/src/main/java");
        let file = root.join("com/example/Hello.java");
        assert_eq!(
            java_path_to_fqcn(&root, &file).as_deref(),
            Some("com.example.Hello")
        );
    }
}

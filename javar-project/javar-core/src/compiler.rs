//! Isolated incremental `javac` — never waits for the IDE.
//! Always emits bytecode compatible with `--release` (default 21) and keeps
//! `javar-agent.jar` on the classpath so `@JavaRManaged` resolves.

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use memmap2::Mmap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{info, warn};
use which::which;

#[derive(Debug, Clone)]
pub struct CompileRequest {
    pub source: PathBuf,
}

impl CompileRequest {
    pub fn from_source(path: &Path) -> Self {
        Self {
            source: path.to_path_buf(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClassArtifact {
    pub class_name: String,
    pub class_path: PathBuf,
    /// Shared bytecode buffer (reference-counted, clone is O(1)).
    pub bytecode: Bytes,
}

#[derive(Debug, Clone)]
pub struct Compiler {
    source_roots: Vec<PathBuf>,
    classpath: Vec<PathBuf>,
    output_dir: PathBuf,
    /// `javac --release N` target (pinned so IDE Java 23 settings cannot win).
    release: u32,
}

impl Compiler {
    pub fn new(source_roots: Vec<PathBuf>, classpath: Vec<PathBuf>, output_dir: PathBuf) -> Self {
        Self::with_release(source_roots, classpath, output_dir, resolve_compiler_release())
    }

    pub fn with_release(
        source_roots: Vec<PathBuf>,
        mut classpath: Vec<PathBuf>,
        output_dir: PathBuf,
        release: u32,
    ) -> Self {
        ensure_agent_on_classpath(&mut classpath);
        if !classpath.iter().any(|p| p == &output_dir) && output_dir.is_dir() {
            classpath.insert(0, output_dir.clone());
        }
        Self {
            source_roots,
            classpath,
            output_dir,
            release,
        }
    }

    pub fn release(&self) -> u32 {
        self.release
    }

    pub async fn compile_async(&self, req: CompileRequest) -> Result<ClassArtifact> {
        std::fs::create_dir_all(&self.output_dir)
            .with_context(|| format!("create output {}", self.output_dir.display()))?;

        let javac = which("javac").context("javac not found on PATH")?;
        let mut cmd = Command::new(javac);

        // Isolated compile — ignore IDE project settings entirely.
        cmd.arg("-g")
            .arg("-parameters")
            .arg("--release")
            .arg(self.release.to_string())
            .arg("-d")
            .arg(&self.output_dir);

        if !self.classpath.is_empty() {
            let cp = std::env::join_paths(self.classpath.iter()).unwrap_or_default();
            cmd.arg("-classpath").arg(cp);
        } else {
            warn!("javac classpath empty — @JavaRManaged may fail to resolve");
        }

        if !self.source_roots.is_empty() {
            let sp = std::env::join_paths(self.source_roots.iter()).unwrap_or_default();
            cmd.arg("-sourcepath").arg(sp);
        }

        cmd.arg(&req.source)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        info!(
            source = %req.source.display(),
            release = self.release,
            "isolated javac (--release, agent on -cp)"
        );
        let output = cmd.output().await.context("spawn javac")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("javac failed:\n{stderr}");
        }

        let class_name = class_name_from_source(&req.source, &self.source_roots)?;
        let class_path = self
            .output_dir
            .join(class_name.replace('.', "/"))
            .with_extension("class");

        if !class_path.exists() {
            bail!("compiled class not found at {}", class_path.display());
        }

        // Verify we did not accidentally emit a newer major (wrong javac).
        if let Some(major) = read_class_major(&class_path) {
            let java = major.saturating_sub(44);
            if java > self.release {
                bail!(
                    "javac emitted Java {java} bytecode (major {major}) despite --release {}; check JAVA_HOME/javac",
                    self.release
                );
            }
        }

        let bytecode = map_class_bytes(&class_path)?;
        info!(%class_name, bytes = bytecode.len(), release = self.release, "compile ok");

        Ok(ClassArtifact {
            class_name,
            class_path,
            bytecode,
        })
    }

    /// Memory-map a `.class` file and wrap as `Bytes` without copying when possible.
    pub fn load_class_bytes(&self, path: &Path) -> Result<ClassArtifact> {
        let class_name = class_name_from_class_file(path, &self.output_dir)?;
        let bytecode = map_class_bytes(path)?;
        Ok(ClassArtifact {
            class_name,
            class_path: path.to_path_buf(),
            bytecode,
        })
    }

    /// True when `.class` major is newer than our `--release` target (IDE contamination).
    pub fn class_newer_than_release(&self, path: &Path) -> bool {
        match read_class_major(path) {
            Some(major) if major >= 52 => major.saturating_sub(44) > self.release,
            _ => false,
        }
    }

    /// Resolve `Foo.java` for `…/target/classes/…/Foo.class` under known source roots.
    pub fn source_for_class(&self, class_path: &Path) -> Option<PathBuf> {
        let class_name = class_name_from_class_file(class_path, &self.output_dir).ok()?;
        let rel = class_name.replace('.', "/") + ".java";
        for root in &self.source_roots {
            let candidate = root.join(&rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }
}

/// `JAVAR_COMPILER_RELEASE` override, else **21** (stable default vs IDE Java 23).
pub fn resolve_compiler_release() -> u32 {
    if let Ok(v) = std::env::var("JAVAR_COMPILER_RELEASE") {
        if let Ok(n) = v.trim().parse::<u32>() {
            if (8..=25).contains(&n) {
                return n;
            }
        }
    }
    // Match a lower runtime when the terminal JVM is 17/21; never follow IDE's 23 by default.
    match detect_runtime_java_major() {
        Some(rt) if rt <= 21 => rt,
        _ => 21,
    }
}

fn detect_runtime_java_major() -> Option<u32> {
    let out = std::process::Command::new("java")
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("java.specification.version = ") {
            let v = rest.trim();
            if let Some(rest) = v.strip_prefix("1.") {
                return rest.parse().ok();
            }
            return v.parse().ok();
        }
    }
    None
}

fn ensure_agent_on_classpath(cp: &mut Vec<PathBuf>) {
    let candidates = [
        std::env::var_os("JAVAR_HOME").map(|h| PathBuf::from(h).join("bin").join("javar-agent.jar")),
        dirs_home().map(|h| h.join(".javar").join("bin").join("javar-agent.jar")),
    ];
    for agent in candidates.into_iter().flatten() {
        if agent.is_file() {
            if !cp.iter().any(|p| p == &agent) {
                // Keep agent near the front so annotation types resolve reliably.
                let insert_at = if cp.first().is_some_and(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n == "classes")
                }) {
                    1
                } else {
                    0
                };
                cp.insert(insert_at, agent);
                info!("compiler classpath includes javar-agent.jar (@JavaRManaged)");
            }
            return;
        }
    }
    warn!(
        "javar-agent.jar not found under ~/.javar/bin — @JavaRManaged will not resolve in incremental javac"
    );
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

pub fn read_class_major(path: &Path) -> Option<u32> {
    let mut f = File::open(path).ok()?;
    let mut hdr = [0u8; 8];
    use std::io::Read;
    f.read_exact(&mut hdr).ok()?;
    if hdr[0..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }
    Some(u16::from_be_bytes([hdr[6], hdr[7]]) as u32)
}

/// Prefer mmap for large classes; small files still benefit from shared `Bytes`.
fn map_class_bytes(path: &Path) -> Result<Bytes> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let meta = file.metadata()?;
    if meta.len() == 0 {
        bail!("empty class file {}", path.display());
    }

    // SAFETY: file is opened read-only; we do not mutate it while mapped.
    let mmap = unsafe { Mmap::map(&file) }.context("mmap class file")?;
    Ok(Bytes::copy_from_slice(&mmap[..]))
}

fn class_name_from_source(source: &Path, roots: &[PathBuf]) -> Result<String> {
    let source = dunce_canonicalize(source);
    for root in roots {
        let root = dunce_canonicalize(root);
        if let Ok(rel) = source.strip_prefix(&root) {
            let mut rel = rel.to_path_buf();
            rel.set_extension("");
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join(".");
            return Ok(name);
        }
    }
    Ok(source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown".into()))
}

fn class_name_from_class_file(path: &Path, output_dir: &Path) -> Result<String> {
    let path = dunce_canonicalize(path);
    let out = dunce_canonicalize(output_dir);
    if let Ok(rel) = path.strip_prefix(&out) {
        let mut rel = rel.to_path_buf();
        rel.set_extension("");
        return Ok(rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("."));
    }
    Ok(path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown".into()))
}

fn dunce_canonicalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

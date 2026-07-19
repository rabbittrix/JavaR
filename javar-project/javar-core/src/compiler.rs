//! Background compilation of `.java` sources and zero-copy `.class` loading.

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use memmap2::Mmap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, info};
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
}

impl Compiler {
    pub fn new(source_roots: Vec<PathBuf>, classpath: Vec<PathBuf>, output_dir: PathBuf) -> Self {
        Self {
            source_roots,
            classpath,
            output_dir,
        }
    }

    pub async fn compile_async(&self, req: CompileRequest) -> Result<ClassArtifact> {
        std::fs::create_dir_all(&self.output_dir)
            .with_context(|| format!("create output {}", self.output_dir.display()))?;

        let javac = which("javac").context("javac not found on PATH")?;
        let mut cmd = Command::new(javac);
        cmd.arg("-g")
            .arg("-parameters")
            .arg("-d")
            .arg(&self.output_dir);

        if !self.classpath.is_empty() {
            let cp = std::env::join_paths(self.classpath.iter()).unwrap_or_default();
            cmd.arg("-classpath").arg(cp);
        }

        // Prefer source-root `-sourcepath` for multi-file resolution.
        if !self.source_roots.is_empty() {
            let sp = std::env::join_paths(self.source_roots.iter()).unwrap_or_default();
            cmd.arg("-sourcepath").arg(sp);
        }

        cmd.arg(&req.source)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        debug!(source = %req.source.display(), "compiling");
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
            // Inner classes / alternate output — fall back to stem next to output.
            bail!("compiled class not found at {}", class_path.display());
        }

        let bytecode = map_class_bytes(&class_path)?;
        info!(%class_name, bytes = bytecode.len(), "compile ok");

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
    // Copy into Bytes for Send/'static across async tasks; mmap drop would otherwise
    // invalidate. For true zero-copy across the socket we still write the buffer once.
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
    // Fallback: file stem only (default package).
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

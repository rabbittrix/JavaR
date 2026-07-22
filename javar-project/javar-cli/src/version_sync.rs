//! Bytecode ↔ JVM version sync (Version Protector) and Spring Boot port cleanup.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>

use crate::smart_run::{self, SmartProject};
use crate::style;
use anyhow::{bail, Result};
use std::fs::File;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Class-file major → Java language major (Java 8 = 52 → 8).
pub fn java_major_from_class_major(class_major: u32) -> Option<u32> {
    if class_major >= 52 {
        Some(class_major - 44)
    } else if class_major >= 45 {
        // 45..51 → Java 1.1 .. 1.7
        Some(class_major.saturating_sub(44))
    } else {
        None
    }
}

/// Read big-endian major version from a `.class` file header (bytes 6–7).
pub fn read_class_file_major(path: &Path) -> Option<u32> {
    let mut f = File::open(path).ok()?;
    let mut hdr = [0u8; 8];
    f.read_exact(&mut hdr).ok()?;
    if hdr[0..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }
    Some(u16::from_be_bytes([hdr[6], hdr[7]]) as u32)
}

/// Active `java` on PATH / JAVA_HOME — the JVM that will run the app.
pub fn runtime_java_major() -> Option<u32> {
    let out = Command::new("java")
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

/// Release to pin on Maven builds: prefer `pom.xml` when it is ≤ runtime JVM, else runtime.
/// Ensures IDE/Java-23 leftovers cannot be reintroduced by `javar build` / auto-clean.
pub fn compiler_release_target(project_root: &Path) -> Option<u32> {
    let runtime = runtime_java_major()?;
    let pom = crate::maven::preferred_java_major(project_root);
    match pom {
        Some(p) if p > 0 && p <= runtime => Some(p),
        _ => Some(runtime),
    }
}

/// Scan `classes_dir` for the highest class-file major (any `.class`).
pub fn newest_class_major(classes_dir: &Path) -> Option<(u32, PathBuf)> {
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in walkdir::WalkDir::new(classes_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("class"))
    {
        let path = entry.path().to_path_buf();
        if let Some(major) = read_class_file_major(&path) {
            match &best {
                Some((m, _)) if *m >= major => {}
                _ => best = Some((major, path)),
            }
        }
    }
    best
}

/// Highest bytecode Java language level under the project's classes dir.
fn scan_bytecode_java(project: &SmartProject) -> Option<(u32, u32, PathBuf)> {
    let dir = project.classes_dir.as_ref()?;
    if !dir.is_dir() {
        return None;
    }
    let (class_major, sample) = newest_class_major(dir)?;
    let java = java_major_from_class_major(class_major)?;
    Some((java, class_major, sample))
}

/// Version Protector: if any class in `target/classes` is newer than the runtime JVM,
/// do **not** start the app — auto `mvn clean compile` with a compatible JDK / release.
pub fn ensure_compatible_bytecode(project: &SmartProject) -> Result<()> {
    let Some(runtime) = runtime_java_major() else {
        style::warn_line("Could not detect runtime java -version — skipping bytecode check");
        return Ok(());
    };

    let Some((bytecode_java, class_major, sample)) = scan_bytecode_java(project) else {
        style::info_line("No .class files under target/classes yet — skipping version check");
        return Ok(());
    };

    if bytecode_java <= runtime {
        style::info_line(format!(
            "Bytecode OK — newest Java {bytecode_java} (class major {class_major}) ≤ runtime Java {runtime}"
        ));
        return Ok(());
    }

    // Do not launch with incompatible classes.
    style::warn_line(format!(
        "⚠️ Incompatible bytecode (Java {bytecode_java}) detected for Java {runtime} runtime. Auto-cleaning project..."
    ));
    style::info_line(format!(
        "Offender: {} (class major {class_major})",
        sample.display()
    ));

    if !project.root.join("pom.xml").is_file() {
        bail!(
            "Bytecode is Java {bytecode_java} but JVM is Java {runtime}, and no pom.xml to rebuild.\n\
             Fix: compile with JDK {runtime}, or run on JDK {bytecode_java}."
        );
    }

    let release = compiler_release_target(&project.root).unwrap_or(runtime);
    let goals: &[&str] = if smart_run::is_spring_boot(&project.root)
        && smart_run::find_spring_boot_jar(&project.root).is_some()
    {
        &["-B", "-DskipTests", "clean", "package"]
    } else {
        &["-B", "-DskipTests", "clean", "compile"]
    };

    style::banner_line(format!(
        "mvn clean {} -Dmaven.compiler.release={release} (JAVA_HOME → JDK {release})",
        goals.last().unwrap_or(&"compile")
    ));
    crate::maven::run_maven_aligned(&project.root, release, goals)?;

    // Re-scan — refuse to start if IDE raced and wrote Java 23 classes again.
    if let Some((again, again_major, again_sample)) = scan_bytecode_java(project) {
        if again > runtime {
            bail!(
                "Still incompatible after clean: Java {again} bytecode (major {again_major}) \
                 for Java {runtime} runtime.\n  {}\n\
                 Tip: set IDE project SDK / bytecode target to Java {runtime}, then retry `javar run`.",
                again_sample.display()
            );
        }
    }

    style::ok(format!(
        "Project cleaned — bytecode now compatible with Java {runtime}"
    ));
    Ok(())
}

/// If `port` is already bound, kill the owning process (Spring Boot zombie cleanup).
pub fn free_tcp_port(port: u16) -> Result<()> {
    if !port_in_use(port) {
        return Ok(());
    }
    style::warn_line(format!(
        "Port {port} is busy — killing the process holding it"
    ));
    let pids = pids_listening_on(port);
    if pids.is_empty() {
        style::warn_line(format!(
            "Port {port} looks busy but PID could not be resolved — you may still hit bind errors"
        ));
        return Ok(());
    }
    for pid in pids {
        kill_pid(pid);
        style::ok(format!("Killed PID {pid} (was holding :{port})"));
    }
    // Brief wait for the OS to release the socket.
    std::thread::sleep(Duration::from_millis(400));
    if port_in_use(port) {
        style::warn_line(format!(
            "Port {port} still busy after kill — Spring Boot may fail to bind"
        ));
    } else {
        style::ok(format!("Port {port} is free"));
    }
    Ok(())
}

fn port_in_use(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
}

fn pids_listening_on(port: u16) -> Vec<u32> {
    if cfg!(windows) {
        pids_windows(port)
    } else {
        pids_unix(port)
    }
}

fn pids_windows(port: u16) -> Vec<u32> {
    let out = Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()
        .ok();
    let Some(out) = out else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = format!(":{port}");
    let mut pids = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.contains(&needle) {
            continue;
        }
        if !line.to_ascii_uppercase().contains("LISTENING") {
            continue;
        }
        // Proto  Local Address  Foreign  State  PID
        let pid = line.split_whitespace().last().and_then(|s| s.parse().ok());
        if let Some(pid) = pid {
            if pid > 0 && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    pids
}

fn pids_unix(port: u16) -> Vec<u32> {
    // lsof -ti tcp:PORT
    if let Ok(out) = Command::new("lsof")
        .args(["-ti", &format!("tcp:{port}")])
        .output()
    {
        if out.status.success() {
            let mut pids = Vec::new();
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    if pid > 0 && !pids.contains(&pid) {
                        pids.push(pid);
                    }
                }
            }
            if !pids.is_empty() {
                return pids;
            }
        }
    }
    // fuser PORT/tcp
    if let Ok(out) = Command::new("fuser")
        .args([format!("{port}/tcp")])
        .output()
    {
        let mut pids = Vec::new();
        for tok in String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .chain(String::from_utf8_lossy(&out.stderr).split_whitespace())
        {
            let digits: String = tok.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(pid) = digits.parse::<u32>() {
                if pid > 0 && !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
        return pids;
    }
    Vec::new()
}

fn kill_pid(pid: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    } else {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_java23_class_major() {
        assert_eq!(java_major_from_class_major(67), Some(23));
        assert_eq!(java_major_from_class_major(65), Some(21));
        assert_eq!(java_major_from_class_major(61), Some(17));
    }
}

//! Write per-project inject files so `mvn spring-boot:run` / IDEs load the agent
//! without global JAVA_TOOL_OPTIONS.
//!
//! Author: Roberto de Souza <rabbittrix@hotmail.com>

use anyhow::{bail, Context, Result};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

pub fn cmd_inject(project: &Path, preferred_port: u16) -> Result<()> {
    let agent = crate::embed::ensure_agent_jar(None)?;
    let _ = crate::embed::force_extract_native();
    let root = crate::workspace_root(project);
    if !agent.is_file() {
        bail!("javar-agent.jar missing - run: javar setup");
    }
    let port = allocate_port(preferred_port);
    let agent_fwd = forward_slashes(&agent);

    style_banner(&root, port);

    if root.join("pom.xml").is_file() {
        let path = write_maven_config(&root, &agent_fwd, port)?;
        crate::style::ok(format!("maven.config -> {}", path.display()));
    } else {
        crate::style::info_line(
            "No pom.xml - skipped .mvn/maven.config (Gradle/IDE use env / vmArgs)",
        );
    }

    let settings = write_vscode_settings(&root, &agent_fwd, port)?;
    crate::style::ok(format!(".vscode/settings.json -> {}", settings.display()));

    crate::style::info_line(format!(
        "Agent port {port}. Restart `mvn spring-boot:run` / Run Java, then: javar dashboard"
    ));
    crate::style::info_line("Open a new integrated terminal if you rely on JAVA_TOOL_OPTIONS.");
    Ok(())
}

fn style_banner(root: &Path, port: u16) {
    crate::style::banner_line("JavaR workspace inject");
    crate::style::info_line(format!("project -> {}", root.display()));
    crate::style::info_line(format!("port    -> {port}"));
}

fn allocate_port(preferred: u16) -> u16 {
    let start = if (19222..=19242).contains(&preferred) {
        preferred
    } else {
        19222
    };
    for p in start..=19242 {
        if port_free(p) {
            return p;
        }
    }
    for p in 19222..=19242 {
        if port_free(p) {
            return p;
        }
    }
    start
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn forward_slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn write_maven_config(root: &Path, agent_fwd: &str, port: u16) -> Result<PathBuf> {
    let dir = root.join(".mvn");
    fs::create_dir_all(&dir)?;
    let path = dir.join("maven.config");
    // maven.config: one CLI arg per line. Comments (# ...) are NOT supported by
    // many Maven versions — they get tokenized as unrecognized options.
    let agents = format!("-Dspring-boot.run.agents={agent_fwd}");
    let jvm = format!("-Dspring-boot.run.jvmArguments=-Djavar.agent.port={port}");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let next = upsert_maven_javar_lines(&existing, &agents, &jvm);
    if next != existing {
        fs::write(&path, with_trailing_newline(next))?;
    }
    Ok(path)
}

fn with_trailing_newline(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Keep non-JavaR args; replace prior JavaR / broken comment marker lines.
fn upsert_maven_javar_lines(existing: &str, agents_line: &str, jvm_line: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for line in existing.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Drop unsupported comment / marker lines and previous JavaR inject.
        if t.starts_with('#')
            || t.contains("javar-agent")
            || t.contains("javar.agent.port")
            || t.contains("spring-boot.run.agents")
            || (t.contains("spring-boot.run.jvmArguments") && t.contains("javar"))
        {
            continue;
        }
        kept.push(t.to_string());
    }
    kept.push(agents_line.to_string());
    kept.push(jvm_line.to_string());
    kept.join("\n")
}

fn write_vscode_settings(root: &Path, agent_fwd: &str, port: u16) -> Result<PathBuf> {
    let dir = root.join(".vscode");
    fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");
    let native_name = if cfg!(windows) {
        "javar_core.dll"
    } else if cfg!(target_os = "macos") {
        "libjavar_core.dylib"
    } else {
        "libjavar_core.so"
    };
    let native = crate::embed::javar_bin_dir().join(native_name);
    let native_fwd = forward_slashes(&native);
    let vm = format!(
        "-javaagent:{agent_fwd}=port={port} -Djavar.native.path={native_fwd} -Djavar.launched.by=vscode"
    );
    let addr = format!("127.0.0.1:{port}");

    let mut v: serde_json::Value = if path.is_file() {
        let text = fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}))
       } else {
        serde_json::json!({})
    };
    if !v.is_object() {
        v = serde_json::json!({});
    }
    let obj = v.as_object_mut().unwrap();
    obj.insert(
        "java.debug.settings.vmArgs".into(),
        serde_json::Value::String(vm.clone()),
    );
    obj.insert("javar.agentPort".into(), serde_json::json!(port));
    obj.insert("javar.injectWorkspace".into(), serde_json::json!(true));

    let term_key = if cfg!(windows) {
        "terminal.integrated.env.windows"
    } else if cfg!(target_os = "macos") {
        "terminal.integrated.env.osx"
    } else {
        "terminal.integrated.env.linux"
    };
    let mut term = obj
        .get(term_key)
        .and_then(|x| x.as_object())
        .cloned()
        .unwrap_or_default();
    term.insert(
        "JAVA_TOOL_OPTIONS".into(),
        serde_json::Value::String(vm.clone()),
    );
    term.insert("MAVEN_OPTS".into(), serde_json::Value::String(vm));
    term.insert(
        "JAVAR_AGENT_ADDR".into(),
        serde_json::Value::String(addr.clone()),
    );
    term.insert(
        "JAVAR_PINNED_ADDR".into(),
        serde_json::Value::String(addr),
    );
    term.insert(
        "JAVAR_AGENT_PORT".into(),
        serde_json::Value::String(port.to_string()),
    );
    term.insert(
        "JAVAR_NATIVE_PATH".into(),
        serde_json::Value::String(native_fwd),
    );
    obj.insert(term_key.into(), serde_json::Value::Object(term));

    let body = serde_json::to_string_pretty(&v).context("serialize settings.json")?;
    fs::write(&path, format!("{body}\n"))?;
    Ok(path)
}

//! Discover live JavaR agents from `~/.javar/agents/*.json`.
//! Author: Roberto de Souza <rabbittrix@hotmail.com>

use serde::{Deserialize, Serialize};
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use sysinfo::{Pid, ProcessesToUpdate, System};

pub const PORT_RANGE_START: u16 = 19222;
pub const PORT_RANGE_END: u16 = 19242;

/// A JavaR-enabled JVM discovered via the agent registry.
pub type JavarProcess = DiscoveredAgent;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveredAgent {
    pub pid: u32,
    pub port: u16,
    /// Canonical registry field (`name`).
    #[serde(default, alias = "project_name")]
    pub name: String,
    #[serde(default)]
    pub cmd: String,
    /// Working directory recorded by the agent (`user.dir`).
    #[serde(default)]
    pub cwd: String,
    /// `javar-run` when launched via the CLI (preferred by Dashboard).
    #[serde(default)]
    pub launched_by: String,
    /// Epoch millis when the agent registered (most-recent wins).
    #[serde(default)]
    pub started_ms: u64,
    #[serde(skip)]
    pub priority: i32,
    #[serde(skip)]
    pub is_user_project: bool,
}

impl DiscoveredAgent {
    pub fn display_name(&self) -> String {
        if !self.name.is_empty() {
            self.name.clone()
        } else if !self.cmd.is_empty() {
            short_cmd(&self.cmd)
        } else {
            format!("pid-{}", self.pid)
        }
    }

    pub fn socket_addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// `[PID] Project Name (Port: 192XX)`
    pub fn picker_label(&self) -> String {
        format!(
            "[{}] {} (Port: {})",
            self.pid,
            self.display_name(),
            self.port
        )
    }
}

fn short_cmd(cmd: &str) -> String {
    let first = cmd.split_whitespace().next().unwrap_or(cmd);
    let name = first.rsplit(['/', '\\']).next().unwrap_or(first);
    if name.chars().count() > 40 {
        format!("{}…", name.chars().take(39).collect::<String>())
    } else {
        name.to_string()
    }
}

fn folder_name(workspace: Option<&Path>) -> Option<String> {
    workspace.and_then(|p| {
        p.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
    })
}

fn launched_by_bonus(launched_by: &str) -> i32 {
    if launched_by.eq_ignore_ascii_case("javar-run") {
        400
    } else {
        0
    }
}

/// Boost agents whose registered `cwd` matches the dashboard workspace.
fn cwd_bonus(cwd: &str, workspace: Option<&Path>) -> i32 {
    let Some(ws) = workspace else {
        return 0;
    };
    if cwd.is_empty() {
        return 0;
    }
    let ws_s = ws
        .canonicalize()
        .unwrap_or_else(|_| ws.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
        .replace('\\', "/");
    let cwd_s = cwd.to_lowercase().replace('\\', "/");
    if cwd_s == ws_s || cwd_s.ends_with(&ws_s) || ws_s.ends_with(&cwd_s) {
        return 220;
    }
    if let Some(folder) = folder_name(workspace) {
        let f = folder.to_lowercase();
        if !f.is_empty() && cwd_s.contains(&f) {
            return 120;
        }
    }
    0
}

/// Pick the best live agent for auto-reconnect (javar-run / priority / newest).
pub fn best_live_agent(agents: &[DiscoveredAgent]) -> Option<usize> {
    agents
        .iter()
        .enumerate()
        .filter(|(_, a)| a.is_user_project || a.priority >= 40)
        .max_by(|(_, a), (_, b)| {
            a.priority
                .cmp(&b.priority)
                .then(a.started_ms.cmp(&b.started_ms))
                .then(a.port.cmp(&b.port))
        })
        .map(|(i, _)| i)
}

/// Score for auto-selection. High = user app; low = IDE / Maven tooling.
pub fn priority_score(name: &str, cmd: &str, workspace: Option<&Path>) -> i32 {
    let cmd_l = cmd.to_lowercase();
    let name_l = name.to_lowercase();
    let mut score = 50;

    // Push IDE / language-server noise firmly to the bottom.
    if is_ide_noise(&name_l, &cmd_l) {
        score -= 200;
    }

    let low_markers = [
        "maven",
        "surefire",
        "intellij",
        "jdt.ls",
        "languageserver",
        "ls.delegate",
        "plexus",
        "classworlds.launcher",
    ];
    for m in low_markers {
        if cmd_l.contains(m) || name_l.contains(m) {
            score -= 80;
            break;
        }
    }

    // Real app main classes beat Maven "Launcher" parents that mention spring-boot:run.
    if name_l.ends_with("application") || cmd_l.contains("application") {
        score += 200;
    }
    if name_l.contains("spring") || cmd_l.contains("springframework") {
        score += 120;
    }
    // Do NOT award +120 merely for "spring-boot:run" on the Maven parent.
    if name_l.contains("demo") || cmd_l.contains("demo") {
        score += 80;
    }
    if name_l == "launcher" || cmd_l.contains("spring-boot:run") {
        score -= 250;
    }
    if let Some(folder) = folder_name(workspace) {
        let f = folder.to_lowercase();
        if !f.is_empty() && (name_l.contains(&f) || cmd_l.contains(&f)) {
            score += 150;
        }
    }

    if !name.is_empty()
        && name != "java-app"
        && name != "unknown"
        && !is_ide_noise(&name_l, &cmd_l)
    {
        score += 40;
    }
    if cmd_l.contains(".jar") && !cmd_l.contains("maven") {
        score += 20;
    }

    score
}

/// eclipse / equinox / JDT / xml language servers — never shown as "your app".
pub fn is_ide_noise(name_l: &str, cmd_l: &str) -> bool {
    let markers = [
        "bloop",
        "bloopserver",
        "scala.cli",
        "eclipse",
        "equinox",
        "equinox.launcher",
        "xmlserver",
        "xmlserverlauncher",
        "lemminx",
        "org.eclipse",
        "redhat.java",
        "jdt.ls",
        "jdt_ws",
        "languageserver",
        "language-server",
        "language server",
        "kotlin-language-server",
        "gradle-language-server",
        "spring-boot-language-server",
        "metals",
        "scala.meta",
        "plexus",
        "classworlds.launcher",
        "intellij",
        "idea64",
        "fsnotifier",
    ];
    if markers
        .iter()
        .any(|m| name_l.contains(m) || cmd_l.contains(m))
    {
        return true;
    }
    if name_l == "launcher" {
        return true;
    }
    (name_l.contains("xml") || cmd_l.contains("xml"))
        && (name_l.contains("server")
            || cmd_l.contains("server")
            || name_l.contains("launcher")
            || cmd_l.contains("launcher"))
}

pub fn is_user_project(name: &str, cmd: &str) -> bool {
    let name_l = name.to_lowercase();
    let cmd_l = cmd.to_lowercase();
    if is_ide_noise(&name_l, &cmd_l) {
        return false;
    }
    priority_score(name, cmd, None) >= 50
}

pub fn javar_agents_dir() -> PathBuf {
    if let Ok(p) = std::env::var("JAVAR_HOME") {
        return PathBuf::from(p).join("agents");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".javar")
        .join("agents")
}

fn last_selection_path(workspace: Option<&Path>) -> PathBuf {
    let base = if let Ok(p) = std::env::var("JAVAR_HOME") {
        PathBuf::from(p)
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".javar")
    };
    if let Some(ws) = workspace {
        let key = ws
            .canonicalize()
            .unwrap_or_else(|_| ws.to_path_buf())
            .to_string_lossy()
            .replace(['\\', '/', ':'], "_");
        base.join("dashboard").join(format!("{key}.last"))
    } else {
        base.join("dashboard").join("last_agent")
    }
}

pub fn remember_selection(workspace: Option<&Path>, agent: &DiscoveredAgent) {
    let path = last_selection_path(workspace);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let body = format!("{}\n{}\n{}", agent.name, agent.pid, agent.port);
    let _ = fs::write(path, body);
}

pub fn load_remembered(workspace: Option<&Path>) -> Option<(String, u32, u16)> {
    let path = last_selection_path(workspace);
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let name = lines.next()?.to_string();
    let pid: u32 = lines.next()?.parse().ok()?;
    let port: u16 = lines.next()?.parse().ok()?;
    Some((name, pid, port))
}

fn pid_alive(sys: &System, pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    sys.process(Pid::from_u32(pid)).is_some()
}

fn port_alive(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
}

/// Agent is usable if its TCP port answers, even when sysinfo briefly misses the PID.
fn agent_reachable(sys: &System, agent: &DiscoveredAgent) -> bool {
    if agent.port < PORT_RANGE_START || agent.port > PORT_RANGE_END {
        return false;
    }
    if port_alive(agent.port) {
        return true;
    }
    // Port closed → only keep if PID still looks alive (race during restart).
    pid_alive(sys, agent.pid)
}

/// Remove dead PIDs and IDE language-server registry junk.
pub fn cleanup_stale_registry(sys: &mut System) -> usize {
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let dir = javar_agents_dir();
    let mut removed = 0usize;
    if !dir.is_dir() {
        return 0;
    }
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(agent) = serde_json::from_str::<DiscoveredAgent>(&text) else {
                let _ = fs::remove_file(&path);
                removed += 1;
                continue;
            };
            let name_l = agent.name.to_lowercase();
            let cmd_l = agent.cmd.to_lowercase();
            // Drop IDE internals from disk so they never pollute the picker.
            if is_ide_noise(&name_l, &cmd_l) {
                let _ = fs::remove_file(&path);
                removed += 1;
                continue;
            }
            if !agent_reachable(sys, &agent) {
                let _ = fs::remove_file(&path);
                removed += 1;
            }
        }
    }
    removed
}

pub fn spawn_stale_cleaner(stop: Arc<AtomicBool>) {
    thread::Builder::new()
        .name("javar-agent-janitor".into())
        .spawn(move || {
            let mut sys = System::new();
            while !stop.load(Ordering::Relaxed) {
                let _ = cleanup_stale_registry(&mut sys);
                for _ in 0..20 {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            }
        })
        .ok();
}

/// Discover monitorable user apps (Spring Boot / projects from any IDE).
/// IDE language servers are excluded. Duplicate ports are collapsed.
pub fn discover_agents(sys: &mut System, workspace: Option<&Path>) -> Vec<DiscoveredAgent> {
    let _ = cleanup_stale_registry(sys);
    let mut by_port: std::collections::HashMap<u16, DiscoveredAgent> =
        std::collections::HashMap::new();

    let dir = javar_agents_dir();
    if dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(mut agent) = serde_json::from_str::<DiscoveredAgent>(&text) else {
                    let _ = fs::remove_file(&path);
                    continue;
                };
                let name_l = agent.name.to_lowercase();
                let cmd_l = agent.cmd.to_lowercase();
                if is_ide_noise(&name_l, &cmd_l) {
                    let _ = fs::remove_file(&path);
                    continue;
                }
                if !agent_reachable(sys, &agent) {
                    let _ = fs::remove_file(&path);
                    continue;
                }
                // Prefer live telemetry identity for this port.
                let addr = agent.socket_addr();
                if let Some((name, pid, tel)) = super::agent::identify(&addr) {
                    agent.name = name;
                    if pid > 0 {
                        agent.pid = pid as u32;
                    }
                    if !tel.jvm_cmd.is_empty() {
                        agent.cmd = tel.jvm_cmd;
                    }
                }
                agent.priority = priority_score(&agent.name, &agent.cmd, workspace)
                    + cwd_bonus(&agent.cwd, workspace)
                    + launched_by_bonus(&agent.launched_by);
                agent.is_user_project = is_user_project(&agent.name, &agent.cmd)
                    || agent.launched_by.eq_ignore_ascii_case("javar-run");
                if is_ide_noise(&agent.name.to_lowercase(), &agent.cmd.to_lowercase()) {
                    let _ = fs::remove_file(&path);
                    continue;
                }
                by_port.insert(agent.port, agent);
            }
        }
    }

    // Probe every JavaR port — source of truth for what's actually listening.
    for port in PORT_RANGE_START..=PORT_RANGE_END {
        if !port_alive(port) {
            continue;
        }
        let addr = format!("127.0.0.1:{port}");
        let Some((name, pid, tel)) = super::agent::identify(&addr) else {
            continue;
        };
        let cmd = tel.jvm_cmd;
        if is_ide_noise(&name.to_lowercase(), &cmd.to_lowercase()) {
            continue;
        }
        let mut agent = DiscoveredAgent {
            pid: if pid > 0 { pid as u32 } else { 0 },
            port,
            name,
            cmd,
            cwd: String::new(),
            launched_by: String::new(),
            started_ms: 0,
            priority: 0,
            is_user_project: false,
        };
        // Keep registry identity if we already saw this port.
        if let Some(prev) = by_port.get(&port) {
            agent.cwd = prev.cwd.clone();
            agent.launched_by = prev.launched_by.clone();
            agent.started_ms = prev.started_ms;
        }
        agent.priority = priority_score(&agent.name, &agent.cmd, workspace)
            + cwd_bonus(&agent.cwd, workspace)
            + launched_by_bonus(&agent.launched_by);
        agent.is_user_project = is_user_project(&agent.name, &agent.cmd)
            || agent.launched_by.eq_ignore_ascii_case("javar-run");
        by_port.insert(port, agent);
    }

    // Sysinfo: Spring Boot / user JVMs with -javaagent that already own a free port.
    for (_pid, proc) in sys.processes() {
        let cmd_joined: String = proc
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let cmd_l = cmd_joined.to_lowercase();
        let looks_user = cmd_l.contains("spring")
            || cmd_l.contains("boot")
            || cmd_l.contains("-jar")
            || workspace
                .and_then(|p| folder_name(Some(p)))
                .map(|f| cmd_l.contains(&f.to_lowercase()))
                .unwrap_or(false);
        let has_agent = cmd_l.contains("javar-agent")
            || (cmd_l.contains("javaagent") && cmd_l.contains("javar"));
        if !looks_user && !has_agent {
            continue;
        }
        if is_ide_noise("", &cmd_l) {
            continue;
        }
        // Find which port this PID actually owns (from by_port or probe).
        let pid = proc.pid().as_u32();
        if by_port.values().any(|a| a.pid == pid) {
            continue;
        }
        if let Some(port) = extract_agent_port(&cmd_joined) {
            if by_port.contains_key(&port) {
                continue;
            }
            if !port_alive(port) {
                continue;
            }
            let addr = format!("127.0.0.1:{port}");
            if let Some((name, tpid, tel)) = super::agent::identify(&addr) {
                if is_ide_noise(&name.to_lowercase(), &tel.jvm_cmd.to_lowercase()) {
                    continue;
                }
                let mut agent = DiscoveredAgent {
                    pid: if tpid > 0 { tpid as u32 } else { pid },
                    port,
                    name,
                    cmd: if tel.jvm_cmd.is_empty() {
                        cmd_joined.chars().take(400).collect()
                    } else {
                        tel.jvm_cmd
                    },
                    cwd: String::new(),
                    launched_by: String::new(),
                    started_ms: 0,
                    priority: 0,
                    is_user_project: false,
                };
                agent.priority = priority_score(&agent.name, &agent.cmd, workspace)
                    + launched_by_bonus(&agent.launched_by);
                agent.is_user_project = is_user_project(&agent.name, &agent.cmd);
                by_port.insert(port, agent);
            }
        }
    }

    let mut found: Vec<_> = by_port.into_values().collect();
    found.sort_by(|a, b| {
        // User / Spring first; IDE never present.
        b.is_user_project
            .cmp(&a.is_user_project)
            .then(b.priority.cmp(&a.priority))
            .then(a.name.cmp(&b.name))
            .then(a.pid.cmp(&b.pid))
    });
    found
}

fn extract_agent_port(cmd: &str) -> Option<u16> {
    // Match …javar-agent.jar=port=19223… or port=19223 in agent args
    for part in cmd.split(|c: char| c.is_whitespace() || c == ',' || c == '=') {
        if let Ok(p) = part.parse::<u16>() {
            if (PORT_RANGE_START..=PORT_RANGE_END).contains(&p) {
                return Some(p);
            }
        }
    }
    // Explicit port=NNNN
    if let Some(idx) = cmd.to_lowercase().find("port=") {
        let rest = &cmd[idx + 5..];
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(p) = digits.parse::<u16>() {
            if (PORT_RANGE_START..=PORT_RANGE_END).contains(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// Startup / reconnect policy for the explicit `javar run` model.
/// Auto-connect to the most recently started `javar-run` agent; `p` still switches.
pub fn auto_select(
    agents: &[DiscoveredAgent],
    workspace: Option<&Path>,
) -> (Option<usize>, bool) {
    if agents.is_empty() {
        return (None, false);
    }

    // Prefer active run-session.json written by `javar run`.
    if let Some(port) = load_run_session_port() {
        if let Some(i) = agents.iter().position(|a| a.port == port) {
            return (Some(i), false);
        }
    }

    // Most recently started process launched via `javar run`.
    let mut run_hits: Vec<(usize, u64)> = agents
        .iter()
        .enumerate()
        .filter(|(_, a)| a.launched_by.eq_ignore_ascii_case("javar-run"))
        .map(|(i, a)| (i, a.started_ms))
        .collect();
    if !run_hits.is_empty() {
        run_hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let best = run_hits[0].0;
        // Multiple microservices → preselect newest, but open picker.
        let need_picker = run_hits.len() > 1;
        return (Some(best), need_picker);
    }

    let folder = folder_name(workspace).unwrap_or_default().to_lowercase();

    // Strong match: workspace folder name appears in process name/cmd.
    if !folder.is_empty() {
        let folder_hits: Vec<usize> = agents
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                let n = a.name.to_lowercase();
                let c = a.cmd.to_lowercase();
                if is_ide_noise(&n, &c) {
                    return None;
                }
                if n.contains(&folder) || c.contains(&folder) || cwd_bonus(&a.cwd, workspace) > 0 {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
        if folder_hits.len() == 1 {
            return (Some(folder_hits[0]), false);
        }
        if folder_hits.len() > 1 {
            return (Some(folder_hits[0]), true);
        }
    }

    if agents.len() > 1 {
        let preselect = best_live_agent(agents).unwrap_or(0);
        return (Some(preselect), true);
    }

    // Exactly one agent — connect directly (still prefer remembered if it matches).
    if let Some((name, pid, port)) = load_remembered(workspace) {
        if let Some(i) = agents.iter().position(|a| {
            (a.pid == pid && a.pid != 0)
                || (a.port == port && (!name.is_empty() && a.name == name))
                || (!name.is_empty() && a.name.eq_ignore_ascii_case(&name))
        }) {
            return (Some(i), false);
        }
    }
    (Some(0), false)
}

fn load_run_session_port() -> Option<u16> {
    let home = crate::embed::javar_home();
    let path = home.join("run-session.json");
    let text = fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let port = v.get("port")?.as_u64()? as u16;
    if (PORT_RANGE_START..=PORT_RANGE_END).contains(&port) {
        Some(port)
    } else {
        None
    }
}

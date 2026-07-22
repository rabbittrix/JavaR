//! Dashboard application loop (crossterm + ratatui).

use super::agent::{self, AgentSnapshot};
use super::discover::{self, JavarProcess};
use super::java_proc::{self, JavaProcSnapshot};
use super::ui;
use anyhow::Result;
use chrono::{DateTime, Local, TimeZone};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;
use ratatui::Terminal;
use std::collections::VecDeque;
use std::io::{stdout, Stdout};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::System;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Performance,
    HotReload,
    GcMetrics,
    Logs,
}

impl Tab {
    pub const ALL: [Tab; 4] = [
        Tab::Performance,
        Tab::HotReload,
        Tab::GcMetrics,
        Tab::Logs,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Performance => "Performance",
            Tab::HotReload => "Hot-Reload",
            Tab::GcMetrics => "GC Metrics",
            Tab::Logs => "Logs",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Tab::Performance => Tab::HotReload,
            Tab::HotReload => Tab::GcMetrics,
            Tab::GcMetrics => Tab::Logs,
            Tab::Logs => Tab::Performance,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Tab::Performance => Tab::Logs,
            Tab::HotReload => Tab::Performance,
            Tab::GcMetrics => Tab::HotReload,
            Tab::Logs => Tab::GcMetrics,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub at: DateTime<Local>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ReloadEvent {
    pub timestamp: String,
    pub class_name: String,
    pub change_type: String,
    pub version: String,
}

/// Dashboard runtime state — process picker + live telemetry.
pub struct App {
    pub tab: Tab,
    pub agent_addr: String,
    pub agent: AgentSnapshot,
    pub java: JavaProcSnapshot,
    pub heap_history: VecDeque<u64>,
    pub managed_history: VecDeque<u64>,
    pub history: Vec<ReloadEvent>,
    pub logs: VecDeque<LogLine>,
    pub last_reload_count: u64,
    pub ticks: u64,
    pub should_quit: bool,
    pub history_cap: usize,
    /// Processes from `~/.javar/agents/*.json`.
    pub available_processes: Vec<JavarProcess>,
    pub agent_index: usize,
    pub show_picker: bool,
    pub picker_state: ListState,
    pub workspace: PathBuf,
    /// Consecutive failed telemetry polls (heartbeat miss).
    missed_heartbeats: u32,
}

impl App {
    pub fn new(fallback_addr: String, workspace: PathBuf, sys: &mut System) -> Self {
        // Cleanup dead PIDs, then load live agents (registry + port probe).
        let available_processes = discover::discover_agents(sys, Some(workspace.as_path()));
        let (idx, mut need_picker) =
            discover::auto_select(&available_processes, Some(workspace.as_path()));
        let agent_index = idx.unwrap_or(0);

        // If we're about to land on an IDE process while a user app exists, force picker.
        if !need_picker {
            if let Some(a) = available_processes.get(agent_index) {
                let n = a.name.to_lowercase();
                let c = a.cmd.to_lowercase();
                if discover::is_ide_noise(&n, &c)
                    && available_processes.iter().any(|x| x.is_user_project)
                {
                    need_picker = true;
                }
            }
        }
        // Do NOT force the picker just because multiple agents exist — folder auto-select wins.

        // Never fall back to a default :19222 — that port is often Bloop/IDE noise
        // and shows a fake ACTIVE dashboard with "(awaiting reload)".
        let _ = fallback_addr;
        let agent_addr = if need_picker {
            String::new()
        } else {
            available_processes
                .get(agent_index)
                .filter(|a| a.is_user_project || !discover::is_ide_noise(
                    &a.name.to_lowercase(),
                    &a.cmd.to_lowercase(),
                ))
                .map(|a| a.socket_addr())
                .unwrap_or_default()
        };

        let mut picker_state = ListState::default();
        if !available_processes.is_empty() {
            picker_state.select(Some(agent_index.min(available_processes.len() - 1)));
        }

        Self {
            tab: Tab::Performance,
            agent_addr,
            agent: AgentSnapshot::default(),
            java: JavaProcSnapshot::default(),
            heap_history: VecDeque::with_capacity(64),
            managed_history: VecDeque::with_capacity(64),
            history: Vec::new(),
            logs: VecDeque::with_capacity(200),
            last_reload_count: 0,
            ticks: 0,
            should_quit: false,
            history_cap: 60,
            available_processes,
            agent_index,
            show_picker: need_picker,
            picker_state,
            workspace,
            missed_heartbeats: 0,
        }
    }

    pub fn monitored_label(&self) -> String {
        let name = if !self.agent.telemetry.project_name.is_empty() {
            self.agent.telemetry.project_name.clone()
        } else if let Some(a) = self.available_processes.get(self.agent_index) {
            a.display_name()
        } else {
            "unknown".into()
        };
        let pid = if self.agent.telemetry.pid > 0 {
            self.agent.telemetry.pid
        } else if let Some(a) = self.available_processes.get(self.agent_index) {
            a.pid as u64
        } else {
            0
        };
        if pid > 0 {
            format!("Monitoring: {name} (PID: {pid})")
        } else {
            format!("Monitoring: {name}")
        }
    }

    pub fn others_hint(&self) -> String {
        let n = self.available_processes.len().saturating_sub(1);
        if n == 0 {
            String::new()
        } else if n == 1 {
            "Press p to switch (1 other app found)".into()
        } else {
            format!("Press p to switch ({n} other apps found)")
        }
    }

    pub fn picker_cursor(&self) -> usize {
        self.picker_state.selected().unwrap_or(0)
    }

    fn push_log(&mut self, text: impl Into<String>) {
        if self.logs.len() >= 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(LogLine {
            at: Local::now(),
            text: text.into(),
        });
    }

    /// Drop charts / history so the next poll paints a clean slate.
    fn clear_telemetry_ui(&mut self) {
        self.heap_history.clear();
        self.managed_history.clear();
        self.history.clear();
        self.logs.clear();
        self.last_reload_count = 0;
        self.missed_heartbeats = 0;
        self.agent = AgentSnapshot::default();
    }

    /// Start / retarget `javar-core` so .java saves redefine THIS process.
    fn ensure_reload_watcher(&mut self) {
        if self.agent_addr.is_empty() {
            return;
        }
        let project = self
            .available_processes
            .get(self.agent_index)
            .and_then(|a| {
                if !a.cwd.is_empty() {
                    Some(PathBuf::from(&a.cwd))
                } else {
                    None
                }
            })
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| self.workspace.clone());

        if let Some(msg) = super::watcher_svc::ensure_pinned_watcher(&project, &self.agent_addr) {
            if msg.starts_with("started ") {
                self.push_log(msg);
            }
        } else if self.ticks == 1 {
            self.push_log(
                "WARNING: could not start javar-core watcher — hot-reload disabled until `javar run`",
            );
        }
    }

    /// When the current agent stops heartbeating, jump to the best live registry entry
    /// for this workspace (handles port migration 19222 → 19223+).
    fn try_auto_reconnect(&mut self, sys: &mut System) {
        self.scan_processes(sys);
        let Some(idx) = discover::best_live_agent(&self.available_processes) else {
            return;
        };
        let Some(selected) = self.available_processes.get(idx).cloned() else {
            return;
        };
        let new_addr = selected.socket_addr();
        if new_addr == self.agent_addr && self.missed_heartbeats < 6 {
            // Same addr still dead — keep waiting a bit longer.
            return;
        }
        if new_addr == self.agent_addr {
            return;
        }
        self.agent_index = idx;
        self.agent_addr = new_addr.clone();
        self.picker_state.select(Some(idx));
        self.missed_heartbeats = 0;
        discover::remember_selection(Some(self.workspace.as_path()), &selected);
        self.agent = agent::poll(&self.agent_addr);
        self.push_log(format!(
            "auto-reconnect → {} @ {} (PID {})",
            selected.display_name(),
            new_addr,
            selected.pid
        ));
    }

    /// Re-scan `~/.javar/agents/*.json`.
    fn scan_processes(&mut self, sys: &mut System) {
        self.available_processes = discover::discover_agents(sys, Some(self.workspace.as_path()));
        if self.available_processes.is_empty() {
            self.picker_state.select(None);
            return;
        }
        let prev = self.agent_addr.clone();
        if let Some(i) = self
            .available_processes
            .iter()
            .position(|a| a.socket_addr() == prev)
        {
            self.agent_index = i;
        }
        let sel = self
            .picker_state
            .selected()
            .unwrap_or(self.agent_index)
            .min(self.available_processes.len() - 1);
        self.picker_state.select(Some(sel));
    }

    /// `p` key — always open the centered process picker (even with 0/1 agents).
    pub fn open_picker(&mut self, sys: &mut System) {
        self.scan_processes(sys);
        self.show_picker = true;
        if self.available_processes.is_empty() {
            self.picker_state.select(None);
            self.push_log(format!(
                "process picker — no live agents in {} (ports {}–{})",
                discover::javar_agents_dir().display(),
                discover::PORT_RANGE_START,
                discover::PORT_RANGE_END
            ));
            return;
        }
        let idx = self
            .agent_index
            .min(self.available_processes.len().saturating_sub(1));
        self.picker_state.select(Some(idx));
        self.push_log(format!(
            "process picker — {} process(es)  ·  ↑/↓ Enter",
            self.available_processes.len()
        ));
    }

    fn picker_up(&mut self) {
        if self.available_processes.is_empty() {
            return;
        }
        let i = self.picker_cursor();
        let next = if i == 0 {
            self.available_processes.len() - 1
        } else {
            i - 1
        };
        self.picker_state.select(Some(next));
    }

    fn picker_down(&mut self) {
        if self.available_processes.is_empty() {
            return;
        }
        let i = self.picker_cursor();
        let next = (i + 1) % self.available_processes.len();
        self.picker_state.select(Some(next));
    }

    /// Enter — disconnect current target, reconnect to selected port, clear UI.
    pub fn confirm_picker(&mut self) {
        let idx = self.picker_cursor();
        if idx >= self.available_processes.len() {
            self.show_picker = false;
            return;
        }
        let selected = self.available_processes[idx].clone();
        let new_addr = selected.socket_addr();

        // Explicit disconnect of previous target (stateless poll; clear local session).
        self.clear_telemetry_ui();
        self.agent_index = idx;
        self.agent_addr = new_addr.clone();
        self.show_picker = false;

        discover::remember_selection(Some(self.workspace.as_path()), &selected);

        // Immediate reconnect probe so UI shows ACTIVE/OFFLINE without waiting a tick.
        self.agent = agent::poll(&self.agent_addr);
        self.ensure_reload_watcher();
        self.push_log(format!(
            "switched → {} @ {} (PID {})",
            selected.display_name(),
            new_addr,
            selected.pid
        ));
        if self.agent.connected && !self.agent.telemetry.reload_history.is_empty() {
            self.push_log(format!(
                "synced {} buffered reload event(s)",
                self.agent.telemetry.reload_history.len()
            ));
            // Apply history immediately after switch.
            self.history = self
                .agent
                .telemetry
                .reload_history
                .iter()
                .map(|e| ReloadEvent {
                    timestamp: format_event_ts(&e.ts),
                    class_name: e.class_name.clone(),
                    change_type: if e.change_type.is_empty() {
                        "Body".into()
                    } else {
                        e.change_type.clone()
                    },
                    version: if e.version > 0 {
                        format!("v{}", e.version)
                    } else {
                        "—".into()
                    },
                })
                .collect();
            self.last_reload_count = self.agent.telemetry.reload_count;
        }
    }

    fn refresh(&mut self, sys: &mut System) {
        self.ticks += 1;
        self.java = java_proc::sample(sys);

        // Keep process list fresh while the modal is open (and periodically otherwise).
        if self.show_picker || self.ticks == 1 || self.ticks % 8 == 0 {
            let prev_sel = self.picker_cursor();
            self.scan_processes(sys);
            if self.show_picker && !self.available_processes.is_empty() {
                self.picker_state
                    .select(Some(prev_sel.min(self.available_processes.len() - 1)));
            }
            // If a second agent appears while running, force the picker open.
            if !self.show_picker && self.available_processes.len() > 1 && self.agent_addr.is_empty()
            {
                self.show_picker = true;
            }
        }

        // Do not poll until the user has chosen a process (startup multi-agent case).
        if self.show_picker && self.agent_addr.is_empty() {
            return;
        }
        // App started after the dashboard (mvn spring-boot:run / Run Java) — attach.
        if self.agent_addr.is_empty() {
            if let Some(idx) = discover::best_live_agent(&self.available_processes) {
                if let Some(selected) = self.available_processes.get(idx).cloned() {
                    self.agent_index = idx;
                    self.agent_addr = selected.socket_addr();
                    self.picker_state.select(Some(idx));
                    discover::remember_selection(Some(self.workspace.as_path()), &selected);
                    self.push_log(format!(
                        "auto-connected to {} (PID {}) @ {}",
                        selected.display_name(),
                        selected.pid,
                        self.agent_addr
                    ));
                    self.ensure_reload_watcher();
                }
            } else {
                return;
            }
        }

        let prev = self.agent.connected;
        self.agent = agent::poll(&self.agent_addr);

        // Hot-reload requires javar-core. Auto-start a watcher pinned to this agent.
        if self.agent.connected && self.ticks % 5 == 1 {
            self.ensure_reload_watcher();
        }

        if self.agent.connected {
            self.missed_heartbeats = 0;
            if let Some(a) = self.available_processes.get_mut(self.agent_index) {
                if a.name.is_empty() && !self.agent.telemetry.project_name.is_empty() {
                    a.name = self.agent.telemetry.project_name.clone();
                }
                if a.pid == 0 && self.agent.telemetry.pid > 0 {
                    a.pid = self.agent.telemetry.pid as u32;
                }
                if a.cmd.is_empty() && !self.agent.telemetry.jvm_cmd.is_empty() {
                    a.cmd = self.agent.telemetry.jvm_cmd.clone();
                    a.priority =
                        discover::priority_score(&a.name, &a.cmd, Some(self.workspace.as_path()));
                    a.is_user_project = discover::is_user_project(&a.name, &a.cmd);
                }
            }
        } else if !self.show_picker {
            // Heartbeat lost → auto-reconnect to latest project-matched registry agent.
            self.missed_heartbeats = self.missed_heartbeats.saturating_add(1);
            if self.missed_heartbeats >= 2 {
                self.try_auto_reconnect(sys);
            }
        }

        if self.agent.connected && !prev {
            self.push_log(format!("connected to agent {}", self.agent_addr));
        } else if !self.agent.connected && prev {
            self.push_log("agent connection lost — scanning ~/.javar/agents for live port…");
        }

        let t = &self.agent.telemetry;
        push_hist(&mut self.heap_history, t.java_heap_used, self.history_cap);
        push_hist(&mut self.managed_history, t.javar_managed, self.history_cap);

        if !t.reload_history.is_empty() {
            self.history = t
                .reload_history
                .iter()
                .map(|e| ReloadEvent {
                    timestamp: format_event_ts(&e.ts),
                    class_name: e.class_name.clone(),
                    change_type: if e.change_type.is_empty() {
                        "Body".into()
                    } else {
                        e.change_type.clone()
                    },
                    version: if e.version > 0 {
                        format!("v{}", e.version)
                    } else {
                        "—".into()
                    },
                })
                .collect();
            self.last_reload_count = t.reload_count;
        } else {
            let reload_count = t.reload_count;
            if reload_count > self.last_reload_count {
                let delta = reload_count - self.last_reload_count;
                self.push_log(format!(
                    "bytecode injection / redefine ×{delta} (total {reload_count})"
                ));
                self.history.insert(
                    0,
                    ReloadEvent {
                        timestamp: format_timestamp(
                            Local::now().timestamp_millis().max(0) as u64,
                        ),
                        class_name: "reload-batch".into(),
                        change_type: "Body".into(),
                        version: format!("v{reload_count}"),
                    },
                );
                if self.history.len() > 40 {
                    self.history.truncate(40);
                }
                self.last_reload_count = reload_count;
            } else if self.last_reload_count == 0 && reload_count > 0 {
                self.last_reload_count = reload_count;
            }
        }
    }
}

/// Format epoch milliseconds as local `HH:MM:SS`.
pub fn format_timestamp(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) as u32).saturating_mul(1_000_000);
    match Local.timestamp_opt(secs, nanos) {
        chrono::LocalResult::Single(dt) => dt.format("%H:%M:%S").to_string(),
        _ => DateTime::from_timestamp(secs, nanos)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| ms.to_string()),
    }
}

fn format_event_ts(ts: &str) -> String {
    let trimmed = ts.trim();
    if let Ok(ms) = trimmed.parse::<u64>() {
        // Heuristic: 13-digit ≈ millis, 10-digit ≈ seconds.
        if ms >= 1_000_000_000_000 {
            return format_timestamp(ms);
        }
        if ms >= 1_000_000_000 {
            return format_timestamp(ms.saturating_mul(1000));
        }
    }
    // Legacy ISO-8601 from Instant.toString()
    if let Some(t) = trimmed.split('T').nth(1) {
        return t.chars().take(8).collect();
    }
    trimmed.chars().take(8).collect()
}

fn push_hist(q: &mut VecDeque<u64>, v: u64, cap: usize) {
    if q.len() >= cap {
        q.pop_front();
    }
    q.push_back(v);
}

fn is_press(kind: KeyEventKind) -> bool {
    // Some Windows terminals emit Release-only or omit kind — accept all non-release.
    !matches!(kind, KeyEventKind::Release)
}

fn handle_key(app: &mut App, sys: &mut System, key: crossterm::event::KeyEvent, last: &mut Instant) {
    let code = key.code;
    if app.show_picker {
        match code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Esc => {
                // On first launch with no connection yet, Esc quits; otherwise just close.
                if app.agent_addr.is_empty() {
                    app.should_quit = true;
                } else {
                    app.show_picker = false;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => app.picker_up(),
            KeyCode::Down | KeyCode::Char('j') => app.picker_down(),
            KeyCode::Enter | KeyCode::Char(' ') => {
                app.confirm_picker();
                *last = Instant::now();
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                app.scan_processes(sys);
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                // Keep picker open (user asked for visible selector); rescans list.
                app.scan_processes(sys);
            }
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('p') | KeyCode::Char('P') => {
            app.open_picker(sys);
        }
        KeyCode::Right | KeyCode::Char('l') => app.tab = app.tab.next(),
        KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => {
            app.tab = app.tab.prev();
        }
        KeyCode::Char('1') => app.tab = Tab::Performance,
        KeyCode::Char('2') => app.tab = Tab::HotReload,
        KeyCode::Char('3') => app.tab = Tab::GcMetrics,
        KeyCode::Char('4') => app.tab = Tab::Logs,
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.push_log("manual refresh");
            app.scan_processes(sys);
            app.refresh(sys);
            *last = Instant::now();
        }
        _ => {}
    }
}

pub fn run_dashboard(agent_addr: String) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let stop_janitor = Arc::new(AtomicBool::new(false));
    discover::spawn_stale_cleaner(stop_janitor.clone());

    let mut sys = System::new_all();
    let mut app = App::new(agent_addr, workspace, &mut sys);
    app.push_log("JavaR Control Center — Press 'p' to change process | 'q' to quit");
    if app.show_picker {
        app.push_log(format!(
            "SELECT PROCESS: {} agent(s) found — ↑/↓ then Enter",
            app.available_processes.len()
        ));
    } else if let Some(a) = app.available_processes.get(app.agent_index) {
        app.push_log(format!(
            "auto-connected to {} (PID {}) @ {}",
            a.display_name(),
            a.pid,
            a.socket_addr()
        ));
    } else {
        app.push_log(format!(
            "waiting for agents in {}",
            discover::javar_agents_dir().display()
        ));
    }

    // Hot-reload needs javar-core — start immediately for the connected agent.
    app.ensure_reload_watcher();

    // Fast poll so Reload history / CRITICAL bar land well under 500ms after Save.
    let tick = Duration::from_millis(200);
    let mut last = Instant::now();
    app.refresh(&mut sys);

    let result = loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        let timeout = tick.saturating_sub(last.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if is_press(key.kind) => {
                    handle_key(&mut app, &mut sys, key, &mut last);
                }
                // Drain paste / resize noise without breaking the loop.
                _ => {}
            }
        }

        if app.should_quit {
            break Ok(());
        }
        if last.elapsed() >= tick {
            app.refresh(&mut sys);
            last = Instant::now();
        }
    };

    stop_janitor.store(true, Ordering::Relaxed);
    super::watcher_svc::shutdown_watcher();
    restore_terminal(&mut terminal)?;
    result
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

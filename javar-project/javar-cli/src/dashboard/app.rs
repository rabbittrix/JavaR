//! Dashboard application loop (crossterm + ratatui).

use super::agent::{self, AgentSnapshot};
use super::java_proc::{self, JavaProcSnapshot};
use super::ui;
use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::collections::VecDeque;
use std::io::{stdout, Stdout};
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
pub struct ShadowEntry {
    pub class_name: String,
    pub version: String,
    pub note: String,
}

pub struct App {
    pub tab: Tab,
    pub agent_addr: String,
    pub agent: AgentSnapshot,
    pub java: JavaProcSnapshot,
    pub heap_history: VecDeque<u64>,
    pub managed_history: VecDeque<u64>,
    pub shadows: Vec<ShadowEntry>,
    pub logs: VecDeque<LogLine>,
    pub last_reload_count: u64,
    pub ticks: u64,
    pub should_quit: bool,
    pub history_cap: usize,
}

impl App {
    pub fn new(agent_addr: String) -> Self {
        Self {
            tab: Tab::Performance,
            agent_addr,
            agent: AgentSnapshot::default(),
            java: JavaProcSnapshot::default(),
            heap_history: VecDeque::with_capacity(64),
            managed_history: VecDeque::with_capacity(64),
            shadows: Vec::new(),
            logs: VecDeque::with_capacity(200),
            last_reload_count: 0,
            ticks: 0,
            should_quit: false,
            history_cap: 60,
        }
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

    fn refresh(&mut self, sys: &mut System) {
        self.ticks += 1;
        self.java = java_proc::sample(sys);
        let prev = self.agent.connected;
        self.agent = agent::poll(&self.agent_addr);

        if self.agent.connected && !prev {
            self.push_log(format!("connected to agent {}", self.agent_addr));
        } else if !self.agent.connected && prev {
            self.push_log("agent connection lost");
        }

        let t = &self.agent.telemetry;
        push_hist(&mut self.heap_history, t.java_heap_used, self.history_cap);
        push_hist(&mut self.managed_history, t.javar_managed, self.history_cap);

        let reload_count = t.reload_count;
        if reload_count > self.last_reload_count {
            let delta = reload_count - self.last_reload_count;
            self.push_log(format!(
                "bytecode injection / redefine ×{delta} (total {reload_count})"
            ));
            self.shadows.insert(
                0,
                ShadowEntry {
                    class_name: "reload-batch".into(),
                    version: format!("v{reload_count}"),
                    note: format!("+{delta} class update(s) via agent"),
                },
            );
            if self.shadows.len() > 40 {
                self.shadows.truncate(40);
            }
            self.last_reload_count = reload_count;
        } else if self.last_reload_count == 0 && reload_count > 0 {
            self.last_reload_count = reload_count;
       }

        // Seed shadow list from agent detail when empty but connected
        if self.shadows.is_empty() && self.agent.connected && self.ticks % 10 == 1 {
            self.shadows.push(ShadowEntry {
                class_name: "(awaiting structural reload)".into(),
                version: "—".into(),
                note: "Shadow classes appear when schema changes land".into(),
            });
        }

        if self.ticks % 15 == 0 {
            let java_n = self.java.processes.len();
            let with_agent = self
                .java
                .processes
                .iter()
                .filter(|p| p.has_javar_agent)
                .count();
            self.push_log(format!(
                "sysinfo: {java_n} JVM process(es), {with_agent} with -javaagent"
            ));
        }
    }
}

fn push_hist(q: &mut VecDeque<u64>, v: u64, cap: usize) {
    if q.len() >= cap {
        q.pop_front();
    }
    q.push_back(v);
}

pub fn run_dashboard(agent_addr: String) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(agent_addr);
    app.push_log("JavaR Control Center online — q quit · ←/→ tabs · r refresh");
    let mut sys = System::new_all();
    app.refresh(&mut sys);

    let tick = Duration::from_millis(750);
    let mut last = Instant::now();

    let result = loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        let timeout = tick.saturating_sub(last.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            app.should_quit = true;
                        }
                        KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => {
                            app.tab = app.tab.next();
                        }
                        KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => {
                            app.tab = app.tab.prev();
                        }
                        KeyCode::Char('1') => app.tab = Tab::Performance,
                        KeyCode::Char('2') => app.tab = Tab::HotReload,
                        KeyCode::Char('3') => app.tab = Tab::GcMetrics,
                        KeyCode::Char('4') => app.tab = Tab::Logs,
                        KeyCode::Char('r') => {
                            app.push_log("manual refresh");
                            app.refresh(&mut sys);
                            last = Instant::now();
                        }
                        _ => {}
                    }
                }
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

    restore_terminal(&mut terminal)?;
    result
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

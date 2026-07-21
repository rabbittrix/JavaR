//! Ratatui widgets for the JavaR Control Center.

use super::app::{App, Tab};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, Borders, Chart, Dataset, Gauge, List, ListItem, Paragraph, Row, Table, Tabs,
};
use ratatui::Frame;

const ORANGE: Color = Color::Rgb(224, 108, 117);
const CYAN: Color = Color::Rgb(0, 229, 255);
const DIM: Color = Color::Rgb(120, 124, 140);

pub fn draw(frame: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(frame.area());

    draw_header(frame, root[0], app);
    draw_tabs(frame, root[1], app);
    match app.tab {
        Tab::Performance => draw_performance(frame, root[2], app),
        Tab::HotReload => draw_hot_reload(frame, root[2], app),
        Tab::GcMetrics => draw_gc(frame, root[2], app),
        Tab::Logs => draw_logs(frame, root[2], app),
    }
    draw_footer(frame, root[3]);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let status = if app.agent.connected {
        Span::styled("● ACTIVE", Style::default().fg(CYAN).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("○ OFFLINE", Style::default().fg(ORANGE))
    };
    let project = if app.agent.telemetry.project_name.is_empty() {
        Span::raw("")
    } else {
        Span::styled(
            format!("「{}」 ", app.agent.telemetry.project_name),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        )
    };
    let title = Paragraph::new(Line::from(vec![
        Span::styled(" JavaR ", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
        Span::styled("Control Center ", Style::default().fg(Color::White)),
        project,
        status,
        Span::raw("  "),
        Span::styled(app.agent_addr.clone(), Style::default().fg(DIM)),
        Span::raw("  "),
        Span::styled(app.agent.detail.clone(), Style::default().fg(DIM)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ORANGE)),
    );
    frame.render_widget(title, area);
}

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = Tab::ALL.iter().map(|t| Line::from(t.title())).collect();
    let idx = Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(idx)
        .block(Block::default().borders(Borders::ALL).title("Tabs"))
        .highlight_style(
            Style::default()
                .fg(CYAN)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .divider("│");
    frame.render_widget(tabs, area);
}

fn draw_performance(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(8),
        ])
        .split(area);

    let heap = app.agent.telemetry.java_heap_used;
    let heap_max = app.agent.telemetry.java_heap_max.max(1);
    let managed = app.agent.telemetry.javar_managed;
    let ratio = (heap as f64 / heap_max as f64).clamp(0.0, 1.0);

    let heap_g = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("JVM Heap"))
        .gauge_style(Style::default().fg(ORANGE))
        .ratio(ratio)
        .label(format!("{} / {}", fmt_bytes(heap), fmt_bytes(heap_max)));
    frame.render_widget(heap_g, chunks[0]);

    let managed_g = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("JavaR Off-Heap (managed)"),
        )
        .gauge_style(Style::default().fg(CYAN))
        .ratio(((managed as f64) / (managed.max(heap_max) as f64)).clamp(0.05, 1.0))
        .label(fmt_bytes(managed));
    frame.render_widget(managed_g, chunks[1]);

    draw_chart(frame, chunks[2], app);
    draw_java_table(frame, chunks[3], app);
}

fn draw_chart(frame: &mut Frame, area: Rect, app: &App) {
    let heap: Vec<(f64, f64)> = app
        .heap_history
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64, *v as f64 / (1024.0 * 1024.0)))
        .collect();
    let managed: Vec<(f64, f64)> = app
        .managed_history
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64, *v as f64 / (1024.0 * 1024.0)))
        .collect();

    let max_y = heap
        .iter()
        .chain(managed.iter())
        .map(|(_, y)| *y)
        .fold(1.0_f64, f64::max)
        * 1.15;

    let datasets = vec![
        Dataset::default()
            .name("Heap MB")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(ORANGE))
            .data(&heap),
        Dataset::default()
            .name("Off-heap MB")
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(CYAN))
            .data(&managed),
    ];

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Live: JVM Heap vs JavaR Off-Heap"),
        )
        .x_axis(
            Axis::default()
                .title("t")
                .style(Style::default().fg(DIM))
                .bounds([0.0, app.history_cap.max(1) as f64]),
        )
        .y_axis(
            Axis::default()
                .title("MB")
                .style(Style::default().fg(DIM))
                .bounds([0.0, max_y])
                .labels([
                    Line::from("0"),
                    Line::from(format!("{:.0}", max_y / 2.0)),
                    Line::from(format!("{:.0}", max_y)),
                ]),
        );
    frame.render_widget(chart, area);
}

fn draw_java_table(frame: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec!["PID", "Name", "RSS", "CPU%", "Agent", "Cmd"])
        .style(Style::default().fg(CYAN).add_modifier(Modifier::BOLD));
    let rows = app.java.processes.iter().take(6).map(|p| {
        Row::new(vec![
            p.pid.to_string(),
            p.name.clone(),
            fmt_bytes(p.memory_bytes),
            format!("{:.1}", p.cpu_percent),
            if p.has_javar_agent {
                "yes".into()
            } else {
                "no".into()
            },
            p.cmd.clone(),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(9),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                "JVM processes (sysinfo) — total RSS {}",
                fmt_bytes(app.java.total_rss)
            )),
    );
    frame.render_widget(table, area);
}

fn draw_hot_reload(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(4)])
        .split(area);

    let reloads = app.agent.telemetry.reload_count;
    let saved_secs = reloads.saturating_mul(8);
    let summary = Paragraph::new(vec![
        Line::from(format!("Shadow / redefine events: {reloads}")),
        Line::from(format!(
            "Est. restart time saved: {} ({} × ~8s cold start)",
            fmt_duration(saved_secs),
            reloads
        )),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Hot-Reload Impact"),
    );
    frame.render_widget(summary, chunks[0]);

    let header = Row::new(vec!["Time", "Class", "Change", "Version"])
        .style(Style::default().fg(CYAN).add_modifier(Modifier::BOLD));
    let rows: Vec<Row> = if app.history.is_empty() {
        vec![Row::new(vec![
            "—".to_string(),
            "(awaiting reload)".to_string(),
            "—".to_string(),
            "—".to_string(),
        ])]
    } else {
        app.history
            .iter()
            .take(24)
            .map(|e| {
                Row::new(vec![
                    e.timestamp.clone(),
                    e.class_name.clone(),
                    e.change_type.clone(),
                    e.version.clone(),
                ])
            })
            .collect()
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Min(24),
            Constraint::Length(12),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Reload history (Timestamp · Class · Change · Version)"),
    );
    frame.render_widget(table, chunks[1]);
}

fn draw_gc(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.agent.telemetry;
    let regions = t.managed_regions;
    let savings = t.gc_savings.max(t.javar_managed);
    // Heuristic: each managed region ≈ one young-gen churn cycle avoided per refresh window.
    let cycles_avoided = regions.saturating_mul(app.ticks.max(1) / 4);

    let text = vec![
        Line::from(Span::styled(
            "@JavaRManaged GC bypass",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("Off-heap regions:     {regions}")),
        Line::from(format!("Bytes kept off heap:  {}", fmt_bytes(savings))),
        Line::from(format!("Est. GC cycles avoided: {cycles_avoided}")),
        Line::from(format!(
            "Loaded classes:       {}",
            t.loaded_classes
        )),
        Line::from(format!(
            "Bridge backend:       {}",
            if t.offheap_backend.is_empty() {
                "n/a"
            } else {
                &t.offheap_backend
            }
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Primitive fields on @JavaRManaged types live in Rust — the JVM shell stays tiny.",
            Style::default().fg(DIM),
        )),
    ];
    let p = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title("GC Metrics"),
    );
    frame.render_widget(p, area);
}

fn draw_logs(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .logs
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .map(|l| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    l.at.format("%H:%M:%S ").to_string(),
                    Style::default().fg(DIM),
                ),
                Span::raw(l.text.clone()),
            ]))
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("System logs — bytecode injections & agent events"),
    );
    frame.render_widget(list, area);
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let help = Paragraph::new(Line::from(vec![
        Span::styled("q", Style::default().fg(ORANGE)),
        Span::raw(" quit  "),
        Span::styled("←/→", Style::default().fg(ORANGE)),
        Span::raw(" tabs  "),
        Span::styled("1-4", Style::default().fg(ORANGE)),
        Span::raw(" jump  "),
        Span::styled("r", Style::default().fg(ORANGE)),
        Span::raw(" refresh  "),
        Span::styled("Roberto de Souza", Style::default().fg(DIM)),
        Span::raw(" · Zero-Restart Java"),
    ]));
    frame.render_widget(help, area);
}

fn fmt_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = n as f64;
    if v >= GB {
        format!("{:.2} GB", v / GB)
    } else if v >= MB {
        format!("{:.1} MB", v / MB)
    } else if v >= KB {
        format!("{:.0} KB", v / KB)
    } else {
        format!("{n} B")
    }
}

fn fmt_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

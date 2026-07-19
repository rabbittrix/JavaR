//! Discover running JVM processes via `sysinfo`.

use sysinfo::{ProcessesToUpdate, System};

#[derive(Debug, Clone)]
pub struct JavaProcess {
    pub pid: u32,
    pub name: String,
    pub cmd: String,
    pub memory_bytes: u64,
    pub cpu_percent: f32,
    pub has_javar_agent: bool,
}

#[derive(Debug, Clone, Default)]
pub struct JavaProcSnapshot {
    pub processes: Vec<JavaProcess>,
    pub total_rss: u64,
}

pub fn sample(sys: &mut System) -> JavaProcSnapshot {
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut processes = Vec::new();
    let mut total_rss = 0u64;

    for (pid, proc) in sys.processes() {
        let name = proc.name().to_string_lossy().to_lowercase();
        let cmd_joined: String = proc
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let cmd_l = cmd_joined.to_lowercase();

        let is_java = name.contains("java")
            || name == "javaw"
            || name == "java.exe"
            || name == "javaw.exe"
            || cmd_l.contains("java.base")
            || (cmd_l.contains("java") && cmd_l.contains("-cp"));

        if !is_java {
            continue;
        }

        let mem = proc.memory(); // bytes (sysinfo 0.32)
        total_rss += mem;
        let has_agent = cmd_l.contains("javar-agent") || cmd_l.contains("javaagent");

        processes.push(JavaProcess {
            pid: pid.as_u32(),
            name: proc.name().to_string_lossy().into_owned(),
            cmd: truncate(&cmd_joined, 120),
            memory_bytes: mem,
            cpu_percent: proc.cpu_usage(),
            has_javar_agent: has_agent,
        });
    }

    processes.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
    JavaProcSnapshot {
        processes,
        total_rss,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

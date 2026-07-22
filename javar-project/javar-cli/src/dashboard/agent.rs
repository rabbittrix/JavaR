//! Poll telemetry / status from the JavaR agent TCP socket.

use anyhow::{Context, Result};
use bytes::Bytes;
use javar_core::protocol::{Frame, Message, MessageKind};
use serde::Deserialize;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentTelemetry {
    #[serde(default, deserialize_with = "de_u64_lossy")]
    pub java_heap_used: u64,
    #[serde(default, deserialize_with = "de_u64_lossy")]
    pub java_heap_max: u64,
    #[serde(default, deserialize_with = "de_u64_lossy")]
    pub javar_managed: u64,
    #[serde(default, deserialize_with = "de_u64_lossy")]
    pub gc_savings: u64,
    #[serde(default, deserialize_with = "de_u64_lossy")]
    pub managed_regions: u64,
    #[serde(default, deserialize_with = "de_u64_lossy")]
    pub reload_count: u64,
    #[serde(default, deserialize_with = "de_u64_lossy")]
    pub loaded_classes: u64,
    #[serde(default)]
    pub offheap_backend: String,
    #[serde(default)]
    pub project_name: String,
    #[serde(default, deserialize_with = "de_u64_lossy")]
    pub pid: u64,
    #[serde(default)]
    pub jvm_cmd: String,
    /// Sticky SYNC ERROR from agent (IDE Java 23 vs runtime).
    #[serde(default)]
    pub sync_alert: String,
    #[serde(default)]
    pub reload_history: Vec<ReloadEventDto>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReloadEventDto {
    /// Epoch millis (number) or legacy ISO / numeric string.
    #[serde(default, deserialize_with = "de_ts_lossy")]
    pub ts: String,
    #[serde(default)]
    pub class_name: String,
    #[serde(default)]
    pub change_type: String,
    #[serde(default, deserialize_with = "de_i64_lossy")]
    pub version: i64,
}

fn de_ts_lossy<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;
    Ok(match v {
        Value::Null => String::new(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s,
        _ => String::new(),
    })
}

/// Accept JSON numbers that may be negative (JVM MemoryMXBean) or floats.
fn de_u64_lossy<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;
    Ok(match v {
        Value::Null => 0,
        Value::Bool(b) => u64::from(b),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                u
            } else if let Some(i) = n.as_i64() {
                i.max(0) as u64
            } else if let Some(f) = n.as_f64() {
                if f.is_finite() && f > 0.0 {
                    f as u64
                } else {
                    0
                }
            } else {
                0
            }
        }
        Value::String(s) => s.parse::<i64>().unwrap_or(0).max(0) as u64,
        _ => 0,
    })
}

fn de_i64_lossy<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;
    Ok(match v {
        Value::Null => 0,
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)).unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    })
}

#[derive(Debug, Clone)]
pub struct AgentSnapshot {
    pub connected: bool,
    pub detail: String,
    pub telemetry: AgentTelemetry,
}

impl Default for AgentSnapshot {
    fn default() -> Self {
        Self {
            connected: false,
            detail: "offline".into(),
            telemetry: AgentTelemetry::default(),
        }
    }
}

/// Quick identify: connect, ping, fetch telemetry name/pid (or None if not a JavaR agent).
pub fn identify(addr: &str) -> Option<(String, u64, AgentTelemetry)> {
    let snap = poll(addr);
    if !snap.connected {
        return None;
    }
    let name = if !snap.telemetry.project_name.is_empty() {
        snap.telemetry.project_name.clone()
    } else {
        "java-app".into()
    };
    Some((name, snap.telemetry.pid, snap.telemetry))
}

pub fn poll(addr: &str) -> AgentSnapshot {
    match poll_inner(addr) {
        Ok(s) => s,
        Err(err) => AgentSnapshot {
            connected: false,
            detail: format!("{err:#}"),
            telemetry: AgentTelemetry::default(),
        },
    }
}

fn parse_telemetry(body: &[u8]) -> AgentTelemetry {
    if let Ok(t) = serde_json::from_slice::<AgentTelemetry>(body) {
        return t;
    }
    // Salvage fields if a single bad value previously wiped the whole payload.
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return AgentTelemetry::default();
    };
    let mut t = AgentTelemetry::default();
    t.java_heap_used = json_u64(&v, "java_heap_used");
    t.java_heap_max = json_u64(&v, "java_heap_max");
    t.javar_managed = json_u64(&v, "javar_managed");
    t.gc_savings = json_u64(&v, "gc_savings");
    t.managed_regions = json_u64(&v, "managed_regions");
    t.reload_count = json_u64(&v, "reload_count");
    t.loaded_classes = json_u64(&v, "loaded_classes");
    t.pid = json_u64(&v, "pid");
    t.offheap_backend = json_str(&v, "offheap_backend");
    t.project_name = json_str(&v, "project_name");
    t.jvm_cmd = json_str(&v, "jvm_cmd");
    t.sync_alert = json_str(&v, "sync_alert");
    if let Some(arr) = v.get("reload_history").and_then(|x| x.as_array()) {
        t.reload_history = arr
            .iter()
            .filter_map(|e| {
                Some(ReloadEventDto {
                    ts: match e.get("ts")? {
                        Value::Number(n) => n.to_string(),
                        Value::String(s) => s.clone(),
                        _ => return None,
                    },
                    class_name: e
                        .get("class_name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("?")
                        .to_string(),
                    change_type: e
                        .get("change_type")
                        .and_then(|x| x.as_str())
                        .unwrap_or("Body")
                        .to_string(),
                    version: e
                        .get("version")
                        .and_then(|x| x.as_i64().or_else(|| x.as_u64().map(|u| u as i64)))
                        .unwrap_or(0),
                })
            })
            .collect();
    }
    t
}

fn json_u64(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_i64().map(|i| i.max(0) as u64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.parse::<i64>().unwrap_or(0).max(0) as u64,
        _ => 0,
    }
}

fn json_str(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn poll_inner(addr: &str) -> Result<AgentSnapshot> {
    let mut stream =
        TcpStream::connect(addr).with_context(|| format!("connect agent at {addr}"))?;
    stream.set_read_timeout(Some(Duration::from_millis(800)))?;
    stream.set_write_timeout(Some(Duration::from_millis(800)))?;
    stream.set_nodelay(true)?;

    write_msg(&mut stream, &Message::ping())?;
    let (_pong, _) = read_msg(&mut stream)?;

    write_msg(
        &mut stream,
        &Message {
            kind: MessageKind::Telemetry,
            body: Bytes::new(),
        },
    )?;
    let (tel, _) = read_msg(&mut stream)?;
    let telemetry = if tel.kind == MessageKind::Telemetry {
        parse_telemetry(&tel.body)
    } else {
        AgentTelemetry::default()
    };

    Ok(AgentSnapshot {
        connected: true,
        detail: {
            let backend = if telemetry.offheap_backend.is_empty() {
                "?"
            } else {
                telemetry.offheap_backend.as_str()
            };
            let hist_n = telemetry.reload_history.len();
            if telemetry.project_name.is_empty() {
                format!(
                    "backend={backend} reloads={} hist={hist_n}",
                    telemetry.reload_count
                )
            } else {
                format!(
                    "project={}  backend={backend} reloads={} hist={hist_n}",
                    telemetry.project_name, telemetry.reload_count
                )
            }
        },
        telemetry,
    })
}

fn write_msg(stream: &mut TcpStream, msg: &Message) -> Result<()> {
    let frame = Frame::encode(msg);
    stream.write_all(&frame.header)?;
    stream.write_all(&frame.payload)?;
    stream.flush()?;
    Ok(())
}

fn read_msg(stream: &mut TcpStream) -> Result<(Message, usize)> {
    let mut header = [0u8; 10];
    stream.read_exact(&mut header)?;
    let payload_len = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        stream.read_exact(&mut payload)?;
    }
    let mut full = Vec::with_capacity(10 + payload_len);
    full.extend_from_slice(&header);
    full.extend_from_slice(&payload);
    Frame::decode(&full).map_err(|e| anyhow::anyhow!("{e}"))
}

//! Poll telemetry / status from the JavaR agent TCP socket.

use anyhow::{Context, Result};
use bytes::Bytes;
use javar_core::protocol::{Frame, Message, MessageKind};
use serde::Deserialize;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentTelemetry {
    #[serde(default)]
    pub java_heap_used: u64,
    #[serde(default)]
    pub java_heap_max: u64,
    #[serde(default)]
    pub javar_managed: u64,
    #[serde(default)]
    pub gc_savings: u64,
    #[serde(default)]
    pub managed_regions: u64,
    #[serde(default)]
    pub reload_count: u64,
    #[serde(default)]
    pub loaded_classes: u64,
    #[serde(default)]
    pub offheap_backend: String,
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

fn poll_inner(addr: &str) -> Result<AgentSnapshot> {
    let mut stream =
        TcpStream::connect(addr).with_context(|| format!("connect agent at {addr}"))?;
    stream.set_read_timeout(Some(Duration::from_millis(800)))?;
    stream.set_write_timeout(Some(Duration::from_millis(800)))?;
    stream.set_nodelay(true)?;

    // Ping
    write_msg(&mut stream, &Message::ping())?;
    let (pong, _) = read_msg(&mut stream)?;
    if pong.kind != MessageKind::Pong && pong.kind != MessageKind::Status {
        // continue anyway
    }

    // Telemetry
    write_msg(
        &mut stream,
        &Message {
            kind: MessageKind::Telemetry,
            body: Bytes::new(),
        },
    )?;
    let (tel, _) = read_msg(&mut stream)?;
    let telemetry = if tel.kind == MessageKind::Telemetry {
        serde_json::from_slice(&tel.body).unwrap_or_default()
    } else {
        AgentTelemetry::default()
    };

    Ok(AgentSnapshot {
        connected: true,
        detail: format!(
            "backend={} reloads={}",
            if telemetry.offheap_backend.is_empty() {
                "?"
            } else {
                &telemetry.offheap_backend
            },
            telemetry.reload_count
        ),
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

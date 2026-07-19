//! Binary framing protocol between javar-core, javar-agent, and IDE clients.
//!
//! Wire format (little-endian):
//! ```text
//! [u32 magic=0x4A415652 "JAVR"][u8 version][u8 kind][u32 payload_len][payload...]
//! ```
//!
//! Payload for `Redefine` is JSON header + raw bytecode appended for zero-copy
//! handoff on the agent side (`bytes::Bytes` sharing).

use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAGIC: u32 = 0x4A41_5652; // "JAVR"
pub const VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageKind {
    Ping = 1,
    Pong = 2,
    Status = 3,
    Error = 4,
    Redefine = 5,
    Rollback = 6,
    Telemetry = 7,
    HotDeploy = 8,
}

impl MessageKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Ping,
            2 => Self::Pong,
            3 => Self::Status,
            4 => Self::Error,
            5 => Self::Redefine,
            6 => Self::Rollback,
            7 => Self::Telemetry,
            8 => Self::HotDeploy,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedefinePayload {
    pub class_name: String,
    /// Offset into the frame payload where bytecode begins (after JSON header).
    pub bytecode_offset: u32,
    pub bytecode_len: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusPayload {
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryPayload {
    pub java_heap_used: u64,
    pub java_heap_max: u64,
    pub javar_managed: u64,
    pub reload_count: u64,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub kind: MessageKind,
    pub body: Bytes,
}

impl Message {
    pub fn status(state: impl Into<String>, detail: impl Into<String>) -> Self {
        let payload = StatusPayload {
            state: state.into(),
            detail: detail.into(),
        };
        Self {
            kind: MessageKind::Status,
            body: Bytes::from(serde_json::to_vec(&payload).expect("status json")),
        }
    }

    pub fn error(detail: impl Into<String>) -> Self {
        let payload = StatusPayload {
            state: "error".into(),
            detail: detail.into(),
        };
        Self {
            kind: MessageKind::Error,
            body: Bytes::from(serde_json::to_vec(&payload).expect("error json")),
        }
    }

    /// Build a redefine message with JSON metadata + raw bytecode (shared `Bytes`).
    pub fn redefine(class_name: impl Into<String>, bytecode: Bytes) -> Self {
        let class_name = class_name.into();
        let meta = RedefinePayload {
            class_name,
            bytecode_offset: 0, // filled after header encode
            bytecode_len: bytecode.len() as u32,
        };
        let mut header = serde_json::to_vec(&meta).expect("redefine json");
        // Rewrite offset to point past the JSON header + length prefix we embed.
        let offset = (4 + header.len()) as u32;
        let meta = RedefinePayload {
            class_name: meta.class_name,
            bytecode_offset: offset,
            bytecode_len: bytecode.len() as u32,
        };
        header = serde_json::to_vec(&meta).expect("redefine json");

        let mut buf = BytesMut::with_capacity(4 + header.len() + bytecode.len());
        buf.put_u32_le(header.len() as u32);
        buf.extend_from_slice(&header);
        // Zero-copy extend: Bytes implements Buf / AsRef
        buf.extend_from_slice(&bytecode);

        Self {
            kind: MessageKind::Redefine,
            body: buf.freeze(),
        }
    }

    pub fn rollback(class_name: impl Into<String>) -> Self {
        let payload = StatusPayload {
            state: "rollback".into(),
            detail: class_name.into(),
        };
        Self {
            kind: MessageKind::Rollback,
            body: Bytes::from(serde_json::to_vec(&payload).expect("rollback json")),
        }
    }

    pub fn telemetry(t: TelemetryPayload) -> Self {
        Self {
            kind: MessageKind::Telemetry,
            body: Bytes::from(serde_json::to_vec(&t).expect("telemetry json")),
        }
    }

    pub fn ping() -> Self {
        Self {
            kind: MessageKind::Ping,
            body: Bytes::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid magic: {0:#x}")]
    InvalidMagic(u32),
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("unknown message kind: {0}")]
    UnknownKind(u8),
    #[error("truncated frame")]
    Truncated,
}

/// Encoded frame ready for writev / vectored IO.
#[derive(Debug, Clone)]
pub struct Frame {
    pub header: [u8; 10],
    pub payload: Bytes,
}

impl Frame {
    pub fn encode(msg: &Message) -> Self {
        let mut header = [0u8; 10];
        header[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        header[4] = VERSION;
        header[5] = msg.kind as u8;
        header[6..10].copy_from_slice(&(msg.body.len() as u32).to_le_bytes());
        Self {
            header,
            payload: msg.body.clone(),
        }
    }

    /// Total bytes on the wire without allocating a contiguous buffer.
    pub fn len(&self) -> usize {
        self.header.len() + self.payload.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Materialize only when needed (prefer writing header + payload separately).
    pub fn to_bytes(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(self.len());
        buf.extend_from_slice(&self.header);
        buf.extend_from_slice(&self.payload);
        buf.freeze()
    }

    pub fn decode(data: &[u8]) -> Result<(Message, usize), ProtocolError> {
        if data.len() < 10 {
            return Err(ProtocolError::Truncated);
        }
        let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
        if magic != MAGIC {
            return Err(ProtocolError::InvalidMagic(magic));
        }
        let version = data[4];
        if version != VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }
        let kind = MessageKind::from_u8(data[5]).ok_or(ProtocolError::UnknownKind(data[5]))?;
        let payload_len = u32::from_le_bytes(data[6..10].try_into().unwrap()) as usize;
        if data.len() < 10 + payload_len {
            return Err(ProtocolError::Truncated);
        }
        let body = Bytes::copy_from_slice(&data[10..10 + payload_len]);
        Ok((Message { kind, body }, 10 + payload_len))
    }
}

/// Split redefine body into metadata + bytecode slice without copying bytecode.
pub fn split_redefine(body: &Bytes) -> Option<(RedefinePayload, Bytes)> {
    if body.len() < 4 {
        return None;
    }
    let header_len = u32::from_le_bytes(body[0..4].try_into().ok()?) as usize;
    if body.len() < 4 + header_len {
        return None;
    }
    let meta: RedefinePayload = serde_json::from_slice(&body[4..4 + header_len]).ok()?;
    let start = 4 + header_len;
    let end = start + meta.bytecode_len as usize;
    if body.len() < end {
        return None;
    }
    // Bytes::slice is reference-counted — zero-copy view of bytecode region.
    Some((meta, body.slice(start..end)))
}

//! TCP socket bridge to the Java Agent (cross-platform, IDE-agnostic).
//! Supports live retargeting when the app migrates off a busy IDE port.

use super::{AgentBridge, BridgeConfig};
use crate::protocol::{Frame, Message};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, info, warn};

pub struct SocketBridge {
    tx: mpsc::Sender<Message>,
    addr: Arc<RwLock<String>>,
    /// Bumped on retarget so the writer drops a stale TCP stream.
    epoch: Arc<std::sync::atomic::AtomicU64>,
    _join: tokio::task::JoinHandle<()>,
}

impl SocketBridge {
    pub async fn connect(config: BridgeConfig) -> Result<Self> {
        match TcpStream::connect(&config.addr).await {
            Ok(mut stream) => {
                let _ = stream.shutdown().await;
            }
            Err(err) if config.reconnect => {
                warn!(
                    addr = %config.addr,
                    ?err,
                    "agent not ready yet; will reconnect"
                );
            }
            Err(err) => {
                return Err(err).with_context(|| format!("initial connect to {}", config.addr));
            }
        }

        let (tx, mut rx) = mpsc::channel::<Message>(256);
        let addr = Arc::new(RwLock::new(config.addr.clone()));
        let epoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let addr_w = addr.clone();
        let epoch_w = epoch.clone();
        let reconnect = config.reconnect;

        let join = tokio::spawn(async move {
            let writer_state: Mutex<(Option<TcpStream>, u64)> = Mutex::new((None, 0));

            loop {
                let current_epoch = epoch_w.load(std::sync::atomic::Ordering::Relaxed);
                {
                    let mut guard = writer_state.lock().await;
                    if guard.1 != current_epoch {
                        guard.0 = None;
                        guard.1 = current_epoch;
                    }
                }

                let target = addr_w.read().await.clone();
                if !ensure_connected(&writer_state, &target, reconnect).await {
                    break;
                }

                let msg = match rx.recv().await {
                    Some(m) => m,
                    None => break,
                };

                let frame = Frame::encode(&msg);
                let mut stream = writer_state.lock().await.0.take();
                let write_result = match stream.as_mut() {
                    Some(stream) => {
                        let result = async {
                            stream.write_all(&frame.header).await?;
                            stream.write_all(&frame.payload).await?;
                            stream.flush().await?;
                            Ok::<(), std::io::Error>(())
                        }
                        .await;
                        Some(result)
                    }
                    None => None,
                };

                match write_result {
                    Some(Ok(())) => {
                        if let Some(ref mut stream) = stream {
                            let _ = read_agent_ack(stream).await;
                        }
                        writer_state.lock().await.0 = stream;
                        debug!(kind = ?msg.kind, bytes = frame.len(), "frame sent");
                    }
                    Some(Err(err)) => {
                        warn!(?err, "write failed, dropping connection");
                        if !reconnect {
                            break;
                        }
                    }
                    None => {
                        if !reconnect {
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            tx,
            addr,
            epoch,
            _join: join,
        })
    }

    pub async fn current_addr(&self) -> String {
        self.addr.read().await.clone()
    }
}

async fn read_agent_ack(stream: &mut TcpStream) -> Result<()> {
    let mut header = [0u8; 10];
    match tokio::time::timeout(
        std::time::Duration::from_millis(2000),
        stream.read_exact(&mut header),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => {
            warn!("agent ack timed out");
            return Ok(());
        }
    }
    let kind = header[5];
    let payload_len = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
    if payload_len > 0 && payload_len < 16 * 1024 * 1024 {
        let mut payload = vec![0u8; payload_len];
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(2000),
            stream.read_exact(&mut payload),
        )
        .await;
        if kind == 10 {
            if let Ok(ev) = serde_json::from_slice::<serde_json::Value>(&payload) {
                let ok = ev
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if ok {
                    info!(
                        class = %ev.get("class_name").and_then(|v| v.as_str()).unwrap_or("?"),
                        change = %ev.get("change_type").and_then(|v| v.as_str()).unwrap_or("Body"),
                        version = ev.get("version").and_then(|v| v.as_u64()).unwrap_or(0),
                        ts = ev.get("ts").and_then(|v| v.as_u64()).unwrap_or(0),
                        "RELOAD_EVENT from agent"
                    );
                } else {
                    warn!(
                        class = %ev.get("class_name").and_then(|v| v.as_str()).unwrap_or("?"),
                        detail = %ev.get("detail").and_then(|v| v.as_str()).unwrap_or("fail"),
                        "RELOAD_EVENT FAIL from agent"
                    );
                }
            }
        }
    }
    Ok(())
}

async fn ensure_connected(
    writer_state: &Mutex<(Option<TcpStream>, u64)>,
    addr: &str,
    reconnect: bool,
) -> bool {
    if writer_state.lock().await.0.is_some() {
        return true;
    }

    match TcpStream::connect(addr).await {
        Ok(stream) => {
            info!(%addr, "connected to JavaR agent");
            writer_state.lock().await.0 = Some(stream);
            true
        }
        Err(err) => {
            warn!(%addr, ?err, "agent connect failed");
            if !reconnect {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            true
        }
    }
}

#[async_trait]
impl AgentBridge for SocketBridge {
    async fn send(&self, msg: Message) -> Result<()> {
        self.tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("bridge channel closed: {e}"))
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    async fn retarget(&self, addr: String) -> Result<()> {
        let mut cur = self.addr.write().await;
        if *cur != addr {
            info!(from = %*cur, to = %addr, "retargeting sidecar → live agent port");
            *cur = addr;
            self.epoch
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }
}

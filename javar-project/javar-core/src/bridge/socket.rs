//! TCP socket bridge to the Java Agent (cross-platform, IDE-agnostic).

use super::{AgentBridge, BridgeConfig};
use crate::protocol::{Frame, Message};
use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};

pub struct SocketBridge {
    tx: mpsc::Sender<Message>,
    _join: tokio::task::JoinHandle<()>,
}

impl SocketBridge {
    pub async fn connect(config: BridgeConfig) -> Result<Self> {
        // Probe once; if the agent is not up yet, continue when reconnect is enabled.
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
        let addr = config.addr.clone();
        let reconnect = config.reconnect;

        let join = tokio::spawn(async move {
            // Never hold the mutex across `.await` — take/replace the stream instead.
            // That keeps the spawned future `Send` and avoids rustc/rust-analyzer false positives.
            let writer_state: Mutex<Option<TcpStream>> = Mutex::new(None);

            loop {
                if !ensure_connected(&writer_state, &addr, reconnect).await {
                    break;
                }

                let msg = match rx.recv().await {
                    Some(m) => m,
                    None => break,
                };

                let frame = Frame::encode(&msg);
                let mut stream = writer_state.lock().await.take();
                let write_result = match stream.as_mut() {
                    Some(stream) => {
                        // Header then payload — avoids allocating a contiguous concat buffer.
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
                        *writer_state.lock().await = stream;
                        debug!(kind = ?msg.kind, bytes = frame.len(), "frame sent");
                    }
                    Some(Err(err)) => {
                        warn!(?err, "write failed, dropping connection");
                        // drop broken stream
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

        Ok(Self { tx, _join: join })
    }
}

/// Connect if needed. Returns `false` when the writer loop should exit.
async fn ensure_connected(
    writer_state: &Mutex<Option<TcpStream>>,
    addr: &str,
    reconnect: bool,
) -> bool {
    if writer_state.lock().await.is_some() {
        return true;
    }

    match TcpStream::connect(addr).await {
        Ok(stream) => {
            info!(%addr, "connected to JavaR agent");
            *writer_state.lock().await = Some(stream);
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
}

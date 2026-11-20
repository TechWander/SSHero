//! Transport-agnostic PTY primitive for the terminal tab.
//!
//! [`open`] performs the same SSH sequence as [`crate::ws`]
//! (`channel_open_session` → `request_pty` → `request_shell`) and then hands
//! back a [`Pty`] handle plus an unbounded receiver of [`PtyEvent`]s produced
//! by a background reader task. The caller decides how bytes reach the UI:
//! * the WebSocket bridge forwards them as binary frames (existing HTTP mode),
//! * a native desktop shell forwards them over an IPC channel/stream —
//!   **no socket involved**.
//!
//! [`Pty::write`], [`Pty::resize`] and [`Pty::close`] map 1:1 to the russh
//! channel operations, so the wire protocol differences live entirely in the
//! transport layer, never here.

use std::sync::Arc;

use anyhow::{Context, Result};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelWriteHalf};
use tokio::sync::mpsc;

use crate::ssh::ManagedConnection;

/// One output event from the remote PTY.
#[derive(Debug)]
pub enum PtyEvent {
    /// raw terminal bytes (stdout and stderr are multiplexed here, matching
    /// the WS bridge, which sends both as binary frames)
    Data(Vec<u8>),
    /// shell/command exited with this status
    Exit(u32),
}

/// A live remote PTY. Drop the returned [`Pty`] or call [`Pty::close`] to
/// tear the channel down; the reader task stops on its own.
pub struct Pty {
    wr: Arc<ChannelWriteHalf<Msg>>,
    rx: mpsc::UnboundedReceiver<PtyEvent>,
}

/// Open a PTY of `cols`×`rows` on `conn` and start pumping its output into the
/// returned receiver.
pub async fn open(
    conn: &Arc<ManagedConnection>,
    cols: u32,
    rows: u32,
) -> Result<Pty> {
    let channel = conn
        .handle
        .channel_open_session()
        .await
        .context("channel_open_session failed")?;
    let (mut rd, wr) = channel.split();
    let wr = Arc::new(wr);
    wr.request_pty(
        true,
        "xterm-256color",
        cols,
        rows,
        cols * 8,
        rows * 16,
        &[],
    )
    .await
    .context("request_pty failed")?;
    wr.request_shell(true).await.context("request_shell failed")?;

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(msg) = rd.wait().await {
            let ev = match msg {
                ChannelMsg::Data { data } => PtyEvent::Data(data.to_vec()),
                ChannelMsg::ExtendedData { data, .. } => PtyEvent::Data(data.to_vec()),
                ChannelMsg::ExitStatus { exit_status } => {
                    let _ = tx.send(PtyEvent::Exit(exit_status));
                    break;
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => continue,
            };
            if tx.send(ev).is_err() {
                break; // receiver gone: UI disconnected
            }
        }
    });

    Ok(Pty { wr, rx })
}

impl Pty {
    /// Send keystroke bytes to the remote shell.
    pub async fn write(&self, bytes: Vec<u8>) -> Result<()> {
        self.wr.data_bytes(bytes).await.context("pty write failed")?;
        Ok(())
    }

    /// Inform the remote of a terminal size change.
    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.wr
            .window_change(cols, rows, cols * 8, rows * 16)
            .await
            .context("pty resize failed")
    }

    /// Pull the next output event (returns `None` once the reader stopped).
    pub async fn recv(&mut self) -> Option<PtyEvent> {
        self.rx.recv().await
    }

    /// Half-close and close the channel.
    pub async fn close(&self) {
        let _ = self.wr.eof().await;
        let _ = self.wr.close().await;
    }
}

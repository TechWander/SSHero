//! WebSocket <-> remote PTY bridge powering the terminal tab.
//!
//! Wire protocol (per direction):
//! * binary frames carry raw terminal bytes (keystrokes out / PTY output in)
//! * text frames carry JSON control messages:
//!   `{"type":"resize","cols":120,"rows":40}` from the client,
//!   `{"type":"exit","code":0}` / `{"type":"error","message":"…"}` from the server.

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as UrlPath, Query, State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use russh::ChannelMsg;
use serde::Deserialize;
use uuid::Uuid;

use crate::{api::AppState, ssh::ManagedConnection};

#[derive(Deserialize)]
pub struct TermQuery {
    #[serde(default = "default_cols")]
    cols: u32,
    #[serde(default = "default_rows")]
    rows: u32,
}

fn default_cols() -> u32 {
    80
}
fn default_rows() -> u32 {
    24
}

pub async fn terminal(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Query(q): Query<TermQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let conn: Arc<ManagedConnection> = match state.conns.get(&id) {
        Some(c) => c.value().clone(),
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": "connection not found"})),
            )
                .into_response()
        }
    };
    ws.on_upgrade(move |socket| pump(socket, conn, q.cols, q.rows))
}

async fn pump(socket: WebSocket, conn: Arc<ManagedConnection>, cols: u32, rows: u32) {
    let channel = match conn.handle.channel_open_session().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("terminal: channel_open_session failed: {e}");
            return;
        }
    };
    let (mut rd, wr) = channel.split();
    let wr = Arc::new(wr);
    if let Err(e) = wr
        .request_pty(
            true,
            "xterm-256color",
            cols,
            rows,
            cols * 8,
            rows * 16,
            &[],
        )
        .await
    {
        tracing::warn!("terminal: request_pty failed: {e}");
        return;
    }
    if let Err(e) = wr.request_shell(true).await {
        tracing::warn!("terminal: request_shell failed: {e}");
        return;
    }

    let (mut sink, mut stream) = socket.split();

    // remote -> browser
    let out_task = tokio::spawn(async move {
        while let Some(msg) = rd.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    if sink.send(Message::Binary(data.to_vec().into())).await.is_err() {
                        break;
                    }
                }
                ChannelMsg::ExtendedData { data, .. } => {
                    if sink
                        .send(Message::Binary(data.to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    let _ = sink
                        .send(Message::Text(
                            serde_json::json!({"type": "exit", "code": exit_status})
                                .to_string()
                                .into(),
                        ))
                        .await;
                    break;
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        let _ = sink.close().await;
    });

    // browser -> remote
    let in_task = {
        let wr = wr.clone();
        async move {
            while let Some(Ok(msg)) = stream.next().await {
                match msg {
                    Message::Binary(b) => {
                        if wr.data_bytes(b).await.is_err() {
                            break;
                        }
                    }
                    Message::Text(t) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(t.as_str()) {
                            if v.get("type").and_then(|s| s.as_str()) == Some("resize") {
                                let c = v.get("cols").and_then(|n| n.as_u64()).unwrap_or(80) as u32;
                                let r = v.get("rows").and_then(|n| n.as_u64()).unwrap_or(24) as u32;
                                let _ = wr.window_change(c, r, c * 8, r * 16).await;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    };

    tokio::select! {
        _ = in_task => {}
        _ = out_task => {}
    }
    let _ = wr.eof().await;
    let _ = wr.close().await;
}

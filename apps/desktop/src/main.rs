//! Tauri desktop shell for sshero.
//!
//! Architecture: the axum backend runs **in-process** on a random loopback
//! port with a freshly generated token; the window loads the SPA served by
//! that backend. All business logic stays in Rust; the webview talks to it
//! over same-origin HTTP/WebSocket on 127.0.0.1 only.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{net::SocketAddr, path::PathBuf};

fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local").join("share").join("sshero")
}

fn main() -> anyhow::Result<()> {
    // 1. start the embedded backend on a random loopback port, token-gated
    let rt = tokio::runtime::Runtime::new()?;
    let token = sshero_backend::generate_token();
    let addr: SocketAddr = "127.0.0.1:0".parse()?;
    let (bound, _server) =
        rt.block_on(sshero_backend::spawn(addr, default_data_dir(), Some(token.clone()), None))?;

    // 2. the UI is loaded from the backend itself, token rides in the query
    let url = format!("http://{bound}/?token={token}");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url.parse()?),
            )
            .title("SSH 文件管理器")
            .inner_size(1280.0, 800.0)
            .min_inner_size(900.0, 560.0)
            // OS file drops are delivered as `tauri://drag-drop` events
            // (paths) which the frontend forwards to /upload-path
            .drag_drop_enabled(true)
            .build()?;
            window.set_focus()?;
            Ok(())
        })
        .build(tauri::generate_context!())?;

    app.run(|_app, event| {
        if let tauri::RunEvent::Exit = event {
            // graceful: backend tasks die with the process
        }
    });
    drop(rt);
    Ok(())
}

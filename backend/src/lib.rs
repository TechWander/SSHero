//! sshero backend as a reusable library.
//!
//! Consumers:
//! * the standalone binary (`src/main.rs`) — classic HTTP mode
//! * the Tauri desktop shell (`apps/desktop`) — embeds the server in-process
//!   on a random loopback port with a generated token (see [`spawn`]).

pub mod api;
pub mod core;
pub mod fsop;
pub mod pty;
pub mod sessions;
pub mod ssh;
pub mod ws;

use std::{net::SocketAddr, path::PathBuf};

use dashmap::DashMap;

use api::AppState;

/// Start the API server on `bind` (use port 0 for a random port).
/// Returns the actually bound address; the server runs until process exit.
pub async fn spawn(
    bind: SocketAddr,
    data_dir: PathBuf,
    token: Option<String>,
    static_dir: Option<PathBuf>,
) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    std::fs::create_dir_all(&data_dir).ok();
    let tofu = ssh::TofuStore::load(data_dir.join("known_hosts.json"));
    let store = sessions::SessionStore::load(data_dir.join("sessions.json"));

    let state = AppState {
        conns: DashMap::new(),
        tofu,
        token,
        sessions: store,
        data_dir,
        allow_local_fs: true,
    };
    let mut app = api::router(state);

    let dist = static_dir.or_else(find_default_dist).filter(|d| d.join("index.html").is_file());
    if let Some(dist) = dist {
        let spa = tower_http::services::ServeDir::new(&dist).not_found_service(
            tower_http::services::ServeFile::new(dist.join("index.html")),
        );
        app = app.fallback_service(spa);
    }

    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let join = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("server error: {e}");
        }
    });
    Ok((addr, join))
}

fn find_default_dist() -> Option<PathBuf> {
    ["../frontend/dist", "frontend/dist", "./dist"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.join("index.html").exists())
}

/// Generate a high-entropy token from two UUIDv4s (CSPRNG-backed via uuid).
pub fn generate_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

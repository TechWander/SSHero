//! sshero-backend: local HTTP server that manages SSH/SFTP sessions and
//! serves the Vue SPA build from `frontend/dist`.

use std::path::PathBuf;

use sshero_backend::{api, ssh};
use dashmap::DashMap;
use tower_http::services::{ServeDir, ServeFile};

struct Args {
    listen: String,
    static_dir: Option<PathBuf>,
    token: Option<String>,
    data_dir: PathBuf,
    allow_local_fs: bool,
    ready_file: Option<PathBuf>,
}

fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local").join("share").join("sshero")
}

fn parse_args() -> Args {
    let mut a = Args {
        listen: "127.0.0.1:8901".into(),
        static_dir: None,
        token: std::env::var("SSHERO_TOKEN").ok().filter(|s| !s.is_empty()),
        data_dir: default_data_dir(),
        allow_local_fs: true,
        ready_file: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(f) = it.next() {
        let mut val = || {
            it.next()
                .unwrap_or_else(|| panic!("missing value for {f}"))
        };
        match f.as_str() {
            "--listen" => a.listen = val(),
            "--static" => a.static_dir = Some(PathBuf::from(val())),
            "--token" => a.token = Some(val()),
            "--data-dir" => a.data_dir = PathBuf::from(val()),
            "--disable-local-fs" => a.allow_local_fs = false,
            "--ready-file" => a.ready_file = Some(PathBuf::from(val())),
            "-h" | "--help" => {
                println!(
                    "usage: sshero-backend [--listen 127.0.0.1:8901] [--static ../frontend/dist] [--token SECRET] [--data-dir DIR]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown flag: {other}");
                std::process::exit(2);
            }
        }
    }
    a
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=warn".into()),
        )
        .init();

    let args = parse_args();
    std::fs::create_dir_all(&args.data_dir).ok();
    let tofu = ssh::TofuStore::load(args.data_dir.join("known_hosts.json"));

    let state = api::AppState {
        conns: DashMap::new(),
        tofu,
        token: args.token.clone(),
        sessions: sshero_backend::sessions::SessionStore::load(
            args.data_dir.join("sessions.json"),
        ),
        data_dir: args.data_dir.clone(),
        allow_local_fs: args.allow_local_fs,
    };
    let mut app = api::router(state);

    // Serve the built SPA when the directory exists.
    let dist = args
        .static_dir
        .or_else(|| find_default_dist())
        .filter(|d| d.join("index.html").is_file());
    if let Some(dist) = dist {
        tracing::info!("serving SPA from {}", dist.display());
        let spa = ServeDir::new(&dist)
            .not_found_service(ServeFile::new(dist.join("index.html")));
        app = app.fallback_service(spa);
    } else {
        app = app.fallback(|| async {
            (
                axum::http::StatusCode::NOT_FOUND,
                "frontend build not found; run `npm run build` in ./frontend or use the vite dev server",
            )
        });
    }

    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .expect("failed to bind listen address");
    let bound = listener.local_addr().expect("local_addr");
    tracing::info!("sshero-backend listening on http://{}", bound);
    if let Some(f) = &args.ready_file {
        let payload = serde_json::json!({
            "port": bound.port(),
            "token": args.token.clone(),
        });
        let _ = std::fs::write(f, serde_json::to_vec(&payload).unwrap_or_default());
    }
    if args.token.is_none() {
        tracing::warn!("no --token set: the API is unauthenticated (safe only on loopback)");
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

fn find_default_dist() -> Option<PathBuf> {
    ["../frontend/dist", "frontend/dist", "./dist"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.join("index.html").exists())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}

//! Transport-agnostic application core.
//!
//! This module is the **server-free** entry point into the SSH/SFTP/session
//! logic. It owns the live-connection registry and exposes every operation as
//! a plain `async fn` returning ordinary serde types — no axum extractor, no
//! HTTP request/response, no WebSocket. It exists so a native desktop shell
//! (Tauri `#[tauri::command]`, or any IPC/FFI layer) can drive the engine
//! directly, with **no resident HTTP server process**.
//!
//! The classic HTTP surface in [`crate::api`] and [`crate::ws`] is just one
//! consumer of the same underlying `ssh`/`fsop`/`sessions` modules; this core
//! is the second, transport-free consumer. Keeping both means the Electron and
//! browser paths stay working while the pure-client path is validated.
//!
//! Binary transfer note: the pure client never moves file bytes through the
//! webview. It uses [`AppCore::upload_path`] (local disk → remote) and
//! [`AppCore::download_to_local`] (remote → local disk), which read/write the
//! host filesystem directly — the in-process equivalent of the HTTP
//! `download`/`upload` streaming routes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use uuid::Uuid;

use crate::fsop::{self, RemoteEntry};
use crate::sessions::{SessionInput, SessionMeta, SessionStore};
use crate::ssh::{
    connect_and_open_sftp, now_epoch, AuthMethod, ConnectParams, ConnectionInfo, ManagedConnection,
    TofuStore,
};

/// Shared, transport-free engine state. Cheap to wrap in an `Arc` and register
/// as native-IPC managed state.
pub struct AppCore {
    pub conns: DashMap<Uuid, Arc<ManagedConnection>>,
    pub tofu: TofuStore,
    pub sessions: SessionStore,
    pub data_dir: PathBuf,
    /// guard for the local-disk transfer ops; a pure client normally keeps
    /// this `true` (drag & drop), `false` disables them.
    pub allow_local_fs: bool,
}

/// Directory-listing result, the plain-data form of the HTTP `ListResponse`.
pub struct ListResult {
    pub path: String,
    pub entries: Vec<RemoteEntry>,
}

/// Outcome of a local→remote recursive upload.
pub struct UploadSummary {
    pub files: u32,
    pub written: u64,
}

impl AppCore {
    /// Load the TOFU host-key store and saved sessions from `data_dir`.
    pub fn new(data_dir: PathBuf, allow_local_fs: bool) -> Self {
        std::fs::create_dir_all(&data_dir).ok();
        AppCore {
            conns: DashMap::new(),
            tofu: TofuStore::load(data_dir.join("known_hosts.json")),
            sessions: SessionStore::load(data_dir.join("sessions.json")),
            data_dir,
            allow_local_fs,
        }
    }

    fn conn(&self, id: Uuid) -> Result<Arc<ManagedConnection>> {
        self.conns
            .get(&id)
            .map(|c| c.value().clone())
            .ok_or_else(|| anyhow!("connection {id} not found (is it connected?)"))
    }

    // ---------- connections ----------

    pub async fn connect(&self, params: ConnectParams) -> Result<ConnectionInfo> {
        if params.host.is_empty() || params.username.is_empty() {
            return Err(anyhow!("host and username are required"));
        }
        if let AuthMethod::Key { key_path, .. } = &params.auth {
            if !Path::new(key_path).is_file() {
                return Err(anyhow!("private key file not found: {key_path}"));
            }
        }

        let (handle, sftp) = connect_and_open_sftp(&params, self.tofu.clone()).await?;
        let _ = self.tofu.persist_if_dirty();

        let id = Uuid::new_v4();
        let info = ConnectionInfo {
            id,
            host: params.host.clone(),
            port: params.port,
            username: params.username.clone(),
            auth_kind: match &params.auth {
                AuthMethod::Password { .. } => "password".into(),
                AuthMethod::Key { .. } => "key".into(),
            },
            connected_at: now_epoch(),
        };
        self.conns.insert(
            id,
            Arc::new(ManagedConnection {
                info: info.clone(),
                params,
                sftp,
                handle,
            }),
        );
        Ok(info)
    }

    pub async fn close(&self, id: Uuid) -> Result<()> {
        let conn = self.conn(id)?;
        conn.close().await;
        self.conns.remove(&id);
        Ok(())
    }

    pub async fn reconnect(&self, id: Uuid) -> Result<ConnectionInfo> {
        let conn = self.conn(id)?;
        let (handle, sftp) = connect_and_open_sftp(&conn.params, self.tofu.clone()).await?;
        let _ = self.tofu.persist_if_dirty();
        let mut info = conn.info.clone();
        info.connected_at = now_epoch();
        conn.close().await;
        self.conns.insert(
            id,
            Arc::new(ManagedConnection {
                info: info.clone(),
                params: conn.params.clone(),
                sftp,
                handle,
            }),
        );
        Ok(info)
    }

    pub fn list_connections(&self) -> Vec<ConnectionInfo> {
        let mut v: Vec<_> = self
            .conns
            .iter()
            .map(|e| e.value().info.clone())
            .collect();
        v.sort_by_key(|c| c.connected_at);
        v
    }

    // ---------- remote filesystem ----------

    pub async fn list(&self, id: Uuid, path: &str) -> Result<ListResult> {
        let conn = self.conn(id)?;
        let (canonical, entries) = fsop::list_dir(&conn.sftp, path).await?;
        Ok(ListResult {
            path: canonical,
            entries,
        })
    }

    pub async fn stat(&self, id: Uuid, path: &str) -> Result<RemoteEntry> {
        let conn = self.conn(id)?;
        fsop::stat(&conn.sftp, path).await
    }

    pub async fn mkdir(&self, id: Uuid, path: &str) -> Result<()> {
        let conn = self.conn(id)?;
        fsop::mkdir_p(&conn.sftp, path).await
    }

    pub async fn remove(&self, id: Uuid, path: &str, recursive: bool) -> Result<()> {
        let conn = self.conn(id)?;
        fsop::remove(&conn.sftp, path, recursive).await
    }

    pub async fn rename(&self, id: Uuid, from: &str, to: &str) -> Result<()> {
        let conn = self.conn(id)?;
        fsop::rename(&conn.sftp, from, to).await
    }

    // ---------- saved session profiles ----------

    pub fn sessions_list(&self) -> Vec<SessionMeta> {
        self.sessions.list()
    }

    pub fn session_upsert(&self, mut input: SessionInput, id: Option<Uuid>) -> Result<SessionMeta> {
        if let Some(id) = id {
            input.id = Some(id);
        }
        self.sessions.upsert(input)
    }

    pub fn session_remove(&self, id: Uuid) -> Result<bool> {
        self.sessions.remove(id)
    }

    pub async fn connect_session(&self, id: Uuid, secret: Option<String>) -> Result<ConnectionInfo> {
        let session = self
            .sessions
            .get(id)
            .ok_or_else(|| anyhow!("session not found"))?;
        let params = session.to_connect_params(&self.data_dir, secret.as_deref())?;
        self.connect(params).await
    }

    // ---------- local-disk transfers (pure-client drag & drop) ----------

    /// Upload a local file or directory (absolute path) into remote `dir`.
    /// Bytes are read from the host FS directly; nothing crosses a webview.
    pub async fn upload_path(&self, id: Uuid, dir: &str, local_path: &str) -> Result<UploadSummary> {
        if !self.allow_local_fs {
            return Err(anyhow!("local path transfers disabled"));
        }
        let conn = self.conn(id)?;
        let src = Path::new(local_path);
        if !src.exists() {
            return Err(anyhow!("local path not found: {local_path}"));
        }
        let mut total = 0u64;
        let mut count = 0u32;
        let mut stack: Vec<(PathBuf, String)> = vec![(src.to_path_buf(), String::new())];
        while let Some((path, rel_prefix)) = stack.pop() {
            if path.is_dir() {
                let effective = if rel_prefix.is_empty() {
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                } else {
                    rel_prefix.clone()
                };
                if !effective.is_empty() {
                    let target = format!("{}/{}", dir.trim_end_matches('/'), effective);
                    let _ = fsop::mkdir_p(&conn.sftp, &target).await;
                }
                for entry in std::fs::read_dir(&path)
                    .with_context(|| format!("reading local dir {}", path.display()))?
                {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let rel = if effective.is_empty() {
                        name.clone()
                    } else {
                        format!("{effective}/{name}")
                    };
                    stack.push((entry.path(), rel));
                }
                continue;
            }
            let rel = if rel_prefix.is_empty() {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                rel_prefix.clone()
            };
            let file = tokio::fs::File::open(&path)
                .await
                .with_context(|| format!("opening local file {}", path.display()))?;
            let stream = tokio_util::io::ReaderStream::new(file);
            let n = fsop::upload_stream(&conn.sftp, dir, &rel, stream).await?;
            total += n;
            count += 1;
        }
        Ok(UploadSummary {
            files: count,
            written: total,
        })
    }

    /// Stream a remote file or directory (as `.tar.gz`) into an absolute local
    /// path, writing to the host FS directly.
    pub async fn download_to_local(&self, id: Uuid, path: &str, dest: &str) -> Result<u64> {
        if !self.allow_local_fs {
            return Err(anyhow!("local path transfers disabled"));
        }
        use futures_util::TryStreamExt as _;
        let conn = self.conn(id)?;
        let md = fsop::stat(&conn.sftp, path).await?;
        let body = if md.is_dir && !md.is_symlink {
            fsop::download_dir_tar_gz(conn.sftp.clone(), path.to_string())
                .await?
                .0
        } else {
            fsop::download_file(conn.sftp.clone(), path.to_string())
                .await?
                .0
        };
        let mut reader = tokio_util::io::StreamReader::new(
            body.into_data_stream().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
        );
        let mut file = tokio::fs::File::create(dest)
            .await
            .with_context(|| format!("creating local file {dest}"))?;
        let written = tokio::io::copy(&mut reader, &mut file)
            .await
            .with_context(|| format!("writing local file {dest}"))?;
        Ok(written)
    }
}

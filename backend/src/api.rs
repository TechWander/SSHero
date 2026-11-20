//! HTTP/JSON API surface exposed to the Vue frontend.

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path as UrlPath, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use anyhow::Context as _;
use dashmap::DashMap;
use futures_util::TryStreamExt as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    fsop,
    sessions::{SessionInput, SessionMeta, SessionStore},
    ssh::{connect_and_open_sftp, now_epoch, AuthMethod, ConnectParams, ManagedConnection, TofuStore},
    ws,
};

pub struct AppState {
    pub conns: DashMap<Uuid, Arc<ManagedConnection>>,
    pub tofu: TofuStore,
    pub token: Option<String>,
    pub sessions: SessionStore,
    pub data_dir: std::path::PathBuf,
    /// local-path transfer endpoints (desktop drag & drop); can be disabled
    pub allow_local_fs: bool,
}

pub type AppResult<T> = Result<T, ApiError>;

/// Error turned into `{"error": "..."}` with a sensible status code.
pub struct ApiError {
    pub status: StatusCode,
    pub msg: String,
}

impl ApiError {
    pub fn internal(e: impl std::fmt::Display) -> Self {
        let msg = e.to_string();
        tracing::warn!("api error: {msg}");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            msg,
        }
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            msg: msg.into(),
        }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            msg: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.msg }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        let msg = format!("{e:#}");
        let status = if msg.contains("authentication failed") || msg.contains("HOST KEY") {
            StatusCode::UNAUTHORIZED
        } else if msg.contains("not found") || msg.contains("No such file") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Self { status, msg }
    }
}

#[derive(Serialize)]
struct ConnectionList {
    connections: Vec<crate::ssh::ConnectionInfo>,
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    path: String,
}

#[derive(Serialize)]
struct ListResponse {
    path: String,
    entries: Vec<fsop::RemoteEntry>,
}

#[derive(Deserialize)]
struct PathBody {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize)]
struct RenameBody {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct UploadQuery {
    /// remote destination directory (absolute or relative to home)
    dir: String,
    /// relative path inside `dir`, may contain subdirectories (folder upload)
    rel: String,
}

fn get_conn(state: &AppState, id: Uuid) -> AppResult<Arc<ManagedConnection>> {
    state
        .conns
        .get(&id)
        .map(|c| c.value().clone())
        .ok_or_else(|| ApiError::not_found(format!("connection {id} not found (is it connected?)")))
}

pub fn router(state: AppState) -> Router {
    let state = Arc::new(state);

    let api = Router::new()
        .route("/connections", get(list_connections).post(create_connection))
        .route("/connections/{id}", delete(close_connection))
        .route("/connections/{id}/reconnect", post(reconnect))
        .route("/connections/{id}/terminal", get(ws::terminal))
        .route("/connections/{id}/upload-path", post(upload_path))
        .route("/connections/{id}/download-to-local", post(download_to_local))
        .route("/sessions", get(list_sessions).post(save_session_new))
        .route("/sessions/{id}", put(save_session).delete(delete_session))
        .route("/sessions/{id}/connect", post(connect_session))
        .route("/connections/{id}/list", get(list_dir))
        .route("/connections/{id}/stat", get(stat_path))
        .route("/connections/{id}/mkdir", post(make_dir))
        .route("/connections/{id}/remove", post(remove_path))
        .route("/connections/{id}/rename", post(rename_path))
        .route("/connections/{id}/download", get(download_file))
        .route("/connections/{id}/download-dir", get(download_dir))
        .route("/connections/{id}/upload", post(upload_file))
        .route("/health", get(health))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            token_check,
        ));

    Router::new()
        .nest("/api", api)
        .with_state(state)
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any)
                .expose_headers([header::CONTENT_DISPOSITION]),
        )
}

async fn token_check(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if let Some(token) = &state.token {
        // header first; WebSocket clients can only pass ?token=
        let header = req
            .headers()
            .get("x-sshero-token")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let query = req.uri().query().unwrap_or("");
        let from_query = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("token="));
        let ok = header == token.as_str()
            || from_query.map(|t| urlencoding::decode(t).map(|d| d == *token).unwrap_or(false)).unwrap_or(false);
        if !ok {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "missing or wrong X-SSHERO-Token"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn list_connections(State(state): State<Arc<AppState>>) -> Json<ConnectionList> {
    let mut v: Vec<_> = state
        .conns
        .iter()
        .map(|e| e.value().info.clone())
        .collect();
    v.sort_by_key(|c| c.connected_at);
    Json(ConnectionList { connections: v })
}

async fn create_connection(
    State(state): State<Arc<AppState>>,
    Json(params): Json<ConnectParams>,
) -> AppResult<Json<crate::ssh::ConnectionInfo>> {
    if params.host.is_empty() || params.username.is_empty() {
        return Err(ApiError::bad_request("host and username are required"));
    }
    if let AuthMethod::Key { key_path, .. } = &params.auth {
        if !std::path::Path::new(key_path).is_file() {
            return Err(ApiError::bad_request(format!(
                "private key file not found on server: {key_path}"
            )));
        }
    }

    let (handle, sftp) = connect_and_open_sftp(&params, state.tofu.clone())
        .await
        .map_err(|e| {
            let msg = format!("{e:#}");
            tracing::warn!("ssh connect failed: {msg}");
            ApiError {
                status: if msg.contains("authentication failed") {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::BAD_GATEWAY
                },
                msg,
            }
        })?;
    let _ = state.tofu.persist_if_dirty();

    let id = Uuid::new_v4();
    let info = crate::ssh::ConnectionInfo {
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
    state.conns.insert(
        id,
        Arc::new(ManagedConnection {
            info: info.clone(),
            params,
            sftp,
            handle,
        }),
    );
    Ok(Json(info))
}

async fn close_connection(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = get_conn(&state, id)?;
    conn.close().await;
    state.conns.remove(&id);
    Ok(Json(json!({ "closed": id.to_string() })))
}

/// Re-establish a session for a known connection (e.g. after a network drop).
/// Credentials live only in backend memory, so this works until a restart.
async fn reconnect(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
) -> AppResult<Json<crate::ssh::ConnectionInfo>> {
    let conn = get_conn(&state, id)?;
    let (handle, sftp) = connect_and_open_sftp(&conn.params, state.tofu.clone())
        .await
        .map_err(|e| ApiError {
            status: StatusCode::BAD_GATEWAY,
            msg: format!("{e:#}"),
        })?;
    let _ = state.tofu.persist_if_dirty();
    let mut info = conn.info.clone();
    info.connected_at = crate::ssh::now_epoch();
    conn.close().await;
    state.conns.insert(
        id,
        Arc::new(ManagedConnection {
            info: info.clone(),
            params: conn.params.clone(),
            sftp,
            handle,
        }),
    );
    Ok(Json(info))
}

async fn list_dir(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<ListResponse>> {
    let conn = get_conn(&state, id)?;
    let (path, entries) = fsop::list_dir(&conn.sftp, &q.path).await?;
    Ok(Json(ListResponse { path, entries }))
}

async fn stat_path(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<fsop::RemoteEntry>> {
    let conn = get_conn(&state, id)?;
    Ok(Json(fsop::stat(&conn.sftp, &q.path).await?))
}

async fn make_dir(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Json(body): Json<PathBody>,
) -> AppResult<StatusCode> {
    let conn = get_conn(&state, id)?;
    fsop::mkdir_p(&conn.sftp, &body.path).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_path(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Json(body): Json<PathBody>,
) -> AppResult<StatusCode> {
    let conn = get_conn(&state, id)?;
    fsop::remove(&conn.sftp, &body.path, body.recursive).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename_path(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Json(body): Json<RenameBody>,
) -> AppResult<StatusCode> {
    let conn = get_conn(&state, id)?;
    fsop::rename(&conn.sftp, &body.from, &body.to).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn content_disposition(filename: &str, extra: &str) -> HeaderMap {
    let mut map = HeaderMap::new();
    let encoded = urlencoding::encode(filename);
    let value = format!("attachment; filename=\"{filename}\"; filename*=UTF-8''{encoded}{extra}");
    if let Ok(v) = HeaderValue::from_str(&value) {
        map.insert(header::CONTENT_DISPOSITION, v);
    }
    map
}

async fn download_file(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Response> {
    let conn = get_conn(&state, id)?;
    let path = q
        .get("path")
        .cloned()
        .ok_or_else(|| ApiError::bad_request("missing ?path"))?;
    let (body, size) = fsop::download_file(conn.sftp.clone(), path.clone()).await?;

    let name = path.rsplit('/').next().unwrap_or("download").to_string();
    let mut headers = content_disposition(&name, "");
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if let Some(sz) = size {
        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&sz.to_string()).unwrap(),
        );
    }
    Ok((headers, body).into_response())
}

async fn download_dir(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Response> {
    let conn = get_conn(&state, id)?;
    let path = q
        .get("path")
        .cloned()
        .ok_or_else(|| ApiError::bad_request("missing ?path"))?;
    let (body, _) = fsop::download_dir_tar_gz(conn.sftp.clone(), path.clone()).await?;

    let name = path.rsplit('/').next().unwrap_or("download").to_string();
    let mut headers = content_disposition(&format!("{name}.tar.gz"), "");
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/gzip"));
    Ok((headers, body).into_response())
}

async fn upload_file(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Query(q): Query<UploadQuery>,
    req: axum::extract::Request,
) -> AppResult<Json<serde_json::Value>> {
    let conn = get_conn(&state, id)?;
    let body = req.into_body().into_data_stream();
    let written = fsop::upload_stream(&conn.sftp, &q.dir, &q.rel, body).await?;
    Ok(Json(json!({ "written": written })))
}

// ---------- saved sessions (sidebar profiles) ----------

async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<Vec<SessionMeta>> {
    Json(state.sessions.list())
}

async fn save_session(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Json(mut input): Json<SessionInput>,
) -> AppResult<Json<SessionMeta>> {
    input.id = Some(id);
    let meta = state
        .sessions
        .upsert(input)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(meta))
}

async fn save_session_new(
    State(state): State<Arc<AppState>>,
    Json(input): Json<SessionInput>,
) -> AppResult<Json<SessionMeta>> {
    let meta = state
        .sessions
        .upsert(input)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(meta))
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
) -> AppResult<StatusCode> {
    state
        .sessions
        .remove(id)
        .map_err(|e| ApiError::internal(e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ConnectSessionBody {
    /// fresh secret when the profile has store_secret=false
    #[serde(default)]
    secret: Option<String>,
}

async fn connect_session(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Json(body): Json<ConnectSessionBody>,
) -> AppResult<Json<crate::ssh::ConnectionInfo>> {
    let session = state
        .sessions
        .get(id)
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let params = session
        .to_connect_params(&state.data_dir, body.secret.as_deref())
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    // reuse the generic connect path
    create_connection(State(state), Json(params)).await
}

// ---------- local-path transfers (desktop shell drag & drop) ----------

#[derive(Deserialize)]
struct UploadPathReq {
    dir: String,
    local_path: String,
}

/// Upload a local file or directory (given by absolute path, as delivered by
/// the Tauri native drag-drop event) into remote `dir`.
async fn upload_path(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Json(req): Json<UploadPathReq>,
) -> AppResult<Json<serde_json::Value>> {
    if !state.allow_local_fs {
        return Err(ApiError::bad_request(
            "local path transfers disabled (start without --disable-local-fs to allow)",
        ));
    }
    let conn = get_conn(&state, id)?;
    let src = std::path::Path::new(&req.local_path);
    if !src.exists() {
        return Err(ApiError::bad_request(format!("local path not found: {}", req.local_path)));
    }
    let mut total = 0u64;
    let mut count = 0u32;
    let mut stack: Vec<(std::path::PathBuf, String)> = vec![(src.to_path_buf(), String::new())];
    while let Some((path, rel_prefix)) = stack.pop() {
        if path.is_dir() {
            // a dragged folder keeps its own name under `dir` and empty
            // subfolders are recreated on the remote so structure survives
            let effective = if rel_prefix.is_empty() {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                rel_prefix.clone()
            };
            if !effective.is_empty() {
                let target = format!("{}/{}", req.dir.trim_end_matches('/'), effective);
                let _ = fsop::mkdir_p(&conn.sftp, &target).await;
            }
            for entry in std::fs::read_dir(&path)
                .with_context(|| format!("reading local dir {}", path.display()))
                .map_err(ApiError::internal)?
            {
                let entry = entry.map_err(ApiError::internal)?;
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
        // a single dragged file keeps its own name
        let rel = if rel_prefix.is_empty() {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            rel_prefix.clone()
        };
        let file = tokio::fs::File::open(&path)
            .await
            .with_context(|| format!("opening local file {}", path.display()))
            .map_err(ApiError::internal)?;
        let stream = tokio_util::io::ReaderStream::new(file);
        let n = fsop::upload_stream(&conn.sftp, &req.dir, &rel, stream)
            .await
            .map_err(ApiError::from)?;
        total += n;
        count += 1;
    }
    Ok(Json(json!({ "files": count, "written": total })))
}

#[derive(Deserialize)]
struct DownloadToLocalReq {
    path: String,
    /// absolute local destination; directories become `<dest>.tar.gz` semantics
    /// are handled by the caller passing the full file name it chose.
    dest: String,
}

async fn download_to_local(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<Uuid>,
    Json(req): Json<DownloadToLocalReq>,
) -> AppResult<Json<serde_json::Value>> {
    if !state.allow_local_fs {
        return Err(ApiError::bad_request("local path transfers disabled"));
    }
    let conn = get_conn(&state, id)?;
    let md = fsop::stat(&conn.sftp, &req.path)
        .await
        .map_err(ApiError::from)?;
    let (body, _) = if md.is_dir && !md.is_symlink {
        fsop::download_dir_tar_gz(conn.sftp.clone(), req.path.clone())
            .await
            .map_err(ApiError::from)?
    } else {
        fsop::download_file(conn.sftp.clone(), req.path.clone())
            .await
            .map_err(ApiError::from)?
    };
    let mut reader =
        tokio_util::io::StreamReader::new(body.into_data_stream().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        }));
    let mut file = tokio::fs::File::create(&req.dest)
        .await
        .with_context(|| format!("creating local file {}", req.dest))
        .map_err(ApiError::internal)?;
    let written = tokio::io::copy(&mut reader, &mut file)
        .await
        .with_context(|| format!("writing local file {}", req.dest))
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "written": written })))
}

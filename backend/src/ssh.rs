//! SSH connection establishment, host-key TOFU verification and the
//! long-lived connection registry (one SFTP session per SSH connection).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{client, ChannelMsg};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Authentication method for a remote SSH server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMethod {
    Password {
        password: String,
    },
    Key {
        /// Absolute path to a private key file on the machine running the backend.
        key_path: String,
        #[serde(default)]
        key_passphrase: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectParams {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
}

pub fn default_port() -> u16 {
    22
}

/// What the API exposes about a connection (secrets stripped).
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInfo {
    pub id: Uuid,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_kind: String,
    /// epoch seconds when the session was established
    pub connected_at: u64,
}

pub struct ManagedConnection {
    pub info: ConnectionInfo,
    pub params: ConnectParams,
    pub sftp: Arc<SftpSession>,
    pub handle: client::Handle<TofuClient>,
}

impl ManagedConnection {
    pub async fn close(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
    }
}

/// Trust-on-first-use host key store, shared between the API and russh handler.
#[derive(Clone)]
pub struct TofuStore {
    inner: Arc<Mutex<HashMap<String, String>>>,
    dirty: Arc<Mutex<bool>>,
    path: Arc<PathBuf>,
}

impl TofuStore {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = Arc::new(path.into());
        let map = read_known_hosts(&path).unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(map)),
            dirty: Arc::new(Mutex::new(false)),
            path,
        }
    }

    fn lookup(&self, host_port: &str) -> Option<String> {
        self.inner.lock().unwrap().get(host_port).cloned()
    }

    fn record_if_unknown(&self, host_port: &str, openssh_line: &str) -> bool {
        let mut map = self.inner.lock().unwrap();
        match map.get(host_port) {
            Some(existing) => existing == openssh_line,
            None => {
                map.insert(host_port.to_string(), openssh_line.to_string());
                *self.dirty.lock().unwrap() = true;
                false // unknown key (accepted, must be persisted)
            }
        }
    }

    pub fn persist_if_dirty(&self) -> Result<()> {
        let mut dirty = self.dirty.lock().unwrap();
        if !*dirty {
            return Ok(());
        }
        let snapshot = self.inner.lock().unwrap().clone();
        write_known_hosts(&self.path, &snapshot)?;
        *dirty = false;
        Ok(())
    }
}

fn read_known_hosts(path: &Path) -> Option<HashMap<String, String>> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

fn write_known_hosts(path: &Path, map: &HashMap<String, String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let json = serde_json::to_vec_pretty(map)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// russh client handler performing TOFU host key verification.
pub struct TofuClient {
    host_port: String,
    store: TofuStore,
}

impl client::Handler for TofuClient {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let line = server_public_key
            .public_key()
            .to_openssh()
            .context("rendering server host key to openssh format")?;
        if let Some(expected) = self.store.lookup(&self.host_port) {
            if expected != line {
                tracing::error!(
                    "HOST KEY CHANGED for {} – refusing connection",
                    self.host_port
                );
                return Ok(false);
            }
            return Ok(true);
        }
        // first contact: accept and remember
        self.store.record_if_unknown(&self.host_port, &line);
        tracing::info!("trusting new host key for {}", self.host_port);
        Ok(true)
    }
}

/// Connect to a remote SSH server and open a persistent SFTP session on it.
pub async fn connect_and_open_sftp(
    params: &ConnectParams,
    store: TofuStore,
) -> Result<(client::Handle<TofuClient>, Arc<SftpSession>)> {
    let config = client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 6,
        ..Default::default()
    };
    let host_port = format!("{}:{}", params.host, params.port);
    let handler = TofuClient {
        host_port: host_port.clone(),
        store,
    };

    let mut handle = client::connect(
        Arc::new(config),
        (params.host.as_str(), params.port),
        handler,
    )
    .await
    .with_context(|| format!("tcp connect / kex with {host_port} failed"))?;

    let ok = match &params.auth {
        AuthMethod::Password { password } => handle
            .authenticate_password(params.username.clone(), password.clone())
            .await
            .context("password authentication errored")?
            .success(),
        AuthMethod::Key {
            key_path,
            key_passphrase,
        } => {
            let key = load_secret_key(key_path, key_passphrase.as_deref())
                .with_context(|| format!("reading private key {key_path}"))?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .context("negotiating rsa hash alg")?
                .flatten();
            let pk = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
            handle
                .authenticate_publickey(params.username.clone(), pk)
                .await
                .context("publickey authentication errored")?
                .success()
        }
    };
    if !ok {
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "auth failed", "en")
            .await;
        bail!(
            "authentication failed for {}@{host_port}",
            params.username
        );
    }

    let channel = handle
        .channel_open_session()
        .await
        .context("opening session channel")?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .context("requesting sftp subsystem (is it enabled on the server?)")?;
    let session = SftpSession::new(channel.into_stream())
        .await
        .context("initializing sftp session")?;
    Ok((handle, Arc::new(session)))
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Keep the SFTP session alive: russh channels die silently when idle and
/// disconnected; a cheap op surfaces that state early.
#[allow(dead_code)]
pub async fn probe(sftp: &SftpSession) -> bool {
    sftp.canonicalize(".").await.is_ok()
}

/// Consume a `ChannelMsg` receiver until the channel is reported closed, used
/// by tests/housekeeping when needed.
#[allow(dead_code)]
pub async fn drain_channel(mut rx: tokio::sync::mpsc::Receiver<ChannelMsg>) {
    while rx.recv().await.is_some() {}
}

//! Persisted SSH session profiles (the left sidebar's data source).
//!
//! Stored as JSON in `<data-dir>/sessions.json` with 0600 permissions.
//! Secrets (passwords / key passphrases / pasted key material) are kept
//! server-side only: the API never echoes them back to the client.

use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ssh::{AuthMethod, ConnectParams};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// "password" | "key-path" | "key-content"
    pub auth_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    /// key material pasted by the user; persisted to <data-dir>/keys/<id>.pem
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_passphrase: Option<String>,
    /// when false the profile can be listed but connect() must supply it
    #[serde(default = "default_true")]
    pub store_secret: bool,
}

fn default_true() -> bool {
    true
}

/// Client-facing shape: secrets masked to presence flags.
#[derive(Debug, Serialize)]
pub struct SessionMeta {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_kind: String,
    pub has_password: bool,
    pub has_key_passphrase: bool,
    pub has_key: bool,
    pub store_secret: bool,
}

impl Session {
    pub fn to_meta(&self) -> SessionMeta {
        SessionMeta {
            id: self.id,
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            auth_kind: self.auth_kind.clone(),
            has_password: self.password.is_some(),
            has_key_passphrase: self.key_passphrase.is_some(),
            has_key: self.key_path.is_some() || self.key_content.is_some(),
            store_secret: self.store_secret,
        }
    }

    /// Resolve into live connection parameters. `secret_override` supplies a
    /// freshly typed password/passphrase when the profile did not store one.
    pub fn to_connect_params(
        &self,
        data_dir: &PathBuf,
        secret_override: Option<&str>,
    ) -> Result<ConnectParams> {
        let auth = match self.auth_kind.as_str() {
            "password" => {
                let pw = self
                    .password
                    .as_deref()
                    .or(secret_override)
                    .ok_or_else(|| anyhow!("no password stored for this session"))?;
                AuthMethod::Password {
                    password: pw.to_string(),
                }
            }
            "key-path" => AuthMethod::Key {
                key_path: self
                    .key_path
                    .clone()
                    .ok_or_else(|| anyhow!("session has no key path"))?,
                key_passphrase: self.key_passphrase.clone().or(secret_override.map(String::from)),
            },
            "key-content" => {
                // materialize pasted key to a 0600 file the ssh layer can read
                let dir = data_dir.join("keys");
                std::fs::create_dir_all(&dir).context("creating keys dir")?;
                let file = dir.join(format!("{}.pem", self.id));
                let content = self
                    .key_content
                    .as_deref()
                    .ok_or_else(|| anyhow!("session has no key material"))?;
                std::fs::write(&file, content).context("writing key file")?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
                }
                AuthMethod::Key {
                    key_path: file.to_string_lossy().into_owned(),
                    key_passphrase: self
                        .key_passphrase
                        .clone()
                        .or(secret_override.map(String::from)),
                }
            }
            other => return Err(anyhow!("unknown auth kind {other:?}")),
        };
        Ok(ConnectParams {
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            auth,
        })
    }
}

/// Fields accepted on create/update; omitted secret fields keep existing
/// values on update (so editing a session doesn't wipe the stored password).
#[derive(Debug, Deserialize)]
pub struct SessionInput {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub name: String,
    pub host: String,
    #[serde(default = "crate::ssh::default_port")]
    pub port: u16,
    pub username: String,
    pub auth_kind: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub key_content: Option<String>,
    #[serde(default)]
    pub key_passphrase: Option<String>,
    #[serde(default = "default_true")]
    pub store_secret: bool,
}

pub struct SessionStore {
    inner: Mutex<HashMap<Uuid, Session>>,
    path: PathBuf,
}

impl SessionStore {
    pub fn load(path: PathBuf) -> Self {
        let map = std::fs::read(&path)
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(map),
            path,
        }
    }

    fn persist(&self) -> Result<()> {
        let snapshot = self.inner.lock().unwrap().clone();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let json = serde_json::to_vec_pretty(&snapshot)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<SessionMeta> {
        let mut v: Vec<_> = self
            .inner
            .lock()
            .unwrap()
            .values()
            .map(|s| s.to_meta())
            .collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        v
    }

    pub fn get(&self, id: Uuid) -> Option<Session> {
        self.inner.lock().unwrap().get(&id).cloned()
    }

    pub fn upsert(&self, input: SessionInput) -> Result<SessionMeta> {
        if input.host.trim().is_empty() || input.username.trim().is_empty() {
            return Err(anyhow!("host and username are required"));
        }
        if !matches!(input.auth_kind.as_str(), "password" | "key-path" | "key-content") {
            return Err(anyhow!(
                "auth_kind must be password | key-path | key-content"
            ));
        }
        let id = input.id.unwrap_or_else(Uuid::new_v4);
        let mut map = self.inner.lock().unwrap();
        let mut session = map
            .get(&id)
            .cloned()
            .unwrap_or(Session {
                id,
                name: String::new(),
                host: String::new(),
                port: 22,
                username: String::new(),
                auth_kind: String::new(),
                password: None,
                key_path: None,
                key_content: None,
                key_passphrase: None,
                store_secret: true,
            });
        session.name = input.name.trim().to_string();
        session.host = input.host.trim().to_string();
        session.port = input.port;
        session.username = input.username.trim().to_string();
        session.auth_kind = input.auth_kind.clone();
        session.store_secret = input.store_secret;
        // only overwrite secrets when new values arrive
        if let Some(p) = input.password.filter(|s| !s.is_empty()) {
            session.password = Some(p);
        }
        if let Some(kp) = input.key_path.filter(|s| !s.trim().is_empty()) {
            session.key_path = Some(kp.trim().to_string());
        }
        if let Some(kc) = input.key_content.filter(|s| !s.is_empty()) {
            session.key_content = Some(kc);
        }
        if let Some(kpass) = input.key_passphrase.filter(|s| !s.is_empty()) {
            session.key_passphrase = Some(kpass);
        }
        if !input.store_secret {
            session.password = None;
            session.key_passphrase = None;
        }
        // auth kind switch clears fields that no longer apply
        match session.auth_kind.as_str() {
            "password" => {
                session.key_path = None;
                session.key_content = None;
                session.key_passphrase = None;
            }
            "key-path" => {
                session.key_content = None;
                session.password = None;
            }
            "key-content" => {
                session.key_path = None;
                session.password = None;
            }
            _ => {}
        }
        let meta = session.to_meta();
        map.insert(id, session);
        drop(map);
        self.persist()?;
        Ok(meta)
    }

    pub fn remove(&self, id: Uuid) -> Result<bool> {
        let existed = self.inner.lock().unwrap().remove(&id).is_some();
        if existed {
            self.persist()?;
        }
        Ok(existed)
    }
}

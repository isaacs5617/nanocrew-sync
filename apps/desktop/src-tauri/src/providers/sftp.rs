use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use russh::{client, keys::decode_secret_key};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{CloudProvider, CompletedPart, FileStat, ListDirResult, ProviderError};

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SftpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SftpAuth,
    pub root_path: String,
    pub known_host_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SftpAuth {
    Password(String),
    PrivateKey { key_pem: String, passphrase: Option<String> },
}

// ── TOFU handler ──────────────────────────────────────────────────────────────

struct TofuHandler {
    expected_fingerprint: Option<String>,
    observed_fingerprint: Arc<Mutex<Option<String>>>,
}

impl client::Handler for TofuHandler {
    type Error = ProviderError;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fp = server_public_key
            .fingerprint(Default::default())
            .to_string();

        match &self.expected_fingerprint {
            None => {
                *self.observed_fingerprint.lock().await = Some(fp);
                Ok(true)
            }
            Some(expected) if expected == &fp => Ok(true),
            Some(expected) => Err(ProviderError::Other(format!(
                "Host key changed — expected {expected}, got {fp}"
            ))),
        }
    }
}

// ── Pool ──────────────────────────────────────────────────────────────────────

const POOL_MAX: usize = 4;

struct Session {
    sftp: SftpSession,
}

pub struct SftpProvider {
    config: SftpConfig,
    pool: Arc<Mutex<Vec<Session>>>,
}

impl SftpProvider {
    pub async fn new(config: SftpConfig) -> Result<Self, ProviderError> {
        Ok(Self {
            config,
            pool: Arc::new(Mutex::new(Vec::with_capacity(POOL_MAX))),
        })
    }

    async fn acquire(&self) -> Result<Session, ProviderError> {
        {
            let mut guard = self.pool.lock().await;
            if let Some(s) = guard.pop() {
                return Ok(s);
            }
        }
        self.connect().await
    }

    fn release(&self, session: Session) {
        let pool = Arc::clone(&self.pool);
        tokio::spawn(async move {
            let mut guard = pool.lock().await;
            if guard.len() < POOL_MAX {
                guard.push(session);
            }
        });
    }

    async fn connect(&self) -> Result<Session, ProviderError> {
        let observed = Arc::new(Mutex::new(None::<String>));
        let handler = TofuHandler {
            expected_fingerprint: self.config.known_host_fingerprint.clone(),
            observed_fingerprint: Arc::clone(&observed),
        };

        let ssh_config = Arc::new(client::Config::default());
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let mut handle = client::connect(ssh_config, addr, handler)
            .await
            .map_err(|e| ProviderError::Other(format!("SSH connect: {e}")))?;

        match &self.config.auth {
            SftpAuth::Password(pw) => {
                let result = handle
                    .authenticate_password(&self.config.username, pw)
                    .await
                    .map_err(|e| ProviderError::Other(format!("SSH auth: {e}")))?;
                if !result.success() {
                    return Err(ProviderError::Other("SSH password authentication failed".into()));
                }
            }
            SftpAuth::PrivateKey { key_pem, passphrase } => {
                let key = decode_secret_key(key_pem, passphrase.as_deref())
                    .map_err(|e| ProviderError::Other(format!("SSH key decode: {e}")))?;
                let key_with_alg = russh::keys::key::PrivateKeyWithHashAlg::new(
                    Arc::new(key),
                    None,
                );
                let result = handle
                    .authenticate_publickey(&self.config.username, key_with_alg)
                    .await
                    .map_err(|e| ProviderError::Other(format!("SSH auth: {e}")))?;
                if !result.success() {
                    return Err(ProviderError::Other("SSH key authentication failed".into()));
                }
            }
        }

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| ProviderError::Other(format!("SSH channel: {e}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| ProviderError::Other(format!("SFTP subsystem: {e}")))?;

        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| ProviderError::Other(format!("SFTP session init: {e}")))?;

        Ok(Session { sftp })
    }

    fn abs_path(&self, rel_key: &str) -> String {
        let root = self.config.root_path.trim_end_matches('/');
        if rel_key.is_empty() {
            root.to_string()
        } else {
            format!("{}/{}", root, rel_key)
        }
    }
}

// ── Multipart accumulator ─────────────────────────────────────────────────────

static SFTP_PARTS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Vec<(i32, Bytes)>>>> =
    std::sync::OnceLock::new();

fn parts_map() -> &'static std::sync::Mutex<HashMap<String, Vec<(i32, Bytes)>>> {
    SFTP_PARTS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn upload_key(key: &str, upload_id: &str) -> String {
    format!("{}\x00{}", key, upload_id)
}

// ── CloudProvider ─────────────────────────────────────────────────────────────

#[async_trait]
impl CloudProvider for SftpProvider {
    async fn list_dir(&self, prefix: &str) -> Result<ListDirResult, ProviderError> {
        let dir_path = self.abs_path(prefix);
        let session = self.acquire().await?;

        let entries = session
            .sftp
            .read_dir(&dir_path)
            .await
            .map_err(|e| ProviderError::Other(format!("readdir {dir_path:?}: {e}")))?;

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let stat = entry.metadata();
            if stat.is_dir() {
                dirs.push(name.to_string());
            } else {
                let size = stat.size.unwrap_or(0);
                let mtime_filetime = stat
                    .mtime
                    .map(|s| unix_secs_to_filetime(s as i64))
                    .unwrap_or(0);
                files.push((name.to_string(), FileStat { size, mtime_filetime }));
            }
        }

        self.release(session);
        Ok(ListDirResult { dirs, files })
    }

    async fn stat(&self, key: &str) -> Result<Option<FileStat>, ProviderError> {
        let path = self.abs_path(key);
        let session = self.acquire().await?;

        let meta = match session.sftp.metadata(&path).await {
            Ok(m) => m,
            Err(e) => {
                self.release(session);
                let msg = e.to_string();
                if msg.contains("No such file")
                    || msg.contains("not found")
                    || msg.contains("ENOENT")
                    || msg.contains("no such")
                {
                    return Ok(None);
                }
                return Err(ProviderError::Other(format!("lstat {path:?}: {msg}")));
            }
        };

        self.release(session);
        let size = meta.size.unwrap_or(0);
        let mtime_filetime = meta.mtime.map(|s| unix_secs_to_filetime(s as i64)).unwrap_or(0);
        Ok(Some(FileStat { size, mtime_filetime }))
    }

    async fn get_range(&self, key: &str, offset: u64, length: u64) -> Result<Bytes, ProviderError> {
        if length == 0 {
            return Ok(Bytes::new());
        }
        let path = self.abs_path(key);
        let session = self.acquire().await?;

        // Read the whole file then slice — russh-sftp 2.x read() returns all bytes.
        let full = session
            .sftp
            .read(&path)
            .await
            .map_err(|e| ProviderError::Other(format!("sftp read {path:?}: {e}")))?;

        self.release(session);

        let start = offset as usize;
        let end = (offset + length) as usize;
        let end = end.min(full.len());
        if start >= full.len() {
            return Ok(Bytes::new());
        }
        Ok(Bytes::copy_from_slice(&full[start..end]))
    }

    async fn put_object(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        let path = self.abs_path(key);
        let session = self.acquire().await?;

        // Ensure parent exists best-effort.
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = session.sftp.create_dir(parent.to_string_lossy().as_ref()).await;
        }

        session
            .sftp
            .write(&path, &data)
            .await
            .map_err(|e| ProviderError::Other(format!("sftp write {path:?}: {e}")))?;

        self.release(session);
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ProviderError> {
        let path = self.abs_path(key);
        let session = self.acquire().await?;
        match session.sftp.remove_file(&path).await {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if !msg.contains("No such file")
                    && !msg.contains("ENOENT")
                    && !msg.contains("no such")
                {
                    self.release(session);
                    return Err(ProviderError::Other(format!("sftp remove {path:?}: {msg}")));
                }
            }
        }
        self.release(session);
        Ok(())
    }

    async fn copy_object(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        let stat = self.stat(from).await?.unwrap_or(FileStat { size: 0, mtime_filetime: 0 });
        let data = self.get_range(from, 0, stat.size).await?;
        self.put_object(to, data).await
    }

    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, ProviderError> {
        let abs_dir = self.abs_path(prefix);
        let mut result = Vec::new();
        self.walk_dir_boxed(abs_dir, prefix.to_string(), &mut result).await?;
        Ok(result)
    }

    async fn create_multipart(&self, key: &str) -> Result<String, ProviderError> {
        let upload_id = uuid::Uuid::new_v4().to_string();
        parts_map()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(upload_key(key, &upload_id), Vec::new());
        Ok(upload_id)
    }

    async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: i32,
        data: Bytes,
    ) -> Result<String, ProviderError> {
        let etag = format!("{:x}", part_number);
        parts_map()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(upload_key(key, upload_id))
            .or_default()
            .push((part_number, data));
        Ok(etag)
    }

    async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        _parts: Vec<CompletedPart>,
    ) -> Result<(), ProviderError> {
        let mut parts = parts_map()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&upload_key(key, upload_id))
            .unwrap_or_default();
        parts.sort_by_key(|(n, _)| *n);
        let total: usize = parts.iter().map(|(_, b)| b.len()).sum();
        let mut combined = bytes::BytesMut::with_capacity(total);
        for (_, chunk) in parts {
            combined.extend_from_slice(&chunk);
        }
        self.put_object(key, combined.freeze()).await
    }

    async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), ProviderError> {
        parts_map()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&upload_key(key, upload_id));
        Ok(())
    }
}

impl SftpProvider {
    fn walk_dir_boxed<'a>(
        &'a self,
        abs_dir: String,
        rel_prefix: String,
        result: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let session = self.acquire().await?;
            let entries = match session.sftp.read_dir(&abs_dir).await {
                Ok(e) => {
                    self.release(session);
                    e
                }
                Err(_) => {
                    self.release(session);
                    return Ok(());
                }
            };
            for entry in entries {
                let name = entry.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let child_rel = if rel_prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{}/{}", rel_prefix, name)
                };
                let child_abs = format!("{}/{}", abs_dir, name);
                if entry.metadata().is_dir() {
                    self.walk_dir_boxed(child_abs, child_rel, result).await?;
                } else {
                    result.push(child_rel);
                }
            }
            Ok(())
        })
    }
}

fn unix_secs_to_filetime(secs: i64) -> u64 {
    let s = secs.max(0) as u64 + 11_644_473_600;
    s * 10_000_000
}

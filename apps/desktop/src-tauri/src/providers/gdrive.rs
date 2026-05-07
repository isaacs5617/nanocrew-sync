//! Google Drive implementation of [`CloudProvider`] via the Drive REST API v3.
//!
//! Authentication uses the OAuth2 PKCE flow (loopback redirect). The refresh
//! token is stored in the drive row's `provider_config` JSON column. Access
//! tokens are cached in-memory and refreshed automatically before each API
//! call.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{CloudProvider, CompletedPart, FileStat, ListDirResult, ProviderError};

// ── Google OAuth2 constants ───────────────────────────────────────────────────

// Register this application in Google Cloud Console and paste the client ID
// here. The redirect URI must be added as an allowed loopback redirect in the
// OAuth 2.0 client configuration (Google allows http://127.0.0.1 loopback
// for desktop apps without listing every port).
pub const GOOGLE_CLIENT_ID: &str = "YOUR_GOOGLE_CLIENT_ID";

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";

const GOOGLE_FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const GOOGLE_WORKSPACE_PREFIX: &str = "application/vnd.google-apps.";

const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

// Files smaller than this are uploaded with the simple multipart path; larger
// files use a resumable session.
const RESUMABLE_THRESHOLD: usize = 5 * 1024 * 1024;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GDriveConfig {
    /// Google OAuth2 client ID baked into the app (see `GOOGLE_CLIENT_ID`).
    pub client_id: String,
    /// Long-lived credential obtained during the PKCE flow. Stored in the
    /// drive row's `provider_config` JSON column (not in the credentials table,
    /// because DPAPI is Windows-only and provider_config already lives in the
    /// same encrypted SQLite database).
    pub refresh_token: String,
    /// Google Drive folder ID to use as the VFS root. Use `"root"` for the
    /// user's My Drive root, or any specific folder ID.
    pub root_folder_id: String,
}

// ── Token state ───────────────────────────────────────────────────────────────

struct TokenState {
    access_token: String,
    expires_at: Instant,
}

impl TokenState {
    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

// ── GDriveProvider ────────────────────────────────────────────────────────────

pub struct GDriveProvider {
    client: reqwest::Client,
    config: GDriveConfig,
    token_state: Arc<Mutex<TokenState>>,
    /// Maps VFS path strings (e.g. `"folder/subfolder/file.txt"`) to the
    /// Google Drive file ID for that entry. Cache misses trigger a component-
    /// by-component API walk. Invalidated on write operations.
    path_cache: Arc<Mutex<HashMap<String, String>>>,
}

impl GDriveProvider {
    pub fn new(config: GDriveConfig, initial_access_token: String, expires_in_secs: u64) -> Self {
        let expires_at = Instant::now() + Duration::from_secs(expires_in_secs.saturating_sub(60));
        Self {
            client: reqwest::Client::new(),
            config,
            token_state: Arc::new(Mutex::new(TokenState {
                access_token: initial_access_token,
                expires_at,
            })),
            path_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn access_token(&self) -> Result<String, ProviderError> {
        let mut state = self.token_state.lock().await;
        if !state.is_expired() {
            return Ok(state.access_token.clone());
        }
        let (token, expires_in) = refresh_access_token(
            &self.client,
            &self.config.client_id,
            &self.config.refresh_token,
        )
        .await?;
        state.access_token = token.clone();
        state.expires_at = Instant::now() + Duration::from_secs(expires_in.saturating_sub(60));
        Ok(token)
    }

    /// Resolve a VFS-relative path to a Google Drive file ID.
    ///
    /// `""` maps to `config.root_folder_id`. All other paths are resolved
    /// component by component, with results cached between calls.
    async fn resolve_id(&self, path: &str) -> Result<Option<String>, ProviderError> {
        if path.is_empty() {
            return Ok(Some(self.config.root_folder_id.clone()));
        }

        {
            let cache = self.path_cache.lock().await;
            if let Some(id) = cache.get(path) {
                return Ok(Some(id.clone()));
            }
        }

        let components: Vec<&str> = path.split('/').collect();
        let mut parent_id = self.config.root_folder_id.clone();

        for (i, component) in components.iter().enumerate() {
            let token = self.access_token().await?;
            let q = format!(
                "'{}' in parents and name = '{}' and trashed = false",
                parent_id,
                component.replace('\'', "\\'")
            );
            let resp = self
                .client
                .get(format!("{}/files", DRIVE_API_BASE))
                .bearer_auth(&token)
                .query(&[
                    ("q", q.as_str()),
                    ("fields", "files(id,name)"),
                    ("pageSize", "1"),
                ])
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive list query: {e}")))?;

            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive list decode: {e}")))?;

            let files = body["files"].as_array().cloned().unwrap_or_default();
            if files.is_empty() {
                return Ok(None);
            }
            let file_id = files[0]["id"]
                .as_str()
                .unwrap_or("")
                .to_string();

            // Cache each prefix we successfully resolve.
            let prefix_path = components[..=i].join("/");
            self.path_cache.lock().await.insert(prefix_path, file_id.clone());
            parent_id = file_id;
        }

        Ok(Some(parent_id))
    }

    /// Resolve a path to its parent folder ID and the filename component.
    async fn resolve_parent(&self, path: &str) -> Result<(String, String), ProviderError> {
        match path.rfind('/') {
            None => {
                Ok((self.config.root_folder_id.clone(), path.to_string()))
            }
            Some(slash) => {
                let parent_path = &path[..slash];
                let name = path[slash + 1..].to_string();
                let parent_id = self
                    .resolve_id(parent_path)
                    .await?
                    .ok_or_else(|| ProviderError::NotFound(format!("parent not found: {parent_path}")))?;
                Ok((parent_id, name))
            }
        }
    }

    fn invalidate_cache_for(&self, path: &str) {
        let path = path.to_string();
        let cache = Arc::clone(&self.path_cache);
        tokio::spawn(async move {
            let mut guard = cache.lock().await;
            guard.remove(&path);
            // Also remove all children.
            let prefix = format!("{}/", path);
            guard.retain(|k, _| !k.starts_with(&prefix));
        });
    }
}

// ── Token helpers ─────────────────────────────────────────────────────────────

/// Exchange a refresh token for a new access token.
/// Returns `(access_token, expires_in_seconds)`.
async fn refresh_access_token(
    client: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<(String, u64), ProviderError> {
    let params = [
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| ProviderError::Other(format!("token refresh request: {e}")))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ProviderError::Other(format!("token refresh decode: {e}")))?;
    let access_token = body["access_token"]
        .as_str()
        .ok_or_else(|| ProviderError::Other("token refresh: missing access_token".into()))?
        .to_string();
    let expires_in = body["expires_in"].as_u64().unwrap_or(3600);
    Ok((access_token, expires_in))
}

// ── ISO 8601 → Windows FILETIME ───────────────────────────────────────────────

fn iso8601_to_filetime(s: &str) -> u64 {
    let Ok(dt) = s.parse::<chrono_lite::DateTime>() else {
        return 0;
    };
    let unix_secs = dt.unix_timestamp().max(0) as u64;
    (unix_secs + 11_644_473_600) * 10_000_000
}

/// Minimal ISO 8601 parser that avoids pulling in a heavy date crate.
/// Supports the subset Google Drive uses: `2024-03-15T10:30:00.000Z`.
mod chrono_lite {
    pub struct DateTime {
        unix_timestamp: i64,
    }

    impl DateTime {
        pub fn unix_timestamp(&self) -> i64 {
            self.unix_timestamp
        }
    }

    impl std::str::FromStr for DateTime {
        type Err = ();

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let s = s.trim_end_matches('Z');
            let (date_part, time_part) = s.split_once('T').ok_or(())?;
            let date_parts: Vec<&str> = date_part.split('-').collect();
            if date_parts.len() != 3 {
                return Err(());
            }
            let year: i64 = date_parts[0].parse().map_err(|_| ())?;
            let month: i64 = date_parts[1].parse().map_err(|_| ())?;
            let day: i64 = date_parts[2].parse().map_err(|_| ())?;

            let time_base = time_part.split('.').next().unwrap_or(time_part);
            let time_parts: Vec<&str> = time_base.split(':').collect();
            if time_parts.len() != 3 {
                return Err(());
            }
            let hour: i64 = time_parts[0].parse().map_err(|_| ())?;
            let min: i64 = time_parts[1].parse().map_err(|_| ())?;
            let sec: i64 = time_parts[2].parse().map_err(|_| ())?;

            // Days since Unix epoch using the Julian Day formula.
            let y = if month <= 2 { year - 1 } else { year };
            let m = if month <= 2 { month + 12 } else { month };
            let jdn = 365 * y + y / 4 - y / 100 + y / 400
                + (153 * m + 2) / 5
                + day
                + 1_721_119;
            let unix_epoch_jdn = 2_440_588i64;
            let days = jdn - unix_epoch_jdn;
            let unix_timestamp = days * 86_400 + hour * 3_600 + min * 60 + sec;
            Ok(Self { unix_timestamp })
        }
    }
}

// ── Multipart accumulator (same pattern as SFTP/FTP providers) ────────────────

static GDRIVE_PARTS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Vec<(i32, Bytes)>>>> =
    std::sync::OnceLock::new();

fn parts_map() -> &'static std::sync::Mutex<HashMap<String, Vec<(i32, Bytes)>>> {
    GDRIVE_PARTS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn upload_key(key: &str, upload_id: &str) -> String {
    format!("{}\x00{}", key, upload_id)
}

// ── CloudProvider ─────────────────────────────────────────────────────────────

#[async_trait]
impl CloudProvider for GDriveProvider {
    async fn list_dir(&self, prefix: &str) -> Result<ListDirResult, ProviderError> {
        let parent_id = match self.resolve_id(prefix).await? {
            Some(id) => id,
            None => return Ok(ListDirResult { dirs: vec![], files: vec![] }),
        };

        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<(String, FileStat)> = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let token = self.access_token().await?;
            let q = format!("'{}' in parents and trashed = false", parent_id);
            let mut query = vec![
                ("q", q.clone()),
                ("fields", "nextPageToken,files(id,name,size,modifiedTime,mimeType)".to_string()),
                ("pageSize", "1000".to_string()),
            ];
            if let Some(ref pt) = page_token {
                query.push(("pageToken", pt.clone()));
            }

            let resp = self
                .client
                .get(format!("{}/files", DRIVE_API_BASE))
                .bearer_auth(&token)
                .query(&query)
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive list_dir: {e}")))?;

            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive list_dir decode: {e}")))?;

            if let Some(items) = body["files"].as_array() {
                for item in items {
                    let name = item["name"].as_str().unwrap_or("").to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let mime = item["mimeType"].as_str().unwrap_or("");
                    let id = item["id"].as_str().unwrap_or("").to_string();

                    // Cache path for this entry.
                    let child_path = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", prefix, name)
                    };
                    self.path_cache.lock().await.insert(child_path, id.clone());

                    if mime == GOOGLE_FOLDER_MIME {
                        dirs.push(name);
                    } else if mime.starts_with(GOOGLE_WORKSPACE_PREFIX) {
                        // Google Workspace files (Docs, Sheets, Slides, …) —
                        // expose as zero-byte read-only placeholders so they
                        // appear in directory listings but cannot be written to.
                        files.push((name, FileStat { size: 0, mtime_filetime: 0 }));
                    } else {
                        let size = item["size"]
                            .as_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(0);
                        let mtime_filetime = item["modifiedTime"]
                            .as_str()
                            .map(iso8601_to_filetime)
                            .unwrap_or(0);
                        files.push((name, FileStat { size, mtime_filetime }));
                    }
                }
            }

            match body["nextPageToken"].as_str() {
                Some(pt) => page_token = Some(pt.to_string()),
                None => break,
            }
        }

        Ok(ListDirResult { dirs, files })
    }

    async fn stat(&self, key: &str) -> Result<Option<FileStat>, ProviderError> {
        let Some(id) = self.resolve_id(key).await? else {
            return Ok(None);
        };
        let token = self.access_token().await?;
        let resp = self
            .client
            .get(format!("{}/files/{}", DRIVE_API_BASE, id))
            .bearer_auth(&token)
            .query(&[("fields", "size,modifiedTime,mimeType")])
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("gdrive stat: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("gdrive stat decode: {e}")))?;

        let mime = body["mimeType"].as_str().unwrap_or("");
        let size = if mime.starts_with(GOOGLE_WORKSPACE_PREFIX) {
            0
        } else {
            body["size"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        };
        let mtime_filetime = body["modifiedTime"]
            .as_str()
            .map(iso8601_to_filetime)
            .unwrap_or(0);
        Ok(Some(FileStat { size, mtime_filetime }))
    }

    async fn get_range(&self, key: &str, offset: u64, length: u64) -> Result<Bytes, ProviderError> {
        if length == 0 {
            return Ok(Bytes::new());
        }
        let Some(id) = self.resolve_id(key).await? else {
            return Err(ProviderError::NotFound(format!("not found: {key}")));
        };
        let token = self.access_token().await?;
        let end = offset + length - 1;
        let resp = self
            .client
            .get(format!("{}/files/{}?alt=media", DRIVE_API_BASE, id))
            .bearer_auth(&token)
            .header("Range", format!("bytes={}-{}", offset, end))
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("gdrive get_range: {e}")))?;

        let status = resp.status();
        if !status.is_success() && status.as_u16() != 206 {
            return Err(ProviderError::Other(format!("gdrive get_range HTTP {status}")));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Other(format!("gdrive get_range body: {e}")))?;
        Ok(bytes)
    }

    async fn put_object(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        let (parent_id, name) = self.resolve_parent(key).await?;

        // If the file already exists, patch it (update content) rather than
        // creating a duplicate entry.
        if let Some(existing_id) = self.resolve_id(key).await? {
            let token = self.access_token().await?;
            let resp = self
                .client
                .patch(format!("{}/files/{}?uploadType=media", UPLOAD_API_BASE, existing_id))
                .bearer_auth(&token)
                .header("Content-Type", "application/octet-stream")
                .body(data)
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive put_object update: {e}")))?;
            if !resp.status().is_success() {
                let status = resp.status();
                return Err(ProviderError::Other(format!("gdrive put_object update HTTP {status}")));
            }
            self.invalidate_cache_for(key);
            return Ok(());
        }

        let token = self.access_token().await?;

        if data.len() < RESUMABLE_THRESHOLD {
            // Simple multipart upload.
            let metadata = serde_json::json!({
                "name": name,
                "parents": [parent_id],
            })
            .to_string();
            let boundary = "NanoCrew_Sync_Boundary";
            let body = format!(
                "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n"
            );
            let mut body_bytes = body.into_bytes();
            body_bytes.extend_from_slice(&data);
            body_bytes.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());

            let resp = self
                .client
                .post(format!("{}/files?uploadType=multipart", UPLOAD_API_BASE))
                .bearer_auth(&token)
                .header(
                    "Content-Type",
                    format!("multipart/related; boundary={boundary}"),
                )
                .body(body_bytes)
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive put_object multipart: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                return Err(ProviderError::Other(format!("gdrive put_object multipart HTTP {status}")));
            }
            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive put_object decode: {e}")))?;
            if let Some(id) = body["id"].as_str() {
                self.path_cache.lock().await.insert(key.to_string(), id.to_string());
            }
        } else {
            // Resumable upload for larger files.
            let metadata = serde_json::json!({
                "name": name,
                "parents": [parent_id],
            });
            let init_resp = self
                .client
                .post(format!("{}/files?uploadType=resumable", UPLOAD_API_BASE))
                .bearer_auth(&token)
                .header("X-Upload-Content-Type", "application/octet-stream")
                .header("X-Upload-Content-Length", data.len().to_string())
                .json(&metadata)
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive resumable init: {e}")))?;

            let upload_url = init_resp
                .headers()
                .get("Location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| ProviderError::Other("gdrive resumable: no Location header".into()))?
                .to_string();

            let len = data.len();
            let resp = self
                .client
                .put(&upload_url)
                .header("Content-Type", "application/octet-stream")
                .header("Content-Range", format!("bytes 0-{}/{}", len - 1, len))
                .body(data)
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive resumable upload: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                return Err(ProviderError::Other(format!("gdrive resumable upload HTTP {status}")));
            }
            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Other(format!("gdrive resumable decode: {e}")))?;
            if let Some(id) = body["id"].as_str() {
                self.path_cache.lock().await.insert(key.to_string(), id.to_string());
            }
        }

        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ProviderError> {
        let Some(id) = self.resolve_id(key).await? else {
            return Ok(());
        };
        let token = self.access_token().await?;
        let resp = self
            .client
            .delete(format!("{}/files/{}", DRIVE_API_BASE, id))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("gdrive delete: {e}")))?;

        let status = resp.status();
        if !status.is_success() && status.as_u16() != 404 {
            return Err(ProviderError::Other(format!("gdrive delete HTTP {status}")));
        }
        self.invalidate_cache_for(key);
        Ok(())
    }

    async fn copy_object(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        let data = {
            let stat = self.stat(from).await?.unwrap_or(FileStat { size: 0, mtime_filetime: 0 });
            self.get_range(from, 0, stat.size).await?
        };
        self.put_object(to, data).await
    }

    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, ProviderError> {
        let mut results = Vec::new();
        self.walk_prefix(prefix, prefix, &mut results).await?;
        Ok(results)
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

impl GDriveProvider {
    fn walk_prefix<'a>(
        &'a self,
        base_prefix: &'a str,
        current: &'a str,
        results: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let listing = self.list_dir(current).await?;
            for (name, _stat) in listing.files {
                let key = if current.is_empty() {
                    name
                } else {
                    format!("{}/{}", current, name)
                };
                results.push(key);
            }
            for dir in listing.dirs {
                let child = if current.is_empty() {
                    dir
                } else {
                    format!("{}/{}", current, dir)
                };
                self.walk_prefix(base_prefix, &child, results).await?;
            }
            Ok(())
        })
    }
}

// ── PKCE auth flow (called from Tauri commands) ───────────────────────────────

/// Result of a completed OAuth2 PKCE flow.
pub struct AuthResult {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

/// Run the full PKCE flow: open a browser tab, spin up a temporary loopback
/// HTTP server to receive the redirect, exchange the code for tokens.
///
/// This function blocks until the user completes the browser flow or the
/// operation is cancelled (e.g. user closes the window). Call it from a
/// `tokio::task::spawn_blocking` or directly from an async Tauri command.
pub async fn run_pkce_flow() -> Result<AuthResult, String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use sha2::{Digest, Sha256};

    // 1. Generate code verifier + challenge.
    let mut verifier_bytes = [0u8; 64];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut verifier_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge_digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(challenge_digest);

    // 2. Bind a random port on loopback.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("PKCE server bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{}/callback", port);

    // 3. Build authorization URL and open the browser.
    let auth_url = format!(
        "{}?client_id={}&response_type=code&scope={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        GOOGLE_AUTH_URL,
        urlencoding::encode(GOOGLE_CLIENT_ID),
        urlencoding::encode(GOOGLE_DRIVE_SCOPE),
        urlencoding::encode(&redirect_uri),
        code_challenge,
    );
    open::that(&auth_url).map_err(|e| format!("open browser: {e}"))?;

    // 4. Accept one request on the loopback server.
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| format!("PKCE server accept: {e}"))?;

    let mut buf = vec![0u8; 4096];
    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
        .await
        .map_err(|e| format!("PKCE server read: {e}"))?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Send a minimal success response.
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n<html><body><p>You can close this tab and return to NanoCrew Sync.</p></body></html>";
    tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
        .await
        .ok();

    // 5. Extract the `code` query parameter from the request line.
    let code = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|path| {
            path.split('?')
                .nth(1)
                .and_then(|qs| {
                    qs.split('&').find_map(|pair| {
                        let (k, v) = pair.split_once('=')?;
                        if k == "code" { Some(v.to_string()) } else { None }
                    })
                })
        })
        .ok_or_else(|| "no authorization code in callback".to_string())?;

    // URL-decode the code (Google sometimes percent-encodes it).
    let code = urlencoding::decode(&code)
        .map(|s| s.into_owned())
        .unwrap_or(code);

    // 6. Exchange code for tokens.
    let client = reqwest::Client::new();
    let params = [
        ("code", code.as_str()),
        ("client_id", GOOGLE_CLIENT_ID),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", code_verifier.as_str()),
        ("grant_type", "authorization_code"),
    ];
    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("token exchange request: {e}"))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("token exchange decode: {e}"))?;

    let access_token = body["access_token"]
        .as_str()
        .ok_or_else(|| format!("token exchange: missing access_token. response: {body}"))?
        .to_string();
    let refresh_token = body["refresh_token"]
        .as_str()
        .ok_or_else(|| "token exchange: missing refresh_token".to_string())?
        .to_string();
    let expires_in = body["expires_in"].as_u64().unwrap_or(3600);

    Ok(AuthResult { access_token, refresh_token, expires_in })
}

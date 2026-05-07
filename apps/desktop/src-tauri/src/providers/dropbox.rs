//! Dropbox API v2 implementation of [`CloudProvider`].

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{CloudProvider, CompletedPart, FileStat, ListDirResult, ProviderError};

// ── Constants ─────────────────────────────────────────────────────────────────

const API_BASE: &str = "https://api.dropboxapi.com/2";
const CONTENT_BASE: &str = "https://content.dropboxapi.com/2";
const TOKEN_URL: &str = "https://api.dropboxapi.com/oauth2/token";

// 150 MiB — Dropbox's maximum per upload-session chunk
const CHUNK_SIZE: usize = 150 * 1024 * 1024;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DropboxConfig {
    pub client_id: String,
    pub refresh_token: String,
    /// Drive root inside Dropbox. `""` = Dropbox root, `"/subfolder"` = that
    /// folder. Must start with "/" when non-empty.
    pub root_path: String,
}

// ── Token state ───────────────────────────────────────────────────────────────

struct TokenState {
    access_token: String,
    /// Expiry as `std::time::Instant`. `None` = treat as expired.
    expires_at: Option<std::time::Instant>,
}

impl TokenState {
    fn is_valid(&self) -> bool {
        match self.expires_at {
            None => false,
            Some(exp) => exp > std::time::Instant::now() + std::time::Duration::from_secs(60),
        }
    }
}

// ── Provider ──────────────────────────────────────────────────────────────────

pub struct DropboxProvider {
    client: reqwest::Client,
    config: DropboxConfig,
    token_state: Arc<Mutex<TokenState>>,
}

impl DropboxProvider {
    pub fn new(config: DropboxConfig, access_token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            token_state: Arc::new(Mutex::new(TokenState {
                access_token,
                expires_at: None,
            })),
            config,
        }
    }

    /// Return a valid access token, refreshing via the refresh_token if needed.
    async fn access_token(&self) -> Result<String, ProviderError> {
        let mut state = self.token_state.lock().await;
        if state.is_valid() {
            return Ok(state.access_token.clone());
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            expires_in: Option<u64>,
        }

        let resp = self
            .client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &self.config.refresh_token),
                ("client_id", &self.config.client_id),
            ])
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("token refresh: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("token refresh failed: {body}")));
        }

        let tr: TokenResp = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("token parse: {e}")))?;

        state.access_token = tr.access_token.clone();
        state.expires_at = tr.expires_in.map(|s| {
            std::time::Instant::now() + std::time::Duration::from_secs(s)
        });

        Ok(tr.access_token)
    }

    /// Resolve a VFS-relative key to an absolute Dropbox path.
    fn abs_path(&self, key: &str) -> String {
        if key.is_empty() {
            self.config.root_path.clone()
        } else {
            format!("{}/{}", self.config.root_path.trim_end_matches('/'), key)
        }
    }

    /// Parse a Dropbox `server_modified` or `client_modified` timestamp
    /// (RFC 3339 / ISO 8601) into a Windows FILETIME.
    fn parse_mtime(ts: &str) -> u64 {
        // Dropbox returns e.g. "2024-01-15T10:30:00Z"
        let secs = ts
            .parse::<chrono_lite::DateTime>()
            .map(|dt| dt.unix_timestamp())
            .unwrap_or(0)
            .max(0) as u64;
        (secs + 11_644_473_600) * 10_000_000
    }
}

// Minimal date parser to avoid a full chrono dependency.
mod chrono_lite {
    pub struct DateTime {
        unix: i64,
    }

    impl DateTime {
        pub fn unix_timestamp(&self) -> i64 {
            self.unix
        }
    }

    impl std::str::FromStr for DateTime {
        type Err = ();

        fn from_str(s: &str) -> Result<Self, ()> {
            // Expected: "2024-01-15T10:30:00Z" or "2024-01-15T10:30:00+00:00"
            let s = s.trim_end_matches('Z').trim_end_matches("+00:00");
            let parts: Vec<&str> = s.splitn(2, 'T').collect();
            if parts.len() != 2 {
                return Err(());
            }
            let date_parts: Vec<u32> = parts[0]
                .split('-')
                .filter_map(|p| p.parse().ok())
                .collect();
            let time_parts: Vec<u32> = parts[1]
                .split(':')
                .filter_map(|p| p.parse().ok())
                .collect();
            if date_parts.len() < 3 || time_parts.len() < 3 {
                return Err(());
            }
            let (y, m, d) = (date_parts[0] as i64, date_parts[1] as i64, date_parts[2] as i64);
            let (h, min, sec) = (time_parts[0] as i64, time_parts[1] as i64, time_parts[2] as i64);

            // Days since Unix epoch (1970-01-01). Gregorian calculation.
            let days = days_from_civil(y, m, d);
            let unix = days * 86400 + h * 3600 + min * 60 + sec;
            Ok(DateTime { unix })
        }
    }

    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }
}

// ── Dropbox metadata types ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct DropboxEntry {
    #[serde(rename = ".tag")]
    tag: String,
    name: String,
    size: Option<u64>,
    server_modified: Option<String>,
}

#[derive(Deserialize)]
struct ListFolderResp {
    entries: Vec<DropboxEntry>,
    cursor: String,
    has_more: bool,
}

#[derive(Deserialize)]
struct ListFolderContinueResp {
    entries: Vec<DropboxEntry>,
    cursor: String,
    has_more: bool,
}

#[derive(Deserialize)]
struct GetMetadataResp {
    #[serde(rename = ".tag")]
    tag: String,
    size: Option<u64>,
    server_modified: Option<String>,
}

#[derive(Deserialize)]
struct UploadSessionStartResp {
    session_id: String,
}

// ── CloudProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl CloudProvider for DropboxProvider {
    async fn list_dir(&self, prefix: &str) -> Result<ListDirResult, ProviderError> {
        let token = self.access_token().await?;
        let path = self.abs_path(prefix);

        let body = serde_json::json!({
            "path": path,
            "recursive": false,
            "include_deleted": false,
        });

        let resp = self
            .client
            .post(format!("{API_BASE}/files/list_folder"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("list_folder: {e}")))?;

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("list_folder {st}: {txt}")));
        }

        let first: ListFolderResp = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("list_folder parse: {e}")))?;

        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut cursor = first.cursor;
        let mut has_more = first.has_more;

        for entry in first.entries {
            match entry.tag.as_str() {
                "folder" => dirs.push(entry.name),
                "file" => {
                    let size = entry.size.unwrap_or(0);
                    let mtime_filetime = entry
                        .server_modified
                        .as_deref()
                        .map(DropboxProvider::parse_mtime)
                        .unwrap_or(0);
                    files.push((entry.name, FileStat { size, mtime_filetime }));
                }
                _ => {}
            }
        }

        while has_more {
            let cont_body = serde_json::json!({ "cursor": cursor });
            let cont_resp = self
                .client
                .post(format!("{API_BASE}/files/list_folder/continue"))
                .bearer_auth(&token)
                .json(&cont_body)
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("list_folder/continue: {e}")))?;

            if !cont_resp.status().is_success() {
                let st = cont_resp.status();
                let txt = cont_resp.text().await.unwrap_or_default();
                return Err(ProviderError::Other(format!("list_folder/continue {st}: {txt}")));
            }

            let cont: ListFolderContinueResp = cont_resp
                .json()
                .await
                .map_err(|e| ProviderError::Other(format!("list_folder/continue parse: {e}")))?;

            for entry in cont.entries {
                match entry.tag.as_str() {
                    "folder" => dirs.push(entry.name),
                    "file" => {
                        let size = entry.size.unwrap_or(0);
                        let mtime_filetime = entry
                            .server_modified
                            .as_deref()
                            .map(DropboxProvider::parse_mtime)
                            .unwrap_or(0);
                        files.push((entry.name, FileStat { size, mtime_filetime }));
                    }
                    _ => {}
                }
            }

            cursor = cont.cursor;
            has_more = cont.has_more;
        }

        Ok(ListDirResult { dirs, files })
    }

    async fn stat(&self, key: &str) -> Result<Option<FileStat>, ProviderError> {
        let token = self.access_token().await?;
        let path = self.abs_path(key);

        let body = serde_json::json!({ "path": path });

        let resp = self
            .client
            .post(format!("{API_BASE}/files/get_metadata"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("get_metadata: {e}")))?;

        if resp.status().as_u16() == 409 {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            if txt.contains("not_found") || txt.contains("path/not_found") {
                return Ok(None);
            }
            return Err(ProviderError::Other(format!("get_metadata {st}: {txt}")));
        }

        let meta: GetMetadataResp = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("get_metadata parse: {e}")))?;

        if meta.tag == "folder" {
            return Ok(None);
        }

        let size = meta.size.unwrap_or(0);
        let mtime_filetime = meta
            .server_modified
            .as_deref()
            .map(DropboxProvider::parse_mtime)
            .unwrap_or(0);

        Ok(Some(FileStat { size, mtime_filetime }))
    }

    async fn get_range(
        &self,
        key: &str,
        offset: u64,
        length: u64,
    ) -> Result<Bytes, ProviderError> {
        if length == 0 {
            return Ok(Bytes::new());
        }

        let token = self.access_token().await?;
        let path = self.abs_path(key);
        let end = offset + length - 1;

        let arg = serde_json::json!({ "path": path }).to_string();

        let resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/download"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", &arg)
            .header("Range", format!("bytes={offset}-{end}"))
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("download {key:?}: {e}")))?;

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("download {st}: {txt}")));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Other(format!("download body: {e}")))?;

        Ok(bytes)
    }

    async fn put_object(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        if data.len() <= CHUNK_SIZE {
            self.upload_single(key, data).await
        } else {
            self.upload_session(key, data).await
        }
    }

    async fn delete(&self, key: &str) -> Result<(), ProviderError> {
        let token = self.access_token().await?;
        let path = self.abs_path(key);

        let body = serde_json::json!({ "path": path });

        let resp = self
            .client
            .post(format!("{API_BASE}/files/delete_v2"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("delete {key:?}: {e}")))?;

        if resp.status().as_u16() == 409 {
            let txt = resp.text().await.unwrap_or_default();
            if txt.contains("not_found") || txt.contains("path_lookup/not_found") {
                return Ok(());
            }
            return Err(ProviderError::Other(format!("delete 409: {txt}")));
        }

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("delete {st}: {txt}")));
        }

        Ok(())
    }

    async fn copy_object(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        let token = self.access_token().await?;
        let from_path = self.abs_path(from);
        let to_path = self.abs_path(to);

        let body = serde_json::json!({
            "from_path": from_path,
            "to_path": to_path,
        });

        let resp = self
            .client
            .post(format!("{API_BASE}/files/copy_v2"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("copy_v2 {from:?}: {e}")))?;

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("copy_v2 {st}: {txt}")));
        }

        Ok(())
    }

    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, ProviderError> {
        let token = self.access_token().await?;
        let path = self.abs_path(prefix);

        #[derive(Deserialize)]
        struct EntryWithPath {
            #[serde(rename = ".tag")]
            tag: String,
            path_display: Option<String>,
        }

        #[derive(Deserialize)]
        struct ListWithPath {
            entries: Vec<EntryWithPath>,
            cursor: String,
            has_more: bool,
        }

        let body = serde_json::json!({
            "path": path,
            "recursive": true,
            "include_deleted": false,
        });

        let resp = self
            .client
            .post(format!("{API_BASE}/files/list_folder"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("list_prefix: {e}")))?;

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("list_prefix {st}: {txt}")));
        }

        let mut batch: ListWithPath = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("list_prefix parse: {e}")))?;

        let abs_root = format!("{}/", self.config.root_path.trim_end_matches('/'));
        let strip_root = |p: &str| -> String {
            p.strip_prefix(&abs_root)
                .unwrap_or(p.trim_start_matches('/'))
                .to_string()
        };

        let mut keys: Vec<String> = Vec::new();

        for entry in &batch.entries {
            if entry.tag == "file" {
                if let Some(ref pd) = entry.path_display {
                    keys.push(strip_root(pd));
                }
            }
        }

        let mut cursor = batch.cursor;
        let mut has_more = batch.has_more;

        while has_more {
            let cont_body = serde_json::json!({ "cursor": cursor });
            let cont_resp = self
                .client
                .post(format!("{API_BASE}/files/list_folder/continue"))
                .bearer_auth(&token)
                .json(&cont_body)
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("list_prefix/continue: {e}")))?;

            if !cont_resp.status().is_success() {
                let st = cont_resp.status();
                let txt = cont_resp.text().await.unwrap_or_default();
                return Err(ProviderError::Other(format!(
                    "list_prefix/continue {st}: {txt}"
                )));
            }

            let cont: ListWithPath = cont_resp
                .json()
                .await
                .map_err(|e| ProviderError::Other(format!("list_prefix/continue parse: {e}")))?;

            for entry in &cont.entries {
                if entry.tag == "file" {
                    if let Some(ref pd) = entry.path_display {
                        keys.push(strip_root(pd));
                    }
                }
            }

            cursor = cont.cursor;
            has_more = cont.has_more;
        }

        Ok(keys)
    }

    // ── Multipart upload (upload sessions) ────────────────────────────────────

    async fn create_multipart(&self, _key: &str) -> Result<String, ProviderError> {
        let token = self.access_token().await?;

        let resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/upload_session/start"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", r#"{"close":false}"#)
            .header("Content-Type", "application/octet-stream")
            .body(Bytes::new())
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("upload_session/start: {e}")))?;

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "upload_session/start {st}: {txt}"
            )));
        }

        let start: UploadSessionStartResp = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("upload_session/start parse: {e}")))?;

        Ok(start.session_id)
    }

    async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: i32,
        data: Bytes,
    ) -> Result<String, ProviderError> {
        let token = self.access_token().await?;
        let offset = (part_number as u64 - 1) * CHUNK_SIZE as u64;
        let data_len = data.len() as u64;

        let arg = serde_json::json!({
            "cursor": {
                "session_id": upload_id,
                "offset": offset,
            },
            "close": false,
        })
        .to_string();

        let resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/upload_session/append_v2"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", &arg)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("upload_session/append_v2 {key:?}: {e}")))?;

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "upload_session/append_v2 {st}: {txt}"
            )));
        }

        // Encode offset + data length in the etag so complete_multipart can
        // reconstruct the total uploaded byte count without external state.
        let end_offset = offset + data_len;
        Ok(format!("{upload_id}:{end_offset}"))
    }

    async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: Vec<CompletedPart>,
    ) -> Result<(), ProviderError> {
        let token = self.access_token().await?;
        let path = self.abs_path(key);

        // Recover the total uploaded byte count from the last part's etag.
        // Each etag is "{upload_id}:{end_offset}"; we take the maximum.
        let total_offset: u64 = parts
            .iter()
            .filter_map(|p| {
                p.etag
                    .rsplit(':')
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0);

        let arg = serde_json::json!({
            "cursor": {
                "session_id": upload_id,
                "offset": total_offset,
            },
            "commit": {
                "path": path,
                "mode": "overwrite",
                "autorename": false,
                "mute": true,
            },
        })
        .to_string();

        let resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/upload_session/finish"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", &arg)
            .header("Content-Type", "application/octet-stream")
            .body(Bytes::new())
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("upload_session/finish {key:?}: {e}")))?;

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "upload_session/finish {st}: {txt}"
            )));
        }

        Ok(())
    }

    async fn abort_multipart(&self, _key: &str, _upload_id: &str) -> Result<(), ProviderError> {
        Ok(())
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

impl DropboxProvider {
    async fn upload_single(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        let token = self.access_token().await?;
        let path = self.abs_path(key);

        let arg = serde_json::json!({
            "path": path,
            "mode": "overwrite",
            "autorename": false,
            "mute": true,
        })
        .to_string();

        let resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/upload"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", &arg)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("upload {key:?}: {e}")))?;

        if !resp.status().is_success() {
            let st = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("upload {st}: {txt}")));
        }

        Ok(())
    }

    async fn upload_session(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        let token = self.access_token().await?;
        let path = self.abs_path(key);

        // Start
        let start_resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/upload_session/start"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", r#"{"close":false}"#)
            .header("Content-Type", "application/octet-stream")
            .body(Bytes::new())
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("upload_session/start: {e}")))?;

        if !start_resp.status().is_success() {
            let st = start_resp.status();
            let txt = start_resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "upload_session/start {st}: {txt}"
            )));
        }

        let start: UploadSessionStartResp = start_resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("upload_session/start parse: {e}")))?;

        let session_id = start.session_id;
        let total = data.len();
        let mut offset = 0usize;

        // Append all but the last chunk
        while offset + CHUNK_SIZE < total {
            let chunk = data.slice(offset..offset + CHUNK_SIZE);
            let arg = serde_json::json!({
                "cursor": { "session_id": &session_id, "offset": offset },
                "close": false,
            })
            .to_string();

            let resp = self
                .client
                .post(format!("{CONTENT_BASE}/files/upload_session/append_v2"))
                .bearer_auth(&token)
                .header("Dropbox-API-Arg", &arg)
                .header("Content-Type", "application/octet-stream")
                .body(chunk)
                .send()
                .await
                .map_err(|e| ProviderError::Other(format!("upload_session/append_v2: {e}")))?;

            if !resp.status().is_success() {
                let st = resp.status();
                let txt = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Other(format!(
                    "upload_session/append_v2 {st}: {txt}"
                )));
            }

            offset += CHUNK_SIZE;
        }

        // Finish with remaining data
        let last_chunk = data.slice(offset..);
        let arg = serde_json::json!({
            "cursor": { "session_id": &session_id, "offset": offset },
            "commit": {
                "path": path,
                "mode": "overwrite",
                "autorename": false,
                "mute": true,
            },
        })
        .to_string();

        let finish_resp = self
            .client
            .post(format!("{CONTENT_BASE}/files/upload_session/finish"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", &arg)
            .header("Content-Type", "application/octet-stream")
            .body(last_chunk)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("upload_session/finish: {e}")))?;

        if !finish_resp.status().is_success() {
            let st = finish_resp.status();
            let txt = finish_resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "upload_session/finish {st}: {txt}"
            )));
        }

        Ok(())
    }
}

// ── OAuth2 PKCE helpers ───────────────────────────────────────────────────────

/// Generates the Dropbox OAuth2 PKCE authorization URL and starts a loopback
/// HTTP listener. Returns `(auth_url, code_verifier, port)`.
pub fn build_auth_url(client_id: &str) -> Result<(String, String, u16), String> {
    use base64::Engine;

    // PKCE code verifier: 32 random bytes → base64url (no padding)
    let mut verifier_bytes = [0u8; 32];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut verifier_bytes);
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);

    // Code challenge = SHA-256(verifier), base64url-no-pad
    use sha2::Digest;
    let digest = sha2::Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

    // Bind an available port on the loopback
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?
        .port();
    drop(listener);

    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let auth_url = format!(
        "https://www.dropbox.com/oauth2/authorize\
         ?client_id={client_id}\
         &response_type=code\
         &redirect_uri={redirect_uri}\
         &code_challenge={code_challenge}\
         &code_challenge_method=S256\
         &token_access_type=offline\
         &scope=files.content.read+files.content.write+files.metadata.read+files.metadata.write"
    );

    Ok((auth_url, code_verifier, port))
}

/// Exchange an authorization code for tokens. Returns `(access_token, refresh_token)`.
pub async fn exchange_code(
    client_id: &str,
    code: &str,
    code_verifier: &str,
    port: u16,
) -> Result<(String, String), String> {
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    #[derive(Deserialize)]
    struct TokenResp {
        access_token: String,
        refresh_token: Option<String>,
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.dropboxapi.com/oauth2/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri),
            ("code_verifier", code_verifier),
            ("client_id", client_id),
        ])
        .send()
        .await
        .map_err(|e| format!("token exchange: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token exchange failed: {body}"));
    }

    let tr: TokenResp = resp
        .json()
        .await
        .map_err(|e| format!("token parse: {e}"))?;

    let refresh_token = tr
        .refresh_token
        .ok_or_else(|| "no refresh_token in response".to_string())?;

    Ok((tr.access_token, refresh_token))
}

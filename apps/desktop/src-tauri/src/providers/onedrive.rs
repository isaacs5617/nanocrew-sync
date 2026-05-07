use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{CloudProvider, CompletedPart, FileStat, ListDirResult, ProviderError};

// ── Auth endpoints ────────────────────────────────────────────────────────────

const AUTHORIZE_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const SCOPES: &str = "Files.ReadWrite offline_access";

// ── Graph API base ────────────────────────────────────────────────────────────

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

// Upload session threshold: use resumable upload above 4 MiB.
const UPLOAD_SESSION_THRESHOLD: usize = 4 * 1024 * 1024;
// Chunk size for upload sessions (must be a multiple of 320 KiB per Graph docs).
const UPLOAD_CHUNK_SIZE: usize = 10 * 320 * 1024; // 3.2 MiB

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OneDriveConfig {
    pub client_id: String,
    /// Stored refresh token (DPAPI-wrapped in DB, decrypted before use).
    pub refresh_token: String,
    /// "me/drive" for personal OneDrive; a SharePoint drive ID otherwise.
    pub drive_id: String,
}

// ── Token state ───────────────────────────────────────────────────────────────

struct TokenState {
    access_token: String,
    expires_at: std::time::Instant,
}

// ── Provider ──────────────────────────────────────────────────────────────────

pub struct OneDriveProvider {
    client: reqwest::Client,
    config: OneDriveConfig,
    token_state: Arc<Mutex<Option<TokenState>>>,
}

impl OneDriveProvider {
    pub async fn new(config: OneDriveConfig) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;
        let provider = Self {
            client,
            config,
            token_state: Arc::new(Mutex::new(None)),
        };
        // Eagerly fetch an access token to validate the config on construction.
        provider.access_token().await?;
        Ok(provider)
    }

    /// Build the root path prefix for Graph API URLs.
    /// `me/drive` → `/me/drive/root:`
    /// `<drive_id>` → `/drives/<drive_id>/root:`
    fn root_prefix(&self) -> String {
        if self.config.drive_id == "me/drive" {
            format!("{GRAPH}/me/drive/root:")
        } else {
            format!("{GRAPH}/drives/{}/root:", self.config.drive_id)
        }
    }

    /// Return a valid access token, refreshing if expired or absent.
    async fn access_token(&self) -> Result<String, ProviderError> {
        let mut guard = self.token_state.lock().await;
        if let Some(ref ts) = *guard {
            if ts.expires_at > std::time::Instant::now() + std::time::Duration::from_secs(60) {
                return Ok(ts.access_token.clone());
            }
        }

        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", self.config.refresh_token.as_str()),
            ("scope", SCOPES),
        ];

        let resp = self
            .client
            .post(TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("token refresh: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("token refresh failed: {text}")));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(format!("token parse: {e}")))?;

        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| ProviderError::Other("missing access_token".into()))?
            .to_string();
        let expires_in = json["expires_in"].as_u64().unwrap_or(3600);

        *guard = Some(TokenState {
            access_token: access_token.clone(),
            expires_at: std::time::Instant::now()
                + std::time::Duration::from_secs(expires_in),
        });

        Ok(access_token)
    }

    /// Graph API URL for a file/folder path. Empty path → drive root item.
    fn item_url(&self, path: &str) -> String {
        if path.is_empty() {
            // Root of the drive
            if self.config.drive_id == "me/drive" {
                format!("{GRAPH}/me/drive/root")
            } else {
                format!("{GRAPH}/drives/{}/root", self.config.drive_id)
            }
        } else {
            format!("{}/{}", self.root_prefix(), url_encode_path(path))
        }
    }

    /// Graph API URL for a folder's children listing.
    fn children_url(&self, path: &str) -> String {
        if path.is_empty() {
            if self.config.drive_id == "me/drive" {
                format!("{GRAPH}/me/drive/root/children")
            } else {
                format!("{GRAPH}/drives/{}/root/children", self.config.drive_id)
            }
        } else {
            format!(
                "{}/{}:/children",
                self.root_prefix(),
                url_encode_path(path)
            )
        }
    }

    /// GET helper — returns the parsed JSON body.
    async fn graph_get(&self, url: &str) -> Result<serde_json::Value, ProviderError> {
        let token = self.access_token().await?;
        let resp = self
            .client
            .get(url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ProviderError::NotFound(url.to_string()));
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("graph GET {url}: {text}")));
        }

        resp.json()
            .await
            .map_err(|e| ProviderError::Other(e.to_string()))
    }

    /// Small-file PUT (≤ 4 MiB).
    async fn put_small(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        let token = self.access_token().await?;
        let url = format!("{}:/content", self.item_url(key));
        let resp = self
            .client
            .put(&url)
            .bearer_auth(&token)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("put {key}: {text}")));
        }
        Ok(())
    }

    /// Large-file upload via upload session.
    async fn put_large(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        let token = self.access_token().await?;
        let session_url = format!("{}:/createUploadSession", self.item_url(key));
        let resp = self
            .client
            .post(&session_url)
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "item": {
                    "@microsoft.graph.conflictBehavior": "replace"
                }
            }))
            .send()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!(
                "createUploadSession {key}: {text}"
            )));
        }

        let session: serde_json::Value =
            resp.json().await.map_err(|e| ProviderError::Other(e.to_string()))?;
        let upload_url = session["uploadUrl"]
            .as_str()
            .ok_or_else(|| ProviderError::Other("missing uploadUrl".into()))?
            .to_string();

        let total = data.len();
        let mut offset = 0usize;
        while offset < total {
            let end = (offset + UPLOAD_CHUNK_SIZE).min(total);
            let chunk = data.slice(offset..end);
            let content_range = format!("bytes {}-{}/{}", offset, end - 1, total);

            let chunk_resp = self
                .client
                .put(&upload_url)
                .header("Content-Range", &content_range)
                .header("Content-Length", chunk.len().to_string())
                .body(chunk)
                .send()
                .await
                .map_err(|e| ProviderError::Io(e.to_string()))?;

            if !chunk_resp.status().is_success()
                && chunk_resp.status().as_u16() != 202
                && chunk_resp.status().as_u16() != 201
            {
                let text = chunk_resp.text().await.unwrap_or_default();
                return Err(ProviderError::Other(format!(
                    "upload chunk {offset}-{}: {text}",
                    end - 1
                )));
            }

            offset = end;
        }

        Ok(())
    }
}

// ── Path helpers ──────────────────────────────────────────────────────────────

/// Percent-encode a path for Graph API URL segments, preserving `/`.
fn url_encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            let mut out = String::new();
            for b in seg.bytes() {
                match b {
                    b'A'..=b'Z'
                    | b'a'..=b'z'
                    | b'0'..=b'9'
                    | b'-'
                    | b'_'
                    | b'.'
                    | b'~' => out.push(b as char),
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Convert an ISO-8601 datetime string to Windows FILETIME.
/// Returns 0 on parse failure.
fn iso8601_to_filetime(s: &str) -> u64 {
    // Parse manually: "2024-01-15T10:30:00Z" or with offset
    // Windows FILETIME = 100-ns intervals since 1601-01-01
    // Unix epoch = seconds since 1970-01-01
    // Offset between them = 11644473600 seconds
    const EPOCH_DELTA_SECS: u64 = 11_644_473_600;

    // Try to extract a Unix timestamp via a simple RFC-3339 parse.
    // We use a hand-rolled approach to avoid pulling in chrono.
    fn parse_unix(s: &str) -> Option<i64> {
        // Minimal: YYYY-MM-DDTHH:MM:SS[.fff]Z or ±HH:MM
        let s = s.trim();
        if s.len() < 19 {
            return None;
        }
        let year: i64 = s[0..4].parse().ok()?;
        let month: i64 = s[5..7].parse().ok()?;
        let day: i64 = s[8..10].parse().ok()?;
        let hour: i64 = s[11..13].parse().ok()?;
        let min: i64 = s[14..16].parse().ok()?;
        let sec: i64 = s[17..19].parse().ok()?;

        // Days from 1970-01-01 via a simple Gregorian formula
        fn days_since_epoch(y: i64, m: i64, d: i64) -> i64 {
            let y = if m <= 2 { y - 1 } else { y };
            let era = y / 400;
            let yoe = y - era * 400;
            let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            era * 146097 + doe - 719468
        }

        Some(days_since_epoch(year, month, day) * 86400 + hour * 3600 + min * 60 + sec)
    }

    let unix = match parse_unix(s) {
        Some(u) => u,
        None => return 0,
    };
    if unix < 0 {
        return 0;
    }
    let unix = unix as u64;
    (unix + EPOCH_DELTA_SECS) * 10_000_000
}

// ── CloudProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl CloudProvider for OneDriveProvider {
    async fn list_dir(&self, prefix: &str) -> Result<ListDirResult, ProviderError> {
        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<(String, FileStat)> = Vec::new();

        let select = "$select=id,name,size,lastModifiedDateTime,folder";
        let base_url = format!("{}?{select}", self.children_url(prefix));
        let mut next_url: Option<String> = Some(base_url);

        while let Some(url) = next_url {
            let json = self.graph_get(&url).await.map_err(|e| match e {
                ProviderError::NotFound(_) => ProviderError::NotFound(prefix.to_string()),
                other => other,
            })?;

            let items = json["value"].as_array().cloned().unwrap_or_default();
            for item in &items {
                let name = item["name"].as_str().unwrap_or("").to_string();
                if name.is_empty() {
                    continue;
                }
                if item.get("folder").is_some() {
                    dirs.push(name);
                } else {
                    let size = item["size"].as_u64().unwrap_or(0);
                    let mtime = item["lastModifiedDateTime"]
                        .as_str()
                        .map(iso8601_to_filetime)
                        .unwrap_or(0);
                    files.push((name, FileStat { size, mtime_filetime: mtime }));
                }
            }

            next_url = json["@odata.nextLink"].as_str().map(str::to_owned);
        }

        Ok(ListDirResult { dirs, files })
    }

    async fn stat(&self, key: &str) -> Result<Option<FileStat>, ProviderError> {
        let url = format!(
            "{}?$select=size,lastModifiedDateTime",
            self.item_url(key)
        );
        match self.graph_get(&url).await {
            Ok(json) => {
                let size = json["size"].as_u64().unwrap_or(0);
                let mtime = json["lastModifiedDateTime"]
                    .as_str()
                    .map(iso8601_to_filetime)
                    .unwrap_or(0);
                Ok(Some(FileStat { size, mtime_filetime: mtime }))
            }
            Err(ProviderError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn get_range(
        &self,
        key: &str,
        offset: u64,
        length: u64,
    ) -> Result<Bytes, ProviderError> {
        let token = self.access_token().await?;
        let url = format!("{}:/content", self.item_url(key));
        let range_header = format!("bytes={}-{}", offset, offset + length - 1);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .header("Range", range_header)
            .send()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ProviderError::NotFound(key.to_string()));
        }
        if !resp.status().is_success() && resp.status().as_u16() != 206 {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("get_range {key}: {text}")));
        }

        resp.bytes()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))
    }

    async fn put_object(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        if data.len() <= UPLOAD_SESSION_THRESHOLD {
            self.put_small(key, data).await
        } else {
            self.put_large(key, data).await
        }
    }

    async fn delete(&self, key: &str) -> Result<(), ProviderError> {
        let token = self.access_token().await?;
        let url = self.item_url(key);
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))?;

        // 404 → treat as success (already gone)
        if resp.status() == reqwest::StatusCode::NOT_FOUND
            || resp.status().as_u16() == 204
            || resp.status().is_success()
        {
            return Ok(());
        }

        let text = resp.text().await.unwrap_or_default();
        Err(ProviderError::Other(format!("delete {key}: {text}")))
    }

    async fn copy_object(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        // Graph copy is async and returns a monitor URL. We poll until done.
        let token = self.access_token().await?;

        // Resolve the destination parent folder and new name.
        let (dest_parent, dest_name) = split_parent_name(to);

        // Build the parentReference for the destination.
        // We need the drive item ID of the destination parent folder.
        let parent_id = if dest_parent.is_empty() {
            // Root — resolve root item ID
            let root_url = if self.config.drive_id == "me/drive" {
                format!("{GRAPH}/me/drive/root?$select=id")
            } else {
                format!("{GRAPH}/drives/{}/root?$select=id", self.config.drive_id)
            };
            let json = self.graph_get(&root_url).await?;
            json["id"]
                .as_str()
                .ok_or_else(|| ProviderError::Other("root item has no id".into()))?
                .to_string()
        } else {
            let parent_url = format!("{}?$select=id", self.item_url(dest_parent));
            let json = self.graph_get(&parent_url).await?;
            json["id"]
                .as_str()
                .ok_or_else(|| ProviderError::Other("parent item has no id".into()))?
                .to_string()
        };

        let drive_ref = if self.config.drive_id == "me/drive" {
            // Graph needs the actual drive ID, not "me/drive"
            let info_url = format!("{GRAPH}/me/drive?$select=id");
            let json = self.graph_get(&info_url).await?;
            json["id"]
                .as_str()
                .unwrap_or("me")
                .to_string()
        } else {
            self.config.drive_id.clone()
        };

        let copy_url = format!("{}:/copy", self.item_url(from));
        let body = serde_json::json!({
            "parentReference": { "driveId": drive_ref, "id": parent_id },
            "name": dest_name,
        });

        let resp = self
            .client
            .post(&copy_url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))?;

        if !resp.status().is_success() && resp.status().as_u16() != 202 {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("copy {from}→{to}: {text}")));
        }

        // Poll the monitor URL until the copy finishes.
        if let Some(monitor) = resp.headers().get("Location") {
            let monitor_url = monitor.to_str().unwrap_or("").to_string();
            for _ in 0..120 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let poll = self
                    .client
                    .get(&monitor_url)
                    .send()
                    .await
                    .map_err(|e| ProviderError::Io(e.to_string()))?;
                if poll.status().is_success() {
                    let json: serde_json::Value =
                        poll.json().await.unwrap_or(serde_json::Value::Null);
                    let status = json["status"].as_str().unwrap_or("");
                    if status == "completed" {
                        return Ok(());
                    }
                    if status == "failed" {
                        return Err(ProviderError::Other(format!(
                            "copy {from}→{to} failed: {}",
                            json["error"]["message"].as_str().unwrap_or("unknown")
                        )));
                    }
                }
            }
            return Err(ProviderError::Other(format!("copy {from}→{to}: timed out")));
        }

        Ok(())
    }

    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, ProviderError> {
        let mut result: Vec<String> = Vec::new();
        self.collect_keys(prefix, prefix, &mut result).await?;
        Ok(result)
    }

    // ── Multipart upload ──────────────────────────────────────────────────────
    // Graph does not use S3-style multipart; we map these to a single
    // upload-session. The upload_id carries the session URL.

    async fn create_multipart(&self, key: &str) -> Result<String, ProviderError> {
        let token = self.access_token().await?;
        let session_url = format!("{}:/createUploadSession", self.item_url(key));
        let resp = self
            .client
            .post(&session_url)
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "item": { "@microsoft.graph.conflictBehavior": "replace" }
            }))
            .send()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("createUploadSession: {text}")));
        }

        let json: serde_json::Value =
            resp.json().await.map_err(|e| ProviderError::Other(e.to_string()))?;
        let upload_url = json["uploadUrl"]
            .as_str()
            .ok_or_else(|| ProviderError::Other("missing uploadUrl".into()))?
            .to_string();

        Ok(upload_url)
    }

    async fn upload_part(
        &self,
        _key: &str,
        upload_id: &str,
        _part_number: i32,
        data: Bytes,
    ) -> Result<String, ProviderError> {
        let len = data.len();
        let resp = self
            .client
            .put(upload_id)
            .header("Content-Length", len.to_string())
            .body(data)
            .send()
            .await
            .map_err(|e| ProviderError::Io(e.to_string()))?;

        if !resp.status().is_success()
            && resp.status().as_u16() != 202
            && resp.status().as_u16() != 201
        {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(format!("upload_part: {text}")));
        }

        Ok(String::new())
    }

    async fn complete_multipart(
        &self,
        _key: &str,
        _upload_id: &str,
        _parts: Vec<CompletedPart>,
    ) -> Result<(), ProviderError> {
        // Upload session auto-completes when the final chunk is received.
        Ok(())
    }

    async fn abort_multipart(&self, _key: &str, upload_id: &str) -> Result<(), ProviderError> {
        // Cancel the upload session by sending DELETE to the session URL.
        let _ = self.client.delete(upload_id).send().await;
        Ok(())
    }

    fn preferred_block_size(&self) -> usize {
        1024 * 1024
    }

    fn supports_byte_ranges(&self) -> bool {
        true
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

impl OneDriveProvider {
    /// Recursively collect all file keys under `dir_prefix`.
    async fn collect_keys(
        &self,
        dir_prefix: &str,
        base: &str,
        out: &mut Vec<String>,
    ) -> Result<(), ProviderError> {
        let result = self.list_dir(dir_prefix).await?;
        for (name, _stat) in result.files {
            let key = if dir_prefix.is_empty() {
                name
            } else {
                format!("{dir_prefix}/{name}")
            };
            out.push(key);
        }
        for sub in result.dirs {
            let sub_prefix = if dir_prefix.is_empty() {
                sub
            } else {
                format!("{dir_prefix}/{sub}")
            };
            Box::pin(self.collect_keys(&sub_prefix, base, out)).await?;
        }
        Ok(())
    }
}

/// Split a path into (parent_path, filename). `""` means root.
fn split_parent_name(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[..idx], &path[idx + 1..]),
        None => ("", path),
    }
}

// ── PKCE auth flow ────────────────────────────────────────────────────────────

/// Run the OAuth2 PKCE flow for OneDrive. Opens the browser, starts a loopback
/// HTTP server, waits for the callback, exchanges the code, and returns the
/// refresh token.
pub async fn run_pkce_flow(client_id: &str) -> Result<String, String> {
    use std::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // ── Step 1: pick a free port ──────────────────────────────────────────────
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://localhost:{port}/callback");

    // ── Step 2: generate PKCE code_verifier + code_challenge ─────────────────
    let code_verifier = pkce_verifier();
    let code_challenge = pkce_challenge(&code_verifier);

    // ── Step 3: build the authorization URL ──────────────────────────────────
    let state_token = uuid::Uuid::new_v4().to_string();
    let auth_url = format!(
        "{AUTHORIZE_URL}?client_id={client_id}\
         &response_type=code\
         &redirect_uri={redir}\
         &scope={scope}\
         &code_challenge={challenge}\
         &code_challenge_method=S256\
         &state={state}",
        redir = urlencoding::encode(&redirect_uri),
        scope = urlencoding::encode(SCOPES),
        challenge = code_challenge,
        state = state_token,
    );

    // ── Step 4: open browser ──────────────────────────────────────────────────
    open::that(&auth_url).map_err(|e| format!("cannot open browser: {e}"))?;

    // ── Step 5: accept the callback ───────────────────────────────────────────
    let listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
    let (mut stream, _) = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        listener.accept(),
    )
    .await
    .map_err(|_| "auth timeout after 120 s".to_string())?
    .map_err(|e| e.to_string())?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Extract the query string from "GET /callback?... HTTP/1.1"
    let query = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|path| path.splitn(2, '?').nth(1))
        .unwrap_or("");

    let code = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("code="))
        .ok_or("no code in callback")?
        .to_string();

    let html = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
        <html><body style='font-family:sans-serif;padding:40px'>\
        <h2>Signed in to OneDrive!</h2>\
        <p>You can close this tab and return to NanoCrew Sync.</p>\
        </body></html>";
    let _ = stream.write_all(html).await;

    // ── Step 6: exchange code for tokens ─────────────────────────────────────
    let client = reqwest::Client::new();
    let params = [
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", code_verifier.as_str()),
    ];
    let resp = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("token exchange: {e}"))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("token exchange failed: {text}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("token parse: {e}"))?;

    let refresh_token = json["refresh_token"]
        .as_str()
        .ok_or("no refresh_token in response")?
        .to_string();

    Ok(refresh_token)
}

fn pkce_verifier() -> String {
    use rand_core::RngCore;
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    base64_url_encode(&bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    base64_url_encode(&hash)
}

fn base64_url_encode(input: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input)
}

// Simple percent-encoding for URL parameters (not path segments).
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
                | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
}

use std::collections::HashMap;

use async_trait::async_trait;
use bytes::Bytes;
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};

use super::{CloudProvider, CompletedPart, FileStat, ListDirResult, ProviderError};

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebDavConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub root_path: String,
    pub accept_invalid_certs: bool,
}

// ── Provider ──────────────────────────────────────────────────────────────────

pub struct WebDavProvider {
    client: Client,
    config: WebDavConfig,
}

impl WebDavProvider {
    pub fn new(config: WebDavConfig) -> Result<Self, ProviderError> {
        let client = reqwest::ClientBuilder::new()
            .danger_accept_invalid_certs(config.accept_invalid_certs)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ProviderError::Other(format!("reqwest client: {e}")))?;
        Ok(Self { client, config })
    }

    /// Build the full URL for a VFS-relative key.
    fn url(&self, key: &str) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        let root = self.config.root_path.trim_matches('/');
        if key.is_empty() {
            if root.is_empty() {
                format!("{base}/")
            } else {
                format!("{base}/{root}/")
            }
        } else {
            let key = key.trim_start_matches('/');
            if root.is_empty() {
                format!("{base}/{key}")
            } else {
                format!("{base}/{root}/{key}")
            }
        }
    }

    /// Build the full URL for a directory (ensures trailing slash).
    fn dir_url(&self, key: &str) -> String {
        let u = self.url(key);
        if u.ends_with('/') { u } else { format!("{u}/") }
    }

    fn auth(&self) -> header::HeaderValue {
        use base64::Engine;
        let raw = format!("{}:{}", self.config.username, self.config.password);
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        header::HeaderValue::from_str(&format!("Basic {encoded}")).unwrap()
    }

    async fn propfind(&self, url: &str, depth: &str) -> Result<String, ProviderError> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:getcontentlength/>
    <D:getlastmodified/>
    <D:resourcetype/>
  </D:prop>
</D:propfind>"#;

        let resp = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), url)
            .header(header::AUTHORIZATION, self.auth())
            .header("Depth", depth)
            .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(body)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("PROPFIND {url}: {e}")))?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Err(ProviderError::NotFound(url.to_string()));
        }
        if !status.is_success() && status.as_u16() != 207 {
            return Err(ProviderError::Other(format!("PROPFIND {url}: HTTP {status}")));
        }

        resp.text()
            .await
            .map_err(|e| ProviderError::Other(format!("PROPFIND read body: {e}")))
    }
}

// ── XML parsing ───────────────────────────────────────────────────────────────

/// One `<D:response>` entry from a PROPFIND multi-status body.
#[derive(Default)]
struct PropEntry {
    href: String,
    content_length: u64,
    last_modified: u64,
    is_collection: bool,
}

fn parse_propfind(xml: &str) -> Vec<PropEntry> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut entries: Vec<PropEntry> = Vec::new();
    let mut current: Option<PropEntry> = None;

    // Which element we're currently accumulating text for.
    enum Collecting {
        Href,
        ContentLength,
        LastModified,
        None,
    }
    let mut collecting = Collecting::None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let _name = e.name();
                let local = local_name(_name.as_ref());
                match local {
                    b"response" => current = Some(PropEntry::default()),
                    b"href" => collecting = Collecting::Href,
                    b"getcontentlength" => collecting = Collecting::ContentLength,
                    b"getlastmodified" => collecting = Collecting::LastModified,
                    b"collection" => {
                        if let Some(ref mut entry) = current {
                            entry.is_collection = true;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let _name = e.name();
                let local = local_name(_name.as_ref());
                if local == b"collection" {
                    if let Some(ref mut entry) = current {
                        entry.is_collection = true;
                    }
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Some(ref mut entry) = current {
                    let text = e.unescape().unwrap_or_default();
                    match collecting {
                        Collecting::Href => entry.href = text.trim().to_string(),
                        Collecting::ContentLength => {
                            entry.content_length = text.trim().parse().unwrap_or(0);
                        }
                        Collecting::LastModified => {
                            entry.last_modified = parse_http_date(text.trim());
                        }
                        Collecting::None => {}
                    }
                }
                collecting = Collecting::None;
            }
            Ok(Event::End(ref e)) => {
                let _name = e.name();
                let local = local_name(_name.as_ref());
                if local == b"response" {
                    if let Some(entry) = current.take() {
                        entries.push(entry);
                    }
                }
                collecting = Collecting::None;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    entries
}

/// Strip namespace prefix from a qualified XML name byte slice.
fn local_name(name: &[u8]) -> &[u8] {
    if let Some(pos) = name.iter().position(|&b| b == b':') {
        &name[pos + 1..]
    } else {
        name
    }
}

/// Parse an RFC 2616 HTTP-date (`Mon, 02 Jan 2006 15:04:05 GMT`) into a
/// Windows FILETIME (100-ns intervals since 1601-01-01). Returns 0 on failure.
fn parse_http_date(s: &str) -> u64 {
    // Use a hand-rolled parser to avoid pulling in chrono.
    // Format: "Mon, 02 Jan 2006 15:04:05 GMT"
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 6 {
        return 0;
    }
    let day: u64 = parts[1].parse().unwrap_or(0);
    let month: u64 = match parts[2] {
        "Jan" => 1, "Feb" => 2, "Mar" => 3, "Apr" => 4,
        "May" => 5, "Jun" => 6, "Jul" => 7, "Aug" => 8,
        "Sep" => 9, "Oct" => 10, "Nov" => 11, "Dec" => 12,
        _ => return 0,
    };
    let year: u64 = parts[3].parse().unwrap_or(0);
    let time_parts: Vec<&str> = parts[4].split(':').collect();
    if time_parts.len() < 3 {
        return 0;
    }
    let hour: u64 = time_parts[0].parse().unwrap_or(0);
    let min: u64  = time_parts[1].parse().unwrap_or(0);
    let sec: u64  = time_parts[2].parse().unwrap_or(0);

    // Days since 1601-01-01 → Unix epoch offset + days in date → seconds
    // Simplified: compute Unix timestamp then convert.
    let unix_secs = date_to_unix(year, month, day, hour, min, sec);
    (unix_secs + 11_644_473_600) * 10_000_000
}

fn date_to_unix(year: u64, month: u64, day: u64, h: u64, m: u64, s: u64) -> u64 {
    // Days from 1970-01-01 to year-month-day (Gregorian proleptic calendar).
    let y = if month <= 2 { year - 1 } else { year };
    let m2 = if month <= 2 { month + 9 } else { month - 3 };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m2 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146097 + doe;
    let days_since_1970 = days_since_epoch.saturating_sub(719468);
    days_since_1970 * 86400 + h * 3600 + m * 60 + s
}

// ── Multipart accumulator ─────────────────────────────────────────────────────

static WEBDAV_PARTS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Vec<(i32, Bytes)>>>> =
    std::sync::OnceLock::new();

fn parts_map() -> &'static std::sync::Mutex<HashMap<String, Vec<(i32, Bytes)>>> {
    WEBDAV_PARTS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn upload_key(key: &str, upload_id: &str) -> String {
    format!("{}\x00{}", key, upload_id)
}

// ── CloudProvider ─────────────────────────────────────────────────────────────

#[async_trait]
impl CloudProvider for WebDavProvider {
    async fn list_dir(&self, prefix: &str) -> Result<ListDirResult, ProviderError> {
        let dir_url = self.dir_url(prefix);
        let xml = match self.propfind(&dir_url, "1").await {
            Ok(x) => x,
            Err(ProviderError::NotFound(_)) => {
                return Ok(ListDirResult { dirs: vec![], files: vec![] });
            }
            Err(e) => return Err(e),
        };

        let entries = parse_propfind(&xml);

        // The first entry is always the directory itself — skip it.
        let base_href = {
            let first = entries.first().map(|e| e.href.as_str()).unwrap_or("");
            first.trim_end_matches('/').to_string()
        };

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries.iter().skip(1) {
            let href = entry.href.trim_end_matches('/');
            let name = href.rsplit('/').next().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            // Skip the self-reference if somehow returned twice.
            if entry.href.trim_end_matches('/') == base_href {
                continue;
            }
            if entry.is_collection {
                dirs.push(name);
            } else {
                files.push((name, FileStat {
                    size: entry.content_length,
                    mtime_filetime: entry.last_modified,
                }));
            }
        }

        Ok(ListDirResult { dirs, files })
    }

    async fn stat(&self, key: &str) -> Result<Option<FileStat>, ProviderError> {
        let url = self.url(key);
        let xml = match self.propfind(&url, "0").await {
            Ok(x) => x,
            Err(ProviderError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };

        let entries = parse_propfind(&xml);
        let entry = match entries.into_iter().next() {
            Some(e) => e,
            None => return Ok(None),
        };

        Ok(Some(FileStat {
            size: entry.content_length,
            mtime_filetime: entry.last_modified,
        }))
    }

    async fn get_range(&self, key: &str, offset: u64, length: u64) -> Result<Bytes, ProviderError> {
        if length == 0 {
            return Ok(Bytes::new());
        }
        let url = self.url(key);
        let end = offset + length - 1;
        let range_val = format!("bytes={offset}-{end}");

        let resp = self
            .client
            .get(&url)
            .header(header::AUTHORIZATION, self.auth())
            .header(header::RANGE, &range_val)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("GET {url}: {e}")))?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Err(ProviderError::NotFound(url));
        }
        if !status.is_success() {
            return Err(ProviderError::Other(format!("GET {url}: HTTP {status}")));
        }

        resp.bytes()
            .await
            .map_err(|e| ProviderError::Other(format!("GET read body: {e}")))
    }

    async fn put_object(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        let url = self.url(key);

        let resp = self
            .client
            .put(&url)
            .header(header::AUTHORIZATION, self.auth())
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(data)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("PUT {url}: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Other(format!("PUT {url}: HTTP {status}")));
        }

        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ProviderError> {
        let url = self.url(key);

        let resp = self
            .client
            .delete(&url)
            .header(header::AUTHORIZATION, self.auth())
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("DELETE {url}: {e}")))?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        if !status.is_success() {
            return Err(ProviderError::Other(format!("DELETE {url}: HTTP {status}")));
        }

        Ok(())
    }

    async fn copy_object(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        let stat = self.stat(from).await?.unwrap_or(FileStat { size: 0, mtime_filetime: 0 });
        let data = self.get_range(from, 0, stat.size).await?;
        self.put_object(to, data).await
    }

    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, ProviderError> {
        let mut result = Vec::new();
        self.walk_dir(prefix, &mut result).await?;
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

impl WebDavProvider {
    fn walk_dir<'a>(
        &'a self,
        rel_prefix: &'a str,
        result: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let dir_url = self.dir_url(rel_prefix);
            let xml = match self.propfind(&dir_url, "1").await {
                Ok(x) => x,
                Err(ProviderError::NotFound(_)) => return Ok(()),
                Err(e) => return Err(e),
            };

            let entries = parse_propfind(&xml);

            for entry in entries.into_iter().skip(1) {
                let href = entry.href.trim_end_matches('/');
                let name = href.rsplit('/').next().unwrap_or("").to_string();
                if name.is_empty() {
                    continue;
                }
                let child_rel = if rel_prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", rel_prefix.trim_end_matches('/'), name)
                };
                if entry.is_collection {
                    self.walk_dir(&child_rel, result).await?;
                } else {
                    result.push(child_rel);
                }
            }

            Ok(())
        })
    }
}

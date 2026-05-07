use std::{collections::HashMap, sync::Arc, time::UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use suppaftp::{list::File, tokio::AsyncFtpStream, FtpError, FtpResult};
use tokio::sync::Mutex;

use super::{CloudProvider, CompletedPart, FileStat, ListDirResult, ProviderError};

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub root_path: String,
    pub use_tls: bool,
}

// ── Pool ──────────────────────────────────────────────────────────────────────

const POOL_MAX: usize = 4;

/// Trait object interface for a pooled FTP connection.
#[async_trait]
trait FtpConn: Send {
    async fn mlsd_lines(&mut self, path: &str) -> FtpResult<Vec<String>>;
    async fn mlst_line(&mut self, path: &str) -> FtpResult<String>;
    async fn retr_all(&mut self, path: &str) -> FtpResult<Vec<u8>>;
    async fn stor(&mut self, path: &str, data: Vec<u8>) -> FtpResult<()>;
    async fn dele(&mut self, path: &str) -> FtpResult<()>;
    async fn rnfr_rnto(&mut self, from: &str, to: &str) -> FtpResult<()>;
    async fn mkd(&mut self, path: &str) -> FtpResult<()>;
    async fn nlst_dir(&mut self, path: &str) -> FtpResult<Vec<String>>;
}

// ── Impl for plain (non-TLS) AsyncFtpStream ──────────────────────────────────

#[async_trait]
impl FtpConn for AsyncFtpStream {
    async fn mlsd_lines(&mut self, path: &str) -> FtpResult<Vec<String>> {
        self.mlsd(Some(path)).await
    }

    async fn mlst_line(&mut self, path: &str) -> FtpResult<String> {
        self.mlst(Some(path)).await
    }

    async fn retr_all(&mut self, path: &str) -> FtpResult<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        let stream = self.retr_as_stream(path).await?;
        let mut buf = Vec::new();
        let mut s = stream;
        s.read_to_end(&mut buf)
            .await
            .map_err(FtpError::ConnectionError)?;
        self.finalize_retr_stream(s).await?;
        Ok(buf)
    }

    async fn stor(&mut self, path: &str, data: Vec<u8>) -> FtpResult<()> {
        let mut cursor = std::io::Cursor::new(data);
        self.put_file(path, &mut cursor).await.map(|_| ())
    }

    async fn dele(&mut self, path: &str) -> FtpResult<()> {
        self.rm(path).await
    }

    async fn rnfr_rnto(&mut self, from: &str, to: &str) -> FtpResult<()> {
        self.rename(from, to).await
    }

    async fn mkd(&mut self, path: &str) -> FtpResult<()> {
        self.mkdir(path).await.map(|_| ())
    }

    async fn nlst_dir(&mut self, path: &str) -> FtpResult<Vec<String>> {
        self.nlst(Some(path)).await
    }
}

// ── Provider ──────────────────────────────────────────────────────────────────

pub struct FtpProvider {
    config: FtpConfig,
    pool: Arc<Mutex<Vec<Box<dyn FtpConn>>>>,
}

impl FtpProvider {
    pub async fn new(config: FtpConfig) -> Result<Self, ProviderError> {
        Ok(Self {
            config,
            pool: Arc::new(Mutex::new(Vec::with_capacity(POOL_MAX))),
        })
    }

    async fn acquire(&self) -> Result<Box<dyn FtpConn>, ProviderError> {
        {
            let mut guard = self.pool.lock().await;
            if let Some(c) = guard.pop() {
                return Ok(c);
            }
        }
        self.connect().await
    }

    fn release(&self, conn: Box<dyn FtpConn>) {
        let pool = Arc::clone(&self.pool);
        tokio::spawn(async move {
            let mut guard = pool.lock().await;
            if guard.len() < POOL_MAX {
                guard.push(conn);
            }
        });
    }

    async fn connect(&self) -> Result<Box<dyn FtpConn>, ProviderError> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let mut ftp: AsyncFtpStream = AsyncFtpStream::connect(&addr)
            .await
            .map_err(|e| ProviderError::Other(format!("FTP connect: {e}")))?;

        ftp.login(&self.config.username, &self.config.password)
            .await
            .map_err(|e| ProviderError::Other(format!("FTP login: {e}")))?;

        ftp.transfer_type(suppaftp::types::FileType::Binary)
            .await
            .map_err(|e| ProviderError::Other(format!("FTP TYPE I: {e}")))?;

        Ok(Box::new(ftp))
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

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_mlsd(lines: Vec<String>) -> Vec<File> {
    lines
        .iter()
        .filter_map(|l| File::from_mlsx_line(l).ok())
        .collect()
}

fn parse_mlst(line: &str) -> Option<File> {
    // MLST response may have a leading space and trailing CRLF.
    let trimmed = line.trim();
    File::from_mlsx_line(trimmed).ok()
}

fn systemtime_to_filetime(st: std::time::SystemTime) -> u64 {
    let secs = st.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    (secs + 11_644_473_600) * 10_000_000
}

fn ftp_err_is_notfound(msg: &str) -> bool {
    msg.contains("550") || msg.contains("No such file") || msg.contains("not found")
}

// ── Multipart accumulator ─────────────────────────────────────────────────────

static FTP_PARTS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Vec<(i32, Bytes)>>>> =
    std::sync::OnceLock::new();

fn parts_map() -> &'static std::sync::Mutex<HashMap<String, Vec<(i32, Bytes)>>> {
    FTP_PARTS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn upload_key(key: &str, upload_id: &str) -> String {
    format!("{}\x00{}", key, upload_id)
}

// ── CloudProvider ─────────────────────────────────────────────────────────────

#[async_trait]
impl CloudProvider for FtpProvider {
    async fn list_dir(&self, prefix: &str) -> Result<ListDirResult, ProviderError> {
        let dir_path = self.abs_path(prefix);
        let mut conn = self.acquire().await?;

        let lines = conn
            .mlsd_lines(&dir_path)
            .await
            .map_err(|e| ProviderError::Other(format!("MLSD {dir_path:?}: {e}")))?;

        self.release(conn);

        let entries = parse_mlsd(lines);
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries {
            let name = entry.name().to_string();
            if name == "." || name == ".." {
                continue;
            }
            if entry.is_directory() {
                dirs.push(name);
            } else {
                let size = entry.size() as u64;
                let mtime_filetime = systemtime_to_filetime(entry.modified());
                files.push((name, FileStat { size, mtime_filetime }));
            }
        }

        Ok(ListDirResult { dirs, files })
    }

    async fn stat(&self, key: &str) -> Result<Option<FileStat>, ProviderError> {
        let path = self.abs_path(key);
        let mut conn = self.acquire().await?;

        let line = match conn.mlst_line(&path).await {
            Ok(l) => l,
            Err(e) => {
                self.release(conn);
                if ftp_err_is_notfound(&e.to_string()) {
                    return Ok(None);
                }
                return Err(ProviderError::Other(format!("MLST {path:?}: {e}")));
            }
        };

        self.release(conn);

        let entry = match parse_mlst(&line) {
            Some(e) => e,
            None => return Ok(None),
        };

        let size = entry.size() as u64;
        let mtime_filetime = systemtime_to_filetime(entry.modified());
        Ok(Some(FileStat { size, mtime_filetime }))
    }

    async fn get_range(&self, key: &str, offset: u64, length: u64) -> Result<Bytes, ProviderError> {
        if length == 0 {
            return Ok(Bytes::new());
        }
        let path = self.abs_path(key);
        let mut conn = self.acquire().await?;

        let full = conn
            .retr_all(&path)
            .await
            .map_err(|e| ProviderError::Other(format!("RETR {path:?}: {e}")))?;

        self.release(conn);

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
        let mut conn = self.acquire().await?;

        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = conn.mkd(parent.to_string_lossy().as_ref()).await;
        }

        conn.stor(&path, data.to_vec())
            .await
            .map_err(|e| ProviderError::Other(format!("STOR {path:?}: {e}")))?;

        self.release(conn);
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ProviderError> {
        let path = self.abs_path(key);
        let mut conn = self.acquire().await?;
        match conn.dele(&path).await {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if !ftp_err_is_notfound(&msg) {
                    self.release(conn);
                    return Err(ProviderError::Other(format!("DELE {path:?}: {msg}")));
                }
            }
        }
        self.release(conn);
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

impl FtpProvider {
    fn walk_dir_boxed<'a>(
        &'a self,
        abs_dir: String,
        rel_prefix: String,
        result: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let mut conn = self.acquire().await?;
            let lines = match conn.mlsd_lines(&abs_dir).await {
                Ok(l) => {
                    self.release(conn);
                    l
                }
                Err(_) => {
                    self.release(conn);
                    return Ok(());
                }
            };
            let entries = parse_mlsd(lines);
            for entry in entries {
                let name = entry.name().to_string();
                if name == "." || name == ".." {
                    continue;
                }
                let child_rel = if rel_prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", rel_prefix, name)
                };
                let child_abs = format!("{}/{}", abs_dir, name);
                if entry.is_directory() {
                    self.walk_dir_boxed(child_abs, child_rel, result).await?;
                } else {
                    result.push(child_rel);
                }
            }
            Ok(())
        })
    }
}

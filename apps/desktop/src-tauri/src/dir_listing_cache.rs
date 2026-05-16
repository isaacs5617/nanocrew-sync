//! Persistent disk-backed cache for S3 directory listings.
//!
//! Survives app restarts so large folders don't have to re-paginate
//! S3 LIST on every launch. Sits beneath the in-memory `list_cache` in
//! `S3Fs` — a disk hit functionally equivalent to a fresh S3 fetch but in
//! milliseconds instead of tens of seconds.
//!
//! Layout: `<cache_root>/drive-<id>/dir-listings/<sha256(prefix)>.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::winfsp_vfs::{CachedList, Meta};

/// Disk entries fresher than this are served as-is.
const DISK_LIST_TTL_SECS: u64 = 24 * 60 * 60; // 24h
/// On a miss, entries older than this are deleted from disk.
const DISK_LIST_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60; // 7d

#[derive(Serialize, Deserialize)]
struct DiskListEntry {
    prefix: String,
    cached_at: u64,
    dirs: Vec<String>,
    files: Vec<DiskFile>,
}

#[derive(Serialize, Deserialize)]
struct DiskFile {
    name: String,
    size: u64,
    mtime_filetime: u64,
}

pub struct DirListingCache {
    base: PathBuf,
}

impl DirListingCache {
    pub fn new(base: PathBuf) -> Self {
        let _ = fs::create_dir_all(&base);
        Self { base }
    }

    fn path_for(&self, prefix: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(prefix.as_bytes());
        let hex = hex_encode(&hasher.finalize());
        self.base.join(format!("{hex}.json"))
    }

    /// Load if present and fresh (<24h). Returns None on miss/stale/error.
    /// Stale entries older than the max-age are evicted as a side effect.
    pub fn load(&self, prefix: &str) -> Option<CachedList> {
        let path = self.path_for(prefix);
        let bytes = fs::read(&path).ok()?;
        let entry: DiskListEntry = serde_json::from_slice(&bytes).ok()?;
        let now = now_secs();
        let age = now.saturating_sub(entry.cached_at);
        if age > DISK_LIST_TTL_SECS {
            if age > DISK_LIST_MAX_AGE_SECS {
                let _ = fs::remove_file(&path);
            }
            return None;
        }
        Some(CachedList::from_parts(
            entry.dirs,
            entry
                .files
                .into_iter()
                .map(|f| {
                    (
                        f.name,
                        Meta::new_file(f.size, f.mtime_filetime),
                    )
                })
                .collect(),
        ))
    }

    /// Persist the listing atomically (temp file + rename).
    pub fn save(&self, prefix: &str, listing: &CachedList) {
        let entry = DiskListEntry {
            prefix: prefix.to_string(),
            cached_at: now_secs(),
            dirs: listing.dirs().to_vec(),
            files: listing
                .files()
                .iter()
                .map(|(name, meta)| DiskFile {
                    name: name.clone(),
                    size: meta.size(),
                    mtime_filetime: meta.mtime_filetime(),
                })
                .collect(),
        };
        let path = self.path_for(prefix);
        let tmp = path.with_extension("json.tmp");
        if let Ok(bytes) = serde_json::to_vec(&entry) {
            if fs::write(&tmp, &bytes).is_ok() {
                let _ = fs::rename(&tmp, &path);
            } else {
                let _ = fs::remove_file(&tmp);
            }
        }
    }

    pub fn invalidate(&self, prefix: &str) {
        let _ = fs::remove_file(self.path_for(prefix));
    }

    #[allow(dead_code)]
    pub fn base_dir(&self) -> &Path {
        &self.base
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

//! Persistent disk-backed cache for S3 directory listings.
//!
//! Survives app restarts so large folders don't have to re-paginate
//! S3 LIST on every launch. Sits beneath the in-memory `list_cache` in
//! `S3Fs` — a disk hit functionally equivalent to a fresh S3 fetch but in
//! milliseconds instead of tens of seconds.
//!
//! Layout: `<cache_root>/drive-<id>/dir-listings/<sha256(prefix)>.json`.
//!
//! v0.2.16 changes:
//!   * `DISK_LIST_TTL_SECS` cut from 24 h → 60 s. The old 24 h value was
//!     the smoking gun of the "user B doesn't see user A's uploads" bug:
//!     any listing under a day old was served straight to Explorer without
//!     any freshness probe. 60 s aligns with the in-memory `LIST_TTL`, so
//!     the disk cache now serves as a "warm restart" store rather than a
//!     stale-window store. Real cross-user freshness comes from the
//!     v0.3.0 coordinator's push invalidations — this TTL is the polling
//!     backstop for when the coordinator is unreachable.
//!   * `save()` writes to a UUID-suffixed tmp file (same race fix pattern
//!     as `cache.rs::put_block` shipped in v0.2.12). Two concurrent savers
//!     for the same prefix (e.g. a foreground `list_dir` finishing at the
//!     same time as the background refresh loop) previously both wrote to
//!     the same tmp path, torn the file, and produced corrupt JSON that
//!     `load()` silently dropped as `None`.
//!   * New `fingerprint` field on each entry (cheap ETag stand-in derived
//!     from the listing's stable properties). Consumers can compare it
//!     against a re-listing without reading the whole payload from disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::winfsp_vfs::{CachedList, Meta};

/// Disk entries fresher than this are served as-is. See module comment —
/// this is now the polling backstop, not the freshness horizon.
const DISK_LIST_TTL_SECS: u64 = 60;
/// On a miss, entries older than this are deleted from disk.
const DISK_LIST_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60; // 7d

#[derive(Serialize, Deserialize)]
struct DiskListEntry {
    prefix: String,
    cached_at: u64,
    /// v0.2.16: stable fingerprint of the listing (count + max mtime + xor
    /// of hashed names). Cheap enough to compute per-listing, useful for
    /// short-circuiting a "did anything change?" probe without deserialising
    /// the whole entry. Old entries without this field default to 0 via
    /// serde which is fine — the first fresh save writes a real value.
    #[serde(default)]
    fingerprint: u64,
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

    /// Load if present and fresh (< TTL). Returns None on miss/stale/error.
    /// Stale entries older than the max-age are evicted as a side effect.
    pub fn load(&self, prefix: &str) -> Option<CachedList> {
        self.load_with_fingerprint(prefix).map(|(list, _)| list)
    }

    /// Same as `load` but also returns the fingerprint so callers can
    /// decide whether to schedule a background revalidation.
    pub fn load_with_fingerprint(&self, prefix: &str) -> Option<(CachedList, u64)> {
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
        let fp = entry.fingerprint;
        let list = CachedList::from_parts(
            entry.dirs,
            entry
                .files
                .into_iter()
                .map(|f| (f.name, Meta::new_file(f.size, f.mtime_filetime)))
                .collect(),
        );
        Some((list, fp))
    }

    /// Compute the stable fingerprint of a listing. Cheap: linear in entry
    /// count. Used by callers for cross-machine "did this listing change
    /// since we last saw it?" checks without paying deserialization cost.
    pub fn fingerprint_of(listing: &CachedList) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a seed
        let dirs = listing.dirs();
        let files = listing.files();
        for d in dirs {
            h = h.wrapping_mul(0x100_0000_01b3);
            for b in d.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        }
        for (name, meta) in files {
            h = h.wrapping_mul(0x100_0000_01b3);
            h ^= meta.size();
            h = h.wrapping_mul(0x100_0000_01b3);
            h ^= meta.mtime_filetime();
            h = h.wrapping_mul(0x100_0000_01b3);
            for b in name.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    /// Persist the listing atomically. v0.2.16: unique-suffix tmp file so
    /// two concurrent savers can't corrupt each other's tmp write.
    pub fn save(&self, prefix: &str, listing: &CachedList) {
        let entry = DiskListEntry {
            prefix: prefix.to_string(),
            cached_at: now_secs(),
            fingerprint: Self::fingerprint_of(listing),
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
        let tmp_suffix = uuid::Uuid::new_v4().simple().to_string();
        let tmp = path.with_extension(format!("{tmp_suffix}.tmp"));
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

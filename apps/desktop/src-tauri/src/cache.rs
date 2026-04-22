//! Per-drive on-disk LRU block cache.
//!
//! Every [`S3Fs::get_range`](crate::winfsp_vfs::S3Fs::get_range) is decomposed
//! into `CACHE_BLOCK`-aligned windows. Each block is fetched once from S3 and
//! then reused until it is explicitly invalidated (our own write/delete/rename
//! paths) or evicted by the LRU sweeper. Pinned keys are exempt from eviction.
//!
//! Layout on disk:
//!
//! ```text
//! %LOCALAPPDATA%\NanoCrew\Sync\cache\
//!   drive-<id>\
//!     <hex[0..2]>\<hex[2..]>\<offset>-<len>.bin
//! ```
//!
//! where `hex = sha256(key)` — using the hash side-steps illegal Windows
//! filename characters (`*?<>|:"\\/`) in object keys without any escape layer.
//!
//! Index: a single `cache_entries` SQLite row per on-disk block; rows are
//! written immediately on `put_block` and updated on every `get_block` hit
//! (via `last_access`). `evict_if_needed` walks the LRU index oldest-first
//! and deletes rows / files until total `SUM(size_bytes)` drops below the
//! configured cap.
//!
//! Concurrency: all DB access goes through a `Mutex<Connection>` opened by
//! `DiskCache::new` — separate from the main app connection, which is safe
//! under SQLite WAL. Multiple reader threads competing for the same cold
//! block will both fetch and both write; the final `put_block` wins. That's
//! wasted bandwidth but not incorrect.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand_core::RngCore;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

/// Nonce length for AES-256-GCM per RFC 5116. Must be unique per (key,
/// encryption) — we generate a fresh random nonce for every `put_block`.
const NONCE_LEN: usize = 12;
/// GCM's 128-bit authentication tag, appended to the ciphertext by the
/// `aes-gcm` crate's `encrypt` API.
const TAG_LEN: usize = 16;

/// Block size we align cached ranges to. 1 MiB is a good trade-off: small
/// enough that partial-file reads don't drag in excessive extra bytes, large
/// enough that the SQLite index stays tractable (~1k rows per GiB of data).
pub const CACHE_BLOCK: u64 = 1 * 1024 * 1024;

/// How often the background sweeper runs.
const EVICT_INTERVAL: Duration = Duration::from_secs(60);

pub struct DiskCache {
    pub drive_id: i64,
    root: PathBuf,
    /// Max on-disk footprint in bytes. 0 = cache disabled (treated as a
    /// pass-through — `get_block` always misses, `put_block` is a no-op).
    max_bytes: AtomicU64,
    enabled: AtomicBool,
    db: Mutex<Connection>,
    stop: Arc<AtomicBool>,
    evict_thread: Mutex<Option<JoinHandle<()>>>,
    /// Per-drive AES-256-GCM cipher. Derived from machine ID + a per-install
    /// random salt + the drive id so a stolen laptop backup can't trivially
    /// decrypt another install's cache. `None` when encryption is disabled
    /// (cache disabled, or salt couldn't be read/written).
    cipher: Option<Aes256Gcm>,
}

impl DiskCache {
    /// Open (or create) a per-drive cache. `db_path` must point at the same
    /// SQLite file the main app uses so that `cache_entries` / `pinned_keys`
    /// rows are visible to the pin/unpin commands.
    pub fn new(
        drive_id: i64,
        root: PathBuf,
        db_path: &Path,
        max_bytes: u64,
        enabled: bool,
    ) -> Result<Arc<Self>, String> {
        fs::create_dir_all(&root)
            .map_err(|e| format!("create cache root {}: {e}", root.display()))?;
        let conn = Connection::open(db_path)
            .map_err(|e| format!("cache db open: {e}"))?;
        // WAL is already enabled by db::open; repeat the pragma so a fresh
        // install that somehow opens us first still gets it.
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;");

        let cipher = derive_cipher(&conn, drive_id);

        Ok(Arc::new(Self {
            drive_id,
            root,
            max_bytes: AtomicU64::new(if enabled { max_bytes } else { 0 }),
            enabled: AtomicBool::new(enabled && max_bytes > 0),
            db: Mutex::new(conn),
            stop: Arc::new(AtomicBool::new(false)),
            evict_thread: Mutex::new(None),
            cipher,
        }))
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Cache path: `<root>/<hex[0..2]>/<hex[2..]>/<offset>-<len>.bin`. The
    /// two-char fanout keeps any individual directory under ~4k entries even
    /// at cache sizes in the hundreds of thousands of blocks.
    fn path_for(&self, key: &str, block_start: u64, len: u64) -> PathBuf {
        let mut h = Sha256::new();
        h.update(key.as_bytes());
        let hex = hex_encode(&h.finalize());
        self.root
            .join(&hex[0..2])
            .join(&hex[2..])
            .join(format!("{block_start}-{len}.bin"))
    }

    /// Return the bytes for a single cached block, or `None` on miss. Bumps
    /// `last_access` on a hit.
    pub fn get_block(&self, key: &str, block_start: u64) -> Option<Vec<u8>> {
        if !self.is_enabled() {
            return None;
        }
        // Find the (possibly short-at-EOF) length recorded for this block.
        let row = {
            let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
            conn.query_row(
                "SELECT len, size_bytes FROM cache_entries
                 WHERE drive_id = ?1 AND key = ?2 AND offset = ?3",
                params![self.drive_id, key, block_start as i64],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .ok()
        };
        let (len, _size) = row?;
        let path = self.path_for(key, block_start, len as u64);
        let mut f = fs::File::open(&path).ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;

        // Decrypt if we have a cipher. Layout: [nonce:12][ciphertext+tag].
        // A failed decrypt means tampering, wrong key, or pre-encryption
        // legacy blob — drop it and re-fetch.
        let plaintext = if let Some(cipher) = &self.cipher {
            if buf.len() < NONCE_LEN + TAG_LEN {
                let _ = self.remove_row(key, block_start);
                let _ = fs::remove_file(&path);
                return None;
            }
            let (nonce_bytes, ct) = buf.split_at(NONCE_LEN);
            let nonce = Nonce::from_slice(nonce_bytes);
            match cipher.decrypt(nonce, ct) {
                Ok(pt) => pt,
                Err(_) => {
                    let _ = self.remove_row(key, block_start);
                    let _ = fs::remove_file(&path);
                    return None;
                }
            }
        } else {
            buf
        };

        if plaintext.len() as i64 != len {
            // Out-of-sync — drop the row so a future hit re-fetches.
            let _ = self.remove_row(key, block_start);
            let _ = fs::remove_file(&path);
            return None;
        }
        let buf = plaintext;
        // Touch LRU. Separate scope so we don't double-lock.
        {
            let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
            let _ = conn.execute(
                "UPDATE cache_entries SET last_access = ?1
                 WHERE drive_id = ?2 AND key = ?3 AND offset = ?4",
                params![now_secs(), self.drive_id, key, block_start as i64],
            );
        }
        Some(buf)
    }

    /// Store a freshly fetched block on disk and index it. The length of
    /// `bytes` may be shorter than `CACHE_BLOCK` at EOF; record whatever it
    /// actually is.
    pub fn put_block(&self, key: &str, block_start: u64, bytes: &[u8]) {
        if !self.is_enabled() || bytes.is_empty() {
            return;
        }
        let len = bytes.len() as u64;
        let path = self.path_for(key, block_start, len);
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::warn!(target: "nanocrew::cache",
                    drive_id = self.drive_id, "mkdir {}: {e}", parent.display());
                return;
            }
        }

        // Encrypt if a cipher is configured. Each block gets its own fresh
        // random nonce, which is prepended to the ciphertext on disk. GCM's
        // 128-bit auth tag is appended by `encrypt` — a corrupted cache file
        // will fail to decrypt and be dropped on read.
        let (disk_bytes, disk_len) = if let Some(cipher) = &self.cipher {
            let mut nonce_bytes = [0u8; NONCE_LEN];
            rand_core::OsRng.fill_bytes(&mut nonce_bytes);
            let nonce = Nonce::from_slice(&nonce_bytes);
            match cipher.encrypt(nonce, bytes) {
                Ok(ct) => {
                    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
                    out.extend_from_slice(&nonce_bytes);
                    out.extend_from_slice(&ct);
                    let disk_len = out.len() as u64;
                    (out, disk_len)
                }
                Err(e) => {
                    tracing::warn!(target: "nanocrew::cache",
                        drive_id = self.drive_id, "encrypt: {e}");
                    return;
                }
            }
        } else {
            (bytes.to_vec(), len)
        };

        // Write to a tmp file and rename so a partial write can't be
        // resurfaced as a bogus cache hit.
        let tmp = path.with_extension("tmp");
        let write_result = (|| -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&disk_bytes)?;
            f.sync_data()?;
            fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if let Err(e) = write_result {
            tracing::warn!(target: "nanocrew::cache",
                drive_id = self.drive_id, "put_block {}: {e}", path.display());
            let _ = fs::remove_file(&tmp);
            return;
        }

        let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
        let _ = conn.execute(
            "INSERT OR REPLACE INTO cache_entries
                (drive_id, key, offset, len, size_bytes, etag, last_access)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
            params![
                self.drive_id,
                key,
                block_start as i64,
                len as i64,
                disk_len as i64,
                now_secs(),
            ],
        );
    }

    /// Drop all cached blocks for a single key (after our own upload /
    /// delete / rename so the next reader sees fresh bytes).
    pub fn invalidate_key(&self, key: &str) {
        // Collect block descriptors first so we can delete files after the
        // lock is released.
        let blocks: Vec<(i64, i64)> = {
            let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = match conn.prepare(
                "SELECT offset, len FROM cache_entries
                 WHERE drive_id = ?1 AND key = ?2",
            ) {
                Ok(s) => s,
                Err(_) => return,
            };
            let rows: Result<Vec<_>, _> = stmt
                .query_map(params![self.drive_id, key], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                })
                .and_then(|it| it.collect());
            let list = rows.unwrap_or_default();
            let _ = conn.execute(
                "DELETE FROM cache_entries WHERE drive_id = ?1 AND key = ?2",
                params![self.drive_id, key],
            );
            list
        };
        for (off, len) in blocks {
            let p = self.path_for(key, off as u64, len as u64);
            let _ = fs::remove_file(&p);
        }
    }

    // Pin helpers. The Tauri `commands::cache` handlers bypass DiskCache and
    // write `pinned_keys` directly so pins persist across mount/unmount; these
    // methods exist for in-process / test use and are kept on the API surface.
    #[allow(dead_code)]
    pub fn pin(&self, key: &str) -> Result<(), String> {
        let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO pinned_keys (drive_id, key) VALUES (?1, ?2)",
            params![self.drive_id, key],
        )
        .map_err(|e| format!("pin: {e}"))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn unpin(&self, key: &str) -> Result<(), String> {
        let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "DELETE FROM pinned_keys WHERE drive_id = ?1 AND key = ?2",
            params![self.drive_id, key],
        )
        .map_err(|e| format!("unpin: {e}"))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn is_pinned(&self, key: &str) -> bool {
        let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
        conn.query_row(
            "SELECT 1 FROM pinned_keys WHERE drive_id = ?1 AND key = ?2",
            params![self.drive_id, key],
            |_| Ok(()),
        )
        .is_ok()
    }

    /// Walk cached entries in LRU order, skipping any key that lives in
    /// `pinned_keys`, deleting rows+files until `SUM(size_bytes)` drops
    /// below the configured cap.
    pub fn evict_if_needed(&self) -> Result<(), String> {
        if !self.is_enabled() {
            return Ok(());
        }
        let cap = self.max_bytes.load(Ordering::Relaxed);
        if cap == 0 {
            return Ok(());
        }

        let (mut total, victims) = {
            let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
            let total: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(size_bytes), 0) FROM cache_entries WHERE drive_id = ?1",
                    params![self.drive_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if (total as u64) <= cap {
                return Ok(());
            }
            // Pull eviction candidates oldest-first, skipping pinned keys.
            let mut stmt = conn
                .prepare(
                    "SELECT c.key, c.offset, c.len, c.size_bytes
                     FROM cache_entries c
                     LEFT JOIN pinned_keys p
                       ON p.drive_id = c.drive_id AND p.key = c.key
                     WHERE c.drive_id = ?1 AND p.key IS NULL
                     ORDER BY c.last_access ASC",
                )
                .map_err(|e| format!("evict prep: {e}"))?;
            let rows: Vec<(String, i64, i64, i64)> = stmt
                .query_map(params![self.drive_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .and_then(|it| it.collect())
                .map_err(|e| format!("evict query: {e}"))?;
            (total, rows)
        };

        for (key, off, len, sz) in victims {
            if (total as u64) <= cap {
                break;
            }
            let p = self.path_for(&key, off as u64, len as u64);
            let _ = fs::remove_file(&p);
            {
                let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
                let _ = conn.execute(
                    "DELETE FROM cache_entries
                     WHERE drive_id = ?1 AND key = ?2 AND offset = ?3 AND len = ?4",
                    params![self.drive_id, key, off, len],
                );
            }
            total -= sz;
        }
        Ok(())
    }

    fn remove_row(&self, key: &str, block_start: u64) -> Result<(), rusqlite::Error> {
        let conn = self.db.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "DELETE FROM cache_entries
             WHERE drive_id = ?1 AND key = ?2 AND offset = ?3",
            params![self.drive_id, key, block_start as i64],
        )?;
        Ok(())
    }

    /// Spawn the background eviction thread. Safe to call multiple times —
    /// second+ calls are no-ops.
    pub fn start_eviction(self: &Arc<Self>) {
        let mut guard = self.evict_thread.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_some() {
            return;
        }
        let me = Arc::clone(self);
        let handle = std::thread::Builder::new()
            .name(format!("nanocrew-evict-{}", me.drive_id))
            .spawn(move || {
                while !me.stop.load(Ordering::Relaxed) {
                    if let Err(e) = me.evict_if_needed() {
                        tracing::warn!(target: "nanocrew::cache",
                            drive_id = me.drive_id, "evict: {e}");
                    }
                    // Wake every second so `stop()` is responsive.
                    for _ in 0..EVICT_INTERVAL.as_secs() {
                        if me.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            })
            .ok();
        *guard = handle;
    }

    /// Signal the eviction thread to exit and wait for it. Called from the
    /// mount thread during teardown.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let handle = {
            let mut guard = self.evict_thread.lock().unwrap_or_else(|p| p.into_inner());
            guard.take()
        };
        if let Some(h) = handle {
            let _ = h.join();
        }
    }
}

impl Drop for DiskCache {
    fn drop(&mut self) {
        // Belt-and-braces — if the mount forgets to call stop() we still
        // don't leak the sweeper thread.
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Derive the per-drive AES-256-GCM key and return a ready-to-use cipher.
///
/// The key is `SHA-256(machine_id || salt || "drive-{id}")` where:
/// - `machine_id` comes from `HKLM\...\MachineGuid` (Windows) — stable across
///   reboots and NanoCrew reinstalls, changes when the OS image is re-laid.
/// - `salt` is 32 random bytes generated on first use and stored hex in the
///   `prefs` table under `cache_key_salt`. This makes each install's cache
///   files unique even if someone recovers `machine_id` out-of-band.
/// - The drive-id suffix prevents a stolen drive-A cache file from decrypting
///   if renamed into drive-B's folder.
///
/// Returns `None` and logs on failure — callers treat that as "encryption
/// disabled" rather than failing the mount outright.
fn derive_cipher(conn: &Connection, drive_id: i64) -> Option<Aes256Gcm> {
    let salt = match load_or_create_salt(conn) {
        Some(s) => s,
        None => {
            tracing::warn!(target: "nanocrew::cache",
                drive_id, "cache encryption disabled: could not read/write cache_key_salt");
            return None;
        }
    };

    let machine = crate::file_lock::machine_id();
    let mut h = Sha256::new();
    h.update(machine.as_bytes());
    h.update(&salt);
    h.update(format!("drive-{drive_id}").as_bytes());
    let digest = h.finalize();

    // `Aes256Gcm::new_from_slice` requires exactly 32 bytes, which SHA-256 is.
    Aes256Gcm::new_from_slice(&digest).ok()
}

/// Fetch `cache_key_salt` from `prefs`; generate + persist one if it's absent.
/// Returns the raw 32 bytes.
fn load_or_create_salt(conn: &Connection) -> Option<Vec<u8>> {
    if let Ok(hex) = conn.query_row(
        "SELECT value FROM prefs WHERE key = 'cache_key_salt'",
        [],
        |r| r.get::<_, String>(0),
    ) {
        if let Some(bytes) = hex_decode(&hex) {
            if bytes.len() == 32 {
                return Some(bytes);
            }
        }
    }
    let mut salt = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut salt);
    let hex = hex_encode(&salt);
    conn.execute(
        "INSERT INTO prefs (key, value) VALUES ('cache_key_salt', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![hex],
    ).ok()?;
    Some(salt.to_vec())
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None; }
    let mut out = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let hi = nibble(b[i])?;
        let lo = nibble(b[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(label: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("nanocrew-cache-test-{label}-{}", now_secs()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn new_cache(label: &str, cap_bytes: u64) -> (Arc<DiskCache>, PathBuf) {
        let base = tmp_dir(label);
        let db_path = base.join("test.db");
        // Bootstrap schema via the normal opener.
        let _ = crate::db::open(&db_path).unwrap();
        let cache_root = base.join("cache");
        let cache = DiskCache::new(1, cache_root.clone(), &db_path, cap_bytes, true).unwrap();
        (cache, base)
    }

    #[test]
    fn put_then_get_round_trips() {
        let (c, _t) = new_cache("roundtrip", 10 * 1024 * 1024);
        let bytes = vec![7u8; 1024];
        c.put_block("some/key.bin", 0, &bytes);
        let got = c.get_block("some/key.bin", 0).expect("hit");
        assert_eq!(got, bytes);
    }

    #[test]
    fn invalidate_removes_all_blocks() {
        let (c, _t) = new_cache("invalidate", 10 * 1024 * 1024);
        c.put_block("k", 0, &vec![1u8; 64]);
        c.put_block("k", CACHE_BLOCK, &vec![2u8; 64]);
        assert!(c.get_block("k", 0).is_some());
        c.invalidate_key("k");
        assert!(c.get_block("k", 0).is_none());
        assert!(c.get_block("k", CACHE_BLOCK).is_none());
    }

    #[test]
    fn pin_survives_eviction() {
        let (c, _t) = new_cache("pin", 1024); // 1 KiB cap
        c.pin("keep").unwrap();
        c.put_block("keep", 0, &vec![0u8; 800]);
        c.put_block("drop", 0, &vec![0u8; 800]); // pushes us over
        c.evict_if_needed().unwrap();
        assert!(c.get_block("keep", 0).is_some(), "pinned block evicted");
        assert!(c.get_block("drop", 0).is_none(), "unpinned block retained");
    }
}

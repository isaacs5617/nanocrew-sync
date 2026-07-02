//! WinFsp-backed S3 filesystem. Replaces the Cloud Filter implementation in
//! `vfs/`. Gives real drive-letter semantics, live remote-change visibility,
//! and real Explorer copy progress via streaming multipart upload.
//!
//! Scope of *this* module:
//!   * Listing (TTL-cached)
//!   * Metadata (HEAD-equivalent, short cache)
//!   * Range reads streamed from S3 (no disk cache yet — deferred)
//!   * Multipart upload on write with Explorer progress via WinFsp's write path
//!   * Delete, rename
//!   * Empty-folder `.keep` markers
//!
//! Deferred to a follow-up session:
//!   * On-disk LRU cache + pin/"keep on device"
//!   * Bandwidth throttling
//!   * Tailscale auto-bypass
//!   * Shell extension overlays
//!
//! Path model: root = empty key. Subpaths use forward slashes. Windows gives
//! us `\foo\bar.txt`; we translate to `foo/bar.txt`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::c_void,
    fs::{File, OpenOptions},
    os::windows::fs::FileExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use aws_sdk_s3::Client;
use bytes::Bytes;
use tokio::runtime::Runtime;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{
            STATUS_ACCESS_DENIED, STATUS_END_OF_FILE, STATUS_INVALID_PARAMETER,
            STATUS_NOT_A_DIRECTORY, STATUS_OBJECT_NAME_COLLISION, STATUS_OBJECT_NAME_NOT_FOUND,
            STATUS_SHARING_VIOLATION,
        },
        Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
    },
};
use winfsp::{
    filesystem::{
        DirBuffer, DirInfo, DirMarker, FileInfo, FileSecurity, FileSystemContext, OpenFileInfo,
        VolumeInfo, WideNameInfo,
    },
    FspError, U16CStr,
};

/// Convert a `windows::Win32::Foundation::NTSTATUS` (any windows crate version)
/// into a `FspError`. Our windows crate version does not match winfsp's, so the
/// blanket `From<NTSTATUS>` impl from winfsp does not apply — we project to i32
/// explicitly.
#[inline]
fn nt(status: windows::Win32::Foundation::NTSTATUS) -> FspError {
    FspError::NTSTATUS(status.0)
}
use winfsp_sys::{FILE_ACCESS_RIGHTS, FILE_FLAGS_AND_ATTRIBUTES};

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
/// `FILE_ATTRIBUTE_OFFLINE` — Windows renders files with this bit using the
/// cloud / offline overlay icon (small ⊘ badge). We set it on every file
/// that isn't yet fully present in the block cache so users get a free
/// "cached vs not-cached" visual cue without writing a shell-extension
/// overlay handler.
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;

use crate::cache::{DiskCache, CACHE_BLOCK};
use crate::file_lock;
use crate::providers::CloudProvider;
use crate::throttle::RateLimiter;
use crate::types::{FileLockEvent, TransferPayload};

// ── Tuning ───────────────────────────────────────────────────────────────────

// 60s matches Mountain Duck's default — long enough that Explorer scrolling
// doesn't thrash the cache, short enough that out-of-band changes show up
// within a minute.
const LIST_TTL: Duration = Duration::from_secs(60);
const META_TTL: Duration = Duration::from_secs(60);
/// S3 multipart minimum is 5 MiB except for the last part. We target 8 MiB to
/// amortise request overhead but still emit progress events frequently.
/// Maximum directory entries enumerated for a single folder. Windows Explorer
/// becomes unresponsive well before millions of items regardless of backend,
/// and WinFsp's DirBuffer holds every entry in memory. Capping bounds memory
/// (~60 MB) and first-load time (~100 S3 calls). Folders larger than this are
/// truncated with a sentinel entry; users should use Search or subfolders.
const MAX_DIR_ENTRIES: usize = 100_000;
/// Filename shown at the end of a truncated listing so the truncation is
/// visible rather than silent.
const TRUNCATION_SENTINEL: &str = "⚠ FOLDER TOO LARGE — listing truncated, use Search.txt";
const PART_TARGET: usize = 16 * 1024 * 1024;
/// Background-refresh task interval (Wave 1: cache freshness).
const BACKGROUND_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);  // 5 min
/// Only re-check folders the user has visited within this window.
const RECENT_VISIT_WINDOW: Duration = Duration::from_secs(60 * 60);          // 1 hour
/// Capacity of the per-S3Fs visited-directories LRU.
const VISITED_DIRS_CAPACITY: usize = 64;
/// How many multipart part uploads we run in parallel. 8 matches the AWS CLI
/// default and saturates most home uplinks without exhausting connection
/// pools.
const UPLOAD_CONCURRENCY: usize = 8;
/// How many cache blocks we fetch in parallel when materializing a file on
/// open. Same rationale as `UPLOAD_CONCURRENCY` — saturates uplinks without
/// exhausting connection pools.
const MATERIALIZE_CONCURRENCY: usize = 8;
/// Only emit transfer_progress for files at or above this size. Smaller files
/// don't need the UI noise.
const MIN_TRANSFER_BYTES: u64 = 256 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

// ── Types ────────────────────────────────────────────────────────────────────

/// Metadata about a single S3 object or virtual directory.
#[derive(Clone, Debug)]
pub struct Meta {
    is_dir: bool,
    size: u64,
    /// Windows FILETIME (100-ns intervals since 1601-01-01). 0 for unknown.
    mtime_filetime: u64,
}

impl Meta {
    /// Construct a non-directory `Meta` from on-disk cache data.
    pub fn new_file(size: u64, mtime_filetime: u64) -> Self {
        Self { is_dir: false, size, mtime_filetime }
    }
    pub fn size(&self) -> u64 { self.size }
    pub fn mtime_filetime(&self) -> u64 { self.mtime_filetime }
}

/// Cached directory listing. Keyed by the directory's S3 prefix.
#[derive(Clone)]
pub struct CachedList {
    /// Subdirectory names (just the last path component, no trailing slash).
    pub(crate) dirs: Vec<String>,
    /// (filename, meta) pairs — filename is the last path component only.
    pub(crate) files: Vec<(String, Meta)>,
}

impl CachedList {
    /// Build a listing from already-decoded parts (used by the disk cache loader).
    pub fn from_parts(dirs: Vec<String>, files: Vec<(String, Meta)>) -> Self {
        Self { dirs, files }
    }
    pub fn dirs(&self) -> &[String] { &self.dirs }
    pub fn files(&self) -> &[(String, Meta)] { &self.files }
}

/// Per-open-handle state. WinFsp stores these behind `Box<OpenFile>` and
/// passes us `&OpenFile` in subsequent callbacks, so all mutable state must
/// live behind interior mutability.
pub enum OpenFile {
    Dir {
        key: String, // "" for root
        /// Per-handle WinFsp directory buffer. The fill happens inside the
        /// `acquire` lock in `read_directory`, which WinFsp uses to serialize
        /// concurrent enumeration of this handle.
        dir_buffer: Arc<DirBuffer>,
    },
    File {
        key: String,
        meta: Mutex<Meta>,
        /// `Some` once a write has begun on this handle. Dropped on `close`.
        write: Mutex<Option<WriteState>>,
        /// `Some` once a qualifying read has begun on this handle. Dropped on
        /// `close` (after the final "done"/"error" event is emitted).
        download: Mutex<Option<DownloadState>>,
        /// Set by `set_delete(true)`, acted on during `cleanup`.
        pending_delete: AtomicBool,
        /// True if this handle took the local writer slot + cross-device
        /// sentinel during `open`/`create`. Consulted on `close` so we only
        /// release what we acquired.
        holds_writer_lock: AtomicBool,
        /// Last `offset + len` seen by `read()` on this handle. Used to
        /// detect sequential access patterns for read-ahead prefetch.
        last_read_offset: Arc<Mutex<Option<u64>>>,
        /// Count of consecutive sequential reads. Reset to 0 on a non-
        /// sequential jump. Prefetch kicks in at >= 2.
        sequential_streak: Arc<AtomicU64>,
    },
}

/// Progress state for a download (read) on a single handle.
///
/// Unlike `WriteState`, downloads have no temp file, no multipart machinery,
/// and no backpressure loop — they are a one-way byte counter. We lazily
/// allocate this on the first `read()` for files at or above
/// `MIN_TRANSFER_BYTES`, emit a `start` event immediately, throttle subsequent
/// `progress` events on `PROGRESS_INTERVAL`, and emit `done`/`error` on
/// `close()` (based on whether `bytes_done` reached `size`).
pub struct DownloadState {
    xfer_id: u64,
    bytes_done: AtomicU64,
    /// Plain `Instant` — the outer `download: Mutex<Option<DownloadState>>`
    /// already serializes all access, so no independent lock is needed.
    last_emit: Instant,
    size: u64,
    filename: String,
}

/// Accumulating state for an in-progress write on a single handle.
///
/// Explorer submits writes in parallel and out of order, so we spool to a
/// local temp file via positioned writes (`seek_write`). We stream upload
/// parts to S3 **concurrently with the write phase**: once a contiguous
/// 16 MiB window is filled, it's dispatched as a multipart part. When the
/// upload pipeline is full (8 parts in flight), `write()` blocks — which
/// throttles Explorer's progress bar to match real S3 throughput instead of
/// local-disk speed.
pub struct WriteState {
    temp_path: PathBuf,
    temp_file: File,
    /// Highest `offset + len` observed across all writes (may include gaps).
    bytes_written: u64,

    // ── Streaming upload pipeline ────────────────────────────────────────
    /// Created lazily on first dispatch. Absent → haven't started an MPU
    /// (either nothing uploaded yet, or file small enough for single PUT).
    upload_id: Option<String>,
    /// Bytes that have been dispatched to upload_part tasks. Monotonic.
    dispatched_bytes: u64,
    /// Next part number to assign (1-based).
    next_part_number: i32,
    /// Collected CompletedParts. Tasks push their result here.
    completed_parts: Arc<std::sync::Mutex<Vec<crate::providers::CompletedPart>>>,
    /// First error encountered by any worker, sticky.
    upload_err: Arc<std::sync::Mutex<Option<String>>>,
    /// Permits = max in-flight parts. Acquiring blocks in `write()` when
    /// saturated — this is our backpressure mechanism.
    inflight_sem: Arc<tokio::sync::Semaphore>,
    /// Bytes confirmed uploaded (for progress reporting).
    bytes_uploaded: Arc<AtomicU64>,
    /// Extents of successful writes: start -> end. Merged on each write so
    /// we can compute `contig_bytes` without scanning.
    extents: BTreeMap<u64, u64>,
    /// Largest `N` such that [0, N) is fully written. Advances monotonically.
    contig_bytes: u64,

    // ── Progress ─────────────────────────────────────────────────────────
    xfer_id: Option<u64>,
    last_emit: Instant,
    filename: String,
    /// Total size hinted by `set_file_size` before copy starts (0 if unknown).
    total_size_hint: u64,
    /// `true` if this handle was opened via `create()` (new file). Reserved
    /// for future cleanup-on-failure semantics.
    #[allow(dead_code)]
    is_new: bool,
}

impl WriteState {
    /// Merge a newly written range into `extents` and advance `contig_bytes`
    /// if the range closed a gap. O(log n) amortised via BTreeMap splitting.
    fn record_extent(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }
        let mut new_start = start;
        let mut new_end = end;

        // Absorb any extent that overlaps or is adjacent to [new_start, new_end).
        let candidates: Vec<u64> = self
            .extents
            .range(..=new_end)
            .filter_map(|(&s, &e)| if e >= new_start { Some(s) } else { None })
            .collect();
        for s in candidates {
            if let Some(e) = self.extents.remove(&s) {
                if s < new_start {
                    new_start = s;
                }
                if e > new_end {
                    new_end = e;
                }
            }
        }
        self.extents.insert(new_start, new_end);

        // If the first extent starts at 0, contig_bytes = its end.
        if let Some((&s, &e)) = self.extents.iter().next() {
            if s == 0 && e > self.contig_bytes {
                self.contig_bytes = e;
            }
        }
    }
}

// ── The filesystem ───────────────────────────────────────────────────────────

pub struct S3Fs {
    pub rt: Runtime,
    /// Storage backend. Accepts VFS-relative keys (no bucket_prefix applied).
    pub provider: Arc<dyn CloudProvider>,
    /// Raw S3 client + bucket name kept exclusively for `file_lock` calls,
    /// which take `&Client` + `&str` directly and are not yet behind the trait.
    pub file_lock_client: Client,
    pub file_lock_bucket: String,
    /// Normalised subdirectory prefix (empty = root, otherwise trailing slash).
    /// Used only to construct absolute S3 keys for `file_lock` sentinel paths.
    pub file_lock_prefix: String,
    pub drive_id: i64,
    pub volume_label: String,

    next_xfer: AtomicU64,
    emit: Box<dyn Fn(TransferPayload) + Send + Sync>,
    /// File-lock events — distinct channel from transfers so the UI can show
    /// "Document.docx is being edited" banners without cluttering the
    /// transfers panel.
    emit_lock: Box<dyn Fn(FileLockEvent) + Send + Sync>,

    list_cache: Arc<Mutex<HashMap<String, (Instant, CachedList)>>>,
    /// Keyed by LOWERCASE input key (S3 is case-sensitive but Windows is
    /// not). The value stores the real-case key alongside the meta so the
    /// fast path can return the canonical S3 key regardless of the case the
    /// caller used — critical because materialize/get_range hit S3 with this
    /// key and a wrong case 404s. `None` is a negative (not-found) cache entry.
    meta_cache: Arc<Mutex<HashMap<String, (Instant, Option<(String, Meta)>)>>>,

    /// Single DACL blob returned for every file/directory. Everyone gets full
    /// access — we don't enforce ACLs on S3.
    security: Vec<u8>,

    /// Lowercased keys of files currently opened for write on THIS machine.
    /// A second writer-open for the same file fails with
    /// `STATUS_SHARING_VIOLATION`. WinFsp's kernel driver already enforces
    /// Windows share-modes inside a single kernel, but we track it ourselves
    /// too so the in-process path is obviously correct.
    local_writers: Mutex<HashSet<String>>,

    /// Stable per-machine identifier, used as the `machine` field of
    /// cross-device sentinel locks in `.nanocrew/locks/`.
    machine_id: String,
    /// Human-readable owner name (username) written into sentinels so a
    /// conflict dialog can say "locked by <user>" rather than a GUID.
    owner: String,

    /// Per-direction byte-rate caps. An unlimited limiter is a no-op on
    /// every `acquire`, so we always call through these regardless of
    /// whether throttling is configured.
    upload_limiter: Arc<RateLimiter>,
    download_limiter: Arc<RateLimiter>,

    /// On-disk block cache (Phase 5.6). Optional — `None` means the cache
    /// was disabled via pref, in which case `get_range` always fetches from
    /// S3 and `invalidate_key` is a no-op.
    cache: Option<Arc<DiskCache>>,

    /// Persistent disk-backed directory-listing cache. Lets large folders
    /// (~hundreds of thousands of files) skip the 30-second S3 LIST
    /// pagination on app restart. `None` when the block cache is disabled —
    /// users who turn caching off expect no disk persistence at all.
    disk_list_cache: Option<Arc<crate::dir_listing_cache::DirListingCache>>,

    /// Network reachability. Flipped to `false` on connection-class S3 errors
    /// and back to `true` on success. Read by `get_drive_connectivity`.
    pub connectivity: Arc<AtomicBool>,
    /// Callback to emit Tauri events (drive_status_changed) when connectivity
    /// flips. Stored as a boxed closure so we don't need the AppHandle in scope
    /// for every S3 call.
    emit_status: Box<dyn Fn(bool) + Send + Sync>,

    /// Per-S3Fs LRU of recently-visited directory prefixes (Wave 1: cache
    /// freshness). Populated by `read_directory` on every call; drained by
    /// the background refresh task.
    visited_dirs: Arc<Mutex<lru::LruCache<String, Instant>>>,
    /// Callback to emit `dir_listing_refreshed` Tauri events when the
    /// background task detects an out-of-band change. Boxed so S3Fs doesn't
    /// need the AppHandle in scope for every call.
    emit_dir_refreshed: Arc<dyn Fn(String) + Send + Sync>,
}

impl S3Fs {
    pub fn new(
        rt: Runtime,
        provider: Arc<dyn CloudProvider>,
        file_lock_client: Client,
        file_lock_bucket: String,
        file_lock_prefix: String,
        drive_id: i64,
        volume_label: String,
        emit: Box<dyn Fn(TransferPayload) + Send + Sync>,
        emit_lock: Box<dyn Fn(FileLockEvent) + Send + Sync>,
        owner: String,
        upload_rate_bps: Option<u64>,
        download_rate_bps: Option<u64>,
        cache: Option<Arc<DiskCache>>,
        emit_status: Box<dyn Fn(bool) + Send + Sync>,
        connectivity: Arc<AtomicBool>,
        disk_list_cache_dir: Option<std::path::PathBuf>,
        emit_dir_refreshed: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<Self, String> {
        let security = build_everyone_sd().map_err(|e| format!("build SD: {e}"))?;
        let machine_id = file_lock::machine_id();
        let disk_list_cache = disk_list_cache_dir
            .map(|p| Arc::new(crate::dir_listing_cache::DirListingCache::new(p)));
        let visited_dirs = Arc::new(Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(VISITED_DIRS_CAPACITY).unwrap(),
        )));

        Ok(Self {
            rt,
            provider,
            file_lock_client,
            file_lock_bucket,
            file_lock_prefix,
            drive_id,
            volume_label,
            next_xfer: AtomicU64::new(2_000_000),
            emit,
            emit_lock,
            list_cache: Arc::new(Mutex::new(HashMap::new())),
            meta_cache: Arc::new(Mutex::new(HashMap::new())),
            security,
            local_writers: Mutex::new(HashSet::new()),
            machine_id,
            owner,
            upload_limiter: Arc::new(RateLimiter::new(upload_rate_bps)),
            download_limiter: Arc::new(RateLimiter::new(download_rate_bps)),
            cache,
            connectivity,
            emit_status,
            disk_list_cache,
            visited_dirs,
            emit_dir_refreshed,
        })
    }

    /// Externally-callable cache invalidation for `refresh_dir_listing`.
    ///
    /// Drops the in-memory list-cache entry for `prefix`, every meta-cache
    /// entry under `prefix/`, and the on-disk listing JSON. Safe to call from
    /// any thread — all backing maps are behind `Arc<Mutex<_>>`.
    ///
    /// Today the mount layer reaches into the same caches via
    /// `RefreshHandle` instead of going through this method (S3Fs is owned
    /// by the WinFsp host, so there's no easy `&S3Fs` to reach from a
    /// Tauri command). Kept around as the canonical in-tree definition of
    /// what "refresh this prefix" means.
    #[allow(dead_code)]
    pub fn refresh_dir(&self, prefix: &str) {
        // In-memory list cache.
        self.list_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(prefix);

        // Meta cache: drop every entry whose key starts with "<prefix>/"
        // (and the prefix itself). Empty prefix = root => clear everything.
        let lc_prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", prefix.to_ascii_lowercase())
        };
        let mut mc = self.meta_cache.lock().unwrap_or_else(|p| p.into_inner());
        if prefix.is_empty() {
            mc.clear();
        } else {
            let lc_self = prefix.to_ascii_lowercase();
            mc.retain(|k, _| !(k.starts_with(&lc_prefix) || k == &lc_self));
        }
        drop(mc);

        // On-disk JSON.
        if let Some(disk) = &self.disk_list_cache {
            disk.invalidate(prefix);
        }
    }

    /// Spawn the background refresh task on `self.rt`. Lifetime tied to the
    /// runtime — when the WinFsp thread tears the runtime down, the task is
    /// cancelled automatically.
    ///
    /// All state the task touches is reached via cloned Arcs, so we never
    /// have to hand it a back-reference to `self` (S3Fs is moved into the
    /// FileSystemHost shortly after `new` returns, so a back-Arc isn't
    /// available anyway).
    pub fn start_background_refresh(&self) {
        let visited_dirs = Arc::clone(&self.visited_dirs);
        let list_cache = Arc::clone(&self.list_cache);
        let meta_cache = Arc::clone(&self.meta_cache);
        let disk_list_cache = self.disk_list_cache.clone();
        let provider = Arc::clone(&self.provider);
        let emit = Arc::clone(&self.emit_dir_refreshed);

        self.rt.spawn(async move {
            loop {
                tokio::time::sleep(BACKGROUND_REFRESH_INTERVAL).await;
                run_background_refresh_cycle(
                    &visited_dirs,
                    &list_cache,
                    &meta_cache,
                    disk_list_cache.as_ref(),
                    provider.as_ref(),
                    emit.as_ref(),
                )
                .await;
            }
        });
    }

    /// Note a visit to `prefix` for the background-refresh LRU.
    fn note_visit(&self, prefix: &str) {
        let mut v = self.visited_dirs.lock().unwrap_or_else(|p| p.into_inner());
        v.put(prefix.to_string(), Instant::now());
    }

    /// Cheap external accessors so the mount layer can build a
    /// `RefreshHandle` without holding an `Arc<S3Fs>`.
    pub fn list_cache_arc(
        &self,
    ) -> Arc<Mutex<HashMap<String, (Instant, CachedList)>>> {
        Arc::clone(&self.list_cache)
    }
    pub fn meta_cache_arc(
        &self,
    ) -> Arc<Mutex<HashMap<String, (Instant, Option<(String, Meta)>)>>> {
        Arc::clone(&self.meta_cache)
    }
    pub fn disk_list_cache_arc(
        &self,
    ) -> Option<Arc<crate::dir_listing_cache::DirListingCache>> {
        self.disk_list_cache.clone()
    }

    // ── Path translation ─────────────────────────────────────────────────────

    /// Convert a WinFsp path (`\foo\bar.txt` or `\`) to an S3 key
    /// (`foo/bar.txt` or empty string for root).
    fn to_key(path: &U16CStr) -> String {
        let s = path.to_string_lossy();
        s.trim_start_matches('\\')
            .replace('\\', "/")
    }

    /// Build the absolute S3 key for `file_lock` sentinel operations.
    fn file_lock_abs_key(&self, rel_key: &str) -> String {
        if self.file_lock_prefix.is_empty() {
            rel_key.to_string()
        } else {
            format!("{}{}", self.file_lock_prefix, rel_key)
        }
    }

    /// Split a key into `(parent_prefix, basename)`. Parent prefix has no
    /// trailing slash; root returns `("", name)`.
    fn split_key(key: &str) -> (&str, &str) {
        match key.rfind('/') {
            Some(i) => (&key[..i], &key[i + 1..]),
            None => ("", key),
        }
    }

    // ── Cache helpers ────────────────────────────────────────────────────────

    fn invalidate_parent(&self, key: &str) {
        let (parent, _) = Self::split_key(key);
        self.list_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(parent);
        // Persistent disk listing: drop the entry so the next list_dir for
        // this prefix re-fetches from S3 instead of replaying stale state
        // after the app restarts.
        if let Some(disk) = &self.disk_list_cache {
            disk.invalidate(parent);
        }
    }

    /// Seed the in-memory meta_cache for every entry in a freshly-acquired
    /// listing. Shared between the S3-fetch path and the disk-hit path so
    /// both populate stat() lookups identically.
    fn seed_meta_from_listing(&self, prefix: &str, listing: &CachedList, now: Instant) {
        let mut mc = self.meta_cache.lock().unwrap_or_else(|p| p.into_inner());
        let parent = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };
        for d in &listing.dirs {
            let full = format!("{parent}{d}");
            mc.insert(
                full.to_ascii_lowercase(),
                (
                    now,
                    Some((full, Meta { is_dir: true, size: 0, mtime_filetime: now_filetime() })),
                ),
            );
        }
        for (name, meta) in &listing.files {
            let full = format!("{parent}{name}");
            mc.insert(full.to_ascii_lowercase(), (now, Some((full, meta.clone()))));
        }
    }

    fn invalidate_meta(&self, key: &str) {
        self.meta_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key.to_ascii_lowercase());
    }

    /// Drop all on-disk cached blocks for `key`. Called after uploads,
    /// deletes, and renames so the next reader sees fresh bytes. No-op when
    /// the disk cache is disabled.
    fn invalidate_cache(&self, key: &str) {
        if let Some(c) = &self.cache {
            c.invalidate_key(key);
        }
    }

    // ── S3 operations ────────────────────────────────────────────────────────

    /// List a single "directory" (delimited by `/`). Result is cached for
    /// LIST_TTL.
    fn list_dir(&self, prefix: &str) -> Result<CachedList, String> {
        {
            let cache = self.list_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((at, cached)) = cache.get(prefix) {
                if at.elapsed() < LIST_TTL {
                    return Ok(cached.clone());
                }
            }
        }

        // Disk-backed listing cache. A fresh hit (<24h) is treated as the
        // moral equivalent of a successful S3 fetch: we populate the
        // in-memory list cache and seed meta_cache, then return without
        // touching the network.
        if let Some(disk) = &self.disk_list_cache {
            if let Some(cached) = disk.load(prefix) {
                let now = Instant::now();
                self.list_cache
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(prefix.to_string(), (now, cached.clone()));
                self.seed_meta_from_listing(prefix, &cached, now);
                return Ok(cached);
            }
        }

        let provider = self.provider.clone();
        let prefix_owned = prefix.to_string();
        let result: Result<CachedList, String> = self.rt.block_on(async move {
            provider
                .list_dir(&prefix_owned)
                .await
                .map(|r| CachedList {
                    dirs: r.dirs,
                    files: r
                        .files
                        .into_iter()
                        .map(|(name, stat)| {
                            (
                                name,
                                Meta {
                                    is_dir: false,
                                    size: stat.size,
                                    mtime_filetime: stat.mtime_filetime,
                                },
                            )
                        })
                        .collect(),
                })
                .map_err(|e| {
                    tracing::error!(target: "nanocrew::vfs", "list_dir {prefix_owned:?}: {e}");
                    e.to_string()
                })
        });

        let listing = result?;
        let now = Instant::now();
        self.list_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(prefix.to_string(), (now, listing.clone()));

        // Persist to disk so the next app launch can skip the S3 LIST
        // pagination for this prefix entirely. Skip empty listings — an
        // empty result is either a genuinely empty folder (cheap to re-list)
        // or a silent failure (e.g. credential migration race on first mount
        // returning zero rows instead of an error). Persisting empty would
        // poison the cache and lock the user into a stale "empty" view.
        if let Some(disk) = &self.disk_list_cache {
            if !listing.dirs.is_empty() || !listing.files.is_empty() {
                disk.save(prefix, &listing);
            }
        }

        // Seed the meta_cache for every entry we just learned about. This
        // turns subsequent stat() calls during folder scrolling into cache
        // hits instead of re-walking the path from the root.
        self.seed_meta_from_listing(prefix, &listing, now);
        Ok(listing)
    }

    /// Resolve the full listing for a directory `key`, consulting (in order)
    /// the in-memory cache, the on-disk listing cache, then the provider
    /// (capped at `MAX_DIR_ENTRIES`). Successful results are written back to
    /// both caches and the meta cache is seeded. This does NOT touch any
    /// WinFsp DirBuffer — the caller fills the buffer while holding the
    /// WinFsp acquire lock (see `read_directory`), which is what serializes
    /// concurrent enumeration of the same handle.
    fn listing_for(&self, key: &str) -> Result<CachedList, String> {
        // In-memory cache.
        {
            let cache = self.list_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((at, cached)) = cache.get(key) {
                if at.elapsed() < LIST_TTL {
                    return Ok(cached.clone());
                }
            }
        }

        // On-disk listing cache.
        if let Some(disk) = &self.disk_list_cache {
            if let Some(cached) = disk.load(key) {
                let now = Instant::now();
                self.list_cache
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(key.to_string(), (now, cached.clone()));
                self.seed_meta_from_listing(key, &cached, now);
                return Ok(cached);
            }
        }

        // Provider enumeration, capped at MAX_DIR_ENTRIES to bound memory and
        // time on pathologically large folders.
        let mut all_dirs: Vec<String> = Vec::new();
        let mut all_files: Vec<(String, Meta)> = Vec::new();
        let mut capped = false;
        let key_for_meta = key.to_string();

        let stream_result: Result<(), String> = self.rt.block_on(async {
            let dirs_ref = &mut all_dirs;
            let files_ref = &mut all_files;
            let capped_ref = &mut capped;
            let mut on_page = |page: crate::providers::ListDirResult| -> bool {
                let mut new_files: Vec<(String, Meta)> = page
                    .files
                    .into_iter()
                    .map(|(n, s)| {
                        (
                            n,
                            Meta {
                                is_dir: false,
                                size: s.size,
                                mtime_filetime: s.mtime_filetime,
                            },
                        )
                    })
                    .collect();

                {
                    let now = Instant::now();
                    let parent = if key_for_meta.is_empty() {
                        String::new()
                    } else {
                        format!("{key_for_meta}/")
                    };
                    let mut mc =
                        self.meta_cache.lock().unwrap_or_else(|p| p.into_inner());
                    for d in &page.dirs {
                        let full = format!("{parent}{d}");
                        mc.insert(
                            full.to_ascii_lowercase(),
                            (
                                now,
                                Some((
                                    full,
                                    Meta {
                                        is_dir: true,
                                        size: 0,
                                        mtime_filetime: now_filetime(),
                                    },
                                )),
                            ),
                        );
                    }
                    for (name, meta) in &new_files {
                        let full = format!("{parent}{name}");
                        mc.insert(full.to_ascii_lowercase(), (now, Some((full, meta.clone()))));
                    }
                }

                dirs_ref.extend(page.dirs);
                files_ref.append(&mut new_files);

                if dirs_ref.len() + files_ref.len() >= MAX_DIR_ENTRIES {
                    *capped_ref = true;
                    return false;
                }
                true
            };

            self.provider
                .list_dir_stream(key, &mut on_page)
                .await
                .map_err(|e| e.to_string())
        });

        stream_result?;

        if capped {
            all_files.push((
                TRUNCATION_SENTINEL.to_string(),
                Meta { is_dir: false, size: 0, mtime_filetime: now_filetime() },
            ));
        }
        let listing = CachedList { dirs: all_dirs, files: all_files };

        let now = Instant::now();
        self.list_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key.to_string(), (now, listing.clone()));
        // Skip disk cache when capped (truncated listing) OR when empty. An
        // empty result is either a genuinely empty folder (cheap to re-list)
        // or a silent failure that poisoned an empty result — persisting
        // would lock the user into a stale "empty" view.
        if !capped && (!listing.dirs.is_empty() || !listing.files.is_empty()) {
            if let Some(disk) = self.disk_list_cache.as_ref() {
                disk.save(key, &listing);
            }
        }
        tracing::info!(
            target: "nanocrew::vfs",
            "enum key={:?} dirs={} files={} capped={}",
            key,
            listing.dirs.len(),
            listing.files.len(),
            capped,
        );
        Ok(listing)
    }

    /// Write a complete listing into an already-acquired WinFsp DirBuffer
    /// lock. Adds "."/".." for non-root directories. The caller must hold the
    /// lock returned by `dir_buffer.acquire(...)` — WinFsp serializes the
    /// acquire so concurrent `read_directory` calls block until release.
    /// Build the full directory listing as a single vector sorted by WinFsp's
    /// case-insensitive (uppercase-fold) order, with "."/".." prepended for
    /// non-root directories and any case-folding collisions removed. This is
    /// the canonical order used by `read_directory`'s own marker pagination.
    fn sorted_dir_entries(&self, key: &str) -> Result<Vec<(String, Meta)>, String> {
        let listing = self.listing_for(key)?;
        let dir_meta = Meta { is_dir: true, size: 0, mtime_filetime: now_filetime() };
        let mut entries: Vec<(String, Meta)> =
            Vec::with_capacity(listing.dirs.len() + listing.files.len() + 2);

        if !key.is_empty() {
            entries.push((".".to_string(), dir_meta.clone()));
            entries.push(("..".to_string(), dir_meta.clone()));
        }
        for d in &listing.dirs {
            entries.push((d.clone(), dir_meta.clone()));
        }
        for (name, meta) in &listing.files {
            entries.push((name.clone(), meta.clone()));
        }

        entries.sort_by(|a, b| {
            let ka = a.0.to_uppercase();
            let kb = b.0.to_uppercase();
            ka.cmp(&kb).then_with(|| a.0.cmp(&b.0))
        });
        entries.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
        Ok(entries)
    }

    /// Resolve a path (which may have arbitrary case from Windows) to its
    /// real-case S3 key + metadata. Walks the path segment by segment, doing
    /// case-insensitive match against each directory's listing.
    ///
    /// Returns `Ok(None)` if any segment doesn't exist.
    fn resolve(&self, key: &str) -> Result<Option<(String, Meta)>, String> {
        if key.is_empty() {
            return Ok(Some((
                String::new(),
                Meta {
                    is_dir: true,
                    size: 0,
                    mtime_filetime: now_filetime(),
                },
            )));
        }
        // Meta cache — keyed by the LOWERCASE input key (case-insensitive).
        // The value carries the real-case S3 key, so we always return the
        // canonical case no matter how the caller cased the request. This is
        // essential: the returned key is used for materialize/get_range, and
        // S3 is case-sensitive — returning the request's case 404s.
        {
            let cache = self.meta_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((at, v)) = cache.get(&key.to_ascii_lowercase()) {
                if at.elapsed() < META_TTL {
                    return Ok(v.clone());
                }
            }
        }

        let segments: Vec<&str> = key.split('/').collect();
        let mut parent_real = String::new();
        let mut last_meta: Option<Meta> = None;
        for (i, seg) in segments.iter().enumerate() {
            let is_last = i == segments.len() - 1;
            let listing = self.list_dir(&parent_real)?;
            let seg_lower = seg.to_ascii_lowercase();

            // Directory first — dirs and files can't collide in the same
            // prefix in practice, but prefer dirs to keep directory traversal
            // working even if the final segment is a file named the same.
            let dir_hit = listing
                .dirs
                .iter()
                .find(|d| d.eq_ignore_ascii_case(&seg_lower) || d.to_ascii_lowercase() == seg_lower)
                .cloned();
            if let Some(real) = dir_hit {
                parent_real = if parent_real.is_empty() {
                    real
                } else {
                    format!("{}/{}", parent_real, real)
                };
                last_meta = Some(Meta {
                    is_dir: true,
                    size: 0,
                    mtime_filetime: 0,
                });
                continue;
            }

            let file_hit = listing
                .files
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(seg))
                .cloned();
            if let Some((real, m)) = file_hit {
                if !is_last {
                    // File appears mid-path — the path is invalid.
                    self.meta_cache
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(key.to_ascii_lowercase(), (Instant::now(), None));
                    return Ok(None);
                }
                parent_real = if parent_real.is_empty() {
                    real
                } else {
                    format!("{}/{}", parent_real, real)
                };
                last_meta = Some(m);
                continue;
            }

            // Not found at this level.
            self.meta_cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(key.to_ascii_lowercase(), (Instant::now(), None));
            return Ok(None);
        }

        let found = last_meta.map(|m| (parent_real, m));
        // Cache the (real-case key, meta) under the lowercase input key.
        self.meta_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key.to_ascii_lowercase(), (Instant::now(), found.clone()));
        Ok(found)
    }

    /// Convenience wrapper when the caller only needs existence/metadata and
    /// not the real-case key.
    fn lookup(&self, key: &str) -> Result<Option<Meta>, String> {
        Ok(self.resolve(key)?.map(|(_, m)| m))
    }

    /// Fetch a byte range from S3. Direct — no cache, no block alignment.
    /// Classify an S3 SDK error string as a network-level failure (as opposed
    /// to an auth / permission / not-found error that the server responded to).
    fn is_network_error(msg: &str) -> bool {
        let m = msg.to_lowercase();
        m.contains("connection refused")
            || m.contains("connection reset")
            || m.contains("timed out")
            || m.contains("timeout")
            || m.contains("dns")
            || m.contains("no such host")
            || m.contains("failed to connect")
            || m.contains("network unreachable")
            || m.contains("dispatch failure")
    }

    /// Record the outcome of an S3 operation and flip connectivity if needed.
    fn record_connectivity(&self, ok: bool) {
        let was = self.connectivity.swap(ok, Ordering::Relaxed);
        if was != ok {
            (self.emit_status)(ok);
        }
    }

    fn fetch_s3_range(&self, key: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let provider = self.provider.clone();
        let limiter = self.download_limiter.clone();
        let key_owned = key.to_string();
        let result = self.rt.block_on(async move {
            limiter.acquire(len).await;
            provider
                .get_range(&key_owned, offset, len)
                .await
                .map(|b| b.to_vec())
                .map_err(|e| e.to_string())
        });
        match &result {
            Ok(_) => self.record_connectivity(true),
            Err(e) if Self::is_network_error(e) => self.record_connectivity(false),
            Err(_) => {}
        }
        result
    }

    /// Return `len` bytes starting at `offset`. When the disk cache is
    /// enabled, this decomposes the request into `CACHE_BLOCK`-aligned
    /// windows: each block is served from disk on a hit, or fetched from S3
    /// (and written through to the cache) on a miss.
    fn get_range(&self, key: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let cache = match &self.cache {
            Some(c) if c.is_enabled() => c,
            _ => return self.fetch_s3_range(key, offset, len),
        };

        let end = offset + len;
        let mut out: Vec<u8> = Vec::with_capacity(len as usize);
        let mut pos = offset;
        while pos < end {
            let block_start = pos - (pos % CACHE_BLOCK);
            let block_limit = block_start + CACHE_BLOCK;
            let chunk_end = end.min(block_limit);

            let block_bytes = if let Some(b) = cache.get_block(key, block_start) {
                b
            } else {
                // Miss: fetch a full block (or whatever remains before EOF —
                // S3 will clamp automatically if the range runs past end-of-
                // object, returning fewer bytes than requested).
                match self.fetch_s3_range(key, block_start, CACHE_BLOCK) {
                    Ok(b) => {
                        cache.put_block(key, block_start, &b);
                        b
                    }
                    Err(e) => return Err(e),
                }
            };

            // Clamp into the user-requested subrange. A short block (EOF)
            // may mean the caller's requested window runs past data; copy
            // only what's actually there.
            let lo = (pos - block_start) as usize;
            let hi = ((chunk_end - block_start) as usize).min(block_bytes.len());
            if hi > lo {
                out.extend_from_slice(&block_bytes[lo..hi]);
            }
            if block_bytes.len() < (block_limit - block_start) as usize {
                // Short block = EOF hit; nothing useful beyond here.
                break;
            }
            pos = block_limit;
        }
        Ok(out)
    }

    /// Materialize an entire file into the block cache by fetching all missing
    /// blocks in parallel. Called synchronously from `open()` for Office docs,
    /// PDFs, and other small files — apps that issue many scattered small
    /// reads (Excel, Word, PowerPoint, PDF viewers) see the file as instantly
    /// available once `open()` returns instead of stuttering through dozens
    /// of 1 MiB block-miss round-trips.
    fn materialize_file(&self, key: &str, size: u64) -> Result<(), String> {
        let Some(cache) = self.cache.as_ref().filter(|c| c.is_enabled()) else {
            return Ok(());
        };
        if size == 0 {
            return Ok(());
        }

        let mut missing: Vec<u64> = Vec::new();
        let mut block_start = 0u64;
        while block_start < size {
            if cache.get_block(key, block_start).is_none() {
                missing.push(block_start);
            }
            block_start += CACHE_BLOCK;
        }
        if missing.is_empty() {
            return Ok(());
        }

        let key_owned = key.to_string();
        let provider = self.provider.clone();
        let cache_cloned = cache.clone();
        let limiter = self.download_limiter.clone();

        self.rt.block_on(async move {
            use futures_util::stream::{FuturesUnordered, StreamExt};
            use std::future::Future;
            use std::pin::Pin;
            type BlockFut = Pin<Box<dyn Future<Output = (u64, Result<bytes::Bytes, crate::providers::ProviderError>)> + Send>>;
            let mut tasks: FuturesUnordered<BlockFut> = FuturesUnordered::new();
            let mut next = 0;

            let spawn_one = |start: u64| -> BlockFut {
                let p = provider.clone();
                let k = key_owned.clone();
                let lim = limiter.clone();
                Box::pin(async move {
                    lim.acquire(CACHE_BLOCK).await;
                    let res = p.get_range(&k, start, CACHE_BLOCK).await;
                    (start, res)
                })
            };

            while next < missing.len() && tasks.len() < MATERIALIZE_CONCURRENCY {
                tasks.push(spawn_one(missing[next]));
                next += 1;
            }

            while let Some((start, res)) = tasks.next().await {
                match res {
                    Ok(bytes) => cache_cloned.put_block(&key_owned, start, bytes.as_ref()),
                    Err(e) => {
                        tracing::warn!(target: "nanocrew::vfs",
                            "materialize block {start}: {e}");
                    }
                }
                if next < missing.len() {
                    tasks.push(spawn_one(missing[next]));
                    next += 1;
                }
            }
        });
        Ok(())
    }

    /// Fire-and-forget background prefetch of the next N blocks starting from
    /// `from_offset`. Called from `read()` once a sequential access pattern is
    /// detected. Silently no-ops when the cache is disabled.
    fn prefetch_ahead(&self, key: &str, from_offset: u64, file_size: u64, block_count: usize) {
        let Some(cache) = self.cache.as_ref().filter(|c| c.is_enabled()) else { return; };
        let key_owned = key.to_string();
        let provider = self.provider.clone();
        let cache_cloned = cache.clone();
        let limiter = self.download_limiter.clone();
        let aligned = from_offset - (from_offset % CACHE_BLOCK);

        self.rt.spawn(async move {
            use futures_util::stream::{FuturesUnordered, StreamExt};
            let mut tasks = FuturesUnordered::new();
            for i in 0..block_count {
                let start = aligned + (i as u64) * CACHE_BLOCK;
                if start >= file_size { break; }
                if cache_cloned.get_block(&key_owned, start).is_some() { continue; }
                let p = provider.clone();
                let k = key_owned.clone();
                let lim = limiter.clone();
                tasks.push(async move {
                    lim.acquire(CACHE_BLOCK).await;
                    let res = p.get_range(&k, start, CACHE_BLOCK).await;
                    (start, res)
                });
            }
            while let Some((start, res)) = tasks.next().await {
                match res {
                    Ok(bytes) => cache_cloned.put_block(&key_owned, start, bytes.as_ref()),
                    Err(e) => {
                        tracing::warn!(target: "nanocrew::vfs",
                            "prefetch block {start}: {e}");
                    }
                }
            }
        });
    }

    // ── Upload path ──────────────────────────────────────────────────────────

    /// Create a fresh `WriteState` backed by a new temp file. The file lives
    /// in `%TEMP%\nanocrew-sync-uploads\` and is removed on cleanup.
    fn new_write_state(&self, key: &str, is_new: bool) -> Result<WriteState, String> {
        let dir = std::env::temp_dir().join("nanocrew-sync-uploads");
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir temp: {e}"))?;
        let id = self.next_xfer.fetch_add(1, Ordering::Relaxed);
        let safe_name: String = key
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let temp_path = dir.join(format!("{}-{}-{}.tmp", self.drive_id, id, safe_name));
        let temp_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&temp_path)
            .map_err(|e| format!("open temp {}: {e}", temp_path.display()))?;
        Ok(WriteState {
            temp_path,
            temp_file,
            bytes_written: 0,
            upload_id: None,
            dispatched_bytes: 0,
            next_part_number: 1,
            completed_parts: Arc::new(std::sync::Mutex::new(Vec::new())),
            upload_err: Arc::new(std::sync::Mutex::new(None)),
            inflight_sem: Arc::new(tokio::sync::Semaphore::new(UPLOAD_CONCURRENCY)),
            bytes_uploaded: Arc::new(AtomicU64::new(0)),
            extents: BTreeMap::new(),
            contig_bytes: 0,
            xfer_id: None,
            last_emit: Instant::now(),
            filename: Self::split_key(key).1.to_string(),
            total_size_hint: 0,
            is_new,
        })
    }

    /// Ensure a multipart upload is in-flight for this write. Called the
    /// first time we're about to dispatch a part.
    fn ensure_multipart(&self, key: &str, state: &mut WriteState) -> Result<(), String> {
        if state.upload_id.is_some() {
            return Ok(());
        }
        let provider = self.provider.clone();
        let key_owned = key.to_string();
        let upload_id = self.rt.block_on(async move {
            provider
                .create_multipart(&key_owned)
                .await
                .map_err(|e| e.to_string())
        })?;
        state.upload_id = Some(upload_id);
        Ok(())
    }

    /// Spawn background tasks to upload every full part that's ready. Blocks
    /// if the upload pipeline is saturated (natural backpressure).
    ///
    /// `is_final` = we're in cleanup and the tail part (may be < PART_TARGET)
    /// should also be dispatched.
    fn dispatch_ready_parts(
        &self,
        key: &str,
        state: &mut WriteState,
        is_final: bool,
    ) -> Result<(), String> {
        // Surface any worker failure from earlier so we stop dispatching.
        if let Some(e) = state.upload_err.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            return Err(e);
        }

        loop {
            // Bytes available for the next part = min(contig_bytes, bytes_written).
            // In the final flush we accept any tail.
            let available = state.contig_bytes;
            let ready_size = if is_final {
                // Final tail: everything not yet dispatched.
                let end = state.bytes_written.max(available);
                end.saturating_sub(state.dispatched_bytes)
            } else if available >= state.dispatched_bytes + PART_TARGET as u64 {
                PART_TARGET as u64
            } else {
                0
            };
            if ready_size == 0 {
                return Ok(());
            }

            self.ensure_multipart(key, state)?;

            // Block here if 8 parts are already in flight. This is what paces
            // Explorer's progress bar against real S3 speed.
            let permit = self
                .rt
                .block_on(state.inflight_sem.clone().acquire_owned())
                .map_err(|e| format!("acquire semaphore: {e}"))?;

            let pn = state.next_part_number;
            let off = state.dispatched_bytes;
            let sz = ready_size as usize;
            let upload_id = state.upload_id.clone().unwrap();
            let provider = self.provider.clone();
            let key_owned = key.to_string();
            let temp_path = state.temp_path.clone();
            let completed_parts = state.completed_parts.clone();
            let upload_err = state.upload_err.clone();
            let bytes_uploaded = state.bytes_uploaded.clone();
            let limiter = self.upload_limiter.clone();

            self.rt.spawn(async move {
                let result: Result<(), String> = async {
                    // Positioned read on a blocking thread.
                    let buf = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
                        let f = File::open(&temp_path)?;
                        let mut buf = vec![0u8; sz];
                        let mut filled = 0usize;
                        while filled < sz {
                            let n = f.seek_read(&mut buf[filled..], off + filled as u64)?;
                            if n == 0 {
                                break;
                            }
                            filled += n;
                        }
                        buf.truncate(filled);
                        Ok(buf)
                    })
                    .await
                    .map_err(|e| format!("spawn_blocking: {e}"))?
                    .map_err(|e| format!("read temp @{off}+{sz}: {e}"))?;

                    let actual = buf.len();
                    // Bandwidth throttle — pace each part's send against the
                    // configured upload cap. No-op on an unlimited limiter.
                    limiter.acquire(actual as u64).await;
                    let etag = provider
                        .upload_part(&key_owned, &upload_id, pn, Bytes::from(buf))
                        .await
                        .map_err(|e| format!("upload_part {pn}: {e}"))?;
                    bytes_uploaded.fetch_add(actual as u64, Ordering::Relaxed);
                    completed_parts
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(crate::providers::CompletedPart {
                            part_number: pn,
                            etag,
                        });
                    Ok(())
                }
                .await;
                if let Err(e) = result {
                    let mut err = upload_err.lock().unwrap_or_else(|p| p.into_inner());
                    if err.is_none() {
                        *err = Some(e);
                    }
                }
                drop(permit); // releases slot for next write()
            });

            state.next_part_number += 1;
            state.dispatched_bytes += ready_size;
        }
    }

    /// Finalize a write. Called from `cleanup` when the file handle closes.
    /// If the upload was streamed during the write phase, we just flush the
    /// tail part and wait for all in-flight uploads. Otherwise (small file),
    /// do a single PUT.
    fn finalize_write(&self, key: &str, mut state: WriteState) -> Result<u64, String> {
        let size = state.bytes_written;
        state.temp_file.sync_all().ok();
        state.temp_file.set_len(size).ok();

        let xfer_id = state
            .xfer_id
            .unwrap_or_else(|| self.next_xfer.fetch_add(1, Ordering::Relaxed));
        let filename = state.filename.clone();

        // Small-file fast path: single PUT. No MPU was started because we
        // never crossed the PART_TARGET boundary.
        let result = if state.upload_id.is_none() {
            // Emit a start event so the UI shows *something* even for small
            // files above the transfer threshold.
            if size >= MIN_TRANSFER_BYTES {
                (self.emit)(TransferPayload {
                    id: xfer_id,
                    drive_id: self.drive_id,
                    filename: filename.clone(),
                    direction: "upload".into(),
                    total_bytes: size,
                    done_bytes: 0,
                    state: "start".into(),
                    error: None,
                });
            }
            drop(state.temp_file);
            let r = self.upload_single_put(key, &state.temp_path);
            let _ = std::fs::remove_file(&state.temp_path);
            r
        } else {
            // Streaming path: flush the tail, then wait for all in-flight
            // parts, then CompleteMultipartUpload.
            if let Err(e) = self.dispatch_ready_parts(key, &mut state, true) {
                let _ = std::fs::remove_file(&state.temp_path);
                if let Some(uid) = state.upload_id.as_deref() {
                    self.abort_multipart(key, uid);
                }
                return Err(e);
            }
            // Wait for the upload pipeline to drain by acquiring all permits
            // via the owned variant (which takes Arc<Semaphore>, no lifetime).
            let sem = state.inflight_sem.clone();
            let _ = self.rt.block_on(async move {
                sem.acquire_many_owned(UPLOAD_CONCURRENCY as u32).await
            });
            drop(state.temp_file);
            let _ = std::fs::remove_file(&state.temp_path);

            // Surface any worker error.
            if let Some(e) = state
                .upload_err
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
            {
                if let Some(uid) = state.upload_id.as_deref() {
                    self.abort_multipart(key, uid);
                }
                Err(e)
            } else {
                // Complete MPU. Parts must be sorted by part_number.
                let mut parts = state
                    .completed_parts
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                parts.sort_by_key(|p| p.part_number);
                let upload_id = state.upload_id.clone().unwrap();
                let provider = self.provider.clone();
                let key_owned = key.to_string();
                self.rt
                    .block_on(async move {
                        provider
                            .complete_multipart(&key_owned, &upload_id, parts)
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .map_err(|e| {
                        if let Some(uid) = state.upload_id.as_deref() {
                            self.abort_multipart(key, uid);
                        }
                        e
                    })
            }
        };

        match &result {
            Ok(_) => {
                if size >= MIN_TRANSFER_BYTES {
                    (self.emit)(TransferPayload {
                        id: xfer_id,
                        drive_id: self.drive_id,
                        filename,
                        direction: "upload".into(),
                        total_bytes: size,
                        done_bytes: size,
                        state: "done".into(),
                        error: None,
                    });
                }
            }
            Err(e) => {
                if size >= MIN_TRANSFER_BYTES {
                    (self.emit)(TransferPayload {
                        id: xfer_id,
                        drive_id: self.drive_id,
                        filename,
                        direction: "upload".into(),
                        total_bytes: size,
                        done_bytes: 0,
                        state: "error".into(),
                        error: Some(e.clone()),
                    });
                }
            }
        }
        result.map(|_| size)
    }

    fn upload_single_put(&self, key: &str, temp_path: &PathBuf) -> Result<(), String> {
        let bytes = std::fs::read(temp_path).map_err(|e| format!("read temp: {e}"))?;
        let provider = self.provider.clone();
        let key_owned = key.to_string();
        let limiter = self.upload_limiter.clone();
        let size = bytes.len() as u64;
        self.rt.block_on(async move {
            limiter.acquire(size).await;
            provider
                .put_object(&key_owned, Bytes::from(bytes))
                .await
                .map_err(|e| e.to_string())
        })
    }


    /// Abort a multipart upload on error (best-effort).
    fn abort_multipart(&self, key: &str, upload_id: &str) {
        let provider = self.provider.clone();
        let key_owned = key.to_string();
        let uid = upload_id.to_string();
        let _ = self
            .rt
            .block_on(async move { provider.abort_multipart(&key_owned, &uid).await });
    }

    /// Emit `transfer_progress` for a read. Lazily allocates `DownloadState`
    /// on the first qualifying byte (file size ≥ `MIN_TRANSFER_BYTES`),
    /// emitting `start` immediately with `done_bytes = n`, then throttles
    /// `progress` events on `PROGRESS_INTERVAL`. The terminal `done`/`error`
    /// event is emitted from `close()` — not from here — because reads are
    /// stateless and we only know completion at handle close.
    fn emit_download_progress(
        &self,
        download: &Mutex<Option<DownloadState>>,
        key: &str,
        size: u64,
        n: u64,
    ) {
        if size < MIN_TRANSFER_BYTES {
            return;
        }
        let mut guard = download.lock().unwrap_or_else(|p| p.into_inner());
        let (xfer_id, done, should_emit, is_start) = match guard.as_mut() {
            Some(d) => {
                let done = d.bytes_done.fetch_add(n, Ordering::Relaxed) + n;
                let should = d.last_emit.elapsed() >= PROGRESS_INTERVAL;
                if should {
                    d.last_emit = Instant::now();
                }
                (d.xfer_id, done, should, false)
            }
            None => {
                let id = self.next_xfer.fetch_add(1, Ordering::Relaxed);
                let filename = key
                    .rsplit('/')
                    .next()
                    .unwrap_or(key)
                    .to_string();
                *guard = Some(DownloadState {
                    xfer_id: id,
                    bytes_done: AtomicU64::new(n),
                    last_emit: Instant::now(),
                    size,
                    filename,
                });
                (id, n, true, true)
            }
        };
        if !should_emit && !is_start {
            return;
        }
        let filename = guard.as_ref().map(|d| d.filename.clone()).unwrap_or_default();
        drop(guard);
        (self.emit)(TransferPayload {
            id: xfer_id,
            drive_id: self.drive_id,
            filename,
            direction: "download".into(),
            total_bytes: size,
            done_bytes: done,
            state: if is_start { "start" } else { "progress" }.into(),
            error: None,
        });
    }

    fn emit_progress(&self, state: &mut WriteState, done_bytes: u64, total: u64, finished: bool) {
        if total < MIN_TRANSFER_BYTES && !finished {
            return;
        }
        if state.xfer_id.is_none() && total >= MIN_TRANSFER_BYTES {
            let id = self.next_xfer.fetch_add(1, Ordering::Relaxed);
            state.xfer_id = Some(id);
            (self.emit)(TransferPayload {
                id,
                drive_id: self.drive_id,
                filename: state.filename.clone(),
                direction: "upload".into(),
                total_bytes: total,
                done_bytes: 0,
                state: "start".into(),
                error: None,
            });
        }
        if let Some(id) = state.xfer_id {
            if finished || state.last_emit.elapsed() >= PROGRESS_INTERVAL {
                (self.emit)(TransferPayload {
                    id,
                    drive_id: self.drive_id,
                    filename: state.filename.clone(),
                    direction: "upload".into(),
                    total_bytes: total,
                    done_bytes,
                    state: if finished { "done" } else { "progress" }.into(),
                    error: None,
                });
                state.last_emit = Instant::now();
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn unix_secs_to_filetime(secs: i64) -> u64 {
    // 11644473600 = seconds between 1601-01-01 and 1970-01-01
    let s = secs.max(0) as u64 + 11_644_473_600;
    s * 10_000_000
}

/// Body of the background refresh loop, broken out as a free function so it
/// can be spawned with just the Arcs it needs (no back-reference to S3Fs,
/// which has been moved into the WinFsp host by the time the task runs).
async fn run_background_refresh_cycle(
    visited_dirs: &Arc<Mutex<lru::LruCache<String, Instant>>>,
    list_cache: &Arc<Mutex<HashMap<String, (Instant, CachedList)>>>,
    meta_cache: &Arc<Mutex<HashMap<String, (Instant, Option<(String, Meta)>)>>>,
    disk_list_cache: Option<&Arc<crate::dir_listing_cache::DirListingCache>>,
    provider: &dyn crate::providers::CloudProvider,
    emit: &(dyn Fn(String) + Send + Sync),
) {
    // Snapshot the recently-visited prefixes, dropping stale entries.
    let candidates: Vec<String> = {
        let mut v = visited_dirs.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        let stale: Vec<String> = v
            .iter()
            .filter_map(|(k, t)| {
                if now.duration_since(*t) > RECENT_VISIT_WINDOW {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect();
        for k in &stale {
            v.pop(k);
        }
        v.iter().map(|(k, _)| k.clone()).collect()
    };

    for prefix in candidates {
        let fresh = match fetch_listing_uncached(provider, &prefix).await {
            Ok(l) => l,
            Err(e) => {
                tracing::debug!(
                    target: "nanocrew::vfs::bgrefresh",
                    "fetch {prefix:?} failed: {e}",
                );
                continue;
            }
        };

        let cached = list_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&prefix)
            .map(|(_, l)| l.clone());

        let changed = match cached {
            Some(c) => !listings_equal(&c, &fresh),
            None => true,
        };
        if !changed {
            continue;
        }

        let now = Instant::now();
        list_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(prefix.clone(), (now, fresh.clone()));
        // Skip disk persistence for empty listings — see rationale in
        // listing_for above (avoid poisoning the cache on silent failures).
        if let Some(disk) = disk_list_cache {
            if !fresh.dirs.is_empty() || !fresh.files.is_empty() {
                disk.save(&prefix, &fresh);
            }
        }
        seed_meta_from_listing_into(meta_cache, &prefix, &fresh, now);

        emit(prefix);
    }
}

async fn fetch_listing_uncached(
    provider: &dyn crate::providers::CloudProvider,
    prefix: &str,
) -> Result<CachedList, String> {
    let mut all_dirs: Vec<String> = Vec::new();
    let mut all_files: Vec<(String, Meta)> = Vec::new();
    {
        let dirs_ref = &mut all_dirs;
        let files_ref = &mut all_files;
        let mut on_page = |page: crate::providers::ListDirResult| -> bool {
            dirs_ref.extend(page.dirs);
            files_ref.extend(page.files.into_iter().map(|(n, s)| {
                (
                    n,
                    Meta {
                        is_dir: false,
                        size: s.size,
                        mtime_filetime: s.mtime_filetime,
                    },
                )
            }));
            dirs_ref.len() + files_ref.len() < MAX_DIR_ENTRIES
        };
        provider
            .list_dir_stream(prefix, &mut on_page)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(CachedList { dirs: all_dirs, files: all_files })
}

/// Free-function variant of `S3Fs::seed_meta_from_listing` so the background
/// task can populate the meta cache without an `&S3Fs` reference.
fn seed_meta_from_listing_into(
    meta_cache: &Arc<Mutex<HashMap<String, (Instant, Option<(String, Meta)>)>>>,
    prefix: &str,
    listing: &CachedList,
    now: Instant,
) {
    let mut mc = meta_cache.lock().unwrap_or_else(|p| p.into_inner());
    let parent = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}/")
    };
    for d in &listing.dirs {
        let full = format!("{parent}{d}");
        mc.insert(
            full.to_ascii_lowercase(),
            (
                now,
                Some((full, Meta { is_dir: true, size: 0, mtime_filetime: now_filetime() })),
            ),
        );
    }
    for (name, meta) in &listing.files {
        let full = format!("{parent}{name}");
        mc.insert(full.to_ascii_lowercase(), (now, Some((full, meta.clone()))));
    }
}

/// Structural equality for two CachedList values. Used by the background
/// refresh task to decide whether to emit a `dir_listing_refreshed` event.
fn listings_equal(a: &CachedList, b: &CachedList) -> bool {
    if a.dirs.len() != b.dirs.len() || a.files.len() != b.files.len() {
        return false;
    }
    // Compare as sets — ordering can differ across pages.
    let mut a_dirs = a.dirs.clone();
    let mut b_dirs = b.dirs.clone();
    a_dirs.sort();
    b_dirs.sort();
    if a_dirs != b_dirs {
        return false;
    }
    let mut a_files: Vec<(String, u64, u64)> = a
        .files
        .iter()
        .map(|(n, m)| (n.clone(), m.size, m.mtime_filetime))
        .collect();
    let mut b_files: Vec<(String, u64, u64)> = b
        .files
        .iter()
        .map(|(n, m)| (n.clone(), m.size, m.mtime_filetime))
        .collect();
    a_files.sort();
    b_files.sort();
    a_files == b_files
}

fn now_filetime() -> u64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    unix_secs_to_filetime(d.as_secs() as i64)
}

/// Build a security descriptor granting everyone full access. Returned as a
/// self-relative byte blob we can copy into WinFsp's buffers repeatedly.
fn build_everyone_sd() -> Result<Vec<u8>, String> {
    // Windows rejects DACL-only descriptors with "security descriptor
    // structure is invalid" — it needs owner + group SIDs. We use Built-in
    // Administrators (BA) for both, with an Everyone (WD) Full Access ACE plus
    // explicit Full Access for BA and Local System (SY).
    let sddl = "O:BAG:BAD:P(A;;FA;;;WD)(A;;FA;;;BA)(A;;FA;;;SY)";
    let w: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut sd_ptr = windows::Win32::Security::PSECURITY_DESCRIPTOR::default();
        let mut size: u32 = 0;
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(w.as_ptr()),
            1, // SDDL_REVISION_1
            &mut sd_ptr,
            Some(&mut size),
        )
        .map_err(|e| format!("ConvertStringSecurityDescriptorToSecurityDescriptorW: {e}"))?;
        let slice = std::slice::from_raw_parts(sd_ptr.0 as *const u8, size as usize);
        let bytes = slice.to_vec();
        let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(sd_ptr.0));
        Ok(bytes)
    }
}

fn copy_sd_into(sd: &[u8], out: Option<&mut [c_void]>) -> u64 {
    if let Some(buf) = out {
        if buf.len() >= sd.len() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    sd.as_ptr(),
                    buf.as_mut_ptr() as *mut u8,
                    sd.len(),
                );
            }
        }
    }
    sd.len() as u64
}

fn fill_file_info(info: &mut FileInfo, meta: &Meta) {
    info.file_attributes = if meta.is_dir {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    info.reparse_tag = 0;
    info.allocation_size = (meta.size + 4095) & !4095;
    info.file_size = meta.size;
    info.creation_time = meta.mtime_filetime;
    info.last_access_time = meta.mtime_filetime;
    info.last_write_time = meta.mtime_filetime;
    info.change_time = meta.mtime_filetime;
    info.index_number = 0;
    info.hard_links = 0;
    info.ea_size = 0;
}

// ── Share / file-lock helpers ────────────────────────────────────────────────

/// Access-rights bits that imply the caller intends to modify the file. Taken
/// from wdm.h — we can't pull Windows constants through winfsp-sys cleanly, so
/// they're inlined with reference to the source.
const FILE_WRITE_DATA: u32 = 0x0002;
const FILE_APPEND_DATA: u32 = 0x0004;
const FILE_WRITE_EA: u32 = 0x0010;
const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
const DELETE_ACCESS: u32 = 0x0001_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const WRITE_MASK: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | DELETE_ACCESS
    | GENERIC_WRITE
    | GENERIC_ALL;

#[inline]
fn is_write_access(granted: FILE_ACCESS_RIGHTS) -> bool {
    (granted as u32) & WRITE_MASK != 0
}

/// Keys under `.nanocrew/` are our own bookkeeping — sentinels must not
/// recurse through `is_write_access`, lockfile detection, or cross-device
/// sentinel checks, or the VFS deadlocks on itself the first time a user
/// opens a file for write.
#[inline]
fn is_internal_key(key: &str) -> bool {
    key == ".nanocrew"
        || key.starts_with(".nanocrew/")
}

/// Decide whether to materialize an entire file into the block cache at
/// `open()` time. Office documents and PDFs always qualify (up to 50 MB)
/// because they issue many small scattered reads; other small files
/// (<= 4 MiB) qualify because the round-trip count savings outweigh the
/// brief `open()` blocking cost.
fn should_materialize(key: &str, size: u64) -> bool {
    const SMALL_FILE_BYTES: u64 = 50 * 1024 * 1024;
    if size > SMALL_FILE_BYTES { return false; }
    let lower = key.to_ascii_lowercase();
    if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") || lower.ends_with(".xlsb")
        || lower.ends_with(".docx") || lower.ends_with(".docm")
        || lower.ends_with(".pptx") || lower.ends_with(".pptm")
        || lower.ends_with(".pdf")
        || lower.ends_with(".xls") || lower.ends_with(".doc") || lower.ends_with(".ppt") {
        return true;
    }
    size <= 4 * 1024 * 1024
}

impl S3Fs {
    /// Emit a file-lock event. Non-fatal if the frontend isn't listening.
    fn emit_lock(&self, ev: FileLockEvent) {
        (self.emit_lock)(ev);
    }

    /// OR `FILE_ATTRIBUTE_OFFLINE` into `info.file_attributes` whenever `key`
    /// is a file that isn't fully present in the block cache (or the cache
    /// is disabled). Dirs are left alone — they have no payload to fetch.
    /// Driven from every code path that fills a `FileInfo` for an existing
    /// S3 object: `open`, `get_file_info`, `get_security_by_name`, and the
    /// `read_directory` per-entry loop.
    fn apply_offline_attr(&self, info: &mut FileInfo, key: &str, meta: &Meta) {
        if meta.is_dir {
            return;
        }
        let offline = match self.cache.as_ref() {
            Some(cache) => !cache.is_fully_cached(key, meta.size),
            None => true,
        };
        if offline {
            info.file_attributes |= FILE_ATTRIBUTE_OFFLINE;
        }
    }
}

// ── FileSystemContext impl ───────────────────────────────────────────────────

impl FileSystemContext for S3Fs {
    type FileContext = Box<OpenFile>;

    fn get_security_by_name(
        &self,
        file_name: &U16CStr,
        security_descriptor: Option<&mut [c_void]>,
        _reparse_point_resolver: impl FnOnce(&U16CStr) -> Option<FileSecurity>,
    ) -> winfsp::Result<FileSecurity> {
        let key = Self::to_key(file_name);
        let meta = self
            .lookup(&key)
            .map_err(|_| nt(STATUS_OBJECT_NAME_NOT_FOUND))?
            .ok_or_else(|| nt(STATUS_OBJECT_NAME_NOT_FOUND))?;

        let sz = copy_sd_into(&self.security, security_descriptor);
        let mut attributes = if meta.is_dir {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        // Same OFFLINE-overlay logic as `read_directory` — Explorer queries
        // single-file attributes via this path when it draws an icon for a
        // file that wasn't in the parent enumeration (e.g. a direct
        // navigation to a path in the address bar).
        if !meta.is_dir {
            let offline = match self.cache.as_ref() {
                Some(cache) => !cache.is_fully_cached(&key, meta.size),
                None => true,
            };
            if offline {
                attributes |= FILE_ATTRIBUTE_OFFLINE;
            }
        }
        Ok(FileSecurity {
            reparse: false,
            sz_security_descriptor: sz,
            attributes,
        })
    }

    fn open(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        granted_access: FILE_ACCESS_RIGHTS,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let input_key = Self::to_key(file_name);
        // Case-insensitively resolve to the actual S3 key so subsequent S3
        // ops on this handle use the real-case key (S3 is case-sensitive).
        let (real_key, meta) = self
            .resolve(&input_key)
            .map_err(|_| nt(STATUS_OBJECT_NAME_NOT_FOUND))?
            .ok_or_else(|| nt(STATUS_OBJECT_NAME_NOT_FOUND))?;

        fill_file_info(file_info.as_mut(), &meta);
        self.apply_offline_attr(file_info.as_mut(), &real_key, &meta);

        // Phase 4.1 + 4.3: acquire writer locks BEFORE handing back a handle.
        // We only do the check for files the caller intends to write to, and
        // skip it entirely for our internal `.nanocrew/` bookkeeping keys.
        let took_local = if !meta.is_dir
            && is_write_access(granted_access)
            && !is_internal_key(&real_key)
        {
            let lk = real_key.to_ascii_lowercase();
            {
                let mut locks = self.local_writers.lock().unwrap_or_else(|p| p.into_inner());
                if locks.contains(&lk) {
                    return Err(nt(STATUS_SHARING_VIOLATION));
                }
                locks.insert(lk.clone());
            }
            // Cross-device advisory check. Failures to read the sentinel (S3
            // hiccups, network blips) are logged but don't block the open —
            // the local-writer set already guarantees single-writer per file
            // on this machine, and we don't want to lock users out because
            // their connection stuttered.
            let client = self.file_lock_client.clone();
            let bucket = self.file_lock_bucket.clone();
            let mid = self.machine_id.clone();
            let key = self.file_lock_abs_key(&real_key);
            let state = self.rt.block_on(async move {
                match file_lock::check(&client, &bucket, &key, &mid).await {
                    Ok(st) => Ok(st),
                    Err(e) => Err(e),
                }
            });
            match state {
                Ok(file_lock::LockState::Foreign(s)) => {
                    // Release our local claim before bailing.
                    self.local_writers
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&lk);
                    self.emit_lock(FileLockEvent {
                        drive_id: self.drive_id,
                        target: real_key.clone(),
                        trigger: real_key.clone(),
                        state: "sentinel_conflict".into(),
                        owner: Some(s.owner),
                        machine: Some(s.machine),
                    });
                    return Err(nt(STATUS_SHARING_VIOLATION));
                }
                Ok(file_lock::LockState::Free) | Err(_) => {}
            }

            // Acquire our own sentinel. Best-effort — if the PUT fails, log
            // and fall through; the local-writer set still protects against
            // same-machine conflicts.
            let client = self.file_lock_client.clone();
            let bucket = self.file_lock_bucket.clone();
            let mid = self.machine_id.clone();
            let owner = self.owner.clone();
            let key = self.file_lock_abs_key(&real_key);
            let _ = self.rt.block_on(async move {
                file_lock::acquire(&client, &bucket, &key, &mid, &owner).await
            });
            true
        } else {
            false
        };

        // Phase 4.2: emit lockfile-detection events for Office/LibreOffice/vim
        // sidecar files so the UI can light up "being edited" badges.
        if !meta.is_dir {
            let (_, base) = Self::split_key(&real_key);
            if let Some(target) = file_lock::classify_lockfile(base) {
                self.emit_lock(FileLockEvent {
                    drive_id: self.drive_id,
                    target,
                    trigger: real_key.clone(),
                    state: "lockfile_created".into(),
                    owner: None,
                    machine: None,
                });
            }
        }

        let handle = if meta.is_dir {
            OpenFile::Dir {
                key: real_key,
                dir_buffer: Arc::new(DirBuffer::new()),
            }
        } else {
            if should_materialize(&real_key, meta.size) {
                let _ = self.materialize_file(&real_key, meta.size);
            }
            OpenFile::File {
                key: real_key,
                meta: Mutex::new(meta),
                write: Mutex::new(None),
                download: Mutex::new(None),
                pending_delete: AtomicBool::new(false),
                holds_writer_lock: AtomicBool::new(took_local),
                last_read_offset: Arc::new(Mutex::new(None)),
                sequential_streak: Arc::new(AtomicU64::new(0)),
            }
        };
        Ok(Box::new(handle))
    }

    fn close(&self, context: Self::FileContext) {
        // Finalize any in-progress download. We treat every clean handle
        // close as "done" regardless of whether bytes_done reached size —
        // Windows apps (thumbnailers, AV, editors, property-sheet probes)
        // routinely open a file, read a partial byte range, and close, and
        // flagging those as errors would flood the Transfers screen with
        // false-positive failures. The cost is that a genuinely cancelled
        // download shows as "done" with a partial byte count; we think that
        // trade is correct for now.
        if let OpenFile::File {
            ref download,
            ref key,
            ref holds_writer_lock,
            ..
        } = *context
        {
            let slot = std::mem::take(
                &mut *download.lock().unwrap_or_else(|p| p.into_inner()),
            );
            if let Some(d) = slot {
                let done = d.bytes_done.load(Ordering::Relaxed);
                (self.emit)(TransferPayload {
                    id: d.xfer_id,
                    drive_id: self.drive_id,
                    filename: d.filename,
                    direction: "download".into(),
                    total_bytes: d.size,
                    done_bytes: done,
                    state: "done".into(),
                    error: None,
                });
            }

            // Phase 4.1/4.3: release writer lock + sentinel if we took one.
            if holds_writer_lock.swap(false, Ordering::Relaxed) {
                self.local_writers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&key.to_ascii_lowercase());
                let client = self.file_lock_client.clone();
                let bucket = self.file_lock_bucket.clone();
                let k = self.file_lock_abs_key(key);
                let _ = self
                    .rt
                    .block_on(async move { file_lock::release(&client, &bucket, &k).await });
            }

            // Phase 4.2: emit lockfile-released for editor sidecars.
            let (_, base) = Self::split_key(key);
            if let Some(target) = file_lock::classify_lockfile(base) {
                self.emit_lock(FileLockEvent {
                    drive_id: self.drive_id,
                    target,
                    trigger: key.clone(),
                    state: "lockfile_released".into(),
                    owner: None,
                    machine: None,
                });
            }
        }
        // Box drops after this.
    }

    fn create(
        &self,
        file_name: &U16CStr,
        create_options: u32,
        _granted_access: FILE_ACCESS_RIGHTS,
        _file_attributes: FILE_FLAGS_AND_ATTRIBUTES,
        _security_descriptor: Option<&[c_void]>,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        _extra_buffer_is_reparse_point: bool,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let input_key = Self::to_key(file_name);
        // Reject overwrite via create (case-insensitive — S3 is case-
        // sensitive but we expose a case-insensitive namespace).
        if self.lookup(&input_key).ok().flatten().is_some() {
            return Err(nt(STATUS_OBJECT_NAME_COLLISION));
        }

        // Anchor the new key to the real-case parent prefix so we don't
        // accidentally spawn a ghost folder like `AGENT-UPLOADS/` next to
        // the real `agent-uploads/`.
        let (input_parent, basename) = Self::split_key(&input_key);
        let key = if input_parent.is_empty() {
            basename.to_string()
        } else {
            match self.resolve(input_parent).ok().flatten() {
                Some((real_parent, m)) if m.is_dir => {
                    if real_parent.is_empty() {
                        basename.to_string()
                    } else {
                        format!("{}/{}", real_parent, basename)
                    }
                }
                // Parent missing — let the create proceed with the input
                // case; S3 will materialize the prefix.
                _ => input_key.clone(),
            }
        };

        // FILE_DIRECTORY_FILE = 0x00000001
        let is_dir = (create_options & 0x00000001) != 0;

        if is_dir {
            // Create a `.keep` marker so the prefix survives an empty folder.
            let marker_key = if key.is_empty() {
                ".keep".to_string()
            } else {
                format!("{}/.keep", key)
            };
            let provider = self.provider.clone();
            let mk = marker_key.clone();
            self.rt
                .block_on(async move {
                    provider
                        .put_object(&mk, Bytes::new())
                        .await
                        .map_err(|e| format!("put_object .keep: {e}"))
                })
                .map_err(|_| nt(STATUS_ACCESS_DENIED))?;

            self.invalidate_parent(&key);
            // Clear any negative meta-cache entry left by Explorer's pre-create
            // existence check, so the immediate post-create lookup succeeds.
            self.invalidate_meta(&key);

            let meta = Meta {
                is_dir: true,
                size: 0,
                mtime_filetime: now_filetime(),
            };
            fill_file_info(file_info.as_mut(), &meta);
            return Ok(Box::new(OpenFile::Dir {
                key,
                dir_buffer: Arc::new(DirBuffer::new()),
            }));
        }

        // New file. Kick off write state immediately so subsequent write()
        // calls have a place to accumulate bytes.
        let write = self
            .new_write_state(&key, true)
            .map_err(|e| {
                eprintln!("[winfsp] new_write_state {key}: {e}");
                nt(STATUS_ACCESS_DENIED)
            })?;

        // Phase 4.1 + 4.3: a `create` is always a writer, so stake the
        // local-writer claim and drop a sentinel. Internal bookkeeping keys
        // bypass — we must be able to write sentinels themselves.
        let took_local = if !is_internal_key(&key) {
            let lk = key.to_ascii_lowercase();
            let mut locks = self.local_writers.lock().unwrap_or_else(|p| p.into_inner());
            if locks.contains(&lk) {
                return Err(nt(STATUS_SHARING_VIOLATION));
            }
            locks.insert(lk);
            drop(locks);

            // Foreign sentinel? Reject before we commit to the upload path.
            let client = self.file_lock_client.clone();
            let bucket = self.file_lock_bucket.clone();
            let mid = self.machine_id.clone();
            let k = self.file_lock_abs_key(&key);
            if let Ok(file_lock::LockState::Foreign(s)) =
                self.rt.block_on(async move { file_lock::check(&client, &bucket, &k, &mid).await })
            {
                self.local_writers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&key.to_ascii_lowercase());
                self.emit_lock(FileLockEvent {
                    drive_id: self.drive_id,
                    target: key.clone(),
                    trigger: key.clone(),
                    state: "sentinel_conflict".into(),
                    owner: Some(s.owner),
                    machine: Some(s.machine),
                });
                return Err(nt(STATUS_SHARING_VIOLATION));
            }

            let client = self.file_lock_client.clone();
            let bucket = self.file_lock_bucket.clone();
            let mid = self.machine_id.clone();
            let owner = self.owner.clone();
            let k = self.file_lock_abs_key(&key);
            let _ = self.rt.block_on(async move {
                file_lock::acquire(&client, &bucket, &k, &mid, &owner).await
            });
            true
        } else {
            false
        };

        // Phase 4.2: emit lockfile-created for editor sidecars.
        let (_, base) = Self::split_key(&key);
        if let Some(target) = file_lock::classify_lockfile(base) {
            self.emit_lock(FileLockEvent {
                drive_id: self.drive_id,
                target,
                trigger: key.clone(),
                state: "lockfile_created".into(),
                owner: None,
                machine: None,
            });
        }

        let meta = Meta {
            is_dir: false,
            size: 0,
            mtime_filetime: now_filetime(),
        };
        fill_file_info(file_info.as_mut(), &meta);
        self.invalidate_parent(&key);
        self.invalidate_meta(&key);
        Ok(Box::new(OpenFile::File {
            key,
            meta: Mutex::new(meta),
            write: Mutex::new(Some(write)),
            download: Mutex::new(None),
            pending_delete: AtomicBool::new(false),
            holds_writer_lock: AtomicBool::new(took_local),
            last_read_offset: Arc::new(Mutex::new(None)),
            sequential_streak: Arc::new(AtomicU64::new(0)),
        }))
    }

    fn cleanup(&self, context: &Self::FileContext, _file_name: Option<&U16CStr>, flags: u32) {
        // FspCleanupDelete = 0x01
        const CLEANUP_DELETE: u32 = 0x01;
        match context.as_ref() {
            OpenFile::Dir { key, .. } => {
                if flags & CLEANUP_DELETE != 0 {
                    // Delete the `.keep` marker if present.
                    let marker = if key.is_empty() {
                        ".keep".to_string()
                    } else {
                        format!("{}/.keep", key)
                    };
                    let provider = self.provider.clone();
                    let marker_key = marker.clone();
                    let _ = self
                        .rt
                        .block_on(async move { provider.delete(&marker_key).await });
                    self.invalidate_parent(key);
                    self.invalidate_meta(key);
                    self.invalidate_cache(&marker);
                }
            }
            OpenFile::File {
                key,
                write,
                pending_delete,
                ..
            } => {
                // If a write was in progress, finalize (or discard on delete).
                let taken = write.lock().unwrap_or_else(|p| p.into_inner()).take();
                if let Some(state) = taken {
                    if pending_delete.load(Ordering::Relaxed) || (flags & CLEANUP_DELETE != 0) {
                        // Pending delete — just drop the temp spool.
                        let path = state.temp_path.clone();
                        drop(state.temp_file);
                        let _ = std::fs::remove_file(&path);
                    } else {
                        // finalize_write owns all transfer_progress emission
                        // (start/progress/done/error) so the UI shows a single
                        // continuous upload.
                        match self.finalize_write(key, state) {
                            Ok(_final_size) => {
                                self.invalidate_meta(key);
                                self.invalidate_parent(key);
                                self.invalidate_cache(key);
                            }
                            Err(e) => {
                                eprintln!("[winfsp] upload failed key={key}: {e}");
                            }
                        }
                    }
                }

                if flags & CLEANUP_DELETE != 0 || pending_delete.load(Ordering::Relaxed) {
                    let provider = self.provider.clone();
                    let k = key.to_string();
                    let _ = self
                        .rt
                        .block_on(async move { provider.delete(&k).await });
                    self.invalidate_parent(key);
                    self.invalidate_meta(key);
                    self.invalidate_cache(key);
                }
            }
        }
    }

    fn flush(&self, _context: Option<&Self::FileContext>, _file_info: &mut FileInfo) -> winfsp::Result<()> {
        // We could flush the current part here, but doing so mid-write would
        // create a part smaller than the S3 5 MiB minimum. Defer to cleanup.
        Ok(())
    }

    fn get_file_info(&self, context: &Self::FileContext, file_info: &mut FileInfo) -> winfsp::Result<()> {
        match context.as_ref() {
            OpenFile::Dir { .. } => {
                let meta = Meta {
                    is_dir: true,
                    size: 0,
                    mtime_filetime: now_filetime(),
                };
                fill_file_info(file_info, &meta);
            }
            OpenFile::File { key, meta, .. } => {
                let m = meta.lock().unwrap_or_else(|p| p.into_inner()).clone();
                fill_file_info(file_info, &m);
                self.apply_offline_attr(file_info, key, &m);
            }
        }
        Ok(())
    }

    fn get_security(
        &self,
        _context: &Self::FileContext,
        security_descriptor: Option<&mut [c_void]>,
    ) -> winfsp::Result<u64> {
        Ok(copy_sd_into(&self.security, security_descriptor))
    }

    fn read_directory(
        &self,
        context: &Self::FileContext,
        _pattern: Option<&U16CStr>,
        marker: DirMarker,
        buffer: &mut [u8],
    ) -> winfsp::Result<u32> {
        let key = match context.as_ref() {
            OpenFile::Dir { key, .. } => key.clone(),
            _ => return Err(nt(STATUS_NOT_A_DIRECTORY)),
        };

        // Track this directory in the recently-visited LRU so the
        // background refresh task can keep its listing fresh.
        self.note_visit(&key);

        // We bypass WinFsp's DirBuffer entirely. Its marker-based pagination
        // re-emits entries at every kernel read-buffer boundary (~338 entries)
        // and never reaches EOF for directories larger than one buffer —
        // sending Explorer into an unbounded loop. Instead we keep our own
        // case-insensitively sorted listing and fill the response buffer
        // directly: a strict `> marker` comparison for continuation, and an
        // explicit EOF marker (finalize_buffer) once the final entry fits.
        let entries = self.sorted_dir_entries(&key).map_err(|e| {
            tracing::error!(target: "nanocrew::vfs", "read_directory enum key={key:?}: {e}");
            nt(STATUS_ACCESS_DENIED)
        })?;

        // Find the continuation point: first entry whose folded name sorts
        // strictly after the marker (exclusive). Entries are sorted by the
        // same fold, so a binary search is correct and O(log n).
        let start = match marker.inner_as_cstr() {
            None => 0,
            Some(m) => {
                let mk = m.to_string_lossy().to_uppercase();
                entries.partition_point(|(name, _)| name.to_uppercase() <= mk)
            }
        };

        let mut cursor: u32 = 0;
        let mut all_fit = true;
        for (name, meta) in &entries[start..] {
            let mut info = DirInfo::<255>::new();
            if info.set_name(name.as_str()).is_err() {
                continue;
            }
            fill_file_info(info.file_info_mut(), meta);
            // Flag not-yet-cached files with FILE_ATTRIBUTE_OFFLINE so
            // Explorer renders the cloud / offline overlay. Dirs and the
            // "." / ".." pseudo-entries are skipped by apply_offline_attr
            // (meta.is_dir == true).
            if name != "." && name != ".." {
                let full_key = if key.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", key, name)
                };
                self.apply_offline_attr(info.file_info_mut(), &full_key, meta);
            }
            if !info.append_to_buffer(buffer, &mut cursor) {
                all_fit = false;
                break;
            }
        }
        if all_fit {
            // All remaining entries fit — write the NULL EOF marker so the
            // kernel knows the enumeration is complete and stops asking.
            DirInfo::<255>::finalize_buffer(buffer, &mut cursor);
        }
        Ok(cursor)
    }

    fn read(&self, context: &Self::FileContext, buffer: &mut [u8], offset: u64) -> winfsp::Result<u32> {
        let (key, size, download, last_read_offset, sequential_streak) = match context.as_ref() {
            OpenFile::File { key, meta, download, last_read_offset, sequential_streak, .. } => {
                let m = meta.lock().unwrap_or_else(|p| p.into_inner());
                (key.clone(), m.size, download, last_read_offset.clone(), sequential_streak.clone())
            }
            _ => return Err(nt(STATUS_INVALID_PARAMETER)),
        };
        if offset >= size {
            return Err(nt(STATUS_END_OF_FILE));
        }
        let avail = (size - offset).min(buffer.len() as u64);

        let last_off = {
            let mut g = last_read_offset.lock().unwrap_or_else(|p| p.into_inner());
            let prev = *g;
            *g = Some(offset + avail);
            prev
        };
        let is_sequential = matches!(last_off, Some(prev) if prev == offset || prev + CACHE_BLOCK >= offset);
        let streak = if is_sequential {
            sequential_streak.fetch_add(1, Ordering::Relaxed) + 1
        } else {
            sequential_streak.store(0, Ordering::Relaxed);
            0
        };

        let bytes = self
            .get_range(&key, offset, avail)
            .map_err(|e| {
                eprintln!("[winfsp] read {key} @{offset}+{avail}: {e}");
                nt(STATUS_INVALID_PARAMETER)
            })?;
        let n = bytes.len().min(buffer.len());
        buffer[..n].copy_from_slice(&bytes[..n]);
        self.emit_download_progress(download, &key, size, n as u64);

        if streak >= 2 && self.cache.as_ref().is_some_and(|c| c.is_enabled()) {
            self.prefetch_ahead(&key, offset + avail, size, 4);
        }
        Ok(n as u32)
    }

    fn write(
        &self,
        context: &Self::FileContext,
        buffer: &[u8],
        offset: u64,
        write_to_eof: bool,
        _constrained_io: bool,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<u32> {
        let (key, write, meta) = match context.as_ref() {
            OpenFile::File {
                key, write, meta, ..
            } => (key, write, meta),
            _ => return Err(nt(STATUS_INVALID_PARAMETER)),
        };

        let mut w_guard = write.lock().unwrap_or_else(|p| p.into_inner());
        if w_guard.is_none() {
            let state = self.new_write_state(key, false).map_err(|e| {
                eprintln!("[winfsp] new_write_state {key}: {e}");
                nt(STATUS_INVALID_PARAMETER)
            })?;
            *w_guard = Some(state);
        }
        let state = w_guard.as_mut().unwrap();

        // Positioned write into the temp spool — handles out-of-order and
        // overlapping writes from Explorer natively.
        let write_offset = if write_to_eof { state.bytes_written } else { offset };
        let mut remaining = buffer;
        let mut cursor = write_offset;
        while !remaining.is_empty() {
            let n = state.temp_file.seek_write(remaining, cursor).map_err(|e| {
                eprintln!("[winfsp] seek_write {key} @{cursor}: {e}");
                nt(STATUS_INVALID_PARAMETER)
            })?;
            if n == 0 {
                break;
            }
            cursor += n as u64;
            remaining = &remaining[n..];
        }
        let end = write_offset + buffer.len() as u64;
        if end > state.bytes_written {
            state.bytes_written = end;
        }
        state.record_extent(write_offset, end);

        // Dispatch any part that's now fully buffered. This is where the
        // upload pipeline fills up; when 8 parts are already in flight, the
        // next call blocks on the semaphore — throttling Explorer's progress
        // bar to match S3 throughput.
        if let Err(e) = self.dispatch_ready_parts(key, state, false) {
            eprintln!("[winfsp] dispatch_ready_parts {key}: {e}");
            return Err(nt(STATUS_INVALID_PARAMETER));
        }

        // Progress — use actual uploaded bytes so the in-app bar matches what
        // Explorer is seeing. For the first PART_TARGET worth of writes
        // bytes_uploaded will still be 0; fall back to bytes_written so the
        // bar at least appears.
        let uploaded_now = state.bytes_uploaded.load(Ordering::Relaxed);
        let progress_bytes = uploaded_now.max(state.bytes_written.min(PART_TARGET as u64));
        let total = meta
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .size
            .max(state.bytes_written);
        state.total_size_hint = total;
        self.emit_progress(state, progress_bytes, total, false);

        // Advertise new file_info (grown size).
        {
            let mut m = meta.lock().unwrap_or_else(|p| p.into_inner());
            m.size = state.bytes_written.max(m.size);
            fill_file_info(file_info, &m);
        }
        Ok(buffer.len() as u32)
    }

    fn set_file_size(
        &self,
        context: &Self::FileContext,
        new_size: u64,
        _set_allocation_size: bool,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        // Windows calls this before a copy to pre-size the file. We record it
        // so the progress bar has a meaningful "total" without actually
        // reserving S3 storage.
        // Always fill file_info — returning Ok() with an uninitialised buffer
        // lets WinFsp see a garbage AllocationSize that can exceed our reported
        // TotalSize (1 TiB), which causes STATUS_FILE_TOO_LARGE.
        match context.as_ref() {
            OpenFile::File { meta, .. } => {
                let mut m = meta.lock().unwrap_or_else(|p| p.into_inner());
                m.size = new_size;
                fill_file_info(file_info, &m);
            }
            OpenFile::Dir { .. } => {
                fill_file_info(file_info, &Meta { is_dir: true, size: 0, mtime_filetime: now_filetime() });
            }
        }
        Ok(())
    }

    fn overwrite(
        &self,
        context: &Self::FileContext,
        _file_attributes: FILE_FLAGS_AND_ATTRIBUTES,
        _replace_file_attributes: bool,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        // Reset any write-state + treat as fresh upload.
        if let OpenFile::File { key, write, meta, .. } = context.as_ref() {
            let mut w = write.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(prev) = w.take() {
                let _ = std::fs::remove_file(&prev.temp_path);
            }
            let fresh = self.new_write_state(key, false).map_err(|e| {
                eprintln!("[winfsp] new_write_state overwrite {key}: {e}");
                nt(STATUS_ACCESS_DENIED)
            })?;
            *w = Some(fresh);
            let mut m = meta.lock().unwrap_or_else(|p| p.into_inner());
            m.size = 0;
            fill_file_info(file_info, &m);
        }
        Ok(())
    }

    fn set_delete(
        &self,
        context: &Self::FileContext,
        _file_name: &U16CStr,
        delete_file: bool,
    ) -> winfsp::Result<()> {
        if let OpenFile::File { pending_delete, .. } = context.as_ref() {
            pending_delete.store(delete_file, Ordering::Relaxed);
        }
        Ok(())
    }

    fn rename(
        &self,
        context: &Self::FileContext,
        _file_name: &U16CStr,
        new_file_name: &U16CStr,
        _replace_if_exists: bool,
    ) -> winfsp::Result<()> {
        // S3 is case-sensitive, but Windows/Explorer is not — the path Windows
        // hands us can be upper/lower-cased arbitrarily (address-bar typing,
        // path caching, etc.). The open handle's `key` was captured from the
        // listing and has the true S3 case, so use it as source of truth.
        let old_key = match context.as_ref() {
            OpenFile::File { key, .. } => key.clone(),
            OpenFile::Dir { key, .. } => key.clone(),
        };
        // For the destination, trust the source's parent (real S3 case) and
        // take only the new basename from Windows. Windows may also upper-case
        // the parent prefix in `new_file_name`, which would create a ghost
        // folder in S3.
        let win_new_key = Self::to_key(new_file_name);
        let (_, new_basename) = Self::split_key(&win_new_key);
        let (old_parent, _) = Self::split_key(&old_key);
        let new_key = if old_parent.is_empty() {
            new_basename.to_string()
        } else {
            format!("{}/{}", old_parent, new_basename)
        };
        if old_key == new_key {
            return Ok(());
        }

        let is_dir = matches!(context.as_ref(), OpenFile::Dir { .. });
        if is_dir {
            // Directory rename: list every object under the old prefix,
            // copy each to the new prefix, then delete the originals.
            // S3 has no native rename; a directory is just a shared key prefix.
            let provider = self.provider.clone();
            let old_prefix_arg = format!("{}/", old_key);
            let old_key_c = old_key.clone();
            let new_key_c = new_key.clone();
            self.rt
                .block_on(async move {
                    let abs_keys = provider
                        .list_prefix(&old_prefix_arg)
                        .await
                        .map_err(|e| format!("list_prefix: {e}"))?;
                    for abs_old in &abs_keys {
                        // list_prefix returns VFS-relative keys. Strip the
                        // old_key prefix to get the per-object suffix
                        // ("/name" or "/.keep"), then reattach under new_key.
                        let suffix = &abs_old[old_key_c.len()..]; // "/name" or "/.keep"
                        let abs_new = format!("{}{}", new_key_c, suffix);
                        provider
                            .copy_object(abs_old, &abs_new)
                            .await
                            .map_err(|e| format!("copy_object {abs_old}: {e}"))?;
                        provider
                            .delete(abs_old)
                            .await
                            .map_err(|e| format!("delete_object {abs_old}: {e}"))?;
                    }
                    Ok::<(), String>(())
                })
                .map_err(|e| {
                    eprintln!("[winfsp] dir rename failed src={old_key} dst={new_key}: {e}");
                    nt(STATUS_INVALID_PARAMETER)
                })?;
        } else {
            // File rename: copy then delete.
            let provider = self.provider.clone();
            let old_k = old_key.clone();
            let new_k = new_key.clone();
            self.rt
                .block_on(async move {
                    provider
                        .copy_object(&old_k, &new_k)
                        .await
                        .map_err(|e| format!("copy_object: {e}"))?;
                    provider
                        .delete(&old_k)
                        .await
                        .map_err(|e| format!("delete_object: {e}"))
                })
                .map_err(|e| {
                    eprintln!("[winfsp] rename failed src={old_key} dst={new_key}: {e}");
                    nt(STATUS_INVALID_PARAMETER)
                })?;
        }
        self.invalidate_parent(&old_key);
        self.invalidate_parent(&new_key);
        self.invalidate_meta(&old_key);
        self.invalidate_meta(&new_key);
        self.invalidate_cache(&old_key);
        self.invalidate_cache(&new_key);
        Ok(())
    }

    fn get_volume_info(&self, out: &mut VolumeInfo) -> winfsp::Result<()> {
        // S3 is effectively unlimited — report 1 TiB total with 1 TiB free so
        // Explorer is happy. Real size could be computed via a bucket-level
        // LIST with usage summation but that'd slow mount.
        const ONE_TIB: u64 = 1024 * 1024 * 1024 * 1024;
        out.total_size = ONE_TIB;
        out.free_size = ONE_TIB;
        out.set_volume_label(&self.volume_label);
        Ok(())
    }
}



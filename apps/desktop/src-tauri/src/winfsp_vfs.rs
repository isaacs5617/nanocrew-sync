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
    collections::{BTreeMap, HashMap},
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

use aws_sdk_s3::{types::CompletedPart, Client};
use tokio::runtime::Runtime;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{
            STATUS_ACCESS_DENIED, STATUS_END_OF_FILE, STATUS_INVALID_PARAMETER,
            STATUS_NOT_A_DIRECTORY, STATUS_OBJECT_NAME_COLLISION, STATUS_OBJECT_NAME_NOT_FOUND,
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

use crate::types::TransferPayload;

// ── Tuning ───────────────────────────────────────────────────────────────────

const LIST_TTL: Duration = Duration::from_secs(5);
const META_TTL: Duration = Duration::from_secs(5);
/// S3 multipart minimum is 5 MiB except for the last part. We target 8 MiB to
/// amortise request overhead but still emit progress events frequently.
const PART_TARGET: usize = 16 * 1024 * 1024;
/// How many multipart part uploads we run in parallel. 8 matches the AWS CLI
/// default and saturates most home uplinks without exhausting connection
/// pools.
const UPLOAD_CONCURRENCY: usize = 8;
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

/// Cached directory listing. Keyed by the directory's S3 prefix.
#[derive(Clone)]
struct CachedList {
    /// Subdirectory names (just the last path component, no trailing slash).
    dirs: Vec<String>,
    /// (filename, meta) pairs — filename is the last path component only.
    files: Vec<(String, Meta)>,
}

/// Per-open-handle state. WinFsp stores these behind `Box<OpenFile>` and
/// passes us `&OpenFile` in subsequent callbacks, so all mutable state must
/// live behind interior mutability.
pub enum OpenFile {
    Dir {
        key: String, // "" for root
        dir_buffer: DirBuffer,
    },
    File {
        key: String,
        meta: Mutex<Meta>,
        /// `Some` once a write has begun on this handle. Dropped on `close`.
        write: Mutex<Option<WriteState>>,
        /// Set by `set_delete(true)`, acted on during `cleanup`.
        pending_delete: AtomicBool,
    },
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
    completed_parts: Arc<std::sync::Mutex<Vec<CompletedPart>>>,
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
    pub client: Client,
    pub bucket: String,
    pub drive_id: i64,
    pub volume_label: String,

    next_xfer: AtomicU64,
    emit: Box<dyn Fn(TransferPayload) + Send + Sync>,

    list_cache: Mutex<HashMap<String, (Instant, CachedList)>>,
    meta_cache: Mutex<HashMap<String, (Instant, Option<Meta>)>>,

    /// Single DACL blob returned for every file/directory. Everyone gets full
    /// access — we don't enforce ACLs on S3.
    security: Vec<u8>,
}

impl S3Fs {
    pub fn new(
        client: Client,
        bucket: String,
        drive_id: i64,
        volume_label: String,
        emit: Box<dyn Fn(TransferPayload) + Send + Sync>,
    ) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(
                std::thread::available_parallelism()
                    .map(|n| n.get().max(4))
                    .unwrap_or(4),
            )
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;

        let security = build_everyone_sd().map_err(|e| format!("build SD: {e}"))?;

        Ok(Self {
            rt,
            client,
            bucket,
            drive_id,
            volume_label,
            next_xfer: AtomicU64::new(2_000_000),
            emit,
            list_cache: Mutex::new(HashMap::new()),
            meta_cache: Mutex::new(HashMap::new()),
            security,
        })
    }

    // ── Path translation ─────────────────────────────────────────────────────

    /// Convert a WinFsp path (`\foo\bar.txt` or `\`) to an S3 key
    /// (`foo/bar.txt` or empty string for root).
    fn to_key(path: &U16CStr) -> String {
        let s = path.to_string_lossy();
        s.trim_start_matches('\\')
            .replace('\\', "/")
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
    }

    fn invalidate_meta(&self, key: &str) {
        self.meta_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
    }

    // ── S3 operations ────────────────────────────────────────────────────────

    /// List a single S3 "directory" (delimited by `/`). Result is cached for
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

        let s3_prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", prefix)
        };
        let client = self.client.clone();
        let bucket = self.bucket.clone();

        let listing: Result<CachedList, String> = self.rt.block_on(async move {
            let mut dirs = Vec::<String>::new();
            let mut files = Vec::<(String, Meta)>::new();
            let mut cont: Option<String> = None;
            loop {
                let mut req = client
                    .list_objects_v2()
                    .bucket(&bucket)
                    .prefix(&s3_prefix)
                    .delimiter("/");
                if let Some(c) = cont.as_ref() {
                    req = req.continuation_token(c);
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| format!("list_objects_v2: {e}"))?;

                for cp in resp.common_prefixes() {
                    if let Some(full) = cp.prefix() {
                        let name = full.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                        if !name.is_empty() {
                            dirs.push(name.to_string());
                        }
                    }
                }
                for obj in resp.contents() {
                    let Some(full) = obj.key() else { continue };
                    // `.keep` markers exist only to keep empty folders alive
                    // server-side; don't surface to Explorer.
                    if full.ends_with("/.keep") || full.ends_with('/') {
                        continue;
                    }
                    let name = full.rsplit('/').next().unwrap_or("");
                    if name.is_empty() || name == ".keep" {
                        continue;
                    }
                    let size = obj.size().unwrap_or(0).max(0) as u64;
                    let mtime_filetime = obj
                        .last_modified()
                        .map(|d| unix_secs_to_filetime(d.secs()))
                        .unwrap_or(0);
                    files.push((
                        name.to_string(),
                        Meta {
                            is_dir: false,
                            size,
                            mtime_filetime,
                        },
                    ));
                }
                match resp.next_continuation_token() {
                    Some(t) => cont = Some(t.to_string()),
                    None => break,
                }
            }
            Ok(CachedList { dirs, files })
        });

        let listing = listing?;
        self.list_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(prefix.to_string(), (Instant::now(), listing.clone()));
        Ok(listing)
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
        // Meta cache — keyed by the INPUT (possibly mis-cased) key so repeated
        // lookups with the same casing hit the cache.
        {
            let cache = self.meta_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((at, v)) = cache.get(key) {
                if at.elapsed() < META_TTL {
                    // We stored the resolved key alongside meta — but the
                    // current cache only holds Option<Meta>. Return the input
                    // key as a best-effort (case will match if the caller used
                    // the canonical case). For mis-cased misses we'll fall
                    // through to re-resolve and repopulate.
                    if let Some(m) = v {
                        return Ok(Some((key.to_string(), m.clone())));
                    } else {
                        return Ok(None);
                    }
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
                        .insert(key.to_string(), (Instant::now(), None));
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
                .insert(key.to_string(), (Instant::now(), None));
            return Ok(None);
        }

        let found = last_meta.map(|m| (parent_real, m));
        // Cache the meta under the input key for fast re-hits.
        self.meta_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                key.to_string(),
                (Instant::now(), found.as_ref().map(|(_, m)| m.clone())),
            );
        Ok(found)
    }

    /// Convenience wrapper when the caller only needs existence/metadata and
    /// not the real-case key.
    fn lookup(&self, key: &str) -> Result<Option<Meta>, String> {
        Ok(self.resolve(key)?.map(|(_, m)| m))
    }

    fn get_range(&self, key: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let key_s = key.to_string();
        self.rt.block_on(async move {
            let end = offset + len - 1;
            let range = format!("bytes={}-{}", offset, end);
            let resp = client
                .get_object()
                .bucket(&bucket)
                .key(&key_s)
                .range(range)
                .send()
                .await
                .map_err(|e| format!("get_object: {e}"))?;
            let bytes = resp
                .body
                .collect()
                .await
                .map_err(|e| format!("body collect: {e}"))?
                .into_bytes();
            Ok(bytes.to_vec())
        })
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
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let key_s = key.to_string();
        let resp = self.rt.block_on(async move {
            client
                .create_multipart_upload()
                .bucket(&bucket)
                .key(&key_s)
                .send()
                .await
                .map_err(|e| format!("create_multipart_upload: {e}"))
        })?;
        state.upload_id = Some(
            resp.upload_id()
                .ok_or_else(|| "missing upload_id".to_string())?
                .to_string(),
        );
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
            let client = self.client.clone();
            let bucket = self.bucket.clone();
            let key_s = key.to_string();
            let temp_path = state.temp_path.clone();
            let completed_parts = state.completed_parts.clone();
            let upload_err = state.upload_err.clone();
            let bytes_uploaded = state.bytes_uploaded.clone();

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
                    let resp = client
                        .upload_part()
                        .bucket(&bucket)
                        .key(&key_s)
                        .upload_id(&upload_id)
                        .part_number(pn)
                        .body(buf.into())
                        .send()
                        .await
                        .map_err(|e| format!("upload_part {pn}: {e}"))?;
                    let etag = resp.e_tag().unwrap_or("").to_string();
                    bytes_uploaded.fetch_add(actual as u64, Ordering::Relaxed);
                    completed_parts
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(
                            CompletedPart::builder()
                                .e_tag(etag)
                                .part_number(pn)
                                .build(),
                        );
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
        eprintln!(
            "[winfsp] finalize_write begin key={key} size={size} upload_id={} dispatched={}",
            state.upload_id.as_deref().unwrap_or("none"),
            state.dispatched_bytes
        );
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
                parts.sort_by_key(|p| p.part_number().unwrap_or(0));
                let upload_id = state.upload_id.clone().unwrap();
                let client = self.client.clone();
                let bucket = self.bucket.clone();
                let key_s = key.to_string();
                self.rt
                    .block_on(async move {
                        let completed = aws_sdk_s3::types::CompletedMultipartUpload::builder()
                            .set_parts(Some(parts))
                            .build();
                        client
                            .complete_multipart_upload()
                            .bucket(&bucket)
                            .key(&key_s)
                            .upload_id(&upload_id)
                            .multipart_upload(completed)
                            .send()
                            .await
                            .map_err(|e| format!("complete_multipart_upload: {e}"))
                            .map(|_| ())
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
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let key_s = key.to_string();
        self.rt
            .block_on(async move {
                client
                    .put_object()
                    .bucket(&bucket)
                    .key(&key_s)
                    .body(bytes.into())
                    .send()
                    .await
                    .map_err(|e| format!("put_object: {e}"))
                    .map(|_| ())
            })
    }


    /// Abort a multipart upload on error (best-effort).
    fn abort_multipart(&self, key: &str, upload_id: &str) {
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let key_s = key.to_string();
        let uid = upload_id.to_string();
        let _ = self.rt.block_on(async move {
            client
                .abort_multipart_upload()
                .bucket(&bucket)
                .key(&key_s)
                .upload_id(&uid)
                .send()
                .await
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

/// Percent-encode an S3 key for use in `x-amz-copy-source`. RFC 3986
/// unreserved chars (`A-Za-z0-9-._~`) plus `/` (kept as path separator) pass
/// through; everything else is `%XX`-encoded. Wasabi rejects raw `:`, `+`,
/// spaces, etc., and AWS recommends encoding too.
fn percent_encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.as_bytes() {
        let c = *b;
        let is_unreserved = c.is_ascii_alphanumeric()
            || matches!(c, b'-' | b'.' | b'_' | b'~' | b'/');
        if is_unreserved {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{:02X}", c));
        }
    }
    out
}

fn unix_secs_to_filetime(secs: i64) -> u64 {
    // 11644473600 = seconds between 1601-01-01 and 1970-01-01
    let s = secs.max(0) as u64 + 11_644_473_600;
    s * 10_000_000
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
        Ok(FileSecurity {
            reparse: false,
            sz_security_descriptor: sz,
            attributes: if meta.is_dir {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            },
        })
    }

    fn open(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        _granted_access: FILE_ACCESS_RIGHTS,
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

        let handle = if meta.is_dir {
            OpenFile::Dir {
                key: real_key,
                dir_buffer: DirBuffer::new(),
            }
        } else {
            OpenFile::File {
                key: real_key,
                meta: Mutex::new(meta),
                write: Mutex::new(None),
                pending_delete: AtomicBool::new(false),
            }
        };
        Ok(Box::new(handle))
    }

    fn close(&self, _context: Self::FileContext) {
        // Box drops, freeing DirBuffer etc.
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
            let client = self.client.clone();
            let bucket = self.bucket.clone();
            let k = marker_key.clone();
            self.rt
                .block_on(async move {
                    client
                        .put_object()
                        .bucket(&bucket)
                        .key(&k)
                        .body(Vec::<u8>::new().into())
                        .send()
                        .await
                        .map_err(|e| format!("put_object .keep: {e}"))
                })
                .map_err(|_| nt(STATUS_ACCESS_DENIED))?;

            self.invalidate_parent(&key);

            let meta = Meta {
                is_dir: true,
                size: 0,
                mtime_filetime: now_filetime(),
            };
            fill_file_info(file_info.as_mut(), &meta);
            return Ok(Box::new(OpenFile::Dir {
                key,
                dir_buffer: DirBuffer::new(),
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

        let meta = Meta {
            is_dir: false,
            size: 0,
            mtime_filetime: now_filetime(),
        };
        fill_file_info(file_info.as_mut(), &meta);
        self.invalidate_parent(&key);
        Ok(Box::new(OpenFile::File {
            key,
            meta: Mutex::new(meta),
            write: Mutex::new(Some(write)),
            pending_delete: AtomicBool::new(false),
        }))
    }

    fn cleanup(&self, context: &Self::FileContext, _file_name: Option<&U16CStr>, flags: u32) {
        eprintln!("[winfsp] cleanup flags={flags:#x}");
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
                    let client = self.client.clone();
                    let bucket = self.bucket.clone();
                    let _ = self.rt.block_on(async move {
                        client
                            .delete_object()
                            .bucket(&bucket)
                            .key(&marker)
                            .send()
                            .await
                    });
                    self.invalidate_parent(key);
                    self.invalidate_meta(key);
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
                            Ok(final_size) => {
                                eprintln!(
                                    "[winfsp] upload complete key={key} bytes={final_size}"
                                );
                                self.invalidate_meta(key);
                                self.invalidate_parent(key);
                            }
                            Err(e) => {
                                eprintln!("[winfsp] upload failed key={key}: {e}");
                            }
                        }
                    }
                }

                if flags & CLEANUP_DELETE != 0 || pending_delete.load(Ordering::Relaxed) {
                    let client = self.client.clone();
                    let bucket = self.bucket.clone();
                    let k = key.clone();
                    let _ = self.rt.block_on(async move {
                        client.delete_object().bucket(&bucket).key(&k).send().await
                    });
                    self.invalidate_parent(key);
                    self.invalidate_meta(key);
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
            OpenFile::File { meta, .. } => {
                let m = meta.lock().unwrap_or_else(|p| p.into_inner()).clone();
                fill_file_info(file_info, &m);
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
        let (key, dir_buffer) = match context.as_ref() {
            OpenFile::Dir { key, dir_buffer } => (key, dir_buffer),
            _ => return Err(nt(STATUS_NOT_A_DIRECTORY)),
        };

        // Populate the dir buffer on first call (marker is None). Reuse on
        // subsequent calls via marker-based pagination.
        if marker.is_none() {
            let listing = self
                .list_dir(key)
                .map_err(|_| nt(STATUS_INVALID_PARAMETER))?;
            let lock = dir_buffer.acquire(true, None)?;

            // "." and ".." for subdirectories (not for root).
            if !key.is_empty() {
                let mut dot = DirInfo::<255>::new();
                dot.set_name(".").ok();
                let m = Meta {
                    is_dir: true,
                    size: 0,
                    mtime_filetime: now_filetime(),
                };
                fill_file_info(dot.file_info_mut(), &m);
                let _ = lock.write(&mut dot);

                let mut dd = DirInfo::<255>::new();
                dd.set_name("..").ok();
                fill_file_info(dd.file_info_mut(), &m);
                let _ = lock.write(&mut dd);
            }

            for d in &listing.dirs {
                let mut info = DirInfo::<255>::new();
                if info.set_name(d.as_str()).is_err() {
                    continue;
                }
                let m = Meta {
                    is_dir: true,
                    size: 0,
                    mtime_filetime: now_filetime(),
                };
                fill_file_info(info.file_info_mut(), &m);
                let _ = lock.write(&mut info);
            }
            for (name, meta) in &listing.files {
                let mut info = DirInfo::<255>::new();
                if info.set_name(name.as_str()).is_err() {
                    continue;
                }
                fill_file_info(info.file_info_mut(), meta);
                let _ = lock.write(&mut info);
            }
        }

        Ok(dir_buffer.read(marker, buffer))
    }

    fn read(&self, context: &Self::FileContext, buffer: &mut [u8], offset: u64) -> winfsp::Result<u32> {
        let (key, size) = match context.as_ref() {
            OpenFile::File { key, meta, .. } => {
                let m = meta.lock().unwrap_or_else(|p| p.into_inner());
                (key.clone(), m.size)
            }
            _ => return Err(nt(STATUS_INVALID_PARAMETER)),
        };
        if offset >= size {
            return Err(nt(STATUS_END_OF_FILE));
        }
        let avail = (size - offset).min(buffer.len() as u64);
        let bytes = self
            .get_range(&key, offset, avail)
            .map_err(|e| {
                eprintln!("[winfsp] read {key} @{offset}+{avail}: {e}");
                nt(STATUS_INVALID_PARAMETER)
            })?;
        let n = bytes.len().min(buffer.len());
        buffer[..n].copy_from_slice(&bytes[..n]);
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
        if let OpenFile::File { meta, .. } = context.as_ref() {
            let mut m = meta.lock().unwrap_or_else(|p| p.into_inner());
            m.size = new_size;
            fill_file_info(file_info, &m);
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
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        // `x-amz-copy-source` must be URL-encoded per S3 spec. `/` is a path
        // separator and must stay un-encoded; other special chars get
        // percent-encoded. Wasabi rejects raw spaces, `+`, `:` etc.
        let copy_src = format!("{}/{}", bucket, percent_encode_key(&old_key));
        let copy_src_c = copy_src.clone();
        let old_k = old_key.clone();
        let new_k = new_key.clone();
        self.rt
            .block_on(async move {
                client
                    .copy_object()
                    .bucket(&bucket)
                    .key(&new_k)
                    .copy_source(&copy_src_c)
                    .send()
                    .await
                    .map_err(|e| {
                        let svc = e.as_service_error().map(|s| format!("{s:?}"));
                        format!("copy_object: {e:?} svc={svc:?}")
                    })?;
                client
                    .delete_object()
                    .bucket(&bucket)
                    .key(&old_k)
                    .send()
                    .await
                    .map_err(|e| format!("delete_object: {e:?}"))
            })
            .map_err(|e| {
                eprintln!("[winfsp] rename failed src={old_key} dst={new_key} copy_src={copy_src}: {e}");
                nt(STATUS_INVALID_PARAMETER)
            })?;
        self.invalidate_parent(&old_key);
        self.invalidate_parent(&new_key);
        self.invalidate_meta(&old_key);
        self.invalidate_meta(&new_key);
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

// ── Pathbuf re-export used by mounts.rs ──────────────────────────────────────

#[allow(dead_code)]
pub fn cache_dir_for_drive(drive_id: i64) -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("NanoCrew").join("Sync").join("cache").join(format!("drive-{drive_id}")))
}


//! FUSE-T-backed cloud filesystem for macOS. The macOS analogue of
//! `winfsp_vfs.rs`.
//!
//! Why FUSE-T and not macFUSE? macFUSE requires a kernel extension which is
//! unsigned-blocked on Apple Silicon by default and requires SIP changes /
//! reboots. FUSE-T (https://github.com/macos-fuse-t/fuse-t) is a userspace
//! reimplementation that proxies FUSE protocol over an NFS loopback — no
//! kext, no SIP override, works the same on Intel and Apple Silicon.
//!
//! Scope of THIS scaffold:
//!
//!   * Inode table (FUSE ↔ S3 key map)
//!   * `lookup` + `getattr` from the listing/meta cache (read path only)
//!   * `readdir` via the provider trait, reusing `dir_listing_cache`
//!   * `read` via `provider.get_range` (no disk cache hookup yet)
//!   * Skeletal `mkdir`/`unlink`/`rmdir`/`rename`/`create`/`write` that
//!     return `EROFS` (read-only volume) — full write path is a TODO.
//!
//! The architecture mirrors `S3Fs`: an `Arc<dyn CloudProvider>` is the
//! storage backend, a long-lived multi-thread tokio runtime is owned by the
//! filesystem struct, and `rt.block_on(...)` is the bridge between fuser's
//! sync callbacks and the async provider trait.

#![cfg(target_os = "macos")]
#![allow(dead_code, unused_variables, unused_imports)] // scaffold — many TODOs

use std::{
    collections::HashMap,
    ffi::OsStr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request,
};
use libc::{c_int, EACCES, EIO, ENOENT, ENOTDIR, EROFS};
use tauri::{AppHandle, Emitter, Manager};
use tokio::runtime::Runtime;

use crate::{
    cache::DiskCache,
    error::AppError,
    http_client,
    mounts::{MountConfig, MountHandle},
    providers::{s3::S3Provider, CloudProvider, FileStat, ListDirResult},
    state::AppState,
    types::{DriveStatusPayload, FileLockEvent, TransferPayload},
};

// ── Tuning ───────────────────────────────────────────────────────────────────

const LIST_TTL: Duration = Duration::from_secs(60);
const META_TTL: Duration = Duration::from_secs(60);
const ATTR_TTL: Duration = Duration::from_secs(1);
const ENTRY_TTL: Duration = Duration::from_secs(1);

const BLOCK_SIZE: u32 = 4096;
const ROOT_INODE: u64 = 1;

// ── Inode table ──────────────────────────────────────────────────────────────

/// Bidirectional map between FUSE inodes and S3 keys.
///
/// `ROOT_INODE` (1) is permanently mapped to the empty key (the bucket root).
/// All other inodes are allocated lazily by `intern()`.
///
/// `forget()` is called from `Filesystem::forget` when the kernel evicts an
/// entry — we drop the mapping to bound memory.
pub struct InodeTable {
    next: AtomicU64,
    by_inode: RwLock<HashMap<u64, String>>,
    by_key: RwLock<HashMap<String, u64>>,
}

impl InodeTable {
    pub fn new() -> Self {
        let mut by_inode = HashMap::new();
        let mut by_key = HashMap::new();
        by_inode.insert(ROOT_INODE, String::new());
        by_key.insert(String::new(), ROOT_INODE);
        Self {
            // Start after the root.
            next: AtomicU64::new(ROOT_INODE + 1),
            by_inode: RwLock::new(by_inode),
            by_key: RwLock::new(by_key),
        }
    }

    /// Return the inode for `key`, allocating one if absent.
    pub fn intern(&self, key: &str) -> u64 {
        if let Some(&ino) = self.by_key.read().unwrap().get(key) {
            return ino;
        }
        let mut by_key = self.by_key.write().unwrap();
        // Re-check after acquiring the write lock.
        if let Some(&ino) = by_key.get(key) {
            return ino;
        }
        let ino = self.next.fetch_add(1, Ordering::Relaxed);
        by_key.insert(key.to_string(), ino);
        self.by_inode.write().unwrap().insert(ino, key.to_string());
        ino
    }

    pub fn lookup_key(&self, ino: u64) -> Option<String> {
        self.by_inode.read().unwrap().get(&ino).cloned()
    }

    pub fn forget(&self, ino: u64) {
        if ino == ROOT_INODE {
            return;
        }
        if let Some(key) = self.by_inode.write().unwrap().remove(&ino) {
            self.by_key.write().unwrap().remove(&key);
        }
    }
}

// ── Cached metadata ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct CachedMeta {
    is_dir: bool,
    size: u64,
    mtime_filetime: u64,
}

#[derive(Clone)]
struct CachedList {
    dirs: Vec<String>,
    files: Vec<(String, CachedMeta)>,
}

// ── Filesystem context ───────────────────────────────────────────────────────

pub struct S3FuseFs {
    rt: Arc<Runtime>,
    provider: Arc<dyn CloudProvider>,
    drive_id: i64,
    volume_label: String,

    inodes: Arc<InodeTable>,

    list_cache: Mutex<HashMap<String, (Instant, CachedList)>>,
    meta_cache: Mutex<HashMap<String, (Instant, Option<CachedMeta>)>>,

    /// Persistent directory-listing cache, reused unchanged from the
    /// platform-agnostic `dir_listing_cache` module.
    disk_list_cache: Option<Arc<crate::dir_listing_cache::DirListingCache>>,

    /// Block cache shared with the mount handle. Reused unchanged from the
    /// Windows path. TODO: wire into the `read()` path.
    cache: Option<Arc<DiskCache>>,

    /// `true` while the last provider op succeeded.
    connectivity: Arc<AtomicBool>,

    /// Set when the volume is read-only (config.readonly).
    readonly: bool,
}

impl S3FuseFs {
    /// Resolve `(parent_inode, name)` to an S3 key.
    fn resolve_child_key(&self, parent: u64, name: &OsStr) -> Option<String> {
        let parent_key = self.inodes.lookup_key(parent)?;
        let name_str = name.to_str()?;
        Some(if parent_key.is_empty() {
            name_str.to_string()
        } else {
            format!("{parent_key}/{name_str}")
        })
    }

    /// Resolve a single object's metadata, consulting the meta cache then
    /// the provider.
    fn stat_key(&self, key: &str) -> Option<CachedMeta> {
        // Short positive/negative cache.
        if let Some((at, entry)) = self.meta_cache.lock().unwrap().get(key) {
            if at.elapsed() < META_TTL {
                return entry.clone();
            }
        }

        let provider = self.provider.clone();
        let key_owned = key.to_string();
        let res = self.rt.block_on(async move { provider.stat(&key_owned).await });

        let now = Instant::now();
        let mapped: Option<CachedMeta> = match res {
            Ok(Some(s)) => Some(CachedMeta {
                is_dir: false,
                size: s.size,
                mtime_filetime: s.mtime_filetime,
            }),
            Ok(None) => None,
            Err(e) => {
                tracing::error!(target: "nanocrew::fuse", "stat {key:?}: {e}");
                self.connectivity.store(false, Ordering::Relaxed);
                return None;
            }
        };
        self.connectivity.store(true, Ordering::Relaxed);
        self.meta_cache
            .lock()
            .unwrap()
            .insert(key.to_string(), (now, mapped.clone()));
        mapped
    }

    fn list_dir_cached(&self, prefix: &str) -> Option<CachedList> {
        if let Some((at, cached)) = self.list_cache.lock().unwrap().get(prefix) {
            if at.elapsed() < LIST_TTL {
                return Some(cached.clone());
            }
        }
        let provider = self.provider.clone();
        let prefix_owned = prefix.to_string();
        let res: Result<ListDirResult, _> = self
            .rt
            .block_on(async move { provider.list_dir(&prefix_owned).await });
        let result = match res {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(target: "nanocrew::fuse", "list_dir {prefix:?}: {e}");
                self.connectivity.store(false, Ordering::Relaxed);
                return None;
            }
        };
        self.connectivity.store(true, Ordering::Relaxed);

        let listing = CachedList {
            dirs: result.dirs,
            files: result
                .files
                .into_iter()
                .map(|(n, s)| {
                    (
                        n,
                        CachedMeta {
                            is_dir: false,
                            size: s.size,
                            mtime_filetime: s.mtime_filetime,
                        },
                    )
                })
                .collect(),
        };
        self.list_cache
            .lock()
            .unwrap()
            .insert(prefix.to_string(), (Instant::now(), listing.clone()));
        Some(listing)
    }

    /// Build a `FileAttr` from a cached meta + inode.
    fn attr_for(&self, ino: u64, meta: &CachedMeta) -> FileAttr {
        let mtime = filetime_to_systemtime(meta.mtime_filetime);
        FileAttr {
            ino,
            size: meta.size,
            blocks: (meta.size + 511) / 512,
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: mtime,
            kind: if meta.is_dir { FileType::Directory } else { FileType::RegularFile },
            // Permissive — Keychain / Finder ACLs aren't enforced by us.
            perm: if meta.is_dir { 0o755 } else { 0o644 },
            nlink: if meta.is_dir { 2 } else { 1 },
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: BLOCK_SIZE,
            flags: 0,
        }
    }

    fn root_attr(&self) -> FileAttr {
        let now = SystemTime::now();
        FileAttr {
            ino: ROOT_INODE,
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: BLOCK_SIZE,
            flags: 0,
        }
    }
}

// ── Filesystem trait impl ────────────────────────────────────────────────────

impl Filesystem for S3FuseFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let Some(child_key) = self.resolve_child_key(parent, name) else {
            reply.error(ENOENT);
            return;
        };

        // Try the parent's cached listing first — if we've enumerated the
        // parent recently, both a file hit and a directory hit are answered
        // without hitting the network.
        let parent_key = self.inodes.lookup_key(parent).unwrap_or_default();
        if let Some(listing) = self.list_cache.lock().unwrap().get(&parent_key).cloned() {
            let name_str = name.to_str().unwrap_or_default();
            if listing.1.dirs.iter().any(|d| d == name_str) {
                let ino = self.inodes.intern(&child_key);
                let meta = CachedMeta { is_dir: true, size: 0, mtime_filetime: 0 };
                let attr = self.attr_for(ino, &meta);
                reply.entry(&ENTRY_TTL, &attr, 0);
                return;
            }
            if let Some((_, file_meta)) =
                listing.1.files.iter().find(|(n, _)| n == name_str)
            {
                let ino = self.inodes.intern(&child_key);
                let attr = self.attr_for(ino, file_meta);
                reply.entry(&ENTRY_TTL, &attr, 0);
                return;
            }
        }

        // Fallback: HEAD-equivalent stat on the provider.
        match self.stat_key(&child_key) {
            Some(meta) => {
                let ino = self.inodes.intern(&child_key);
                let attr = self.attr_for(ino, &meta);
                reply.entry(&ENTRY_TTL, &attr, 0);
            }
            None => {
                // Could still be a virtual directory — try listing it.
                if self
                    .list_dir_cached(&child_key)
                    .map(|l| !l.dirs.is_empty() || !l.files.is_empty())
                    .unwrap_or(false)
                {
                    let ino = self.inodes.intern(&child_key);
                    let meta = CachedMeta { is_dir: true, size: 0, mtime_filetime: 0 };
                    let attr = self.attr_for(ino, &meta);
                    reply.entry(&ENTRY_TTL, &attr, 0);
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        if ino == ROOT_INODE {
            reply.attr(&ATTR_TTL, &self.root_attr());
            return;
        }
        let Some(key) = self.inodes.lookup_key(ino) else {
            reply.error(ENOENT);
            return;
        };
        // Prefer the most recently seen cached meta. If we have nothing,
        // fall back to a stat() call.
        if let Some((at, Some(meta))) = self.meta_cache.lock().unwrap().get(&key).cloned() {
            if at.elapsed() < META_TTL {
                reply.attr(&ATTR_TTL, &self.attr_for(ino, &meta));
                return;
            }
        }
        match self.stat_key(&key) {
            Some(meta) => reply.attr(&ATTR_TTL, &self.attr_for(ino, &meta)),
            None => {
                // Might be a virtual dir.
                if self.list_dir_cached(&key).is_some() {
                    let meta = CachedMeta { is_dir: true, size: 0, mtime_filetime: 0 };
                    reply.attr(&ATTR_TTL, &self.attr_for(ino, &meta));
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let Some(key) = self.inodes.lookup_key(ino) else {
            reply.error(ENOENT);
            return;
        };

        let Some(listing) = self.list_dir_cached(&key) else {
            reply.error(EIO);
            return;
        };

        // Standard . / .. entries.
        let mut entries: Vec<(u64, FileType, String)> = vec![
            (ino, FileType::Directory, ".".to_string()),
            (ino, FileType::Directory, "..".to_string()),
        ];

        for d in &listing.dirs {
            let child_key = if key.is_empty() {
                d.clone()
            } else {
                format!("{key}/{d}")
            };
            let child_ino = self.inodes.intern(&child_key);
            entries.push((child_ino, FileType::Directory, d.clone()));
        }
        for (name, _meta) in &listing.files {
            let child_key = if key.is_empty() {
                name.clone()
            } else {
                format!("{key}/{name}")
            };
            let child_ino = self.inodes.intern(&child_key);
            entries.push((child_ino, FileType::RegularFile, name.clone()));
        }

        for (i, (child_ino, kind, name)) in entries.into_iter().enumerate().skip(offset as usize)
        {
            // reply.add returns true if the buffer is full.
            if reply.add(child_ino, (i + 1) as i64, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let Some(key) = self.inodes.lookup_key(ino) else {
            reply.error(ENOENT);
            return;
        };
        if key.is_empty() {
            reply.error(EIO);
            return;
        }

        // TODO: wire into `self.cache` (DiskCache) the same way
        // `winfsp_vfs::read` does. For the scaffold, range-GET straight
        // from the provider.
        let provider = self.provider.clone();
        let key_owned = key.clone();
        let res = self.rt.block_on(async move {
            provider
                .get_range(&key_owned, offset as u64, size as u64)
                .await
        });
        match res {
            Ok(bytes) => reply.data(&bytes),
            Err(e) => {
                tracing::error!(target: "nanocrew::fuse", "read {key:?}: {e}");
                self.connectivity.store(false, Ordering::Relaxed);
                reply.error(EIO);
            }
        }
    }

    fn forget(&mut self, _req: &Request, ino: u64, _nlookup: u64) {
        self.inodes.forget(ino);
    }

    // ── Write path: returns EROFS in the scaffold ────────────────────────────
    //
    // TODO(v0.3.0): port the streaming multipart machinery from
    // `winfsp_vfs::WriteState`. The shape will be the same: open creates a
    // temp file, write() spools to it and dispatches 16 MiB parts to
    // upload_part with a bounded semaphore, release() drains the pipeline
    // and calls complete_multipart. For now everything is read-only.

    fn create(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        reply.error(EROFS);
    }

    fn write(
        &mut self,
        _req: &Request,
        _ino: u64,
        _fh: u64,
        _offset: i64,
        _data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        reply.error(EROFS);
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        reply.error(EROFS);
    }

    fn unlink(&mut self, _req: &Request, _parent: u64, _name: &OsStr, reply: ReplyEmpty) {
        reply.error(EROFS);
    }

    fn rmdir(&mut self, _req: &Request, _parent: u64, _name: &OsStr, reply: ReplyEmpty) {
        reply.error(EROFS);
    }

    fn rename(
        &mut self,
        _req: &Request,
        _parent: u64,
        _name: &OsStr,
        _newparent: u64,
        _newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        reply.error(EROFS);
    }
}

// ── Mount lifecycle ──────────────────────────────────────────────────────────

/// macOS implementation of `mounts::spawn_mount`. Invoked from `mounts.rs`'s
/// cfg-gated `spawn_mount` wrapper.
pub fn spawn_mount(
    config: MountConfig,
    app_handle: AppHandle,
) -> Result<MountHandle, AppError> {
    // 1. Build the tokio runtime owned by the filesystem.
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(
                std::thread::available_parallelism()
                    .map(|n| n.get().max(4))
                    .unwrap_or(4),
            )
            .enable_all()
            .build()
            .map_err(|e| AppError::Mount(format!("tokio init: {e}")))?,
    );

    // 2. Build the S3 client + provider. Mirrors the Windows path.
    let creds = aws_credential_types::Credentials::new(
        &config.access_key_id,
        &config.secret_access_key,
        None,
        None,
        "nanocrew-sync",
    );
    let http = {
        let state: tauri::State<AppState> = app_handle.state();
        http_client::build_from_prefs(&state.db)
            .map_err(|e| AppError::Mount(format!("http_client build: {e}")))?
    };
    let retry_config = aws_config::retry::RetryConfig::adaptive().with_max_attempts(8);
    let aws_cfg = rt.block_on(async {
        aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(config.region.clone()))
            .endpoint_url(format!("https://{}", config.endpoint))
            .credentials_provider(creds)
            .retry_config(retry_config)
            .http_client(http)
            .load()
            .await
    });
    let needs_path_style = matches!(
        config.provider.to_lowercase().as_str(),
        "minio" | "cloudflare" | "r2" | "backblaze" | "other"
    );
    let s3_conf = aws_sdk_s3::config::Builder::from(&aws_cfg)
        .force_path_style(needs_path_style)
        .build();
    let client = aws_sdk_s3::Client::from_conf(s3_conf);

    let provider: Arc<dyn CloudProvider> = Arc::new(S3Provider::new(
        client.clone(),
        config.bucket.clone(),
        config.bucket_prefix.clone(),
    ));

    // 3. Build the disk cache. Reused as-is from the Windows path.
    let cache: Option<Arc<DiskCache>> =
        if config.cache_enabled && config.cache_max_bytes > 0 {
            let root = config
                .cache_root
                .join(format!("drive-{}", config.drive_id));
            match DiskCache::new(
                config.drive_id,
                root,
                &config.db_path,
                config.cache_max_bytes,
                true,
            ) {
                Ok(c) => {
                    c.start_eviction();
                    Some(c)
                }
                Err(e) => {
                    tracing::warn!(target: "nanocrew::cache",
                        drive_id = config.drive_id, "cache init failed: {e}");
                    None
                }
            }
        } else {
            None
        };

    let disk_list_cache: Option<Arc<crate::dir_listing_cache::DirListingCache>> =
        if config.cache_enabled && config.cache_max_bytes > 0 {
            Some(Arc::new(crate::dir_listing_cache::DirListingCache::new(
                config
                    .cache_root
                    .join(format!("drive-{}", config.drive_id))
                    .join("dir-listings"),
            )))
        } else {
            None
        };

    let connectivity = Arc::new(AtomicBool::new(true));

    // 4. Build the filesystem context.
    let fs = S3FuseFs {
        rt: rt.clone(),
        provider,
        drive_id: config.drive_id,
        volume_label: format!("NanoCrew-{}", config.bucket),
        inodes: Arc::new(InodeTable::new()),
        list_cache: Mutex::new(HashMap::new()),
        meta_cache: Mutex::new(HashMap::new()),
        disk_list_cache,
        cache: cache.clone(),
        connectivity: connectivity.clone(),
        readonly: config.readonly,
    };

    // 5. Mount point. `MountConfig.letter` is reused on macOS as the volume
    //    slug. We mount under `/Volumes/NanoCrew-<bucket>`.
    let mount_slug = sanitize_slug(&config.bucket);
    let mount_point = PathBuf::from(format!("/Volumes/NanoCrew-{mount_slug}"));

    // Ensure the mount-point directory exists. macOS requires the directory
    // to exist before mount; the unmount path removes it.
    if let Err(e) = std::fs::create_dir_all(&mount_point) {
        // EEXIST is fine.
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(AppError::Mount(format!(
                "create mount point {}: {e}",
                mount_point.display()
            )));
        }
    }

    // 6. Mount options. `local` makes macOS treat us as a local volume
    //    (visible in Finder sidebar). `noappledouble` / `noapplexattr`
    //    suppress the resource-fork churn that Finder otherwise generates.
    let volname = format!("NanoCrew-{mount_slug}");
    let fsname = format!("nanocrew-{}", config.drive_id);
    let mut options = vec![
        MountOption::FSName(fsname),
        MountOption::CUSTOM(format!("volname={volname}")),
        MountOption::CUSTOM("local".to_string()),
        MountOption::CUSTOM("noappledouble".to_string()),
        MountOption::CUSTOM("noapplexattr".to_string()),
        MountOption::DefaultPermissions,
    ];
    if config.readonly {
        options.push(MountOption::RO);
    }

    // 7. Spawn the background session. fuser owns the worker pool; we hold
    //    the `BackgroundSession` for the lifetime of the mount and drop it
    //    on stop to unmount.
    let session = fuser::spawn_mount2(fs, &mount_point, &options)
        .map_err(|e| AppError::Mount(format!("fuser spawn_mount2: {e}")))?;

    // Notify the UI that we're mounted.
    let _ = app_handle.emit(
        "drive_status_changed",
        DriveStatusPayload {
            drive_id: config.drive_id,
            status: "mounted".into(),
            message: None,
        },
    );

    // 8. We need to bridge fuser's `BackgroundSession` into the existing
    //    `MountHandle` shape (stop_tx + JoinHandle). We spawn a tiny holder
    //    thread that owns the session and unmounts on stop.
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let app_handle_clone = app_handle.clone();
    let drive_id = config.drive_id;
    let mount_point_clone = mount_point.clone();
    let thread = std::thread::Builder::new()
        .name(format!("fuse-t-{}", config.drive_id))
        .spawn(move || {
            // Hold the session here. Dropping it triggers fuser's unmount.
            let _session_holder: BackgroundSession = session;
            let _ = stop_rx.blocking_recv();
            // Best-effort: remove the mount-point directory we created.
            // (BackgroundSession::drop unmounts; the empty dir lingers.)
            let _ = std::fs::remove_dir(&mount_point_clone);

            let _ = app_handle_clone.emit(
                "drive_status_changed",
                DriveStatusPayload {
                    drive_id,
                    status: "offline".into(),
                    message: None,
                },
            );
        })
        .map_err(|e| AppError::Mount(format!("spawn fuse-t holder thread: {e}")))?;

    Ok(MountHandle {
        drive_id: config.drive_id,
        letter: config.letter,
        stop_tx,
        thread: Some(thread),
        cache,
        connectivity,
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Sanitize a bucket name into a string that's safe for `/Volumes/<name>`.
/// Replaces characters that confuse Finder (slashes, control chars) with `-`.
fn sanitize_slug(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '-',
        })
        .collect()
}

/// Convert a Windows FILETIME (100-ns ticks since 1601-01-01) into a
/// `SystemTime`. 0 maps to UNIX_EPOCH so FUSE doesn't reject the attr.
fn filetime_to_systemtime(ft: u64) -> SystemTime {
    if ft == 0 {
        return UNIX_EPOCH;
    }
    // FILETIME epoch is 1601-01-01; Unix epoch is 1970-01-01.
    // Delta = 11_644_473_600 seconds.
    let secs_since_1601 = ft / 10_000_000;
    let secs_since_1970 = secs_since_1601.saturating_sub(11_644_473_600);
    UNIX_EPOCH + Duration::from_secs(secs_since_1970)
}

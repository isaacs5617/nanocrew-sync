//! Mount lifecycle for a cloud-storage-backed drive.
//!
//! On **Windows** each mount boots a [`winfsp::host::FileSystemHost`] which:
//!   1. Creates the virtual volume (user-mode WinFsp driver)
//!   2. Mounts it directly at a Windows drive letter (no `subst`)
//!   3. Dispatches IO to our [`crate::winfsp_vfs::S3Fs`] implementation
//!
//! On **macOS** each mount boots a [`fuser`] session backed by FUSE-T (the
//! kernel-extension-free userspace FUSE for macOS — see
//! `crate::fuse_t_vfs`). Mounts land at `/Volumes/NanoCrew-<bucket>`.
//!
//! Teardown is symmetric: stop the dispatcher, unmount, drop the host.
//!
//! `MountConfig` and `MountHandle` are platform-agnostic; the heavy lifting
//! is in the `#[cfg]`-gated `spawn_mount` impls below.

use crate::{cache::DiskCache, error::AppError};

// ── Types (platform-agnostic) ────────────────────────────────────────────────

/// All the S3 / mount parameters the host needs.
#[allow(dead_code)]
pub struct MountConfig {
    pub drive_id: i64,
    pub letter: String,
    pub provider: String,
    pub endpoint: String,
    pub bucket: String,
    /// Normalised subdirectory prefix (empty = root; non-empty always has
    /// trailing slash). Restricts the WinFsp volume to a bucket subdirectory.
    pub bucket_prefix: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub readonly: bool,
    /// Human-readable owner recorded in cross-device sentinel locks
    /// (`.nanocrew/locks/…`). Typically the signed-in username; falls back to
    /// a generic tag for auto-mount-at-startup when no user is signed in yet.
    pub owner: String,
    /// Global upload/download caps in bytes-per-second. `None` = unlimited.
    /// Read from the `upload_rate_bps` / `download_rate_bps` prefs keys at
    /// the two MountConfig build sites (manual mount + auto_mount_drives).
    pub upload_rate_bps: Option<u64>,
    pub download_rate_bps: Option<u64>,

    /// On-disk cache parameters (Phase 5.6). `cache_enabled=false` disables
    /// the block cache entirely — `get_range` becomes a pass-through.
    pub cache_enabled: bool,
    /// Cache size cap in bytes. Derived from `drives.cache_size_gb`.
    pub cache_max_bytes: u64,
    /// Absolute path to the app SQLite file — the cache opens its own
    /// connection so it never fights the main app's Mutex.
    pub db_path: std::path::PathBuf,
    /// Cache root directory (Phase 7.4). Resolved from the `cache_root` pref
    /// (or `%LOCALAPPDATA%\NanoCrew\Sync\cache` as default) at both
    /// MountConfig build sites. Per-drive data lands at
    /// `<cache_root>/drive-<id>/`.
    pub cache_root: std::path::PathBuf,
}

/// Shared handles into the live VFS so commands running on the main app
/// thread (e.g. `refresh_dir_listing`) can invalidate caches without
/// needing access to the WinFsp-owned `S3Fs` instance.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub struct RefreshHandle {
    pub list_cache: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<
                String,
                (std::time::Instant, crate::winfsp_vfs::CachedList),
            >,
        >,
    >,
    pub meta_cache: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<
                String,
                (std::time::Instant, Option<(String, crate::winfsp_vfs::Meta)>),
            >,
        >,
    >,
    pub disk_list_cache:
        Option<std::sync::Arc<crate::dir_listing_cache::DirListingCache>>,
}

#[cfg(target_os = "windows")]
impl RefreshHandle {
    /// Same invalidation semantics as `S3Fs::refresh_dir` — drop the
    /// in-memory listing for `prefix`, every meta-cache entry beneath it,
    /// and the on-disk JSON.
    pub fn refresh(&self, prefix: &str) {
        self.list_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(prefix);

        let mut mc = self.meta_cache.lock().unwrap_or_else(|p| p.into_inner());
        if prefix.is_empty() {
            mc.clear();
        } else {
            let lc_prefix = format!("{}/", prefix.to_ascii_lowercase());
            let lc_self = prefix.to_ascii_lowercase();
            mc.retain(|k, _| !(k.starts_with(&lc_prefix) || k == &lc_self));
        }
        drop(mc);

        if let Some(disk) = &self.disk_list_cache {
            disk.invalidate(prefix);
        }
    }
}

/// A live mounted drive. Dropping `stop_tx` unblocks the host thread.
#[allow(dead_code)]
pub struct MountHandle {
    pub drive_id: i64,
    pub letter: String,
    pub stop_tx: tokio::sync::oneshot::Sender<()>,
    pub thread: Option<std::thread::JoinHandle<()>>,
    /// Shared reference to the disk cache so external commands (pin/unpin)
    /// can poke it while the mount is live. `None` when caching is disabled
    /// for this drive.
    pub cache: Option<std::sync::Arc<DiskCache>>,
    /// Shared connectivity flag from the VFS layer. `true` = last S3 op succeeded.
    pub connectivity: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Windows-only handles into the VFS caches so commands like
    /// `refresh_dir_listing` can invalidate the in-memory + on-disk listing
    /// caches from outside the WinFsp thread.
    #[cfg(target_os = "windows")]
    pub refresh: Option<RefreshHandle>,
}

impl MountHandle {
    pub fn stop(mut self) {
        // Stop the sweeper thread before tearing down the mount so we don't
        // leave it running against a half-demolished context.
        if let Some(c) = &self.cache {
            c.stop();
        }
        let _ = self.stop_tx.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ── Windows: WinFsp-backed mount ─────────────────────────────────────────────

#[cfg(target_os = "windows")]
use std::{
    sync::{mpsc, OnceLock},
    time::Duration,
};
#[cfg(target_os = "windows")]
use tauri::{Emitter, Manager};
#[cfg(target_os = "windows")]
use winfsp::{
    host::{FileSystemHost, VolumeParams},
    FspInit,
};
#[cfg(target_os = "windows")]
use crate::{
    http_client,
    providers::{s3::S3Provider, CloudProvider},
    state::AppState,
    types::{DriveStatusPayload, FileLockEvent, TransferPayload},
    winfsp_vfs::S3Fs,
};

/// WinFsp must be initialised exactly once per process. `winfsp_init` loads the
/// DLL lazily (we delay-link it in `build.rs`) and returns an `FspInit` token.
/// Cached so subsequent mounts are free.
#[cfg(target_os = "windows")]
static WINFSP_INIT: OnceLock<Result<FspInit, String>> = OnceLock::new();

#[cfg(target_os = "windows")]
fn ensure_winfsp() -> Result<(), String> {
    let res = WINFSP_INIT.get_or_init(|| {
        winfsp::winfsp_init().map_err(|e| format!("winfsp_init: {e:?}"))
    });
    match res {
        Ok(_) => Ok(()),
        Err(e) => Err(e.clone()),
    }
}

/// Boot a WinFsp-backed S3 volume, mount it at the target drive letter, and
/// block until the dispatcher is ready. Returns a handle whose `stop()`
/// unmounts cleanly.
#[cfg(target_os = "windows")]
pub fn spawn_mount(
    config: MountConfig,
    app_handle: tauri::AppHandle,
) -> Result<MountHandle, AppError> {
    ensure_winfsp().map_err(AppError::Mount)?;

    let (init_tx, init_rx) = mpsc::channel::<Result<Option<RefreshHandle>, String>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    let drive_id = config.drive_id;
    let letter = config.letter.clone();

    // Build the disk cache up-front so we can (a) hand a clone into the
    // WinFsp thread for S3Fs, and (b) return another clone on the
    // MountHandle for pin/unpin commands to use while the drive is live.
    let cache: Option<std::sync::Arc<DiskCache>> = if config.cache_enabled
        && config.cache_max_bytes > 0
    {
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
    let cache_for_thread = cache.clone();
    let connectivity = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let connectivity_for_thread = std::sync::Arc::clone(&connectivity);

    // Persistent dir-listing cache root. Tied to the same enable/quota flag
    // as the block cache so the two are managed together — disabling the
    // cache also stops writing dir-listing JSON to disk.
    let disk_list_cache_dir: Option<std::path::PathBuf> =
        if config.cache_enabled && config.cache_max_bytes > 0 {
            Some(
                config
                    .cache_root
                    .join(format!("drive-{}", config.drive_id))
                    .join("dir-listings"),
            )
        } else {
            None
        };

    let thread = std::thread::Builder::new()
        .name(format!("winfsp-{}", config.letter))
        .spawn(move || {
            // 1. Build the long-lived multi-thread tokio runtime that S3Fs will
            //    own for the lifetime of the mount. Use it once here to load
            //    the AWS config, then hand it to S3Fs — no short-lived
            //    bootstrap runtime, no drop-on-block.
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(
                    std::thread::available_parallelism()
                        .map(|n| n.get().max(4))
                        .unwrap_or(4),
                )
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = init_tx.send(Err(format!("tokio init: {e}")));
                    return;
                }
            };

            let creds = aws_credential_types::Credentials::new(
                &config.access_key_id,
                &config.secret_access_key,
                None,
                None,
                "nanocrew-sync",
            );

            // Build the shared HTTP client (rustls + optional proxy + optional
            // extra CA) from the prefs DB. We do this inside the thread so we
            // always pick up the latest saved values.
            let http = {
                let state: tauri::State<AppState> = app_handle.state();
                match http_client::build_from_prefs(&state.db) {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = init_tx.send(Err(format!("http_client build: {e}")));
                        return;
                    }
                }
            };
            // Retry plumbing (Phase 5.7): the AWS SDK's default retry mode is
            // "Standard" with 3 attempts. We bump to 8 attempts with adaptive
            // backoff so a transient network blip (Wi-Fi handoff, DNS hiccup,
            // brief provider throttling) retries quietly rather than bubbling
            // up as an Explorer "copy failed" dialog mid-upload.
            let retry_config = aws_config::retry::RetryConfig::adaptive()
                .with_max_attempts(8);
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
            // Force path-style only for providers whose endpoints don't support
            // virtual-hosted-style bucket URLs. Wasabi and AWS use virtual-hosted
            // style (bucket.endpoint) and can fail with force_path_style when the
            // SDK constructs the URL against a custom endpoint_url. MinIO,
            // Cloudflare R2, and generic "other" endpoints need path-style.
            let needs_path_style = matches!(
                config.provider.to_lowercase().as_str(),
                "minio" | "cloudflare" | "r2" | "backblaze" | "other"
            );
            let s3_conf = aws_sdk_s3::config::Builder::from(&aws_cfg)
                .force_path_style(needs_path_style)
                .build();
            let client = aws_sdk_s3::Client::from_conf(s3_conf);

            // 2. Build the filesystem context. The runtime we just used moves
            //    into S3Fs and stays alive for every subsequent IO call.
            let provider: std::sync::Arc<dyn CloudProvider> = std::sync::Arc::new(S3Provider::new(
                client.clone(),
                config.bucket.clone(),
                config.bucket_prefix.clone(),
            ));
            let emit_app = app_handle.clone();
            let emit_app_lock = app_handle.clone();
            let emit_app_status = app_handle.clone();
            let emit_app_refresh = app_handle.clone();
            let status_drive_id = config.drive_id;
            let refresh_drive_id = config.drive_id;
            let label = format!("NanoCrew-{}", config.bucket);
            let ctx = match S3Fs::new(
                rt,
                provider,
                client,
                config.bucket.clone(),
                config.bucket_prefix.clone(),
                config.drive_id,
                label.clone(),
                Box::new(move |p: TransferPayload| {
                    let _ = emit_app.emit("transfer_progress", p);
                }),
                Box::new(move |p: FileLockEvent| {
                    let _ = emit_app_lock.emit("file_lock_event", p);
                }),
                config.owner.clone(),
                config.upload_rate_bps,
                config.download_rate_bps,
                cache_for_thread,
                Box::new(move |online: bool| {
                    let _ = emit_app_status.emit(
                        "drive_status_changed",
                        crate::types::DriveStatusPayload {
                            drive_id: status_drive_id,
                            status: if online { "online".into() } else { "offline".into() },
                            message: None,
                        },
                    );
                }),
                connectivity_for_thread,
                disk_list_cache_dir,
                std::sync::Arc::new(move |prefix: String| {
                    let _ = emit_app_refresh.emit(
                        "dir_listing_refreshed",
                        crate::types::DirListingRefreshedPayload {
                            drive_id: refresh_drive_id,
                            prefix,
                        },
                    );
                }),
            ) {
                Ok(c) => c,
                Err(e) => {
                    let _ = init_tx.send(Err(format!("S3Fs init: {e}")));
                    return;
                }
            };

            // Extract cache Arcs BEFORE the ctx moves into the host, then
            // kick off the background refresh task on the S3Fs runtime. The
            // RefreshHandle is sent back to the spawning thread via init_tx
            // and stored on the MountHandle for `refresh_dir_listing` to use.
            let refresh_handle = RefreshHandle {
                list_cache: ctx.list_cache_arc(),
                meta_cache: ctx.meta_cache_arc(),
                disk_list_cache: ctx.disk_list_cache_arc(),
            };
            ctx.start_background_refresh();

            // 3. Volume parameters. These are NTFS-ish defaults tuned for an
            //    object-storage-backed volume: case-preserved but not
            //    case-sensitive (Windows apps expect this), Unicode on disk,
            //    4 KiB sectors, 4 KiB clusters.
            // FILETIME for "now" in 100ns units since 1601 — WinFsp rejects
            // volumes with a zero creation time on some Windows builds.
            let now_ft = {
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0) as i64;
                ((secs + 11_644_473_600) as u64) * 10_000_000
            };

            let mut vp = VolumeParams::new();
            vp.sector_size(4096)
                .sectors_per_allocation_unit(1)
                .max_component_length(255)
                .volume_creation_time(now_ft)
                .volume_serial_number(config.drive_id as u32)
                .file_info_timeout(1000)
                .case_preserved_names(true)
                .case_sensitive_search(false)
                .unicode_on_disk(true)
                // Keep ACLs off — we don't persist per-file ACLs. WinFsp will
                // accept our Everyone-FA descriptor as advisory without
                // enforcing access checks against it.
                .persistent_acls(false)
                .post_cleanup_when_modified_only(true)
                .pass_query_directory_pattern(false)
                .flush_and_purge_on_cleanup(false)
                // Without these three, Explorer does a preflight check on large
                // copies and bails with "File is too large for the destination
                // file system" — classic FAT32 dialog. memfs sample sets them.
                .reparse_points(true)
                .post_disposition_only_when_necessary(true)
                .allow_open_in_kernel_mode(true)
                .read_only_volume(config.readonly)
                .filesystem_name("NanoCrewSync");

            // 4. Build the host.
            let mut host = match FileSystemHost::new(vp, ctx) {
                Ok(h) => h,
                Err(e) => {
                    let _ = init_tx.send(Err(format!("FileSystemHost::new: {e:?}")));
                    return;
                }
            };

            // 5. Mount. `mount` takes a string-like value; `"Z:"` is the
            //    canonical form. We normalize the user input defensively.
            let mount_point = normalize_letter(&config.letter);
            if let Err(e) = host.mount(mount_point.clone()) {
                let _ = init_tx.send(Err(format!("mount {mount_point}: {e:?}")));
                return;
            }

            // 6. Start the dispatcher.
            if let Err(e) = host.start() {
                host.unmount();
                let _ = init_tx.send(Err(format!("start dispatcher: {e:?}")));
                return;
            }

            // 7. Ready.
            let _ = init_tx.send(Ok(Some(refresh_handle)));
            let _ = app_handle.emit(
                "drive_status_changed",
                DriveStatusPayload {
                    drive_id: config.drive_id,
                    status: "mounted".into(),
                    message: None,
                },
            );

            // 8. Park until stop.
            let _ = stop_rx.blocking_recv();

            // 9. Teardown. Order matters: stop the dispatcher first so no more
            //    IOs come in, then remove the mount point, then drop the host
            //    (which also drops the S3Fs context and its runtime).
            host.stop();
            host.unmount();
            drop(host);

            let _ = app_handle.emit(
                "drive_status_changed",
                DriveStatusPayload {
                    drive_id: config.drive_id,
                    status: "offline".into(),
                    message: None,
                },
            );
        })
        .map_err(|e| AppError::Mount(e.to_string()))?;

    match init_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(Ok(refresh)) => Ok(MountHandle {
            drive_id,
            letter,
            stop_tx,
            thread: Some(thread),
            cache,
            connectivity,
            refresh,
        }),
        Ok(Err(msg)) => {
            if let Some(c) = &cache {
                c.stop();
            }
            let _ = thread.join();
            Err(AppError::Mount(msg))
        }
        Err(_) => {
            if let Some(c) = &cache {
                c.stop();
            }
            let _ = stop_tx.send(());
            let _ = thread.join();
            Err(AppError::Mount("Mount timed out after 30 s".into()))
        }
    }
}

/// Normalize user input like "Z" / "Z:" / "z:\" to the canonical WinFsp form
/// `"Z:"`.
#[cfg(target_os = "windows")]
fn normalize_letter(raw: &str) -> String {
    let mut s = raw.trim().trim_end_matches('\\').trim_end_matches(':').to_string();
    s.make_ascii_uppercase();
    format!("{s}:")
}

// ── macOS: FUSE-T-backed mount ───────────────────────────────────────────────
//
// On macOS the WinFsp host is replaced with a FUSE-T session. Drive letters
// don't exist; `MountConfig.letter` is reused as a slug under
// `/Volumes/NanoCrew-<letter>`. The fuser crate negotiates with the installed
// FUSE implementation (FUSE-T on macOS) via `mount_fuse-t`, which runs
// entirely in userspace over an NFS loopback — no kext, no SIP override.

#[cfg(target_os = "macos")]
pub fn spawn_mount(
    config: MountConfig,
    app_handle: tauri::AppHandle,
) -> Result<MountHandle, AppError> {
    crate::fuse_t_vfs::spawn_mount(config, app_handle)
}

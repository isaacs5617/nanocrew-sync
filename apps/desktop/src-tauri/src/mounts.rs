//! Mount lifecycle for an S3-backed drive.
//!
//! Each mount boots a [`winfsp::host::FileSystemHost`] which:
//!   1. Creates the virtual volume (user-mode WinFsp driver)
//!   2. Mounts it directly at a Windows drive letter (no `subst`)
//!   3. Dispatches IO to our [`S3Fs`] implementation
//!
//! Teardown: stop the dispatcher, unmount the drive letter, drop the host.
//! Cloud-filter-specific state (sync root registration, placeholder folders,
//! upload watcher, `subst`) is gone — WinFsp owns the volume end-to-end.

use std::{
    sync::{mpsc, OnceLock},
    time::Duration,
};

use tauri::{Emitter, Manager};
use winfsp::{
    host::{FileSystemHost, VolumeParams},
    FspInit,
};

use crate::{
    error::AppError,
    http_client,
    state::AppState,
    types::{DriveStatusPayload, FileLockEvent, TransferPayload},
    winfsp_vfs::S3Fs,
};

// ── Global WinFsp init ───────────────────────────────────────────────────────

/// WinFsp must be initialised exactly once per process. `winfsp_init` loads the
/// DLL lazily (we delay-link it in `build.rs`) and returns an `FspInit` token.
/// Cached so subsequent mounts are free.
static WINFSP_INIT: OnceLock<Result<FspInit, String>> = OnceLock::new();

fn ensure_winfsp() -> Result<(), String> {
    let res = WINFSP_INIT.get_or_init(|| {
        winfsp::winfsp_init().map_err(|e| format!("winfsp_init: {e:?}"))
    });
    match res {
        Ok(_) => Ok(()),
        Err(e) => Err(e.clone()),
    }
}

// ── Types ────────────────────────────────────────────────────────────────────

/// All the S3 / mount parameters the host needs.
#[allow(dead_code)]
pub struct MountConfig {
    pub drive_id: i64,
    pub letter: String,
    pub provider: String,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub readonly: bool,
    /// Human-readable owner recorded in cross-device sentinel locks
    /// (`.nanocrew/locks/…`). Typically the signed-in username; falls back to
    /// a generic tag for auto-mount-at-startup when no user is signed in yet.
    pub owner: String,
}

/// A live mounted drive. Dropping `stop_tx` unblocks the host thread.
#[allow(dead_code)]
pub struct MountHandle {
    pub drive_id: i64,
    pub letter: String,
    pub stop_tx: tokio::sync::oneshot::Sender<()>,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl MountHandle {
    pub fn stop(mut self) {
        let _ = self.stop_tx.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ── spawn_mount ──────────────────────────────────────────────────────────────

/// Boot a WinFsp-backed S3 volume, mount it at the target drive letter, and
/// block until the dispatcher is ready. Returns a handle whose `stop()`
/// unmounts cleanly.
pub fn spawn_mount(
    config: MountConfig,
    app_handle: tauri::AppHandle,
) -> Result<MountHandle, AppError> {
    ensure_winfsp().map_err(AppError::Mount)?;

    let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    let drive_id = config.drive_id;
    let letter = config.letter.clone();

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
            let s3_conf = aws_sdk_s3::config::Builder::from(&aws_cfg)
                .force_path_style(true)
                .build();
            let client = aws_sdk_s3::Client::from_conf(s3_conf);

            // 2. Build the filesystem context. The runtime we just used moves
            //    into S3Fs and stays alive for every subsequent IO call.
            let emit_app = app_handle.clone();
            let emit_app_lock = app_handle.clone();
            let label = format!("NanoCrew-{}", config.bucket);
            let ctx = match S3Fs::new(
                rt,
                client,
                config.bucket.clone(),
                config.drive_id,
                label.clone(),
                Box::new(move |p: TransferPayload| {
                    let _ = emit_app.emit("transfer_progress", p);
                }),
                Box::new(move |p: FileLockEvent| {
                    let _ = emit_app_lock.emit("file_lock_event", p);
                }),
                config.owner.clone(),
            ) {
                Ok(c) => c,
                Err(e) => {
                    let _ = init_tx.send(Err(format!("S3Fs init: {e}")));
                    return;
                }
            };

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
            let _ = init_tx.send(Ok(()));
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
        Ok(Ok(())) => Ok(MountHandle {
            drive_id,
            letter,
            stop_tx,
            thread: Some(thread),
        }),
        Ok(Err(msg)) => {
            let _ = thread.join();
            Err(AppError::Mount(msg))
        }
        Err(_) => {
            let _ = stop_tx.send(());
            let _ = thread.join();
            Err(AppError::Mount("Mount timed out after 30 s".into()))
        }
    }
}

/// Normalize user input like "Z" / "Z:" / "z:\" to the canonical WinFsp form
/// `"Z:"`.
fn normalize_letter(raw: &str) -> String {
    let mut s = raw.trim().trim_end_matches('\\').trim_end_matches(':').to_string();
    s.make_ascii_uppercase();
    format!("{s}:")
}

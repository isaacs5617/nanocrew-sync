#[cfg(not(target_os = "android"))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

mod auth;
mod cache;
mod commands;
#[cfg(target_os = "windows")]
mod credentials;
mod db;
mod dir_listing_cache;
#[cfg(target_os = "windows")]
mod dpapi;
mod error;
mod file_lock;
mod http_client;
mod license;
mod logging;
#[cfg(not(target_os = "android"))]
mod mounts;
mod providers;
mod state;
mod throttle;
mod types;
#[cfg(target_os = "windows")]
mod winfsp_vfs;

// ── macOS-only modules (Track v0.3.0 macOS beta) ────────────────────────────
#[cfg(target_os = "macos")]
mod fuse_t_vfs;
#[cfg(target_os = "macos")]
mod keychain;

// ── Android-only modules (Track v0.4.0 Android beta) ────────────────────────
// JNI bridge that the Kotlin DocumentsProvider calls into. The Rust side
// owns the CloudProvider trait + on-disk cache the same way the WinFsp /
// FUSE-T dispatchers do on desktop. See docs/android-port-design.md.
#[cfg(target_os = "android")]
mod android_provider;
#[cfg(target_os = "android")]
mod jni_helpers;

#[cfg(not(target_os = "android"))]
use state::AppState;
#[cfg(not(target_os = "android"))]
use types::DriveStatusPayload;

// ── Android entry point ─────────────────────────────────────────────────────
// On Android there's no tray, no drive letter, no auto-mount loop. The OS
// drives IO through the Kotlin DocumentsProvider → JNI; the Tauri webview is
// only here for drive management UI and license activation.
#[cfg(target_os = "android")]
#[tauri::mobile_entry_point]
pub fn run() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("nanocrew"),
    );
    tracing::info!(target: "nanocrew", "android mobile_entry_point: starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|_app| {
            // TODO: DB bootstrap, license check, mobile AppState wiring.
            // `app.path().app_data_dir()` resolves to the app's private
            // /data/data/dev.nanocrew.sync/files dir on Android, which is
            // fine for the SQLite file; the auto_mount loop is meaningless
            // (no drive letters / mountpoints exist on Android).
            Ok(())
        })
        // TODO: cfg-trimmed mobile invoke_handler. The desktop one references
        // several Windows-only commands (drive letter helpers, WinFsp check,
        // autostart). For the scaffold we expose nothing — UI surfaces an
        // "Android build, drive management TBD" placeholder until the mobile
        // command surface lands.
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error building nanocrew sync (android)");
}

#[cfg(not(target_os = "android"))]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _sentry = sentry::init((
        std::env::var("SENTRY_DSN").unwrap_or_default(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    ));
    sentry::configure_scope(|scope| {
        let machine_id = license::machine_fingerprint();
        scope.set_user(Some(sentry::User {
            id: Some(machine_id),
            ..Default::default()
        }));
    });

    tauri::Builder::default()
        // Enforce single-instance: a second launch refocuses the existing
        // window (and un-hides it from tray) instead of spawning a twin
        // taskbar icon / duplicate WinFsp mount attempt.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            let db_path = app
                .path()
                .app_data_dir()
                .expect("cannot resolve app data dir")
                .join("nanocrew.db");

            let conn = db::open(&db_path).expect("failed to open database");
            let state = AppState::new(conn);

            // ── Logging ───────────────────────────────────────────────────────
            // Initialize tracing BEFORE we manage state so the subscriber is
            // active for auto_mount_drives and any subsequent events. Read
            // `verbose_logging` directly from the fresh connection — prefs
            // module not yet reachable via State<AppState> because we haven't
            // called app.manage() yet.
            let verbose = {
                let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
                db.query_row(
                    "SELECT value FROM prefs WHERE key = 'verbose_logging'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .ok()
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false)
            };
            let log_dir = db_path.parent().map(|p| p.join("logs"))
                .unwrap_or_else(|| std::path::PathBuf::from("logs"));
            if let Some(guard) = logging::init(&log_dir, verbose) {
                state.attach_log_guard(guard);
            }
            tracing::info!(target: "nanocrew", "startup: log dir = {}", log_dir.display());

            // Disable Sentry if the user has opted out of telemetry.
            {
                let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
                let telemetry_on = db.query_row(
                    "SELECT value FROM prefs WHERE key = 'telemetry_enabled'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .ok()
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true);
                if !telemetry_on {
                    sentry::Hub::current().client().map(|c| c.close(None));
                }
            }

            app.manage(state);

            // ── System tray ───────────────────────────────────────────────────
            let show = MenuItem::with_id(app, "show", "Show NanoCrew Sync", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("NanoCrew Sync")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            #[cfg(debug_assertions)]
            {
                let window = app.get_webview_window("main").unwrap();
                window.open_devtools();
            }

            // ── Start minimized to tray ──────────────────────────────────────
            // Two ways to boot hidden: the `--hidden` argv flag (set by the
            // autostart registry value we write in commands::system), or the
            // "start_minimized" preference toggled from Settings → General.
            // Either one hides the window before the user sees it flash.
            {
                let started_hidden_arg = std::env::args().any(|a| a == "--hidden");
                let state: tauri::State<AppState> = app.state();
                let pref_hidden = commands::prefs::get_bool(&state.db, "start_minimized", false);
                if started_hidden_arg || pref_hidden {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.hide();
                    }
                }
            }

            // Sync drive display names to bucket/folder before mounting, so
            // the UI label matches what users actually see in the S3 path.
            sync_drive_names_to_folder(app.handle());

            // Kick off auto-mounts asynchronously so setup() returns immediately
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                auto_mount_drives(handle).await;
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Close button hides to tray instead of quitting
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::has_account,
            commands::auth::create_admin,
            commands::auth::sign_in,
            commands::auth::sign_out,
            commands::auth::get_account,
            commands::auth::change_password,
            commands::auth::verify_password,
            commands::auth::record_lock_event,
            commands::auth::clear_cache,
            commands::drives::list_drives,
            commands::drives::add_drive,
            commands::drives::remove_drive,
            commands::drives::mount_drive,
            commands::drives::unmount_drive,
            commands::drives::test_connection,
            commands::drives::get_available_letters,
            commands::drives::list_drive_objects,
            commands::drives::list_buckets,
            commands::drives::set_drive_prefix,
            commands::drives::set_drive_credentials,
            commands::drives::create_folder,
            commands::drives::rename_object,
            commands::drives::refresh_dir_listing,
            commands::drives::open_path,
            commands::drives::check_winfsp,
            commands::drives::get_drive_cache_stats,
            commands::drives::set_drive_cache_quota,
            commands::drives::clear_drive_cache,
            commands::drives::set_drive_cache_enabled,
            commands::drives::get_drive_connectivity,
            commands::drives::get_drive_offline_coverage,
            commands::drives::prefetch_pinned,
            commands::drives::set_drive_bandwidth,
            commands::drives::test_sftp_connection,
            commands::drives::add_sftp_drive,
            commands::drives::test_ftp_connection,
            commands::drives::add_ftp_drive,
            commands::drives::test_webdav_connection,
            commands::drives::add_webdav_drive,
            commands::drives::start_gdrive_auth,
            commands::drives::add_gdrive_drive,
            commands::drives::start_dropbox_auth,
            commands::drives::add_dropbox_drive,
            commands::drives::start_onedrive_auth,
            commands::drives::add_onedrive_drive,
            commands::system::get_autostart,
            commands::system::set_autostart,
            commands::activity::list_activity,
            commands::activity::clear_activity,
            commands::activity::export_activity_csv,
            commands::prefs::get_pref,
            commands::prefs::set_pref,
            commands::prefs::clear_pref,
            commands::prefs::get_cache_root_info,
            commands::cache::pin_file,
            commands::cache::unpin_file,
            commands::cache::is_file_pinned,
            commands::cache::list_pinned_files,
            commands::locks::list_file_locks,
            commands::locks::break_file_lock,
            license::get_license_status,
            license::activate_license,
            license::deactivate_license,
            license::request_trial,
        ])
        .build(tauri::generate_context!())
        .expect("error building nanocrew sync")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // Unmount all live drives before the process exits so WinFsp
                // drive letters are released cleanly.
                let state: tauri::State<AppState> = app_handle.state();
                let handles: Vec<_> = state
                    .mounts
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .drain()
                    .map(|(_, h)| h)
                    .collect();

                for handle in handles {
                    handle.stop();
                }
            }
        });
}

/// Mount every drive that has `auto_mount = 1` and is not already live.
/// On every app start, sync every drive's display `name` to either the last
/// path segment of its `bucket_prefix` (if set) or the bucket name. Keeps the
/// label in the Dashboard matching the folder the user is actually browsing,
/// even if the drive was added before the auto-name behavior shipped.
fn sync_drive_names_to_folder(app: &tauri::AppHandle) {
    let state: tauri::State<AppState> = app.state();
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    let rows: Vec<(i64, String, String, String)> = {
        let mut stmt = match db.prepare(
            "SELECT id, name, bucket, COALESCE(bucket_prefix, '') FROM drives",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(target: "nanocrew::drive_names", "prepare: {e}");
                return;
            }
        };
        match stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(target: "nanocrew::drive_names", "query: {e}");
                return;
            }
        }
    };

    let mut changed = false;
    for (id, current_name, bucket, prefix) in rows {
        let target = if prefix.trim_matches('/').is_empty() {
            bucket
        } else {
            prefix
                .trim_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_string()
        };
        if target.is_empty() || target == current_name {
            continue;
        }
        if let Err(e) = db.execute(
            "UPDATE drives SET name = ?1 WHERE id = ?2",
            rusqlite::params![target, id],
        ) {
            tracing::warn!(target: "nanocrew::drive_names",
                "rename drive {id} -> {target:?}: {e}");
        } else {
            tracing::info!(target: "nanocrew::drive_names",
                "renamed drive {id}: {current_name:?} -> {target:?}");
            changed = true;
        }
    }
    if changed {
        let _ = app.emit("drives_changed", ());
    }
}

async fn auto_mount_drives(app: tauri::AppHandle) {
    // ── Pull drive rows from DB ───────────────────────────────────────────────
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i64, String, String, String, String, String, String, bool, String, i64, i64, f64, f64)> = {
        let state: tauri::State<AppState> = app.state();
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = match db.prepare(
            "SELECT id, endpoint, bucket, region, letter, access_key_id, provider, readonly,
                    COALESCE(bucket_prefix,''),
                    COALESCE(cache_max_bytes, 10737418240),
                    COALESCE(cache_enabled, 1),
                    COALESCE(upload_rate_mbps, 0.0),
                    COALESCE(download_rate_mbps, 0.0)
             FROM drives WHERE auto_mount = 1",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(target: "nanocrew::auto_mount", "prepare failed: {e}");
                return;
            }
        };

        match stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, bool>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, i64>(10)?,
                    r.get::<_, f64>(11)?,
                    r.get::<_, f64>(12)?,
                ))
            })
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(target: "nanocrew::auto_mount", "query failed: {e}");
                return;
            }
        }
    }; // db lock released

    // Resolve the app data DB path once — passed into every cache per drive.
    let db_path = match app.path().app_data_dir() {
        Ok(p) => p.join("nanocrew.db"),
        Err(e) => {
            tracing::error!(target: "nanocrew::auto_mount", "app_data_dir: {e}");
            return;
        }
    };
    // Cache root — same value for every auto-mounted drive this boot.
    let cache_root = {
        let state: tauri::State<AppState> = app.state();
        match commands::prefs::get_cache_root(&state.db) {
            Some(p) => p,
            None => {
                tracing::error!(target: "nanocrew::auto_mount",
                    "LOCALAPPDATA not set — cannot resolve cache root");
                return;
            }
        }
    };

    for (id, endpoint, bucket, region, letter, aki, provider, readonly, bucket_prefix,
         drive_cache_max_bytes, drive_cache_enabled, drive_upload_mbps, drive_download_mbps) in rows {
        let state: tauri::State<AppState> = app.state();

        // Skip if already mounted (e.g. user mounted manually during setup window)
        if state.mounts.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&id) {
            continue;
        }

        let secret = match credentials::retrieve(&state.db, id) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(target: "nanocrew::auto_mount", drive_id = id, "credential error: {e}");
                let msg = e.to_string();
                let _ = app.emit(
                    "drive_status_changed",
                    DriveStatusPayload { drive_id: id, status: "error".into(), message: Some(msg.clone()) },
                );
                commands::activity::record(
                    &state.db, &app, "mount", "mount_failed",
                    commands::activity::SEV_ERROR,
                    Some(id), Some("auto-mount"), Some(&letter), Some(&msg),
                );
                continue;
            }
        };

        // Per-drive bandwidth overrides: if the drive row has a non-zero value,
        // use it; otherwise fall back to the global pref.
        let upload_rate_bps = if drive_upload_mbps > 0.0 {
            Some((drive_upload_mbps * 1_048_576.0) as u64)
        } else {
            commands::prefs::get_rate_bps(&state.db, "upload_rate_mbps")
        };
        let download_rate_bps = if drive_download_mbps > 0.0 {
            Some((drive_download_mbps * 1_048_576.0) as u64)
        } else {
            commands::prefs::get_rate_bps(&state.db, "download_rate_mbps")
        };
        let cache_enabled = drive_cache_enabled != 0;
        let cache_max_bytes = drive_cache_max_bytes.max(0) as u64;

        let config = mounts::MountConfig {
            drive_id: id,
            letter: letter.clone(),
            provider,
            endpoint,
            bucket,
            bucket_prefix,
            region,
            access_key_id: aki,
            secret_access_key: secret,
            readonly,
            // Startup auto-mount runs before any user signs in, so we tag the
            // sentinel owner generically. Manual `mount_drive` calls from an
            // authed session supply the real username.
            owner: "auto-mount".to_string(),
            upload_rate_bps,
            download_rate_bps,
            cache_enabled,
            cache_max_bytes,
            db_path: db_path.clone(),
            cache_root: cache_root.clone(),
        };

        let _ = app.emit(
            "drive_status_changed",
            DriveStatusPayload { drive_id: id, status: "mounting".into(), message: None },
        );

        let app2 = app.clone();
        let app3 = app.clone();
        let letter_for_log = letter.clone();
        tokio::task::spawn_blocking(move || mounts::spawn_mount(config, app2))
            .await
            .map(|result| match result {
                Ok(handle) => {
                    let state: tauri::State<AppState> = app3.state();
                    state.mounts.lock().unwrap_or_else(|p| p.into_inner()).insert(id, handle);
                    commands::activity::record(
                        &state.db, &app3, "mount", "mount",
                        commands::activity::SEV_INFO,
                        Some(id), Some("auto-mount"),
                        Some(&letter_for_log), None,
                    );
                    // "mounted" event already emitted by the WinFsp thread
                }
                Err(e) => {
                    let state: tauri::State<AppState> = app3.state();
                    let msg = e.to_string();
                    let _ = app3.emit(
                        "drive_status_changed",
                        DriveStatusPayload { drive_id: id, status: "error".into(), message: Some(msg.clone()) },
                    );
                    commands::activity::record(
                        &state.db, &app3, "mount", "mount_failed",
                        commands::activity::SEV_ERROR,
                        Some(id), Some("auto-mount"),
                        Some(&letter_for_log), Some(&msg),
                    );
                }
            })
            .unwrap_or_default();
    }
}

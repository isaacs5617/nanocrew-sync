use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

mod auth;
mod cache;
mod commands;
mod credentials;
mod db;
mod dpapi;
mod error;
mod file_lock;
mod http_client;
mod logging;
mod mounts;
mod state;
mod throttle;
mod types;
mod winfsp_vfs;

use state::AppState;
use types::DriveStatusPayload;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            commands::drives::open_path,
            commands::drives::check_winfsp,
            commands::system::get_autostart,
            commands::system::set_autostart,
            commands::activity::list_activity,
            commands::activity::clear_activity,
            commands::activity::export_activity_csv,
            commands::prefs::get_pref,
            commands::prefs::set_pref,
            commands::prefs::clear_pref,
            commands::cache::pin_file,
            commands::cache::unpin_file,
            commands::cache::is_file_pinned,
            commands::cache::list_pinned_files,
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
async fn auto_mount_drives(app: tauri::AppHandle) {
    // ── Pull drive rows from DB ───────────────────────────────────────────────
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i64, String, String, String, String, String, String, bool, i64)> = {
        let state: tauri::State<AppState> = app.state();
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = match db.prepare(
            "SELECT id, endpoint, bucket, region, letter, access_key_id, provider, readonly, cache_size_gb
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
                    r.get::<_, i64>(8)?,
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

    for (id, endpoint, bucket, region, letter, aki, provider, readonly, cache_size_gb) in rows {
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

        // Read global bandwidth caps from prefs so auto-mounted drives
        // respect them too.
        let upload_rate_bps = commands::prefs::get_rate_bps(&state.db, "upload_rate_mbps");
        let download_rate_bps = commands::prefs::get_rate_bps(&state.db, "download_rate_mbps");
        let cache_enabled = commands::prefs::get_bool(&state.db, "cache_enabled", true);
        let cache_max_bytes = (cache_size_gb.max(0) as u64).saturating_mul(1_073_741_824);

        let config = mounts::MountConfig {
            drive_id: id,
            letter: letter.clone(),
            provider,
            endpoint,
            bucket,
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

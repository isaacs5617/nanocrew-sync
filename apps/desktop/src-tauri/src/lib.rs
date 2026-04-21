use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

mod auth;
mod commands;
mod credentials;
mod db;
mod error;
mod mounts;
mod state;
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
            app.manage(AppState::new(conn));

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
    let rows: Vec<(i64, String, String, String, String, String, String, bool)> = {
        let state: tauri::State<AppState> = app.state();
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = match db.prepare(
            "SELECT id, endpoint, bucket, region, letter, access_key_id, provider, readonly
             FROM drives WHERE auto_mount = 1",
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("auto_mount: prepare failed: {e}");
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
                ))
            })
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        {
            Ok(v) => v,
            Err(e) => {
                eprintln!("auto_mount: query failed: {e}");
                return;
            }
        }
    }; // db lock released

    for (id, endpoint, bucket, region, letter, aki, provider, readonly) in rows {
        let state: tauri::State<AppState> = app.state();

        // Skip if already mounted (e.g. user mounted manually during setup window)
        if state.mounts.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&id) {
            continue;
        }

        let secret = match credentials::retrieve(&state.db, id) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("auto_mount drive {id}: credential error: {e}");
                let _ = app.emit(
                    "drive_status_changed",
                    DriveStatusPayload { drive_id: id, status: "error".into(), message: Some(e.to_string()) },
                );
                continue;
            }
        };

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
        };

        let _ = app.emit(
            "drive_status_changed",
            DriveStatusPayload { drive_id: id, status: "mounting".into(), message: None },
        );

        let app2 = app.clone();
        let app3 = app.clone();
        tokio::task::spawn_blocking(move || mounts::spawn_mount(config, app2))
            .await
            .map(|result| match result {
                Ok(handle) => {
                    let state: tauri::State<AppState> = app3.state();
                    state.mounts.lock().unwrap_or_else(|p| p.into_inner()).insert(id, handle);
                    // "mounted" event already emitted by the WinFsp thread
                }
                Err(e) => {
                    let _ = app3.emit(
                        "drive_status_changed",
                        DriveStatusPayload { drive_id: id, status: "error".into(), message: Some(e.to_string()) },
                    );
                }
            })
            .unwrap_or_default();
    }
}

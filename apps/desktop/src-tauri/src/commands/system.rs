//! Host OS integration commands — autostart, notification preferences, etc.
//!
//! These are intentionally scoped narrow: each command does exactly one thing
//! to the environment (read or write a single registry value). Anything more
//! complex goes in its own module.

use tauri::{AppHandle, Emitter, Manager, State};

use crate::{auth::require_auth, error::AppError, state::AppState};

/// Registry path under HKCU that Windows reads at user sign-in to launch
/// background apps.
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// Registry value name. Using the bundle identifier keeps us from colliding
/// with other tools that share the word "nanocrew".
const RUN_VALUE_NAME: &str = "NanoCrewSync";

/// Reads the HKCU Run key to see whether NanoCrew Sync is registered to
/// auto-start at sign-in. Returns `false` on any read error — the toggle is a
/// UI convenience, not a security boundary.
#[tauri::command]
pub async fn get_autostart(
    state: State<'_, AppState>,
    token: String,
) -> Result<bool, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    Ok(read_run_value().is_some())
}

/// Enables or disables auto-start at Windows sign-in by writing (or deleting)
/// an HKCU\Software\Microsoft\Windows\CurrentVersion\Run value. HKCU means we
/// don't need UAC elevation — only the current user's sign-in is affected.
#[tauri::command]
pub async fn set_autostart(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
    enabled: bool,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    if enabled {
        let exe = std::env::current_exe()
            .map_err(|e| AppError::Io(e).to_string())?
            .to_string_lossy()
            .to_string();
        // `--hidden` tells our own main() to start minimized to tray. We
        // don't want every sign-in to pop the window in the user's face.
        let command = format!("\"{}\" --hidden", exe);
        write_run_value(&command).map_err(|e| e.to_string())?;
    } else {
        delete_run_value().map_err(|e| e.to_string())?;
    }
    // Reuse the webview's app-data dir to flush any other host-side state —
    // no-op today, but the handle is here so future state writes can piggy-back.
    let _ = app.path().app_data_dir();
    Ok(())
}

// ── Registry helpers (Windows only) ──────────────────────────────────────────

#[cfg(windows)]
fn read_run_value() -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        REG_VALUE_TYPE,
    };

    unsafe {
        let mut key: HKEY = HKEY::default();
        let subkey: Vec<u16> = RUN_KEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return None;
        }

        let name: Vec<u16> = RUN_VALUE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut ty: REG_VALUE_TYPE = REG_VALUE_TYPE(0);
        let mut buf = [0u8; 1024];
        let mut len = buf.len() as u32;
        let q = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr()),
            Some(&mut len),
        );
        let _ = RegCloseKey(key);
        if q.is_err() {
            return None;
        }
        let used = (len as usize).min(buf.len());
        let wide: Vec<u16> = buf[..used]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        Some(String::from_utf16_lossy(&wide))
    }
}

#[cfg(windows)]
fn write_run_value(command: &str) -> Result<(), AppError> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    unsafe {
        let mut key: HKEY = HKEY::default();
        let subkey: Vec<u16> = RUN_KEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        );
        if status.is_err() {
            return Err(AppError::Mount(format!(
                "RegCreateKeyExW Run: {:?}",
                status
            )));
        }

        let name: Vec<u16> = RUN_VALUE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let data: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: &[u8] = std::slice::from_raw_parts(
            data.as_ptr() as *const u8,
            data.len() * 2,
        );
        let res = RegSetValueExW(key, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes));
        let _ = RegCloseKey(key);
        if res.is_err() {
            return Err(AppError::Mount(format!("RegSetValueExW Run: {:?}", res)));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn delete_run_value() -> Result<(), AppError> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
    };

    unsafe {
        let mut key: HKEY = HKEY::default();
        let subkey: Vec<u16> = RUN_KEY
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
        .is_err()
        {
            // Key doesn't exist → nothing to delete; treat as success.
            return Ok(());
        }
        let name: Vec<u16> = RUN_VALUE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // Ignore "value not present" — the post-condition (no autostart) holds.
        let _ = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
        let _ = RegCloseKey(key);
    }
    Ok(())
}

#[cfg(not(windows))]
fn read_run_value() -> Option<String> {
    None
}

#[cfg(not(windows))]
fn write_run_value(_command: &str) -> Result<(), AppError> {
    Ok(())
}

#[cfg(not(windows))]
fn delete_run_value() -> Result<(), AppError> {
    Ok(())
}

// ── Factory reset ────────────────────────────────────────────────────────────

/// Emergency-recovery command that wipes ALL locally-stored state — accounts,
/// drives, credentials, cache blocks, listing caches, activity log, prefs. The
/// only things it does NOT touch are the sentinel objects in the user's
/// buckets (those are S3-side) and the app binary itself.
///
/// UX: exposed in Settings → Danger Zone as a "Reset all data" button, guarded
/// by a confirmation dialog. On success the app forces a sign-out and the
/// frontend routes back to the setup screen; the user is expected to recreate
/// their account and re-add drives with their Wasabi secrets from a password
/// manager.
///
/// Best-effort: any single unmount / delete that fails is logged and skipped so
/// we always end in the intended "clean" state. Requires an authenticated
/// session so a stale token can't nuke someone's data.
#[tauri::command]
pub async fn factory_reset(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    // 1. Unmount every currently-mounted drive so WinFsp releases file
    //    handles and drive letters before we drop the DB rows. We reach
    //    directly into `state.mounts` rather than looping through
    //    `unmount_drive` because that command also records activity /
    //    requires the letter lookup — post-wipe we don't care about
    //    either.
    let mounted_ids: Vec<i64> = {
        let mounts = state.mounts.lock().unwrap_or_else(|p| p.into_inner());
        mounts.keys().copied().collect()
    };
    for id in &mounted_ids {
        let handle = {
            let mut mounts = state.mounts.lock().unwrap_or_else(|p| p.into_inner());
            mounts.remove(id)
        };
        if let Some(h) = handle {
            h.stop();
            let _ = app.emit(
                "drive_status_changed",
                crate::types::DriveStatusPayload {
                    drive_id: *id,
                    status: "offline".into(),
                    message: None,
                },
            );
        }
    }

    // 2. Truncate all data tables. VACUUM shrinks the file so leftover pages
    //    with credential fragments can't be recovered off disk.
    {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let sql = "
            DELETE FROM accounts;
            DELETE FROM drives;
            DELETE FROM cache_entries;
            DELETE FROM pinned_keys;
            DELETE FROM activity;
            DELETE FROM prefs;
        ";
        if let Err(e) = db.execute_batch(sql) {
            tracing::error!(target: "nanocrew::factory_reset", "table wipe: {e}");
        }
        if let Err(e) = db.execute_batch("VACUUM;") {
            tracing::warn!(target: "nanocrew::factory_reset", "vacuum: {e}");
        }
    }

    // 3. Wipe on-disk cache blocks + directory listings for every drive.
    let cache_root = crate::commands::prefs::get_cache_root(&state.db);
    if let Some(root) = cache_root {
        if root.exists() {
            if let Err(e) = std::fs::remove_dir_all(&root) {
                tracing::warn!(
                    target: "nanocrew::factory_reset",
                    "wipe cache root {}: {e}",
                    root.display()
                );
            }
        }
    }

    // 4. Emit a signal the frontend can listen for to route to Setup.
    let _ = app.emit("factory_reset_complete", ());

    tracing::info!(
        target: "nanocrew::factory_reset",
        "wiped {} mounted drive(s) + all local state",
        mounted_ids.len()
    );
    Ok(())
}

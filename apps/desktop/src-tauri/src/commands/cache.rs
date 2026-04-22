//! Tauri commands for the on-disk cache (Phase 5.6).
//!
//! Pin/unpin is modelled as rows in the `pinned_keys` table — *not* as state
//! on the in-memory `DiskCache`. That gives us three things for free:
//!
//! 1. **Survives unmount.** Pins set while a drive is online persist even
//!    after the mount is stopped. When the drive remounts, the eviction
//!    sweeper sees the same pins and keeps honoring them.
//! 2. **Works on offline drives.** The file browser can pin/unpin regardless
//!    of whether the drive is currently mounted — useful for pre-configuring
//!    "keep these on device" before first mount.
//! 3. **No lock routing.** Every call is a single SQLite statement; no need
//!    to thread the live `Arc<DiskCache>` from `MountHandle` out to every
//!    command.
//!
//! The live eviction loop inside `DiskCache` reads `pinned_keys` on every
//! sweep, so changes are picked up automatically within one `EVICT_INTERVAL`.

use rusqlite::params;
use tauri::State;

use crate::{
    auth::require_auth,
    error::AppError,
    state::AppState,
};

#[tauri::command]
pub async fn pin_file(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
    key: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    db.execute(
        "INSERT OR IGNORE INTO pinned_keys (drive_id, key) VALUES (?1, ?2)",
        params![drive_id, key],
    )
    .map_err(|e| AppError::Db(e).to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn unpin_file(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
    key: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    db.execute(
        "DELETE FROM pinned_keys WHERE drive_id = ?1 AND key = ?2",
        params![drive_id, key],
    )
    .map_err(|e| AppError::Db(e).to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn is_file_pinned(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
    key: String,
) -> Result<bool, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    Ok(db
        .query_row(
            "SELECT 1 FROM pinned_keys WHERE drive_id = ?1 AND key = ?2",
            params![drive_id, key],
            |_| Ok(()),
        )
        .is_ok())
}

#[tauri::command]
pub async fn list_pinned_files(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
) -> Result<Vec<String>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    let mut stmt = db
        .prepare("SELECT key FROM pinned_keys WHERE drive_id = ?1 ORDER BY key")
        .map_err(|e| AppError::Db(e).to_string())?;
    let rows = stmt
        .query_map(params![drive_id], |r| r.get::<_, String>(0))
        .and_then(|it| it.collect::<Result<Vec<_>, _>>())
        .map_err(|e| AppError::Db(e).to_string())?;
    Ok(rows)
}

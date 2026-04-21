//! Key/value preferences store — a thin `prefs` table + helpers.
//!
//! Settings that don't warrant their own schema column live here so we can
//! extend the Settings UI without a migration for every new toggle.

use rusqlite::{params, Connection};
use std::sync::Mutex;
use tauri::State;

use crate::{auth::require_auth, error::AppError, state::AppState};

/// Low-level read for internal callers (startup, etc.). Returns `None` if the
/// key is absent. Errors become `None` — startup reads should never fail the
/// boot path.
pub fn get(db: &Mutex<Connection>, key: &str) -> Option<String> {
    let conn = db.lock().unwrap_or_else(|p| p.into_inner());
    conn.query_row(
        "SELECT value FROM prefs WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

/// `get` with a boolean interpretation: "1"/"true" → true, everything else → `default`.
pub fn get_bool(db: &Mutex<Connection>, key: &str, default: bool) -> bool {
    match get(db, key).as_deref() {
        Some("1") | Some("true") => true,
        Some("0") | Some("false") => false,
        _ => default,
    }
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// Read a preference as a string. Returns `None` when the key isn't set yet.
#[tauri::command]
pub async fn get_pref(
    state: State<'_, AppState>,
    token: String,
    key: String,
) -> Result<Option<String>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    Ok(get(&state.db, &key))
}

/// Write a preference. Passing an empty value still writes a row — callers
/// that want to reset should use `clear_pref`.
#[tauri::command]
pub async fn set_pref(
    state: State<'_, AppState>,
    token: String,
    key: String,
    value: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    db.execute(
        "INSERT INTO prefs (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| AppError::Db(e).to_string())?;
    Ok(())
}

/// Delete a preference row. No-op if the key doesn't exist.
#[tauri::command]
pub async fn clear_pref(
    state: State<'_, AppState>,
    token: String,
    key: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    db.execute("DELETE FROM prefs WHERE key = ?1", params![key])
        .map_err(|e| AppError::Db(e).to_string())?;
    Ok(())
}

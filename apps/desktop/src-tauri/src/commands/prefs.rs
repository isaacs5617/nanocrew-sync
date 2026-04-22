//! Key/value preferences store — a thin `prefs` table + helpers.
//!
//! Settings that don't warrant their own schema column live here so we can
//! extend the Settings UI without a migration for every new toggle.

use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

use crate::{auth::require_auth, error::AppError, state::AppState};

/// Default cache root: `%LOCALAPPDATA%\NanoCrew\Sync\cache`. Returns `None`
/// only when `LOCALAPPDATA` is unset (should never happen on a real Windows
/// session).
pub fn default_cache_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("NanoCrew").join("Sync").join("cache"))
}

/// Resolve the effective cache root — honors the `cache_root` pref if set
/// (and non-empty), otherwise falls back to the default. Callers get a
/// concrete path even when the pref is unset, so mount code never has to
/// re-compute the default itself.
pub fn get_cache_root(db: &Mutex<Connection>) -> Option<PathBuf> {
    match get(db, "cache_root") {
        Some(s) if !s.trim().is_empty() => Some(PathBuf::from(s.trim())),
        _ => default_cache_root(),
    }
}

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

/// Read a MB/s rate pref and return it as bytes/second. Returns `None` for
/// an unset, empty, non-numeric, or zero / negative value — all of which
/// mean "unlimited". Accepts decimals (e.g. "0.5" → 524288 B/s).
pub fn get_rate_bps(db: &Mutex<Connection>, key: &str) -> Option<u64> {
    let raw = get(db, key)?;
    let mbps: f64 = raw.trim().parse().ok()?;
    if !mbps.is_finite() || mbps <= 0.0 {
        return None;
    }
    Some((mbps * 1_048_576.0) as u64)
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

/// Return a snapshot of cache-location info for the Settings screen:
/// `(effective, default, is_custom)`. `effective` is what a fresh mount
/// would use right now; `default` is the LOCALAPPDATA path we'd fall back
/// to; `is_custom` is whether the pref is currently overriding the default.
#[tauri::command]
pub async fn get_cache_root_info(
    state: State<'_, AppState>,
    token: String,
) -> Result<(String, String, bool), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let default = default_cache_root()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let override_pref = get(&state.db, "cache_root")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (effective, is_custom) = match override_pref {
        Some(p) => (p, true),
        None => (default.clone(), false),
    };
    Ok((effective, default, is_custom))
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

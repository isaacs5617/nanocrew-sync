//! Audit log — append-only event stream persisted in the `activity` table.
//!
//! Other modules call [`record`] to add an entry; the UI reads via
//! [`list_activity`] and subscribes to the `activity_appended` Tauri event for
//! live updates.

use std::sync::Mutex;

use rusqlite::{params, Connection};
use tauri::{AppHandle, Emitter, State};

use crate::{auth::require_auth, error::AppError, state::AppState, types::ActivityEntry};

/// Severity levels used by the UI to colour rows. Kept as free-form strings in
/// the DB to avoid schema churn when we add new levels (e.g. "debug").
pub const SEV_INFO: &str = "info";
#[allow(dead_code)] // reserved for soft-failure paths (e.g. partial mount)
pub const SEV_WARN: &str = "warn";
pub const SEV_ERROR: &str = "error";

/// Append one row to the activity log. All domain modules funnel through this
/// so we have a single place to emit the Tauri event too.
///
/// Takes the shared `Mutex<Connection>` + an `AppHandle` so callers don't have
/// to thread both through every sitepass. Errors are swallowed on purpose —
/// failing to log an event shouldn't fail the action it describes.
pub fn record(
    db: &Mutex<Connection>,
    app: &AppHandle,
    kind: &str,
    action: &str,
    severity: &str,
    drive_id: Option<i64>,
    actor: Option<&str>,
    target: Option<&str>,
    message: Option<&str>,
) {
    let id = {
        let conn = db.lock().unwrap_or_else(|p| p.into_inner());
        match conn.execute(
            "INSERT INTO activity (kind, action, severity, drive_id, actor, target, message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![kind, action, severity, drive_id, actor, target, message],
        ) {
            Ok(_) => conn.last_insert_rowid(),
            Err(e) => {
                eprintln!("activity: insert failed: {e}");
                return;
            }
        }
    };

    // Emit for live UI. We construct the payload here rather than re-reading
    // from SQLite to skip the extra round-trip — ts is approx. now anyway.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let entry = ActivityEntry {
        id,
        ts,
        kind: kind.to_string(),
        action: action.to_string(),
        severity: severity.to_string(),
        drive_id,
        actor: actor.map(str::to_owned),
        target: target.map(str::to_owned),
        message: message.map(str::to_owned),
    };
    let _ = app.emit("activity_appended", &entry);
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// Returns the most recent activity rows (newest first). Optional filters:
///
/// - `kinds`: if non-empty, only rows whose `kind` is in this set
/// - `severity`: if Some, only rows at that severity
/// - `since`: if Some, only rows with ts ≥ since (unix seconds)
/// - `limit`: max rows to return (default 500, max 5000)
#[tauri::command]
pub async fn list_activity(
    state: State<'_, AppState>,
    token: String,
    kinds: Option<Vec<String>>,
    severity: Option<String>,
    since: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<ActivityEntry>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    let limit = limit.unwrap_or(500).clamp(1, 5000);

    // Rather than build a WHERE clause with variable params, we fetch a small
    // window and filter in-memory. With the ts DESC index and limit ≤ 5k this
    // is still fast, and keeps the SQL simple.
    let rows = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = db
            .prepare(
                "SELECT id, ts, kind, action, severity, drive_id, actor, target, message
                 FROM activity ORDER BY ts DESC, id DESC LIMIT ?1",
            )
            .map_err(|e| AppError::Db(e).to_string())?;
        let r: Result<Vec<_>, _> = stmt
            .query_map(params![limit], |r| {
                Ok(ActivityEntry {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    kind: r.get(2)?,
                    action: r.get(3)?,
                    severity: r.get(4)?,
                    drive_id: r.get(5)?,
                    actor: r.get(6)?,
                    target: r.get(7)?,
                    message: r.get(8)?,
                })
            })
            .map_err(|e| AppError::Db(e).to_string())?
            .collect();
        r.map_err(|e| AppError::Db(e).to_string())?
    };

    let kinds_set: Option<std::collections::HashSet<String>> =
        kinds.filter(|v| !v.is_empty()).map(|v| v.into_iter().collect());

    let filtered = rows
        .into_iter()
        .filter(|e| kinds_set.as_ref().map_or(true, |s| s.contains(&e.kind)))
        .filter(|e| severity.as_ref().map_or(true, |s| &e.severity == s))
        .filter(|e| since.map_or(true, |s| e.ts >= s))
        .collect();

    Ok(filtered)
}

/// Permanently truncates the activity log. Used by the "Clear log" button.
#[tauri::command]
pub async fn clear_activity(
    state: State<'_, AppState>,
    token: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    db.execute("DELETE FROM activity", [])
        .map_err(|e| AppError::Db(e).to_string())?;
    Ok(())
}

/// Writes a CSV dump of the activity log to `path`. The caller (frontend) is
/// responsible for picking the path via a save dialog.
#[tauri::command]
pub async fn export_activity_csv(
    state: State<'_, AppState>,
    token: String,
    path: String,
) -> Result<usize, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    let rows: Vec<ActivityEntry> = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = db
            .prepare(
                "SELECT id, ts, kind, action, severity, drive_id, actor, target, message
                 FROM activity ORDER BY ts ASC, id ASC",
            )
            .map_err(|e| AppError::Db(e).to_string())?;
        let r: Result<Vec<_>, _> = stmt
            .query_map([], |r| {
                Ok(ActivityEntry {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    kind: r.get(2)?,
                    action: r.get(3)?,
                    severity: r.get(4)?,
                    drive_id: r.get(5)?,
                    actor: r.get(6)?,
                    target: r.get(7)?,
                    message: r.get(8)?,
                })
            })
            .map_err(|e| AppError::Db(e).to_string())?
            .collect();
        r.map_err(|e| AppError::Db(e).to_string())?
    };

    let mut out = String::with_capacity(rows.len() * 96);
    out.push_str("id,ts,kind,action,severity,drive_id,actor,target,message\n");
    for e in &rows {
        use std::fmt::Write;
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{}",
            e.id,
            e.ts,
            csv_field(&e.kind),
            csv_field(&e.action),
            csv_field(&e.severity),
            e.drive_id.map(|d| d.to_string()).unwrap_or_default(),
            csv_field(e.actor.as_deref().unwrap_or("")),
            csv_field(e.target.as_deref().unwrap_or("")),
            csv_field(e.message.as_deref().unwrap_or("")),
        );
    }

    std::fs::write(&path, out.as_bytes()).map_err(|e| AppError::Io(e).to_string())?;
    Ok(rows.len())
}

/// Escape a single CSV field. Quotes fields containing commas, quotes, or
/// newlines; doubles internal quotes per RFC 4180.
fn csv_field(s: &str) -> String {
    let needs_quotes = s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
    if !needs_quotes {
        return s.to_string();
    }
    let escaped = s.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_field_quotes_when_needed() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("has,comma"), "\"has,comma\"");
        assert_eq!(csv_field("quote\"inside"), "\"quote\"\"inside\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
    }
}

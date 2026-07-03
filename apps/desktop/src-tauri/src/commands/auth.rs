use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::{
    auth::require_auth,
    commands::activity,
    dpapi,
    error::AppError,
    state::{ActiveSession, AppState},
    types::AccountInfo,
};

/// Key in the `prefs` table where the DPAPI-wrapped persisted session lives.
const PERSISTED_SESSION_KEY: &str = "persisted_session_v1";

/// How long a "Remember me" session is valid before we force a fresh sign-in.
/// 30 days matches the industry norm for consumer apps and keeps the leaked-
/// token blast radius bounded even if DPAPI somehow leaks (e.g. offline
/// bruteforce of the ntds.dit master key).
const REMEMBER_ME_TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// Payload we DPAPI-wrap and stash in `prefs` when the user ticks Remember me.
#[derive(Serialize, Deserialize)]
struct PersistedSession {
    account_id: i64,
    username: String,
    expires_at: u64, // seconds since UNIX epoch
}

#[tauri::command]
pub async fn create_admin(
    state: State<'_, AppState>,
    app: AppHandle,
    username: String,
    password: String,
) -> Result<(), String> {
    if username.trim().is_empty() || password.len() < 8 {
        return Err(AppError::InvalidInput(
            "Username required and password must be at least 8 characters".into(),
        )
        .to_string());
    }

    let hash = hash_password(&password).map_err(|e| e.to_string())?;

    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM accounts",
        [],
        |r| r.get(0),
    )
    .map_err(|e| AppError::Db(e).to_string())?;

    if count > 0 {
        return Err(AppError::AlreadyExists.to_string());
    }

    db.execute(
        "INSERT INTO accounts (username, password_hash) VALUES (?1, ?2)",
        rusqlite::params![username, hash],
    )
    .map_err(|e| AppError::Db(e).to_string())?;
    drop(db);

    activity::record(
        &state.db, &app, "auth", "account_created", activity::SEV_INFO,
        None, Some(&username), None, None,
    );

    Ok(())
}

#[tauri::command]
pub async fn sign_in(
    state: State<'_, AppState>,
    app: AppHandle,
    username: String,
    password: String,
    remember: Option<bool>,
) -> Result<String, String> {
    let (account_id, stored_hash) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT id, password_hash FROM accounts WHERE username = ?1",
            rusqlite::params![username],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|_| AppError::InvalidCredentials.to_string())?
    };

    let parsed = PasswordHash::new(&stored_hash)
        .map_err(|e| AppError::PasswordHash(e.to_string()).to_string())?;

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| AppError::InvalidCredentials.to_string())?;

    let token = Uuid::new_v4().to_string();
    state.sessions.lock().unwrap_or_else(|p| p.into_inner()).insert(
        token.clone(),
        ActiveSession { account_id, username: username.clone() },
    );

    // Optionally persist a DPAPI-wrapped session so the next launch can skip
    // the sign-in screen. We do NOT store the token itself — we store the
    // account identity so the next boot can mint a fresh in-memory token
    // (leaked persistent tokens can't be replayed on a different machine
    // because DPAPI is bound to this Windows user + this machine).
    if remember.unwrap_or(false) {
        if let Err(e) = persist_session(&state, account_id, &username) {
            // Non-fatal: sign-in still succeeds; user just won't be
            // remembered on next launch. Log so it's visible.
            tracing::warn!(target: "nanocrew::auth", "persist_session failed: {e}");
        }
    }

    activity::record(
        &state.db, &app, "auth", "sign_in", activity::SEV_INFO,
        None, Some(&username), None, None,
    );

    Ok(token)
}

/// On app boot, attempt to restore the previously-remembered session.
/// Returns `Ok(Some(token))` on success (skip sign-in screen), `Ok(None)`
/// when there's nothing to restore (or it's expired), and `Err(...)` only
/// on genuine failures (DB errors, DPAPI unwrap failed — e.g. copied to a
/// different Windows profile).
#[tauri::command]
pub async fn try_restore_session(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<String>, String> {
    let raw = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        match db.query_row(
            "SELECT value FROM prefs WHERE key = ?1",
            rusqlite::params![PERSISTED_SESSION_KEY],
            |r| r.get::<_, String>(0),
        ) {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(AppError::Db(e).to_string()),
        }
    };

    // Try to decode + unwrap. If any of these fail, wipe the stored value
    // so we don't loop on a corrupt / foreign blob forever.
    let session = match decode_persisted(&raw) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "nanocrew::auth", "clearing corrupt persisted session: {e}");
            let _ = clear_persisted_session(&state);
            return Ok(None);
        }
    };

    let now = now_secs();
    if session.expires_at <= now {
        let _ = clear_persisted_session(&state);
        return Ok(None);
    }

    // Verify the account still exists and hasn't been renamed under us.
    let username = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT username FROM accounts WHERE id = ?1",
            rusqlite::params![session.account_id],
            |r| r.get::<_, String>(0),
        )
        .ok()
    };
    let username = match username {
        Some(u) if u == session.username => u,
        _ => {
            let _ = clear_persisted_session(&state);
            return Ok(None);
        }
    };

    // Mint a fresh in-memory session token.
    let token = Uuid::new_v4().to_string();
    state.sessions.lock().unwrap_or_else(|p| p.into_inner()).insert(
        token.clone(),
        ActiveSession { account_id: session.account_id, username: username.clone() },
    );

    // Roll the expiry forward so a daily user stays logged in indefinitely
    // without a re-sign-in prompt.
    let _ = persist_session(&state, session.account_id, &username);

    activity::record(
        &state.db, &app, "auth", "sign_in_restored", activity::SEV_INFO,
        None, Some(&username), None, None,
    );

    Ok(Some(token))
}

#[tauri::command]
pub async fn sign_out(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
) -> Result<(), String> {
    let user = state
        .sessions
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&token)
        .map(|s| s.username);

    // An explicit sign-out always clears the persisted "remember me" record
    // — the user just told us they want to actually sign out.
    let _ = clear_persisted_session(&state);

    if let Some(u) = user.as_deref() {
        activity::record(
            &state.db, &app, "auth", "sign_out", activity::SEV_INFO,
            None, Some(u), None, None,
        );
    }
    Ok(())
}

/// Returns true if at least one account exists — used for first-run detection.
#[tauri::command]
pub async fn has_account(state: State<'_, AppState>) -> Result<bool, String> {
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
        .map_err(|e| AppError::Db(e).to_string())?;
    Ok(count > 0)
}

#[tauri::command]
pub async fn get_account(
    state: State<'_, AppState>,
    token: String,
) -> Result<AccountInfo, String> {
    let account_id = require_auth(&state, &token).map_err(|e| e.to_string())?;

    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    db.query_row(
        "SELECT id, username, created_at FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |r| {
            Ok(AccountInfo {
                id: r.get(0)?,
                username: r.get(1)?,
                created_at: r.get(2)?,
            })
        },
    )
    .map_err(|e| AppError::Db(e).to_string())
}

#[tauri::command]
pub async fn change_password(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
    current_password: String,
    new_password: String,
) -> Result<(), String> {
    let account_id = require_auth(&state, &token).map_err(|e| e.to_string())?;

    if new_password.len() < 8 {
        return Err(AppError::InvalidInput(
            "New password must be at least 8 characters".into(),
        ).to_string());
    }

    let stored_hash = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT password_hash FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| AppError::Db(e).to_string())?
    };

    let parsed = PasswordHash::new(&stored_hash)
        .map_err(|e| AppError::PasswordHash(e.to_string()).to_string())?;

    Argon2::default()
        .verify_password(current_password.as_bytes(), &parsed)
        .map_err(|_| AppError::InvalidCredentials.to_string())?;

    let new_hash = hash_password(&new_password).map_err(|e| e.to_string())?;

    {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.execute(
            "UPDATE accounts SET password_hash = ?1 WHERE id = ?2",
            rusqlite::params![new_hash, account_id],
        )
        .map_err(|e| AppError::Db(e).to_string())?;
    }

    let actor = state
        .sessions
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&token)
        .map(|s| s.username.clone());
    activity::record(
        &state.db, &app, "auth", "password_changed", activity::SEV_INFO,
        None, actor.as_deref(), None, None,
    );

    Ok(())
}

/// Re-verify the authenticated user's password without issuing a new token.
/// Used by the session-lock flow: drives stay mounted, the session token
/// stays valid, we just re-prove the user is at the keyboard.
#[tauri::command]
pub async fn verify_password(
    state: State<'_, AppState>,
    token: String,
    password: String,
) -> Result<(), String> {
    let account_id = require_auth(&state, &token).map_err(|e| e.to_string())?;

    let stored_hash: String = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT password_hash FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::InvalidCredentials.to_string())?
    };

    let parsed = PasswordHash::new(&stored_hash)
        .map_err(|e| AppError::PasswordHash(e.to_string()).to_string())?;

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| AppError::InvalidCredentials.to_string())?;

    Ok(())
}

/// Audit-log a lock/unlock transition. Locking is handled UI-side (no server
/// state changes) but we want a trail for security review. `reason` is a free
/// short tag — `"minimize"`, `"idle"`, `"manual"`, etc.
#[tauri::command]
pub async fn record_lock_event(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
    locked: bool,
    reason: Option<String>,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let actor = state
        .sessions
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&token)
        .map(|s| s.username.clone());
    let action = if locked { "lock" } else { "unlock" };
    activity::record(
        &state.db, &app, "auth", action, activity::SEV_INFO,
        None, actor.as_deref(), None, reason.as_deref(),
    );
    Ok(())
}

#[tauri::command]
pub async fn clear_cache(
    state: State<'_, AppState>,
    token: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    // The S3 metadata cache is in-memory with a TTL and evicts automatically.
    // Nothing persistent to delete — command exists so the UI button has a real endpoint.
    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::PasswordHash(e.to_string()))
}

/// DPAPI-wrap the current sign-in identity and stash it in `prefs`.
fn persist_session(state: &AppState, account_id: i64, username: &str) -> Result<(), AppError> {
    let payload = PersistedSession {
        account_id,
        username: username.to_string(),
        expires_at: now_secs() + REMEMBER_ME_TTL_SECS,
    };
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|e| AppError::InvalidInput(format!("serialize persisted session: {e}")))?;
    let wrapped = dpapi::protect(&plaintext)?;
    let b64 = B64.encode(&wrapped);

    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    db.execute(
        "INSERT INTO prefs (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![PERSISTED_SESSION_KEY, b64],
    )
    .map_err(AppError::Db)?;
    Ok(())
}

fn decode_persisted(b64: &str) -> Result<PersistedSession, AppError> {
    let wrapped = B64
        .decode(b64.as_bytes())
        .map_err(|e| AppError::InvalidInput(format!("base64: {e}")))?;
    let plaintext = dpapi::unprotect(&wrapped)?;
    serde_json::from_slice(&plaintext)
        .map_err(|e| AppError::InvalidInput(format!("deserialize persisted session: {e}")))
}

fn clear_persisted_session(state: &AppState) -> Result<(), AppError> {
    let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
    db.execute(
        "DELETE FROM prefs WHERE key = ?1",
        rusqlite::params![PERSISTED_SESSION_KEY],
    )
    .map_err(AppError::Db)?;
    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

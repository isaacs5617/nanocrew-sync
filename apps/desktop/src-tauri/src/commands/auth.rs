use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand_core::OsRng;
use tauri::State;
use uuid::Uuid;

use crate::{
    auth::require_auth,
    error::AppError,
    state::{ActiveSession, AppState},
    types::AccountInfo,
};

#[tauri::command]
pub async fn create_admin(
    state: State<'_, AppState>,
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

    Ok(())
}

#[tauri::command]
pub async fn sign_in(
    state: State<'_, AppState>,
    username: String,
    password: String,
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
        ActiveSession { account_id, username },
    );

    Ok(token)
}

#[tauri::command]
pub async fn sign_out(
    state: State<'_, AppState>,
    token: String,
) -> Result<(), String> {
    state.sessions.lock().unwrap_or_else(|p| p.into_inner()).remove(&token);
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

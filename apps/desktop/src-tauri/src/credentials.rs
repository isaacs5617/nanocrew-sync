//! Credential-at-rest storage for S3 secret keys.
//!
//! Secrets live in the `drives.secret_key` column, encoded with a version
//! prefix so we can evolve the storage format without losing existing rows:
//!
//!   * `v1:<base64-dpapi-ciphertext>` — current format. The plaintext secret
//!     was wrapped with [`crate::dpapi::protect`], which binds the ciphertext
//!     to the current Windows user on the current machine.
//!   * `<anything-else>` — legacy plaintext. Read as-is, then re-wrapped and
//!     written back on first access so the row is upgraded transparently.
//!
//! The migration-on-read approach means users who upgrade from v0.1.x get
//! their credentials re-encrypted the next time the drive is touched, with
//! no manual action and no mass-rewrite at boot time.

use std::sync::Mutex;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rusqlite::Connection;

use crate::{dpapi, error::AppError};

const V1_PREFIX: &str = "v1:";

/// Write `secret` to the `drives.secret_key` column, DPAPI-wrapped.
pub fn store(db: &Mutex<Connection>, drive_id: i64, secret: &str) -> Result<(), AppError> {
    let wrapped = wrap(secret)?;
    let conn = db.lock().unwrap_or_else(|p| p.into_inner());
    conn.execute(
        "UPDATE drives SET secret_key = ?1 WHERE id = ?2",
        rusqlite::params![wrapped, drive_id],
    )
    .map_err(AppError::Db)?;
    Ok(())
}

/// Read the secret for `drive_id`, transparently migrating legacy plaintext
/// rows to the v1 DPAPI format.
pub fn retrieve(db: &Mutex<Connection>, drive_id: i64) -> Result<String, AppError> {
    let raw: String = {
        let conn = db.lock().unwrap_or_else(|p| p.into_inner());
        conn.query_row(
            "SELECT secret_key FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| r.get(0),
        )
        .map_err(|_| missing_credential())?
    };

    if raw.is_empty() {
        return Err(missing_credential());
    }

    match unwrap(&raw) {
        // Already-wrapped row: return plaintext, no migration needed.
        UnwrapResult::Wrapped(plain) => Ok(plain),
        // Legacy plaintext: return it, and re-wrap in the background so the
        // next startup finds a v1 row.
        UnwrapResult::Legacy(plain) => {
            // Best-effort migration. A write failure here is not fatal — the
            // caller still gets the secret. We'll retry next read.
            let _ = store(db, drive_id, &plain);
            Ok(plain)
        }
        UnwrapResult::Invalid(e) => Err(e),
    }
}

/// Secrets are stored in the drive row, so deleting the row is sufficient.
pub fn delete(_db: &Mutex<Connection>, _drive_id: i64) -> Result<(), AppError> {
    Ok(())
}

// ── Internals ────────────────────────────────────────────────────────────────

fn missing_credential() -> AppError {
    AppError::Keyring(
        "No stored credential found for this drive. Remove and re-add the drive to restore access.".into(),
    )
}

/// Wrap a plaintext secret into the `v1:<base64>` envelope.
fn wrap(secret: &str) -> Result<String, AppError> {
    let ct = dpapi::protect(secret.as_bytes())?;
    Ok(format!("{V1_PREFIX}{}", B64.encode(ct)))
}

enum UnwrapResult {
    Wrapped(String),
    Legacy(String),
    Invalid(AppError),
}

fn unwrap(raw: &str) -> UnwrapResult {
    if let Some(b64) = raw.strip_prefix(V1_PREFIX) {
        let ct = match B64.decode(b64.as_bytes()) {
            Ok(b) => b,
            Err(e) => return UnwrapResult::Invalid(AppError::Keyring(format!("bad base64: {e}"))),
        };
        return match dpapi::unprotect(&ct) {
            Ok(pt) => match String::from_utf8(pt) {
                Ok(s) => UnwrapResult::Wrapped(s),
                Err(e) => UnwrapResult::Invalid(AppError::Keyring(format!("bad utf8: {e}"))),
            },
            Err(e) => UnwrapResult::Invalid(e),
        };
    }
    UnwrapResult::Legacy(raw.to_string())
}

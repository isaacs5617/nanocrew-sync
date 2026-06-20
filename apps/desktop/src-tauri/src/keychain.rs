//! macOS Keychain wrapper — the DPAPI equivalent for credential-at-rest.
//!
//! On Windows, drive secrets are wrapped with `CryptProtectData`. On macOS we
//! use the Keychain via `security-framework`. Each drive's secret is stored
//! as a generic password item:
//!
//!   * Service: `dev.nanocrew.sync`  (matches the bundle identifier)
//!   * Account: `drive-<drive_id>-secret_key`
//!
//! The Keychain enforces two properties that DPAPI also enforces:
//!
//!   1. A different local user account cannot read another user's items.
//!   2. The encrypted form on disk is bound to the device (the wrap key
//!      lives in the Secure Enclave on Apple Silicon).
//!
//! For v0.3.0 beta we keep the API intentionally narrow: store/retrieve/
//! delete by drive_id. The integration with `credentials.rs` (which today is
//! Windows-only and writes into the `drives.secret_key` SQLite column) is a
//! follow-up — see `docs/macos-port-design.md`.

use crate::error::AppError;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

/// Keychain service name. Aligns with the Tauri `identifier` so the Keychain
/// UI groups items under "NanoCrew Sync".
const SERVICE: &str = "dev.nanocrew.sync";

fn account_for(drive_id: i64) -> String {
    format!("drive-{drive_id}-secret_key")
}

/// Persist `secret` for the given drive. Replaces any existing entry.
pub fn store(drive_id: i64, secret: &str) -> Result<(), AppError> {
    let account = account_for(drive_id);
    set_generic_password(SERVICE, &account, secret.as_bytes())
        .map_err(|e| AppError::Keyring(format!("keychain store: {e}")))
}

/// Read the secret for `drive_id`. Returns `Err` if the item is missing or
/// the user denied access (e.g. they hit "Deny" on the access prompt).
pub fn retrieve(drive_id: i64) -> Result<String, AppError> {
    let account = account_for(drive_id);
    let bytes = get_generic_password(SERVICE, &account)
        .map_err(|e| AppError::Keyring(format!("keychain retrieve: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|e| AppError::Keyring(format!("keychain retrieve: bad utf8: {e}")))
}

/// Delete the secret for `drive_id`. A missing entry is treated as success.
pub fn delete(drive_id: i64) -> Result<(), AppError> {
    let account = account_for(drive_id);
    match delete_generic_password(SERVICE, &account) {
        Ok(()) => Ok(()),
        // The crate returns a SecError when the item doesn't exist. We
        // treat that as success — the desired post-condition (the item is
        // absent) is met.
        Err(_) => Ok(()),
    }
}

// TODO(v0.3.0 follow-up): introduce a `secret_store` abstraction in
// credentials.rs that fans out to `dpapi::{protect,unprotect}` on Windows
// and `keychain::{store,retrieve}` on macOS. Today credentials.rs is
// `#[cfg(target_os = "windows")]`-gated; on macOS, drive_secret_key reads
// happen through this module directly via a cfg-branched helper in
// commands/drives.rs (still TODO — not blocking the scaffold compile).

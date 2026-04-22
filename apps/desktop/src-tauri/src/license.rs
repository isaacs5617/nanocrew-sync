//! License key verification + tier resolution (Phases 9.1–9.5).
//!
//! **Trust model.** License keys are compact JWTs signed by the NanoCrew
//! issuer (out-of-repo). We ship the issuer's RS256 public key as a
//! compile-time constant and only ever *verify* here — private-key signing
//! happens in the separate license-server workflow.
//!
//! **Shipping placeholder.** `ISSUER_PUBKEY_PEM` below is a placeholder
//! until the production keypair is generated for GA. Ship-blocker for
//! Phase 11.5 (GA launch) — flip the constant and rebuild.
//!
//! **Flow.** On first run we record `install_at` and kick off a 14-day
//! `Trial` tier (Phase 9.5). Users paste a JWT into Settings → About →
//! Activate; we verify the signature, check that the bound machine
//! fingerprint matches this machine (or is empty = unbound), check
//! `exp`, and persist the raw JWT to the `prefs` table. Revocation is
//! not required for now — short `exp`s + re-issuance cover abuse cases.
//!
//! **Tier enforcement.** This module computes a `LicenseStatus` which the
//! frontend uses to render the badge + CTAs. Hard feature gates (e.g.
//! max_drives) are intentionally *not* wired into the rest of the code
//! yet — the intent is to ship a "soft" launch where the UX nudges
//! users to upgrade rather than hard-blocking functionality. That keeps
//! the binary honest if a key server hiccup leaves paying users without
//! a token for a few minutes.

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use tauri::{AppHandle, State};

use crate::{
    auth::require_auth,
    commands::{activity, prefs},
    state::AppState,
};

/// Duration of the free Pro trial granted on first run.
const TRIAL_DAYS: u64 = 14;
/// Seconds in a day.
const DAY_SECS: u64 = 86_400;

/// PEM-encoded RSA public key used to verify license JWTs. **Placeholder**
/// for development — replace with the real GA issuer pubkey before
/// shipping. Keeping the constant in-tree means offline machines can verify
/// without phoning home; the tradeoff is that key rotation requires a
/// NanoCrew Sync app update.
const ISSUER_PUBKEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAu1SU1LfVLPHCozMxH2Mo
4lgOEePzNm0tRgeLezV6ffAt0gunVTLw7onLRnrq0/IzW7yWR7QkrmBL7jTKEn5u
+qKhbwKfBstIs+bMY2Zkp18gnTxKLxoS2tFczGkPLPgizskuemMghRniWaoLcyeh
kd3qqGElvW/VDL5AaWTg0nLVkjRo9z+40RQzuVaE8AkAFmxZzow3x+VJYKdjykkJ
0iT9wCS0DRTXu269V264Vf/3jvredZiKRkgwlL9xNAwxXFg0x/XFw005UWVRIkdg
cKWTjpBP2dPwVZ4WWC+9aGVd+Gyn1o0CLelf4rEjGoXbAAEgAqeGUxrcIlbjXfbc
mwIDAQAB
-----END PUBLIC KEY-----";

/// Top-level claims we extract from the JWT. The issuer is free to add
/// more; unknown fields are ignored by `serde`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseClaims {
    /// Short id for support ("NC-2026-AX9…").
    pub key_id: String,
    /// Tier string — one of `personal`, `pro`, `team` (case-insensitive).
    pub tier: String,
    /// Seat count for team licenses; 1 for personal.
    pub seats: u32,
    /// `iat` seconds-since-epoch — issued-at.
    pub iat: u64,
    /// `exp` seconds-since-epoch — license expiry.
    pub exp: u64,
    /// How many distinct machine fingerprints may bind to this key.
    pub machine_limit: u32,
    /// Optional bound fingerprint (SHA-256 hex of machine id). Empty = the
    /// key hasn't been bound yet and will bind to the first activating
    /// machine. We honor the bind on our side but the issuer is the source
    /// of truth for how many machines have redeemed a key.
    #[serde(default)]
    pub machine_fingerprint: String,
    /// Billing email, for support lookups. Optional.
    #[serde(default)]
    pub email: String,
}

/// Status the Settings/About screen renders from. Computed fresh on each
/// `get_license_status` call so trial countdown + expiry update without
/// restart.
#[derive(Debug, Clone, Serialize)]
pub struct LicenseStatus {
    /// `"trial"` / `"free"` / `"personal"` / `"pro"` / `"team"` / `"expired"`.
    pub tier: String,
    /// Is the tier granting Pro-level features right now?
    pub is_pro: bool,
    /// Unix seconds when the current tier ends (trial end, license exp).
    /// `0` = no expiry.
    pub expires_at: u64,
    /// Days remaining in the trial window, or until license exp. `0` = not
    /// applicable or already expired.
    pub days_remaining: u64,
    /// Masked key id if a license is active (e.g. `NC-2026-AX9•••`).
    pub key_id: Option<String>,
    /// Billing email if present in claims. Useful for support CTAs.
    pub email: Option<String>,
    /// Machine fingerprint for this install (SHA-256 hex, first 16 chars).
    /// Shown on the About screen so users can tell support which one they
    /// activated.
    pub machine_fingerprint_short: String,
}

/// SHA-256 hex of `"nanocrew-sync-v1|" || machine_id`. Stable per OS image.
/// Phase 9.3 specifies "hardware UUID + MAC hash" but `machine_id` already
/// derives from `HKLM\...\MachineGuid` which is hardware-bound and survives
/// NanoCrew reinstalls; adding MAC would only add churn on NIC changes.
pub fn machine_fingerprint() -> String {
    let mid = crate::file_lock::machine_id();
    let mut h = Sha256::new();
    h.update(b"nanocrew-sync-v1|");
    h.update(mid.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Verify a JWT against the embedded issuer pubkey. Checks signature,
/// expiry, and (if non-empty) the bound machine fingerprint. Returns the
/// validated claims or a human-readable error.
pub fn verify_jwt(jwt: &str) -> Result<LicenseClaims, String> {
    let key = DecodingKey::from_rsa_pem(ISSUER_PUBKEY_PEM.as_bytes())
        .map_err(|e| format!("issuer key invalid: {e}"))?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = true;
    // We don't set an audience — issuer is implicit (it's our own pubkey).
    let data = decode::<LicenseClaims>(jwt, &key, &validation)
        .map_err(|e| format!("invalid license: {e}"))?;
    let claims = data.claims;
    if !claims.machine_fingerprint.is_empty() {
        let ours = machine_fingerprint();
        if claims.machine_fingerprint != ours {
            return Err("license is bound to a different machine".into());
        }
    }
    Ok(claims)
}

/// Compute the effective status from what's in prefs. Never errors — we
/// always fall back to `free`/`trial` so the UI can still render.
pub fn compute_status(db: &Mutex<rusqlite::Connection>) -> LicenseStatus {
    let now = now_secs();
    let fp_short: String = machine_fingerprint().chars().take(16).collect();

    // Ensure install_at is recorded.
    let install_at: u64 = {
        match prefs::get(db, "install_at").and_then(|s| s.parse::<u64>().ok()) {
            Some(v) => v,
            None => {
                let conn = db.lock().unwrap_or_else(|p| p.into_inner());
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO prefs (key, value) VALUES ('install_at', ?1)",
                    rusqlite::params![now.to_string()],
                );
                now
            }
        }
    };

    // Priority: active valid JWT > trial window > free.
    if let Some(jwt) = prefs::get(db, "license_jwt") {
        match verify_jwt(&jwt) {
            Ok(claims) => {
                let tier = claims.tier.to_lowercase();
                let is_pro = matches!(tier.as_str(), "pro" | "team" | "personal");
                let days_remaining = claims.exp.saturating_sub(now) / DAY_SECS;
                return LicenseStatus {
                    tier,
                    is_pro,
                    expires_at: claims.exp,
                    days_remaining,
                    key_id: Some(mask_key_id(&claims.key_id)),
                    email: if claims.email.is_empty() { None } else { Some(claims.email) },
                    machine_fingerprint_short: fp_short,
                };
            }
            Err(_) => {
                // Invalid/expired — fall through to trial/free logic.
            }
        }
    }

    let trial_end = install_at + TRIAL_DAYS * DAY_SECS;
    if now < trial_end {
        return LicenseStatus {
            tier: "trial".into(),
            is_pro: true,
            expires_at: trial_end,
            days_remaining: (trial_end - now) / DAY_SECS + 1,
            key_id: None,
            email: None,
            machine_fingerprint_short: fp_short,
        };
    }

    LicenseStatus {
        tier: "free".into(),
        is_pro: false,
        expires_at: 0,
        days_remaining: 0,
        key_id: None,
        email: None,
        machine_fingerprint_short: fp_short,
    }
}

/// Show the first 8 chars then mask — keeps the support-visible prefix
/// ("NC-2026-…") without exposing the whole key in screenshots.
fn mask_key_id(id: &str) -> String {
    if id.len() <= 8 {
        return id.to_string();
    }
    let prefix: String = id.chars().take(8).collect();
    format!("{prefix}•••")
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_license_status(
    state: State<'_, AppState>,
    token: String,
) -> Result<LicenseStatus, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    Ok(compute_status(&state.db))
}

#[tauri::command]
pub async fn activate_license(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
    license_jwt: String,
) -> Result<LicenseStatus, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let trimmed = license_jwt.trim();
    if trimmed.is_empty() {
        return Err("please paste your license key".into());
    }
    // Verify *before* persisting so bad keys don't overwrite a good one.
    let claims = verify_jwt(trimmed)?;
    {
        let conn = state.db.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO prefs (key, value) VALUES ('license_jwt', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![trimmed],
        )
        .map_err(|e| format!("persist license: {e}"))?;
    }
    let actor = state
        .sessions
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&token)
        .map(|s| s.username.clone());
    activity::record(
        &state.db, &app, "license", "activate", activity::SEV_INFO,
        None, actor.as_deref(), None, Some(&claims.key_id),
    );
    Ok(compute_status(&state.db))
}

/// Remove the stored license — e.g. when moving the license to a new
/// machine. Returns the fresh status (will drop to `trial` or `free`).
#[tauri::command]
pub async fn deactivate_license(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
) -> Result<LicenseStatus, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    {
        let conn = state.db.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute("DELETE FROM prefs WHERE key = 'license_jwt'", [])
            .map_err(|e| format!("clear license: {e}"))?;
    }
    let actor = state
        .sessions
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&token)
        .map(|s| s.username.clone());
    activity::record(
        &state.db, &app, "license", "deactivate", activity::SEV_INFO,
        None, actor.as_deref(), None, None,
    );
    Ok(compute_status(&state.db))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_hex_and_stable() {
        let a = machine_fingerprint();
        let b = machine_fingerprint();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn mask_key_id_truncates() {
        assert_eq!(mask_key_id("NC-2026-AX9-VERY-LONG"), "NC-2026-•••");
        assert_eq!(mask_key_id("short"), "short");
    }

    #[test]
    fn rejects_garbage_jwt() {
        assert!(verify_jwt("not-a-jwt").is_err());
        assert!(verify_jwt("").is_err());
    }
}

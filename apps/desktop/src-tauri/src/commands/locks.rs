//! Cross-device lock listing + admin "break lock" action.
//!
//! `list_file_locks` enumerates every sentinel under `.nanocrew/locks/` in
//! the drive's bucket and returns the subset that's still within its TTL.
//! The File Browser uses this to render a red padlock next to locked files.
//!
//! `break_file_lock` force-releases a sentinel — the admin escape hatch
//! when another machine crashed mid-upload (e.g. VPN drop, power loss) and
//! the 15-minute TTL is too long to wait. Records an audit row tagged
//! `mount/break_lock` so there's a trail of who broke what.

use aws_sdk_s3::Client;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{
    auth::require_auth,
    commands::activity,
    credentials,
    error::AppError,
    file_lock::{self, LOCK_PREFIX},
    http_client,
    state::AppState,
};

/// A single active sentinel, flattened for the frontend. Expired sentinels
/// are filtered out server-side — the File Browser shouldn't paint padlocks
/// for locks that will never reject a writer.
#[derive(Debug, Clone, Serialize)]
pub struct FileLockEntry {
    /// The key the sentinel protects (e.g. `docs/report.docx`).
    pub key: String,
    /// Writer's machine GUID. Matches ours → `is_ours = true`.
    pub machine: String,
    /// Human-readable owner (username at acquire time).
    pub owner: String,
    pub acquired_at: u64,
    pub expires_at: u64,
    pub is_ours: bool,
}

async fn build_client_for_drive(
    state: &State<'_, AppState>,
    drive_id: i64,
) -> Result<(Client, String), String> {
    let (endpoint, bucket, region, aki) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT endpoint, bucket, region, access_key_id FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            )),
        )
        .map_err(|_| AppError::DriveNotFound.to_string())?
    };

    let secret = credentials::retrieve(&state.db, drive_id).map_err(|e| e.to_string())?;
    let creds = aws_credential_types::Credentials::new(
        aki, secret, None, None, "nanocrew-sync",
    );
    let http = http_client::build_from_prefs(&state.db).map_err(|e| e.to_string())?;
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region))
        .endpoint_url(format!("https://{}", endpoint))
        .credentials_provider(creds)
        .http_client(http)
        .load()
        .await;
    Ok((aws_sdk_s3::Client::new(&config), bucket))
}

/// List all live sentinels for a drive. Read-only — cheap enough to call on
/// File Browser load and on refresh. O(sentinels), not O(files).
#[tauri::command]
pub async fn list_file_locks(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
) -> Result<Vec<FileLockEntry>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let (client, bucket) = build_client_for_drive(&state, drive_id).await?;
    let our_machine = file_lock::machine_id();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut out = Vec::new();
    let mut continuation: Option<String> = None;
    loop {
        let mut req = client.list_objects_v2().bucket(&bucket).prefix(LOCK_PREFIX);
        if let Some(ref tok) = continuation {
            req = req.continuation_token(tok);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = e.to_string();
                // NoSuchBucket or permission errors — treat as "no locks".
                if msg.contains("NoSuchKey") || msg.contains("NoSuchBucket") {
                    return Ok(Vec::new());
                }
                return Err(format!("list sentinels: {msg}"));
            }
        };

        for obj in resp.contents() {
            let Some(sk) = obj.key() else { continue };
            // Fetch + parse the sentinel body. A corrupt sentinel is silently
            // skipped — matches `file_lock::check`'s "treat as free" semantics.
            let body = match client
                .get_object()
                .bucket(&bucket)
                .key(sk)
                .send()
                .await
            {
                Ok(r) => match r.body.collect().await {
                    Ok(b) => b.into_bytes(),
                    Err(_) => continue,
                },
                Err(_) => continue,
            };
            let s: file_lock::Sentinel = match serde_json::from_slice(&body) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if s.expires_at <= now { continue; }
            out.push(FileLockEntry {
                is_ours: s.machine == our_machine,
                key: s.key,
                machine: s.machine,
                owner: s.owner,
                acquired_at: s.acquired_at,
                expires_at: s.expires_at,
            });
        }

        if resp.is_truncated().unwrap_or(false) {
            continuation = resp.next_continuation_token().map(str::to_owned);
        } else {
            break;
        }
    }
    Ok(out)
}

/// Force-release a sentinel. Audit-logged as `mount/break_lock` with the
/// acting user + affected key. The admin escape-hatch for orphaned locks.
#[tauri::command]
pub async fn break_file_lock(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
    drive_id: i64,
    key: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let actor = state
        .sessions
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&token)
        .map(|s| s.username.clone());

    let (client, bucket) = build_client_for_drive(&state, drive_id).await?;
    file_lock::release(&client, &bucket, &key).await?;

    activity::record(
        &state.db, &app, "mount", "break_lock", activity::SEV_WARN,
        Some(drive_id), actor.as_deref(), None, Some(&key),
    );
    Ok(())
}

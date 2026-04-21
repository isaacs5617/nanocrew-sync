use tauri::{AppHandle, Emitter, State};

use crate::{
    auth::require_auth,
    credentials,
    error::AppError,
    mounts::{self, MountConfig},
    state::AppState,
    types::{AddDriveInput, DriveInfo, DriveStatusPayload, S3Entry, TestConnectionInput},
};

// ── Drive CRUD ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_drives(
    state: State<'_, AppState>,
    token: String,
) -> Result<Vec<DriveInfo>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    // Collect rows first, then drop the DB lock before acquiring mounts lock
    // to avoid AB/BA deadlock (mount_drive acquires mounts then db).
    let rows: Vec<(i64, String, String, String, String, String, String, String, i64, bool, bool, i64)> = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = db
            .prepare(
                "SELECT id, name, provider, endpoint, bucket, region, letter,
                        access_key_id, cache_size_gb, auto_mount, readonly, created_at
                 FROM drives ORDER BY created_at",
            )
            .map_err(|e| AppError::Db(e).to_string())?;

        let result: Result<Vec<_>, _> = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, bool>(9)?,
                    r.get::<_, bool>(10)?,
                    r.get::<_, i64>(11)?,
                ))
            })
            .map_err(|e| AppError::Db(e).to_string())?
            .collect();

        result.map_err(|e| AppError::Db(e).to_string())?
    }; // db lock released here

    let mount_map = state.mounts.lock().unwrap_or_else(|p| p.into_inner());
    let drives = rows
        .into_iter()
        .map(|(id, name, provider, endpoint, bucket, region, letter, aki, csz, am, ro, ca)| {
            let status = if mount_map.contains_key(&id) { "mounted" } else { "offline" }.to_string();
            DriveInfo { id, name, provider, endpoint, bucket, region, letter, access_key_id: aki, cache_size_gb: csz, auto_mount: am, readonly: ro, created_at: ca, status }
        })
        .collect();

    Ok(drives)
}

#[tauri::command]
pub async fn add_drive(
    state: State<'_, AppState>,
    token: String,
    input: AddDriveInput,
) -> Result<DriveInfo, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    validate_letter(&input.letter)?;

    let letter = input.letter.to_uppercase();

    // Insert the row with an empty secret placeholder, then let credentials::store
    // write the DPAPI-wrapped blob. Two-step so the secret is never plaintext
    // in SQLite even briefly.
    let (id, created_at) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.execute(
            "INSERT INTO drives
             (name, provider, endpoint, bucket, region, letter,
              access_key_id, secret_key, cache_size_gb, auto_mount, readonly)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'',?8,?9,?10)",
            rusqlite::params![
                input.name, input.provider, input.endpoint, input.bucket,
                input.region, letter, input.access_key_id,
                input.cache_size_gb, input.auto_mount, input.readonly,
            ],
        )
        .map_err(|e| AppError::Db(e).to_string())?;

        let id = db.last_insert_rowid();
        let created_at: i64 = db
            .query_row("SELECT created_at FROM drives WHERE id = ?1", [id], |r| r.get(0))
            .map_err(|e| AppError::Db(e).to_string())?;
        (id, created_at)
    };

    // Write the wrapped secret. If this fails we back the row out so the
    // user doesn't end up with an un-mountable drive in the UI.
    if let Err(e) = credentials::store(&state.db, id, &input.secret_access_key) {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let _ = db.execute("DELETE FROM drives WHERE id = ?1", rusqlite::params![id]);
        return Err(e.to_string());
    }

    Ok(DriveInfo {
        id,
        name: input.name,
        provider: input.provider,
        endpoint: input.endpoint,
        bucket: input.bucket,
        region: input.region,
        letter,
        access_key_id: input.access_key_id,
        cache_size_gb: input.cache_size_gb,
        auto_mount: input.auto_mount,
        readonly: input.readonly,
        created_at,
        status: "offline".into(),
    })
}

#[tauri::command]
pub async fn remove_drive(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    if state.mounts.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&drive_id) {
        return Err(AppError::DriveStillMounted.to_string());
    }

    // Delete from DB first; a stale orphan credential in keyring is harmless,
    // but a DB row pointing at a missing credential causes permanent mount failure.
    {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.execute("DELETE FROM drives WHERE id = ?1", rusqlite::params![drive_id])
            .map_err(|e| AppError::Db(e).to_string())?;
    }

    credentials::delete(&state.db, drive_id).map_err(|e| e.to_string())?;

    Ok(())
}

// ── Mount / unmount ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn mount_drive(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
    drive_id: i64,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    if state.mounts.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&drive_id) {
        return Err(AppError::AlreadyMounted.to_string());
    }

    let (_name, provider, endpoint, bucket, region, letter, aki, readonly) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT name,provider,endpoint,bucket,region,letter,access_key_id,readonly
             FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, bool>(7)?,
            )),
        )
        .map_err(|_| AppError::DriveNotFound.to_string())?
    }; // db lock released before keyring and thread spawn

    let secret = credentials::retrieve(&state.db, drive_id).map_err(|e| e.to_string())?;

    // Emit "mounting" immediately so the UI responds
    let _ = app.emit(
        "drive_status_changed",
        DriveStatusPayload { drive_id, status: "mounting".into(), message: None },
    );

    // spawn_mount blocks until WinFsp is up; run it off the async runtime.
    let mount_config = MountConfig {
        drive_id, letter, provider, endpoint, bucket, region,
        access_key_id: aki, secret_access_key: secret, readonly,
    };
    let app2 = app.clone();
    let handle = tokio::task::spawn_blocking(move || mounts::spawn_mount(mount_config, app2))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    // "mounted" event is emitted by the WinFsp thread on success
    state.mounts.lock().unwrap_or_else(|p| p.into_inner()).insert(drive_id, handle);

    Ok(())
}

#[tauri::command]
pub async fn unmount_drive(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
    drive_id: i64,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    let handle = state
        .mounts
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&drive_id)
        .ok_or_else(|| "Drive is not mounted".to_string())?;

    handle.stop();

    let _ = app.emit(
        "drive_status_changed",
        DriveStatusPayload { drive_id, status: "offline".into(), message: None },
    );

    Ok(())
}

// ── Utilities ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    token: String,
    input: TestConnectionInput,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    let creds = aws_credential_types::Credentials::new(
        input.access_key_id,
        input.secret_access_key,
        None,
        None,
        "nanocrew-sync",
    );

    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(input.region))
        .endpoint_url(format!("https://{}", input.endpoint))
        .credentials_provider(creds)
        .load()
        .await;

    let client = aws_sdk_s3::Client::new(&config);

    client
        .list_objects_v2()
        .bucket(&input.bucket)
        .max_keys(1)
        .send()
        .await
        .map_err(|e| AppError::ConnectionTest(e.to_string()).to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn get_available_letters(
    state: State<'_, AppState>,
    token: String,
) -> Result<Vec<String>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    let in_use = used_drive_letters();

    let configured: std::collections::HashSet<String> = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = db
            .prepare("SELECT letter FROM drives")
            .map_err(|e| AppError::Db(e).to_string())?;
        let result: Result<Vec<_>, _> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| AppError::Db(e).to_string())?
            .collect();
        result
            .map_err(|e| AppError::Db(e).to_string())?
            .into_iter()
            .map(|l: String| l.to_uppercase())
            .collect()
    };

    let available = ('D'..='Z')
        .map(|c| format!("{c}:"))
        .filter(|l| !in_use.contains(l) && !configured.contains(l))
        .collect();

    Ok(available)
}

// ── Bucket browser ───────────────────────────────────────────────────────────

/// List the objects/directories directly under `prefix` in a drive's bucket.
/// `prefix` should be empty for the root, or end with `/` for a subdirectory.
#[tauri::command]
pub async fn list_drive_objects(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
    prefix: String,
) -> Result<Vec<S3Entry>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

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
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region))
        .endpoint_url(format!("https://{}", endpoint))
        .credentials_provider(creds)
        .load()
        .await;

    let client = aws_sdk_s3::Client::new(&config);

    let mut entries: Vec<S3Entry> = Vec::new();
    let mut continuation: Option<String> = None;

    loop {
        let mut req = client
            .list_objects_v2()
            .bucket(&bucket)
            .delimiter("/");
        if !prefix.is_empty() {
            req = req.prefix(&prefix);
        }
        if let Some(ref tok) = continuation {
            req = req.continuation_token(tok);
        }

        let resp = req.send().await.map_err(|e| AppError::ConnectionTest(e.to_string()).to_string())?;

        // Common prefixes → directories
        for cp in resp.common_prefixes() {
            let Some(p) = cp.prefix() else { continue };
            let name = p.strip_prefix(&prefix).unwrap_or(p)
                .trim_end_matches('/').to_string();
            if name.is_empty() || name.contains('/') { continue; }
            entries.push(S3Entry { name, key: p.to_string(), is_dir: true, size: 0, modified: 0 });
        }

        // Object keys → files
        for obj in resp.contents() {
            let Some(key) = obj.key() else { continue };
            let name = key.strip_prefix(&prefix).unwrap_or(key).to_string();
            if name.is_empty() || name.contains('/') || name.ends_with('/') { continue; }
            let size = obj.size().unwrap_or(0).max(0);
            let modified = obj.last_modified().map(|d| d.secs()).unwrap_or(0);
            entries.push(S3Entry { name, key: key.to_string(), is_dir: false, size, modified });
        }

        if resp.is_truncated().unwrap_or(false) {
            continuation = resp.next_continuation_token().map(str::to_owned);
        } else {
            break;
        }
    }

    Ok(entries)
}

// ── Bucket discovery ─────────────────────────────────────────────────────────

/// List all buckets accessible with the given credentials (used in Add Drive flow).
#[tauri::command]
pub async fn list_buckets(
    state: State<'_, AppState>,
    token: String,
    endpoint: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
) -> Result<Vec<String>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    let creds = aws_credential_types::Credentials::new(
        access_key_id, secret_access_key, None, None, "nanocrew-sync",
    );
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region))
        .endpoint_url(format!("https://{}", endpoint))
        .credentials_provider(creds)
        .load()
        .await;

    let client = aws_sdk_s3::Client::new(&config);
    let resp = client
        .list_buckets()
        .send()
        .await
        .map_err(|e| AppError::ConnectionTest(e.to_string()).to_string())?;

    let names = resp
        .buckets()
        .iter()
        .filter_map(|b| b.name().map(str::to_owned))
        .collect();

    Ok(names)
}

// ── System checks ────────────────────────────────────────────────────────────

/// Returns true if the filesystem driver is available. With the Cloud Filter
/// backend this is built into Windows 10 1709+ and always available.
#[tauri::command]
pub async fn check_winfsp(
    state: State<'_, AppState>,
    token: String,
) -> Result<bool, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    Ok(true)
}

// ── Shell helpers ─────────────────────────────────────────────────────────────

/// Open a path or URL in the default Windows application (Explorer, browser, etc.)
#[tauri::command]
pub async fn open_path(
    state: State<'_, AppState>,
    token: String,
    path: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    // explorer.exe handles both file paths and https:// URLs reliably.
    // cmd /c start "" <url> is fragile when the empty-string arg is passed as a
    // separate argument via the Rust Command API on some Windows versions.
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Private helpers ──────────────────────────────────────────────────────────

fn validate_letter(letter: &str) -> Result<(), String> {
    let up = letter.to_uppercase();
    let valid = up.len() == 2
        && up.ends_with(':')
        && up.starts_with(|c: char| ('D'..='Z').contains(&c));
    if !valid {
        return Err(AppError::InvalidInput("Drive letter must be D: through Z:".into()).to_string());
    }
    Ok(())
}

/// Returns the set of drive letters currently in use on this Windows machine.
/// Falls back gracefully on any error.
fn used_drive_letters() -> std::collections::HashSet<String> {
    let Ok(output) = std::process::Command::new("fsutil")
        .args(["fsinfo", "drives"])
        .output()
    else {
        return Default::default();
    };

    // Output: "Drives: C:\ D:\ E:\"
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter(|s| s.len() >= 2 && s.ends_with('\\'))
        .map(|s| s[..2].to_uppercase())
        .collect()
}

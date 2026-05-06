use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    auth::require_auth,
    commands::activity,
    credentials,
    error::AppError,
    http_client,
    mounts::{self, MountConfig},
    state::AppState,
    types::{AddDriveInput, DriveInfo, DriveStatusPayload, S3Entry, TestConnectionInput},
};

/// Look up the username tied to `token`, if any — used purely for activity-log
/// attribution.
fn actor_for(state: &State<'_, AppState>, token: &str) -> Option<String> {
    state
        .sessions
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(token)
        .map(|s| s.username.clone())
}

// ── Drive CRUD ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_drives(
    state: State<'_, AppState>,
    token: String,
) -> Result<Vec<DriveInfo>, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    // Collect rows first, then drop the DB lock before acquiring mounts lock
    // to avoid AB/BA deadlock (mount_drive acquires mounts then db).
    let rows: Vec<(i64, String, String, String, String, String, String, String, i64, bool, bool, i64, String)> = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = db
            .prepare(
                "SELECT id, name, provider, endpoint, bucket, region, letter,
                        access_key_id, cache_size_gb, auto_mount, readonly, created_at,
                        COALESCE(bucket_prefix, '')
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
                    r.get::<_, String>(12)?,
                ))
            })
            .map_err(|e| AppError::Db(e).to_string())?
            .collect();

        result.map_err(|e| AppError::Db(e).to_string())?
    }; // db lock released here

    let mount_map = state.mounts.lock().unwrap_or_else(|p| p.into_inner());
    let drives = rows
        .into_iter()
        .map(|(id, name, provider, endpoint, bucket, region, letter, aki, csz, am, ro, ca, bp)| {
            let status = if mount_map.contains_key(&id) { "mounted" } else { "offline" }.to_string();
            DriveInfo { id, name, provider, endpoint, bucket, bucket_prefix: bp, region, letter, access_key_id: aki, cache_size_gb: csz, auto_mount: am, readonly: ro, created_at: ca, status }
        })
        .collect();

    Ok(drives)
}

#[tauri::command]
pub async fn add_drive(
    state: State<'_, AppState>,
    app: AppHandle,
    token: String,
    input: AddDriveInput,
) -> Result<DriveInfo, String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    validate_letter(&input.letter)?;
    let actor = actor_for(&state, &token);

    let letter = input.letter.to_uppercase();
    // Normalise prefix: no leading slash, always trailing slash if non-empty.
    let bucket_prefix = normalise_prefix(&input.bucket_prefix);

    // Insert the row with an empty secret placeholder, then let credentials::store
    // write the DPAPI-wrapped blob. Two-step so the secret is never plaintext
    // in SQLite even briefly.
    let (id, created_at) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.execute(
            "INSERT INTO drives
             (name, provider, endpoint, bucket, bucket_prefix, region, letter,
              access_key_id, secret_key, cache_size_gb, auto_mount, readonly)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'',?8,?9,?10,?11)",
            rusqlite::params![
                input.name, input.provider, input.endpoint, input.bucket, bucket_prefix,
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

    activity::record(
        &state.db, &app, "drive", "add_drive", activity::SEV_INFO,
        Some(id), actor.as_deref(),
        Some(&format!("{letter} {}", input.name)),
        Some(&format!("{} — {}", input.provider, input.bucket)),
    );

    Ok(DriveInfo {
        id,
        name: input.name,
        provider: input.provider,
        endpoint: input.endpoint,
        bucket: input.bucket,
        bucket_prefix,
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
    app: AppHandle,
    token: String,
    drive_id: i64,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;
    let actor = actor_for(&state, &token);

    if state.mounts.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&drive_id) {
        return Err(AppError::DriveStillMounted.to_string());
    }

    // Pull the display name before we delete, so the activity entry has a
    // human-friendly target rather than a bare row id.
    let label: Option<String> = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT letter || ' ' || name FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| r.get::<_, String>(0),
        )
        .ok()
    };

    // Delete from DB first; a stale orphan credential in keyring is harmless,
    // but a DB row pointing at a missing credential causes permanent mount failure.
    {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.execute("DELETE FROM drives WHERE id = ?1", rusqlite::params![drive_id])
            .map_err(|e| AppError::Db(e).to_string())?;
    }

    credentials::delete(&state.db, drive_id).map_err(|e| e.to_string())?;

    activity::record(
        &state.db, &app, "drive", "remove_drive", activity::SEV_INFO,
        Some(drive_id), actor.as_deref(), label.as_deref(), None,
    );

    Ok(())
}

/// Update a drive's `bucket_prefix` without touching credentials.
/// The drive must be unmounted first.
#[tauri::command]
pub async fn set_drive_prefix(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
    bucket_prefix: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    if state.mounts.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&drive_id) {
        return Err("Unmount the drive first before changing its prefix.".into());
    }

    let prefix = normalise_prefix(&bucket_prefix);
    state.db.lock().unwrap_or_else(|p| p.into_inner())
        .execute(
            "UPDATE drives SET bucket_prefix = ?1 WHERE id = ?2",
            rusqlite::params![prefix, drive_id],
        )
        .map_err(|e| AppError::Db(e).to_string())?;

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

    // Grab the signed-in username so cross-device sentinel locks are tagged
    // with "owner = <that user>" rather than an opaque GUID.
    let owner = state
        .sessions
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&token)
        .map(|s| s.username.clone())
        .unwrap_or_else(|| "user".to_string());

    if state.mounts.lock().unwrap_or_else(|p| p.into_inner()).contains_key(&drive_id) {
        return Err(AppError::AlreadyMounted.to_string());
    }

    let (_name, provider, endpoint, bucket, bucket_prefix, region, letter, aki, readonly, cache_size_gb) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT name,provider,endpoint,bucket,COALESCE(bucket_prefix,''),region,letter,
                    access_key_id,readonly,cache_size_gb
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
                r.get::<_, String>(7)?,
                r.get::<_, bool>(8)?,
                r.get::<_, i64>(9)?,
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

    // Clone before moving into MountConfig so the activity entry after the
    // await still has the fields it needs.
    let letter_for_log = letter.clone();
    let owner_for_log = owner.clone();

    // spawn_mount blocks until WinFsp is up; run it off the async runtime.
    // Bandwidth caps — prefs store MB/s, MountConfig carries B/s.
    let upload_rate_bps = crate::commands::prefs::get_rate_bps(&state.db, "upload_rate_mbps");
    let download_rate_bps = crate::commands::prefs::get_rate_bps(&state.db, "download_rate_mbps");

    // Disk cache (Phase 5.6). `cache_enabled` pref defaults to on; size comes
    // from the per-drive `drives.cache_size_gb` column.
    let cache_enabled = crate::commands::prefs::get_bool(&state.db, "cache_enabled", true);
    let cache_max_bytes = (cache_size_gb.max(0) as u64).saturating_mul(1_073_741_824);
    let db_path = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?
        .join("nanocrew.db");
    // Cache location (Phase 7.4). `cache_root` pref overrides the
    // LOCALAPPDATA default; the per-drive subdirectory is appended in
    // `mounts::spawn_mount`.
    let cache_root = crate::commands::prefs::get_cache_root(&state.db)
        .ok_or_else(|| "LOCALAPPDATA not set — cannot resolve cache root".to_string())?;

    let mount_config = MountConfig {
        drive_id, letter, provider, endpoint, bucket, bucket_prefix, region,
        access_key_id: aki, secret_access_key: secret, readonly,
        owner,
        upload_rate_bps,
        download_rate_bps,
        cache_enabled,
        cache_max_bytes,
        db_path,
        cache_root,
    };
    let app2 = app.clone();
    let handle = tokio::task::spawn_blocking(move || mounts::spawn_mount(mount_config, app2))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    // "mounted" event is emitted by the WinFsp thread on success
    state.mounts.lock().unwrap_or_else(|p| p.into_inner()).insert(drive_id, handle);

    activity::record(
        &state.db, &app, "mount", "mount", activity::SEV_INFO,
        Some(drive_id), Some(&owner_for_log),
        Some(&letter_for_log), None,
    );

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

    let actor = actor_for(&state, &token);

    let handle = state
        .mounts
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&drive_id)
        .ok_or_else(|| "Drive is not mounted".to_string())?;

    // Best-effort letter lookup for the log entry.
    let letter: Option<String> = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT letter FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| r.get::<_, String>(0),
        )
        .ok()
    };

    handle.stop();

    let _ = app.emit(
        "drive_status_changed",
        DriveStatusPayload { drive_id, status: "offline".into(), message: None },
    );

    activity::record(
        &state.db, &app, "mount", "unmount", activity::SEV_INFO,
        Some(drive_id), actor.as_deref(), letter.as_deref(), None,
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

    let http = http_client::build_from_prefs(&state.db).map_err(|e| e.to_string())?;
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(input.region))
        .endpoint_url(format!("https://{}", input.endpoint))
        .credentials_provider(creds)
        .http_client(http)
        .load()
        .await;

    let client = aws_sdk_s3::Client::new(&config);

    let volume_prefix = normalise_prefix(&input.bucket_prefix);
    let mut req = client.list_objects_v2().bucket(&input.bucket).max_keys(1);
    if !volume_prefix.is_empty() {
        req = req.prefix(&volume_prefix);
    }
    req.send()
        .await
        .map_err(|e| {
            // Extract the S3 error code + message from the SDK's service-error
            // metadata so prettifyError can match on "AccessDenied", "403", etc.
            // The default e.to_string() only yields the useless "service error".
            let detail = e.as_service_error()
                .map(|se| {
                    let code = se.meta().code().unwrap_or("service_error");
                    let msg  = se.meta().message().unwrap_or("no message");
                    format!("{code}: {msg}")
                })
                .unwrap_or_else(|| e.to_string());
            AppError::ConnectionTest(detail).to_string()
        })?;

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

    let (endpoint, bucket, bucket_prefix_raw, region, aki) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT endpoint, bucket, COALESCE(bucket_prefix,''), region, access_key_id
             FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            )),
        )
        .map_err(|_| AppError::DriveNotFound.to_string())?
    };
    // Build the absolute S3 prefix by prepending bucket_prefix to the
    // caller-supplied directory prefix.
    let volume_prefix = normalise_prefix(&bucket_prefix_raw);
    let abs_prefix = format!("{volume_prefix}{prefix}");

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

    let client = aws_sdk_s3::Client::new(&config);

    let mut entries: Vec<S3Entry> = Vec::new();
    let mut continuation: Option<String> = None;

    loop {
        let mut req = client
            .list_objects_v2()
            .bucket(&bucket)
            .delimiter("/");
        if !abs_prefix.is_empty() {
            req = req.prefix(&abs_prefix);
        }
        if let Some(ref tok) = continuation {
            req = req.continuation_token(tok);
        }

        let resp = req.send().await.map_err(|e| AppError::ConnectionTest(e.to_string()).to_string())?;

        // Common prefixes → directories. Strip the absolute prefix so keys
        // returned to the frontend are relative to the volume root.
        for cp in resp.common_prefixes() {
            let Some(p) = cp.prefix() else { continue };
            let rel_key = p.strip_prefix(&volume_prefix).unwrap_or(p);
            let name = rel_key.strip_prefix(&prefix).unwrap_or(rel_key)
                .trim_end_matches('/').to_string();
            if name.is_empty() || name.contains('/') { continue; }
            let rel_full = rel_key.to_string();
            entries.push(S3Entry { name, key: rel_full, is_dir: true, size: 0, modified: 0 });
        }

        // Object keys → files. Strip volume_prefix from keys returned to frontend.
        for obj in resp.contents() {
            let Some(key) = obj.key() else { continue };
            let rel_key = key.strip_prefix(&volume_prefix).unwrap_or(key);
            let name = rel_key.strip_prefix(&prefix).unwrap_or(rel_key).to_string();
            if name.is_empty() || name.contains('/') || name.ends_with('/') || name == ".keep" { continue; }
            let size = obj.size().unwrap_or(0).max(0);
            let modified = obj.last_modified().map(|d| d.secs()).unwrap_or(0);
            entries.push(S3Entry { name, key: rel_key.to_string(), is_dir: false, size, modified });
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
    let http = http_client::build_from_prefs(&state.db).map_err(|e| e.to_string())?;
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region))
        .endpoint_url(format!("https://{}", endpoint))
        .credentials_provider(creds)
        .http_client(http)
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

// ── Folder operations ────────────────────────────────────────────────────────

/// Create a virtual S3 folder by PUT-ing a zero-byte `.keep` marker at
/// `<volume_prefix><prefix><name>/.keep`. Using `.keep` instead of a bare
/// trailing-slash key avoids HTTP 500/400 errors on R2, MinIO, and B2.
/// `prefix` is the current directory prefix (empty = root), relative to the
/// drive's `bucket_prefix`.
#[tauri::command]
pub async fn create_folder(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
    prefix: String,
    name: String,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    let name = name.trim().to_string();
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err("Folder name must not be empty or contain slashes.".into());
    }

    let (endpoint, bucket, bucket_prefix_raw, region, aki) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT endpoint, bucket, COALESCE(bucket_prefix,''), region, access_key_id
             FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            )),
        )
        .map_err(|_| AppError::DriveNotFound.to_string())?
    };

    let secret = credentials::retrieve(&state.db, drive_id).map_err(|e| e.to_string())?;
    let client = build_s3_client(&state.db, &endpoint, &region, &aki, &secret).await?;

    let volume_prefix = normalise_prefix(&bucket_prefix_raw);
    // Use a `.keep` marker instead of a bare trailing-slash key: some S3
    // providers (Cloudflare R2, MinIO, Backblaze B2) reject zero-byte objects
    // whose key ends with `/` with an HTTP 500 or 400.
    let marker_key = format!("{volume_prefix}{prefix}{name}/.keep");

    client
        .put_object()
        .bucket(&bucket)
        .key(&marker_key)
        .content_length(0)
        .body(aws_sdk_s3::primitives::ByteStream::from(bytes::Bytes::new()))
        .send()
        .await
        .map_err(|e| format!("create folder: {e}"))?;

    Ok(())
}

/// Rename a file or folder in S3 (copy + delete; for folders, bulk copy/delete
/// of all keys under the prefix).
/// `old_key` and `new_key` are relative to the drive's bucket_prefix.
#[tauri::command]
pub async fn rename_object(
    state: State<'_, AppState>,
    token: String,
    drive_id: i64,
    old_key: String,
    new_key: String,
    is_dir: bool,
) -> Result<(), String> {
    require_auth(&state, &token).map_err(|e| e.to_string())?;

    let (endpoint, bucket, bucket_prefix_raw, region, aki) = {
        let db = state.db.lock().unwrap_or_else(|p| p.into_inner());
        db.query_row(
            "SELECT endpoint, bucket, COALESCE(bucket_prefix,''), region, access_key_id
             FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            )),
        )
        .map_err(|_| AppError::DriveNotFound.to_string())?
    };

    let secret = credentials::retrieve(&state.db, drive_id).map_err(|e| e.to_string())?;
    let client = build_s3_client(&state.db, &endpoint, &region, &aki, &secret).await?;

    let vp = normalise_prefix(&bucket_prefix_raw);

    if is_dir {
        // Collect all keys under the old prefix (including the marker object).
        let src_prefix = format!("{vp}{old_key}");
        let mut keys_to_move: Vec<String> = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let mut req = client.list_objects_v2().bucket(&bucket).prefix(&src_prefix);
            if let Some(ref tok) = continuation {
                req = req.continuation_token(tok);
            }
            let resp = req.send().await.map_err(|e| format!("list for rename: {e}"))?;
            for obj in resp.contents() {
                if let Some(k) = obj.key() {
                    keys_to_move.push(k.to_string());
                }
            }
            if resp.is_truncated().unwrap_or(false) {
                continuation = resp.next_continuation_token().map(str::to_owned);
            } else {
                break;
            }
        }

        let dest_prefix = format!("{vp}{new_key}");
        for abs_src in &keys_to_move {
            let suffix = abs_src.strip_prefix(&src_prefix).unwrap_or("");
            let abs_dst = format!("{dest_prefix}{suffix}");
            let copy_src = format!("{}/{}", bucket, abs_src);
            client
                .copy_object()
                .bucket(&bucket)
                .copy_source(&copy_src)
                .key(&abs_dst)
                .send()
                .await
                .map_err(|e| format!("copy {abs_src} → {abs_dst}: {e}"))?;
            client
                .delete_object()
                .bucket(&bucket)
                .key(abs_src)
                .send()
                .await
                .map_err(|e| format!("delete {abs_src}: {e}"))?;
        }
    } else {
        let abs_src = format!("{vp}{old_key}");
        let abs_dst = format!("{vp}{new_key}");
        let copy_src = format!("{}/{}", bucket, abs_src);
        client
            .copy_object()
            .bucket(&bucket)
            .copy_source(&copy_src)
            .key(&abs_dst)
            .send()
            .await
            .map_err(|e| format!("copy: {e}"))?;
        client
            .delete_object()
            .bucket(&bucket)
            .key(&abs_src)
            .send()
            .await
            .map_err(|e| format!("delete: {e}"))?;
    }

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

/// Normalise a bucket prefix: strip leading slash, ensure trailing slash if
/// non-empty. `"users/alice"` → `"users/alice/"`, `""` → `""`.
fn normalise_prefix(raw: &str) -> String {
    let s = raw.trim().trim_start_matches('/');
    if s.is_empty() {
        String::new()
    } else if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

/// Build an S3 client from per-drive credentials (shared by listing, folder
/// ops, and rename — avoids copy-pasting the config block).
async fn build_s3_client(
    db: &std::sync::Mutex<rusqlite::Connection>,
    endpoint: &str,
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
) -> Result<aws_sdk_s3::Client, String> {
    let creds = aws_credential_types::Credentials::new(
        access_key_id, secret_access_key, None, None, "nanocrew-sync",
    );
    let http = http_client::build_from_prefs(db).map_err(|e| e.to_string())?;
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()))
        .endpoint_url(format!("https://{}", endpoint))
        .credentials_provider(creds)
        .http_client(http)
        .load()
        .await;
    let s3_conf = aws_sdk_s3::config::Builder::from(&config)
        .force_path_style(true)
        .build();
    Ok(aws_sdk_s3::Client::from_conf(s3_conf))
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

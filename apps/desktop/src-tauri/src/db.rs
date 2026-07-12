use std::path::Path;
use rusqlite::Connection;
use crate::error::AppError;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS accounts (
    id            INTEGER PRIMARY KEY NOT NULL,
    username      TEXT    NOT NULL UNIQUE,
    password_hash TEXT    NOT NULL,
    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS drives (
    id            INTEGER PRIMARY KEY NOT NULL,
    name          TEXT    NOT NULL,
    provider      TEXT    NOT NULL,
    endpoint      TEXT    NOT NULL,
    bucket        TEXT    NOT NULL,
    region        TEXT    NOT NULL,
    letter        TEXT    NOT NULL UNIQUE,
    access_key_id TEXT    NOT NULL,
    secret_key    TEXT    NOT NULL DEFAULT '',
    cache_size_gb INTEGER NOT NULL DEFAULT 5,
    auto_mount    INTEGER NOT NULL DEFAULT 0,
    readonly      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Append-only audit log. Every meaningful event gets a row here so the
-- Activity screen can show history across sessions, not just what happened
-- since the last launch.
CREATE TABLE IF NOT EXISTS activity (
    id        INTEGER PRIMARY KEY NOT NULL,
    ts        INTEGER NOT NULL DEFAULT (unixepoch()),
    kind      TEXT    NOT NULL,  -- auth | drive | mount | file | system | error
    action    TEXT    NOT NULL,  -- sign_in, mount, unmount, add_drive, error, ...
    severity  TEXT    NOT NULL DEFAULT 'info',  -- info | warn | error
    drive_id  INTEGER,
    actor     TEXT,
    target    TEXT,
    message   TEXT
);
CREATE INDEX IF NOT EXISTS idx_activity_ts   ON activity(ts DESC);
CREATE INDEX IF NOT EXISTS idx_activity_kind ON activity(kind);

-- Tiny key/value table for user preferences that aren't worth their own
-- schema (toggles, defaults, picks). String values only — callers serialize
-- anything structured.
CREATE TABLE IF NOT EXISTS prefs (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

-- On-disk range cache index. Each row records a byte range we've fetched
-- from S3 and still have on disk. The filename on disk is derived from
-- sha256(key) + offset + len — see cache.rs. `last_access` is a unix
-- timestamp (seconds) refreshed on every hit; the eviction loop evicts
-- oldest first, skipping any row whose `key` is in `pinned_keys`.
CREATE TABLE IF NOT EXISTS cache_entries (
    drive_id    INTEGER NOT NULL,
    key         TEXT    NOT NULL,
    offset      INTEGER NOT NULL,
    len         INTEGER NOT NULL,
    size_bytes  INTEGER NOT NULL,
    etag        TEXT,
    last_access INTEGER NOT NULL,
    PRIMARY KEY (drive_id, key, offset, len)
);
CREATE INDEX IF NOT EXISTS idx_cache_lru ON cache_entries(drive_id, last_access);
CREATE INDEX IF NOT EXISTS idx_cache_key ON cache_entries(drive_id, key);

-- Per-drive pin list. Pinned keys are exempt from LRU eviction. Matched
-- as exact object keys (case-sensitive, as S3 itself is).
CREATE TABLE IF NOT EXISTS pinned_keys (
    drive_id   INTEGER NOT NULL,
    key        TEXT    NOT NULL,
    pinned_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (drive_id, key)
);
";

pub fn open(path: &Path) -> Result<Connection, AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::InvalidInput(e.to_string()))?;
    }

    // v0.2.15: integrity-check + auto-heal on startup. Corrupted `cache_entries`
    // indexes cause get_block() to misroute lookups, which surfaces to users
    // as silent xlsx/PDF/Word corruption (bytes look right length but come
    // from the wrong on-disk block). Same class of bug destroyed the DB in
    // production on 2026-07-09: B-tree rowid ordering broke, secret_key
    // columns landed the DEFAULT of an unrelated column, drives failed to
    // mount on restart. Detect it once, back the bad file up, and salvage
    // what we can before opening — the alternative is silently reading a
    // damaged file and writing new damage on top.
    if path.exists() {
        if let Err(e) = try_salvage_if_corrupt(path) {
            tracing::warn!(
                target: "nanocrew::db",
                "integrity-check pass failed (non-fatal, continuing): {e}"
            );
        }
    }

    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    // Aggressive WAL checkpointing so the -wal file can't quietly grow into
    // multi-MB territory (we saw a 4 MB WAL accumulate in prod). At 1000
    // pages (~4 MB) SQLite auto-truncates on next commit — cap that at 200
    // pages (~800 KB) so recovery from a crash replays fewer changes.
    let _ = conn.execute_batch("PRAGMA wal_autocheckpoint = 200;");
    // Migration: add secret_key column to existing databases that pre-date it.
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN secret_key TEXT NOT NULL DEFAULT ''",
        [],
    );
    // Migration: add bucket_prefix for subdirectory-volume support.
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN bucket_prefix TEXT NOT NULL DEFAULT ''",
        [],
    );
    // Migration: per-drive cache quota (Track A1). 10 GiB default.
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN cache_max_bytes INTEGER NOT NULL DEFAULT 10737418240",
        [],
    );
    // Migration: per-drive cache enable toggle (Track A1).
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN cache_enabled INTEGER NOT NULL DEFAULT 1",
        [],
    );
    // Migration: per-drive bandwidth overrides (Track E4). 0 = inherit global pref.
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN upload_rate_mbps REAL NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN download_rate_mbps REAL NOT NULL DEFAULT 0",
        [],
    );
    // Migration: provider type tag for non-S3 providers (Track C2).
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN provider_type TEXT NOT NULL DEFAULT 's3'",
        [],
    );
    // Migration: JSON blob for SFTP / FTP config (Track C2).
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN provider_config TEXT",
        [],
    );
    Ok(conn)
}

/// Run `PRAGMA integrity_check`. If the result is anything other than "ok",
/// rename the current DB out of the way (so the next open() creates a fresh
/// one) and best-effort dump the salvageable rows next to it as a JSON blob
/// the user can point at if they want to hand-recover credentials.
fn try_salvage_if_corrupt(path: &Path) -> Result<(), AppError> {
    // Open read-only for the check so we can't accidentally damage a healthy
    // DB while probing it.
    let ro_uri = format!(
        "file:{}?mode=ro",
        path.to_string_lossy().replace('\\', "/")
    );
    let conn = match Connection::open_with_flags(
        &ro_uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) {
        Ok(c) => c,
        Err(_) => return Ok(()), // Can't even open — let normal open() surface the error.
    };

    let result: String = conn
        .query_row("PRAGMA integrity_check(1);", [], |r| r.get(0))
        .unwrap_or_else(|_| "unknown".into());
    drop(conn);

    if result == "ok" {
        return Ok(());
    }

    tracing::error!(
        target: "nanocrew::db",
        "integrity_check FAILED: {result:?} — quarantining {} and salvaging drives",
        path.display()
    );

    // Try to read the salvageable rows before we move the file — accounts +
    // drives are what the user cares about; cache_entries is regeneratable.
    let salvage = attempt_salvage(&ro_uri);

    // Move the corrupted files out of the way. Use a timestamp so multiple
    // corruptions don't clobber earlier evidence.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bad = path.with_file_name(format!("nanocrew.db.corrupted-{ts}"));
    let _ = std::fs::rename(path, &bad);
    for sfx in ["-wal", "-shm"] {
        let src = path.with_extension(format!("db{sfx}"));
        let dst = bad.with_extension(format!("db{sfx}"));
        let _ = std::fs::rename(&src, &dst);
    }

    // Write the salvage report (accounts count + drive configs) next to it
    // so the user can hand-restore if we can't automate it.
    if let Some(json) = salvage {
        let report = path.with_file_name(format!("nanocrew.db.salvage-{ts}.json"));
        let _ = std::fs::write(&report, json.as_bytes());
        tracing::info!(
            target: "nanocrew::db",
            "wrote salvage report to {}",
            report.display()
        );
    }

    Ok(())
}

/// Best-effort dump of the accounts + drives tables into a JSON string.
/// Returns None if we can't read anything usable.
fn attempt_salvage(ro_uri: &str) -> Option<String> {
    let conn = Connection::open_with_flags(
        ro_uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;

    let mut out = String::from("{\n");
    out.push_str(&format!("  \"salvaged_at\": {},\n", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)));

    // Accounts — just the usernames, no password hashes (user needs to
    // reset anyway).
    let usernames: Vec<String> = conn
        .prepare("SELECT username FROM accounts")
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(0))
                .and_then(|it| it.collect())
        })
        .unwrap_or_default();
    out.push_str("  \"accounts\": [");
    out.push_str(
        &usernames
            .iter()
            .map(|u| format!("\"{}\"", u.replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str("],\n");

    // Drives — full config so a hand-recover just needs the plaintext
    // Wasabi secret from the user's password manager.
    let drives: Vec<String> = conn
        .prepare(
            "SELECT id, name, provider, endpoint, bucket, COALESCE(bucket_prefix,'') AS prefix, \
             region, letter, access_key_id FROM drives",
        )
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok(format!(
                    "    {{\"id\": {}, \"name\": \"{}\", \"provider\": \"{}\", \
                     \"endpoint\": \"{}\", \"bucket\": \"{}\", \"bucket_prefix\": \"{}\", \
                     \"region\": \"{}\", \"letter\": \"{}\", \"access_key_id\": \"{}\"}}",
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?.replace('"', "\\\""),
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                ))
            })
            .and_then(|it| it.collect())
        })
        .unwrap_or_default();
    out.push_str("  \"drives\": [\n");
    out.push_str(&drives.join(",\n"));
    out.push_str("\n  ]\n}\n");

    Some(out)
}

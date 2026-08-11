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

    // v0.3.2 recovery: users updated between v0.2.15 and v0.3.1 may have
    // had their DB falsely quarantined by the (now-removed) auto-quarantine
    // code — see `try_salvage_if_corrupt` doc comment. If we now see a
    // tiny fresh DB alongside a `nanocrew.db.corrupted-<ts>` that actually
    // passes integrity_check, swap them back so those users get their
    // drives + accounts restored on next launch. Non-fatal on any error.
    try_restore_from_bad_quarantine(path);

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

/// v0.3.2: probe DB integrity but NEVER rename the file automatically.
///
/// The previous auto-quarantine (v0.2.15 – v0.3.1) destroyed user data in
/// production. Two bugs combined into a disaster:
///   1) The read-only integrity probe couldn't see the un-checkpointed WAL,
///      so it false-positived on healthy DBs.
///   2) The rename step used `Path::with_extension` on a filename whose
///      "extension" from Rust's POV was `corrupted-<ts>` — so the WAL/SHM
///      companions were written to `nanocrew.db.db-wal/-shm` instead of
///      `nanocrew.db.corrupted-<ts>-wal/-shm`. Fresh DB + orphaned WAL =
///      confused startup, empty prefs, missing drives, "app opens with
///      everything gone" reports.
///
/// The safe behaviour is to LOG the result and let the human decide via
/// the Factory Reset button already exposed in Settings → Danger Zone.
fn try_salvage_if_corrupt(path: &Path) -> Result<(), AppError> {
    let ro_uri = format!(
        "file:{}?mode=ro",
        path.to_string_lossy().replace('\\', "/")
    );
    let conn = match Connection::open_with_flags(
        &ro_uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    let result: String = conn
        .query_row("PRAGMA integrity_check(1);", [], |r| r.get(0))
        .unwrap_or_else(|_| "unknown".into());
    drop(conn);

    if result != "ok" {
        tracing::warn!(
            target: "nanocrew::db",
            "startup integrity_check reported {:?} for {} — leaving file in place. \
             If the app misbehaves, use Settings → Danger Zone → Reset all data.",
            result,
            path.display()
        );
    }
    Ok(())
}

// v0.3.2: `attempt_salvage()` was removed alongside the auto-quarantine
// path. Its silent `unwrap_or_default()` on the query result made a
// misleading empty-drives salvage report which confused hand-recovery.
// The Factory Reset button owned by the user is the correct human path
// for a truly corrupted DB.

/// v0.3.2 self-heal for users hit by v0.2.15 – v0.3.1's buggy auto-quarantine.
///
/// Symptom: after updating, users open the app to find no account, no
/// drives, no prefs — everything looks like a fresh install even though
/// they had real config. Their real DB was renamed to
/// `nanocrew.db.corrupted-<ts>` by the old code (usually with a false-
/// positive integrity flag), leaving a tiny empty DB in its place.
///
/// This function: if `path` doesn't exist OR is under ~16 KiB (fresh
/// empty DB is ~4 KiB, this bounds "real user DBs"), look for any sibling
/// `nanocrew.db.corrupted-*` file, integrity-check the newest one, and
/// if it passes, swap it into place. Idempotent (does nothing when the
/// live DB already has real content). Non-fatal on any error — we'd
/// rather users get a fresh empty DB than a hang on this recovery path.
fn try_restore_from_bad_quarantine(path: &Path) {
    // Heuristic: user has meaningful data if their DB is bigger than a
    // freshly-created empty one. 16 KiB is a healthy buffer over the
    // ~4 KiB baseline.
    const EMPTY_DB_SUSPECT_MAX: u64 = 16 * 1024;
    let live_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if live_size > EMPTY_DB_SUSPECT_MAX {
        return;
    }

    let parent = match path.parent() {
        Some(p) => p,
        None => return,
    };
    let entries = match std::fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut candidates: Vec<(std::path::PathBuf, u64)> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name();
        let name_s = name.to_string_lossy();
        if name_s.starts_with("nanocrew.db.corrupted-") {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            candidates.push((e.path(), size));
        }
    }
    // Prefer the biggest — most likely the real user data. Ties broken
    // by mtime don't matter here because size is a stronger signal.
    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    for (cand, size) in candidates {
        if size <= EMPTY_DB_SUSPECT_MAX {
            continue;
        }
        // Integrity-check the candidate as a healthy DB before trusting it.
        let ok = Connection::open_with_flags(
            &cand,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok()
        .and_then(|c| {
            c.query_row("PRAGMA integrity_check(1);", [], |r| r.get::<_, String>(0))
                .ok()
        })
        .map(|s| s == "ok")
        .unwrap_or(false);
        if !ok {
            continue;
        }
        // Remove the empty live DB (if present) and any stray -wal/-shm
        // that the rename bug also created (`nanocrew.db.db-wal` etc.).
        // Then copy the candidate into place. Copy (not rename) so the
        // quarantine backup stays on disk as evidence.
        for stray in [
            path.with_extension("db-wal"),
            path.with_extension("db-shm"),
            parent.join("nanocrew.db.db-wal"),
            parent.join("nanocrew.db.db-shm"),
        ] {
            let _ = std::fs::remove_file(&stray);
        }
        let _ = std::fs::remove_file(path);
        if std::fs::copy(&cand, path).is_ok() {
            tracing::warn!(
                target: "nanocrew::db",
                "v0.3.2 self-heal: restored {} from {} (v0.2.15-v0.3.1 quarantine was buggy)",
                path.display(), cand.display()
            );
            return;
        }
    }
}

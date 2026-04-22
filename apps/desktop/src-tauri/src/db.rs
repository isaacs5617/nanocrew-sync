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
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    // Migration: add secret_key column to existing databases that pre-date it.
    let _ = conn.execute(
        "ALTER TABLE drives ADD COLUMN secret_key TEXT NOT NULL DEFAULT ''",
        [],
    );
    Ok(conn)
}

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

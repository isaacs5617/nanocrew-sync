use std::collections::HashMap;
use std::sync::Mutex;

use tracing_appender::non_blocking::WorkerGuard;

use crate::mounts::MountHandle;

pub struct ActiveSession {
    pub account_id: i64,
    pub username: String,
}

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub mounts: Mutex<HashMap<i64, MountHandle>>,
    pub sessions: Mutex<HashMap<String, ActiveSession>>,
    /// Keeps the tracing-appender background thread alive for the whole
    /// process. Drop on shutdown flushes any buffered log lines. `Option`
    /// because `logging::init` returns `None` if the subscriber is already
    /// set (shouldn't happen in normal runs, but is harmless).
    pub _log_guard: Mutex<Option<WorkerGuard>>,
}

impl AppState {
    pub fn new(conn: rusqlite::Connection) -> Self {
        Self {
            db: Mutex::new(conn),
            mounts: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            _log_guard: Mutex::new(None),
        }
    }

    /// Attach the log-writer guard to app state so it lives as long as the
    /// process does. Called once from `setup()` after the DB is open.
    pub fn attach_log_guard(&self, guard: WorkerGuard) {
        *self._log_guard.lock().unwrap_or_else(|p| p.into_inner()) = Some(guard);
    }
}

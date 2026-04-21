use std::collections::HashMap;
use std::sync::Mutex;
use crate::mounts::MountHandle;

pub struct ActiveSession {
    pub account_id: i64,
    pub username: String,
}

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub mounts: Mutex<HashMap<i64, MountHandle>>,
    pub sessions: Mutex<HashMap<String, ActiveSession>>,
}

impl AppState {
    pub fn new(conn: rusqlite::Connection) -> Self {
        Self {
            db: Mutex::new(conn),
            mounts: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

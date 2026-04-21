use serde::{Deserialize, Serialize};

// ── Serializable types returned to the frontend ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: i64,
    pub username: String,
    pub created_at: i64,
}

/// Drive as returned to the frontend — never includes the secret key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    pub id: i64,
    pub name: String,
    pub provider: String,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub letter: String,
    pub access_key_id: String,
    pub cache_size_gb: i64,
    pub auto_mount: bool,
    pub readonly: bool,
    pub created_at: i64,
    /// Live status injected from MountRegistry — never stored in DB.
    pub status: String,
}

// ── Input types received from the frontend ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddDriveInput {
    pub name: String,
    pub provider: String,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub letter: String,
    pub access_key_id: String,
    /// Written to keyring immediately; never stored in DB.
    pub secret_access_key: String,
    pub cache_size_gb: i64,
    pub auto_mount: bool,
    pub readonly: bool,
}

#[derive(Debug, Deserialize)]
pub struct TestConnectionInput {
    pub provider: String,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

/// One entry in a bucket listing returned by list_drive_objects.
#[derive(Debug, Clone, Serialize)]
pub struct S3Entry {
    pub name: String,
    pub key: String,
    pub is_dir: bool,
    pub size: i64,
    pub modified: i64,
}

// ── Tauri event payloads ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DriveStatusPayload {
    pub drive_id: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Emitted as `file_lock_event` when the VFS detects an editor lockfile being
/// created/removed or a cross-device sentinel conflict. `state` is one of
/// `"lockfile_created"` | `"lockfile_released"` | `"sentinel_conflict"`.
#[derive(Debug, Clone, Serialize)]
pub struct FileLockEvent {
    pub drive_id: i64,
    /// Filesystem key being affected (the *target* file for lockfiles).
    pub target: String,
    /// The path that triggered the event (the lockfile itself, or the file
    /// whose sentinel is being challenged).
    pub trigger: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// Emitted as `transfer_progress` whenever a file transfer starts, makes
/// progress, completes, or errors.  `state` is one of "start" | "progress" |
/// "done" | "error".
#[derive(Debug, Clone, Serialize)]
pub struct TransferPayload {
    pub id: u64,
    pub drive_id: i64,
    pub filename: String,
    pub direction: String,   // "upload" | "download"
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

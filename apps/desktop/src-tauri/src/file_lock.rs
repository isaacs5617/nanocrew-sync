//! Cross-device advisory file locks for S3-mounted drives.
//!
//! Two NanoCrew Sync installs on different machines can both mount the same
//! bucket at the same time. Windows share-modes only protect within a single
//! kernel — nothing stops machine B from opening the same file for write while
//! machine A is streaming an upload. We layer an advisory lock on top: when a
//! file is opened for writing we drop a sentinel object at
//! `.nanocrew/locks/<sha256(key)>` with a short TTL; subsequent writer-opens
//! from other machines check the sentinel, see it hasn't expired, and are
//! rejected with `STATUS_SHARING_VIOLATION`.
//!
//! Design notes:
//!   - Sentinels are JSON so they're human-readable in the bucket console and
//!     easy to evolve. Keep the shape conservative.
//!   - TTL + wall-clock expires_at means a crashed writer doesn't leave the
//!     file locked forever. Fifteen minutes is long enough for a single
//!     Explorer copy of a large-ish file, short enough that "my other machine
//!     crashed" isn't a support ticket.
//!   - Heartbeats happen from the caller side (per-mount background task) so
//!     this module stays pure I/O.
//!   - Machine ID comes from `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`
//!     which survives reinstalls and is stable per-OS-image. Falls back to the
//!     hostname if the registry read fails.

use aws_sdk_s3::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Directory inside the bucket where sentinel objects live. Users who browse
/// their bucket directly will see this — document it in the README.
pub const LOCK_PREFIX: &str = ".nanocrew/locks/";
/// How long a sentinel is considered authoritative after its last write.
pub const LOCK_TTL_SECS: u64 = 15 * 60;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Sentinel {
    /// Schema version; bumped if the JSON shape changes.
    pub v: u32,
    /// Key of the file the sentinel protects.
    pub key: String,
    /// Machine GUID of the writer. Sentinels owned by this machine are
    /// ignored on check (same-machine acquires are fine).
    pub machine: String,
    /// Human-readable owner — username at time of acquire, for support.
    pub owner: String,
    /// Seconds-since-epoch the sentinel was created.
    pub acquired_at: u64,
    /// Seconds-since-epoch after which readers should treat the sentinel as
    /// expired and proceed regardless.
    pub expires_at: u64,
}

/// Result of `check`.
#[derive(Debug, Clone)]
pub enum LockState {
    /// No sentinel, or sentinel expired, or sentinel owned by us.
    Free,
    /// Sentinel held by a different machine and not yet expired.
    Foreign(Sentinel),
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Stable per-key object name. We sha256 the key so sentinels fit within S3's
/// key-length limit even for long paths, and so the sentinel listing doesn't
/// leak the structure of the underlying tree to casual bucket-console visitors.
pub fn sentinel_key(key: &str) -> String {
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    let digest = h.finalize();
    let mut hex = String::with_capacity(LOCK_PREFIX.len() + 64 + 5);
    hex.push_str(LOCK_PREFIX);
    for b in digest.iter() {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex.push_str(".json");
    hex
}

/// Check whether a sentinel exists for `key` and whether it belongs to us.
pub async fn check(
    client: &Client,
    bucket: &str,
    key: &str,
    machine: &str,
) -> Result<LockState, String> {
    let obj = client
        .get_object()
        .bucket(bucket)
        .key(sentinel_key(key))
        .send()
        .await;

    let resp = match obj {
        Ok(r) => r,
        Err(e) => {
            // NoSuchKey → Free; anything else surfaces so the caller can decide.
            let msg = e.to_string();
            if msg.contains("NoSuchKey") || msg.contains("404") {
                return Ok(LockState::Free);
            }
            return Err(format!("check sentinel: {msg}"));
        }
    };

    let body = resp
        .body
        .collect()
        .await
        .map_err(|e| format!("read sentinel body: {e}"))?
        .into_bytes();

    let s: Sentinel = match serde_json::from_slice(&body) {
        Ok(s) => s,
        Err(_) => {
            // Corrupt or alien sentinel — treat as free. A stray hand-edited
            // file in `.nanocrew/locks/` shouldn't lock the whole drive.
            return Ok(LockState::Free);
        }
    };

    if s.expires_at <= now_secs() {
        return Ok(LockState::Free);
    }
    if s.machine == machine {
        return Ok(LockState::Free);
    }
    Ok(LockState::Foreign(s))
}

/// Write a sentinel for `key`. Returns the sentinel that was written, which
/// the caller keeps so it can `release` later.
pub async fn acquire(
    client: &Client,
    bucket: &str,
    key: &str,
    machine: &str,
    owner: &str,
) -> Result<Sentinel, String> {
    let now = now_secs();
    let s = Sentinel {
        v: 1,
        key: key.to_string(),
        machine: machine.to_string(),
        owner: owner.to_string(),
        acquired_at: now,
        expires_at: now + LOCK_TTL_SECS,
    };
    let body = serde_json::to_vec(&s).map_err(|e| format!("serialize sentinel: {e}"))?;

    client
        .put_object()
        .bucket(bucket)
        .key(sentinel_key(key))
        .content_type("application/json")
        .body(body.into())
        .send()
        .await
        .map_err(|e| format!("put sentinel: {e}"))?;

    Ok(s)
}

/// Refresh an existing sentinel — extends `expires_at` by another TTL window.
/// Cheaper to re-PUT than to round-trip through GET + mutate; S3 doesn't
/// support partial updates. Reserved for a future background refresh task;
/// current acquire uses a TTL long enough that single-session writes don't
/// need refresh.
#[allow(dead_code)]
pub async fn heartbeat(
    client: &Client,
    bucket: &str,
    key: &str,
    machine: &str,
    owner: &str,
) -> Result<(), String> {
    let _ = acquire(client, bucket, key, machine, owner).await?;
    Ok(())
}

/// Delete the sentinel. No-op if it's already gone.
pub async fn release(client: &Client, bucket: &str, key: &str) -> Result<(), String> {
    client
        .delete_object()
        .bucket(bucket)
        .key(sentinel_key(key))
        .send()
        .await
        .map_err(|e| format!("delete sentinel: {e}"))?;
    Ok(())
}

/// Read the machine GUID from the Windows registry. This is stable across
/// reboots and reinstalls of NanoCrew Sync, but changes on a fresh OS install
/// or disk image. Fall back to the hostname if the registry is unreadable.
pub fn machine_id() -> String {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
            KEY_WOW64_64KEY, REG_VALUE_TYPE,
        };

        unsafe {
            let mut key: HKEY = HKEY::default();
            let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography\0".encode_utf16().collect();
            let open = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(subkey.as_ptr()),
                0,
                KEY_READ | KEY_WOW64_64KEY,
                &mut key,
            );
            if open.is_err() {
                return hostname_fallback();
            }

            let name: Vec<u16> = "MachineGuid\0".encode_utf16().collect();
            let mut ty: REG_VALUE_TYPE = REG_VALUE_TYPE(0);
            let mut buf = [0u8; 256];
            let mut len = buf.len() as u32;
            let q = RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut ty),
                Some(buf.as_mut_ptr()),
                Some(&mut len),
            );
            let _ = RegCloseKey(key);
            if q.is_err() {
                return hostname_fallback();
            }

            // Value is stored as UTF-16 LE wide string including trailing null.
            let used = (len as usize).min(buf.len());
            let wide: Vec<u16> = buf[..used]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect();
            return String::from_utf16_lossy(&wide);
        }
    }
    #[cfg(not(windows))]
    {
        hostname_fallback()
    }
}

fn hostname_fallback() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown-machine".into())
}

// ── Lockfile pattern detection ──────────────────────────────────────────────

/// Classify a filename as a well-known editor lockfile. Returns the *target*
/// filename that the lockfile protects, if any.
///
/// Patterns recognised:
///   - `~$<name>`              Microsoft Office (Word, Excel, PowerPoint)
///   - `.~lock.<name>#`        LibreOffice / OpenOffice
///   - `.<name>.swp`           vim swap files
///   - `<name>.lock`           generic .lock suffix (last resort)
pub fn classify_lockfile(basename: &str) -> Option<String> {
    // Office — "~$Document.docx" → "Document.docx"
    if let Some(rest) = basename.strip_prefix("~$") {
        if rest.ends_with(".docx")
            || rest.ends_with(".xlsx")
            || rest.ends_with(".pptx")
            || rest.ends_with(".doc")
            || rest.ends_with(".xls")
            || rest.ends_with(".ppt")
        {
            return Some(rest.to_string());
        }
    }
    // LibreOffice — ".~lock.Document.odt#" → "Document.odt"
    if let Some(rest) = basename.strip_prefix(".~lock.") {
        if let Some(target) = rest.strip_suffix('#') {
            return Some(target.to_string());
        }
    }
    // Vim — ".file.swp" → "file"
    if let Some(rest) = basename.strip_prefix('.') {
        if let Some(target) = rest.strip_suffix(".swp") {
            return Some(target.to_string());
        }
    }
    // Generic — "file.lock" → "file"
    if let Some(target) = basename.strip_suffix(".lock") {
        if !target.is_empty() {
            return Some(target.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_key_is_stable() {
        let a = sentinel_key("docs/report.docx");
        let b = sentinel_key("docs/report.docx");
        assert_eq!(a, b);
        assert!(a.starts_with(LOCK_PREFIX));
        assert!(a.ends_with(".json"));
    }

    #[test]
    fn classify_office() {
        assert_eq!(
            classify_lockfile("~$Report.docx"),
            Some("Report.docx".into())
        );
        assert_eq!(
            classify_lockfile("~$Budget.xlsx"),
            Some("Budget.xlsx".into())
        );
    }

    #[test]
    fn classify_libreoffice() {
        assert_eq!(
            classify_lockfile(".~lock.Report.odt#"),
            Some("Report.odt".into())
        );
    }

    #[test]
    fn classify_vim_and_generic() {
        assert_eq!(classify_lockfile(".notes.swp"), Some("notes".into()));
        assert_eq!(classify_lockfile("build.lock"), Some("build".into()));
        assert_eq!(classify_lockfile("normal.txt"), None);
    }

    #[test]
    fn machine_id_is_nonempty() {
        let id = machine_id();
        assert!(!id.is_empty());
    }
}

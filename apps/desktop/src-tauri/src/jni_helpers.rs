//! JNI <-> Rust type-conversion helpers used by `android_provider.rs`.
//!
//! Everything here is cfg'd to Android — the file is `mod`'d only under
//! `#[cfg(target_os = "android")]` in `lib.rs`. Keeping these helpers in a
//! separate module means the JNI surface in `android_provider` stays a pure
//! signature list, not a wall of `env.get_string(...)?.into()` boilerplate.

#![cfg(target_os = "android")]

use jni::{
    objects::{JObject, JString, JValue},
    sys::{jlong, jobject},
    JNIEnv,
};

use crate::providers::FileStat;

/// Pull a Rust `String` out of a Java `String` jobject. Returns an empty
/// string on any JNI error — the caller is expected to validate non-empty
/// keys at the Rust layer anyway.
pub fn jstring_to_string(env: &mut JNIEnv, s: JString) -> String {
    match env.get_string(&s) {
        Ok(js) => js.into(),
        Err(e) => {
            tracing::warn!(target: "nanocrew::jni", "jstring_to_string: {e}");
            String::new()
        }
    }
}

/// Allocate a Java `String` from a Rust `&str`. Returns a null jobject on
/// allocation failure (caller decides whether that's recoverable).
pub fn string_to_jstring<'a>(env: &mut JNIEnv<'a>, s: &str) -> JString<'a> {
    env.new_string(s).unwrap_or_else(|e| {
        tracing::warn!(target: "nanocrew::jni", "string_to_jstring: {e}");
        // Safety: a null JString is fine to pass back to Kotlin; the caller
        // will see `null` and translate to "missing".
        unsafe { JString::from_raw(std::ptr::null_mut()) }
    })
}

/// Build a row for `MatrixCursor.addRow(Object[])`, mapping a [`FileStat`]
/// onto the Android `DocumentsContract.Document.*` column set:
///
///   * `COLUMN_DOCUMENT_ID`   — `<drive_id>:<key>`
///   * `COLUMN_DISPLAY_NAME`  — last path segment
///   * `COLUMN_MIME_TYPE`     — guessed from extension; folders use
///     `vnd.android.document/directory`
///   * `COLUMN_SIZE`          — bytes
///   * `COLUMN_LAST_MODIFIED` — millis since epoch
///   * `COLUMN_FLAGS`         — `FLAG_SUPPORTS_WRITE | _DELETE | _RENAME`
///     for files; `FLAG_DIR_SUPPORTS_CREATE` for dirs.
///
/// Implementation is TODO — currently returns a null jobject so the cursor
/// row is skipped. See docs/android-port-design.md for the SAF column spec.
pub fn document_row_array<'a>(
    _env: &mut JNIEnv<'a>,
    _drive_id: jlong,
    _key: &str,
    _stat: &FileStat,
    _is_dir: bool,
) -> jobject {
    // TODO: env.new_object_array(...) and env.set_object_array_element(...)
    // for each column. Pull column ordering from the matching MatrixCursor
    // construction in NanoCrewDocumentsProvider.kt.
    std::ptr::null_mut()
}

/// Convert a Windows FILETIME (100-ns intervals since 1601-01-01 UTC) to
/// Unix millis. Returns 0 when the input is 0 (unknown). The providers
/// already normalize to FILETIME for the WinFsp dispatcher; we go the other
/// way for SAF, which wants `COLUMN_LAST_MODIFIED` in millis-since-epoch.
pub fn filetime_to_unix_millis(filetime: u64) -> i64 {
    if filetime == 0 {
        return 0;
    }
    // FILETIME epoch (1601-01-01) to Unix epoch (1970-01-01) in 100-ns ticks:
    // 11_644_473_600 seconds * 10_000_000.
    const EPOCH_OFFSET_100NS: i64 = 116_444_736_000_000_000;
    let unix_100ns = filetime as i64 - EPOCH_OFFSET_100NS;
    unix_100ns / 10_000 // -> millis
}

/// Best-effort MIME-type guess from a filename. Defaults to
/// `application/octet-stream`. Kotlin can override via
/// `MimeTypeMap.getSingleton().getMimeTypeFromExtension(...)`, but doing it
/// here lets the Rust side populate the cursor in one pass.
pub fn guess_mime(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "txt"  => "text/plain",
        "md"   => "text/markdown",
        "json" => "application/json",
        "pdf"  => "application/pdf",
        "png"  => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif"  => "image/gif",
        "mp3"  => "audio/mpeg",
        "mp4"  => "video/mp4",
        "zip"  => "application/zip",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _      => "application/octet-stream",
    }
}

/// Parse a SAF document id (`"<drive_id>:<key>"`) into its parts. The drive
/// id encodes which `CloudProvider` to dispatch to; the key is provider-
/// relative (no leading slash). A root document is `"<drive_id>:"`.
pub fn parse_doc_id(doc_id: &str) -> Option<(i64, &str)> {
    let (drive, key) = doc_id.split_once(':')?;
    drive.parse::<i64>().ok().map(|d| (d, key))
}

/// Construct a SAF document id from its parts.
pub fn make_doc_id(drive_id: i64, key: &str) -> String {
    format!("{drive_id}:{}", key.trim_start_matches('/'))
}

/// Cheap wrapper around `JObject::null()` so call sites read clearly when
/// they intentionally hand Kotlin a null cursor (e.g. on error before TODOs
/// are implemented).
pub fn null_object<'a>() -> JObject<'a> {
    JObject::null()
}

/// Shim — converts a `Result<T, ProviderError>` into a JNI exception by
/// throwing a `java.io.IOException` with the error string. Returns `None`
/// when the result was `Err` (caller should return the default jobject).
pub fn throw_on_err<T>(
    env: &mut JNIEnv,
    res: Result<T, crate::providers::ProviderError>,
) -> Option<T> {
    match res {
        Ok(v) => Some(v),
        Err(e) => {
            let msg = e.to_string();
            let _ = env.throw_new("java/io/IOException", &msg);
            tracing::warn!(target: "nanocrew::jni", "throwing IOException: {msg}");
            None
        }
    }
}

// `JValue` is re-exported for convenience by android_provider when building
// argument arrays; silences unused-import warnings on partial-impl builds.
#[allow(dead_code)]
const _: fn() -> JValue<'static, 'static> = || JValue::Long(0);

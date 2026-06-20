//! JNI surface that bridges the Kotlin `NanoCrewDocumentsProvider` to the
//! cross-platform `CloudProvider` trait.
//!
//! On desktop (Windows + macOS) the equivalent dispatch layer is the
//! WinFsp / FUSE-T loop — it pulls a row out of `drives`, looks up the
//! right `CloudProvider`, calls `list_dir` / `get_range` / `put_object`,
//! and translates results back into FS callbacks. Here we do the same
//! thing, but Android's Storage Access Framework drives us instead: Files
//! (or Word, or any picker) calls `queryChildDocuments` / `openDocument`
//! on the Kotlin shim, the shim forwards over JNI into these functions,
//! and we marshal the result back into a `MatrixCursor` or
//! `ParcelFileDescriptor`.
//!
//! Everything in this file is TODO bodies for v0.4.0 scaffolding — signatures
//! locked in, behavior is `todo!()` or null-object returns. See
//! docs/android-port-design.md for the full plan.

#![cfg(target_os = "android")]

use jni::{
    objects::{JClass, JString},
    sys::{jboolean, jint, jlong, jobject, JNI_FALSE},
    JNIEnv,
};

use crate::jni_helpers::{
    document_row_array, jstring_to_string, make_doc_id, null_object, parse_doc_id,
    string_to_jstring,
};

// JNI naming follows the Kotlin package: dev.nanocrew.sync.NanoCrewDocumentsProvider.
// External name = Java_<package_underscored>_<class>_<method>.
// Underscores in the class name itself would need `_1` escapes but our class
// has none.

// ── Roots ────────────────────────────────────────────────────────────────────

/// Build the roots cursor backing `queryRoots`. Each row corresponds to a
/// drive in the SQLite `drives` table — Android renders these as top-level
/// entries in Files ("NanoCrew · my-bucket", "NanoCrew · sftp-staging", ...).
///
/// Returns a `MatrixCursor` jobject. On the Kotlin side this is cast directly
/// to `Cursor`. TODO body — currently returns null which Kotlin treats as
/// "provider isn't ready yet".
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_queryRootsNative<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
) -> jobject {
    tracing::debug!(target: "nanocrew::android", "queryRootsNative");
    // TODO:
    //  1. Borrow the global AppState (initialized in mobile_entry_point).
    //  2. SELECT id, name, bucket, bucket_prefix FROM drives.
    //  3. For each row, addRow(arrayOf(root_id, flags, title, document_id,
    //     icon, summary)).
    //  4. Return the populated MatrixCursor.
    let _ = env;
    null_object().into_raw()
}

// ── Listing ──────────────────────────────────────────────────────────────────

/// `queryChildDocuments` -> list the children of `parent_doc_id`. Parent is
/// `"<drive_id>:<prefix>"`. We resolve the drive's `CloudProvider`, call
/// `list_dir(prefix)`, and emit one `MatrixCursor` row per dir + file.
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_queryChildDocumentsNative<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    parent_doc_id: JString<'a>,
) -> jobject {
    let id = jstring_to_string(&mut env, parent_doc_id);
    let parsed = parse_doc_id(&id);
    tracing::debug!(target: "nanocrew::android", "queryChildDocumentsNative {id} parsed={parsed:?}");
    // TODO:
    //  1. Look up the CloudProvider for the drive id.
    //  2. tokio::runtime::Handle::current().block_on(provider.list_dir(key))
    //     — or stash a Tokio runtime in a OnceCell on first JNI call.
    //  3. For each entry, document_row_array(..) and addRow().
    //  4. Return the MatrixCursor.
    let _ = document_row_array;
    null_object().into_raw()
}

/// `queryDocument` -> a one-row cursor describing the given document.
/// Roots return the bucket/prefix as a folder; everything else round-trips
/// through `provider.stat(key)`.
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_queryDocumentNative<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    doc_id: JString<'a>,
) -> jobject {
    let id = jstring_to_string(&mut env, doc_id);
    tracing::debug!(target: "nanocrew::android", "queryDocumentNative {id}");
    // TODO: parse, provider.stat(key).await, return single-row cursor.
    null_object().into_raw()
}

// ── Open / read / write ─────────────────────────────────────────────────────

/// `openDocument` -> hand SAF a `ParcelFileDescriptor`. SAF expects an FD,
/// but our backend is a remote object store — so we materialize the object
/// to a temp file under the app's private cache dir, then PFD.open() it.
///
/// `mode_flags` corresponds to Android's `Intent.FLAG_GRANT_READ_URI_PERMISSION`
/// style bits. We map them to `O_RDONLY` / `O_RDWR` for the FD.
///
/// On close (RW only), Kotlin's `OnCloseListener` calls us back to upload
/// the diff — that's a separate JNI method, `closeDocumentNative`.
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_openDocumentNative<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    doc_id: JString<'a>,
    mode_flags: jint,
) -> jobject {
    let id = jstring_to_string(&mut env, doc_id);
    tracing::debug!(target: "nanocrew::android", "openDocumentNative {id} mode={mode_flags}");
    // TODO: materialize-on-open. See docs/android-port-design.md.
    //  1. Resolve (drive_id, key) via parse_doc_id.
    //  2. Hit the on-disk cache; on miss, provider.get_range() in chunks
    //     into a temp file under app_cache_dir.
    //  3. ParcelFileDescriptor.open(file, mode) via JNI, return that.
    //  4. Track the open in a SAFOpenFile registry for the close hook.
    null_object().into_raw()
}

/// Companion to `openDocumentNative` — Kotlin invokes this from a PFD
/// `OnCloseListener` so we can upload the modified file back to the
/// provider. No-op for read-only opens.
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_closeDocumentNative<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    doc_id: JString<'a>,
    was_writable: jboolean,
) {
    let id = jstring_to_string(&mut env, doc_id);
    tracing::debug!(
        target: "nanocrew::android",
        "closeDocumentNative {id} writable={}",
        was_writable != JNI_FALSE
    );
    // TODO: if writable, upload temp file via provider.put_object or
    // multipart; then evict the registry entry and (optionally) the temp
    // file (the cache layer may want to keep it around).
}

// ── Mutations ────────────────────────────────────────────────────────────────

/// `createDocument` -> create a folder or empty file at `parent / display_name`
/// with the given MIME type. SAF passes the MIME as a hint; we use
/// `vnd.android.document/directory` as the "this is a folder" sentinel.
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_createDocumentNative<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    parent_doc_id: JString<'a>,
    mime_type: JString<'a>,
    display_name: JString<'a>,
) -> jobject {
    let parent = jstring_to_string(&mut env, parent_doc_id);
    let mime = jstring_to_string(&mut env, mime_type);
    let name = jstring_to_string(&mut env, display_name);
    tracing::debug!(
        target: "nanocrew::android",
        "createDocumentNative parent={parent} mime={mime} name={name}"
    );
    // TODO:
    //  - parse parent, build child key = parent_key + "/" + name.
    //  - if mime == "vnd.android.document/directory": provider.put_object(
    //      key + "/.keep", &[]) — matches the desktop "directory marker"
    //      convention from winfsp_vfs.
    //  - else: provider.put_object(key, &[]) to create an empty placeholder.
    //  - return the new doc_id as a Java string.
    let _ = make_doc_id(0, "");
    string_to_jstring(&mut env, "").into_raw()
}

/// `deleteDocument`. Folders need recursive `list_prefix` + delete-each,
/// matching the desktop directory-delete behavior.
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_deleteDocumentNative<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    doc_id: JString<'a>,
) {
    let id = jstring_to_string(&mut env, doc_id);
    tracing::debug!(target: "nanocrew::android", "deleteDocumentNative {id}");
    // TODO: provider.delete or recursive list_prefix + delete loop.
}

/// `renameDocument` — desktop maps this to `copy_object(old, new) +
/// delete(old)` for object stores; SFTP/FTP/WebDAV have a native rename.
/// Returns the new document id.
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_renameDocumentNative<'a>(
    mut env: JNIEnv<'a>,
    _class: JClass<'a>,
    doc_id: JString<'a>,
    display_name: JString<'a>,
) -> jobject {
    let id = jstring_to_string(&mut env, doc_id);
    let name = jstring_to_string(&mut env, display_name);
    tracing::debug!(target: "nanocrew::android", "renameDocumentNative {id} -> {name}");
    // TODO: parse, rename via CloudProvider abstraction, return new doc id.
    string_to_jstring(&mut env, "").into_raw()
}

// ── Capabilities ─────────────────────────────────────────────────────────────

/// Probe — Kotlin uses this to gate the rest of the provider behind a
/// "drives are configured AND user is signed in" check. Returns true once
/// the React UI has unlocked the cache.
///
/// Today: always returns 0 (false). Once mobile auth lands, flip to read
/// the unlocked flag off `AppState`.
#[no_mangle]
pub extern "system" fn Java_dev_nanocrew_sync_NanoCrewDocumentsProvider_isReadyNative<'a>(
    _env: JNIEnv<'a>,
    _class: JClass<'a>,
) -> jboolean {
    JNI_FALSE
}

// Silences "unused parameter" warnings on the partial-impl scaffold and
// keeps the helpers reachable for incremental implementation.
#[allow(dead_code)]
fn _silence_unused() {
    let _ = (jlong::default(), jboolean::default(), jint::default());
}

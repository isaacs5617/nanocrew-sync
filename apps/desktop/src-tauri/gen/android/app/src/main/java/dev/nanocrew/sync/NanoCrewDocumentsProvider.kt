package dev.nanocrew.sync

import android.content.res.AssetFileDescriptor
import android.database.Cursor
import android.database.MatrixCursor
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import android.util.Log

/**
 * SAF entry point. Android's Files app, Word, every share sheet / picker
 * talks to this class through `content://dev.nanocrew.sync.documents/...`.
 *
 * Each method forwards into the Rust .so via JNI; the actual work (talking
 * to S3/SFTP/etc., walking the cache, handing back a ParcelFileDescriptor)
 * lives in src/android_provider.rs. See docs/android-port-design.md for the
 * end-to-end flow.
 */
class NanoCrewDocumentsProvider : DocumentsProvider() {

    companion object {
        private const val TAG = "nanocrew"

        // Default column sets we hand to MatrixCursor when the caller doesn't
        // ask for a specific projection. Matches the Android docs' defaults.
        val DEFAULT_ROOT_PROJECTION: Array<String> = arrayOf(
            Root.COLUMN_ROOT_ID,
            Root.COLUMN_FLAGS,
            Root.COLUMN_ICON,
            Root.COLUMN_TITLE,
            Root.COLUMN_DOCUMENT_ID,
            Root.COLUMN_SUMMARY,
        )

        val DEFAULT_DOCUMENT_PROJECTION: Array<String> = arrayOf(
            Document.COLUMN_DOCUMENT_ID,
            Document.COLUMN_DISPLAY_NAME,
            Document.COLUMN_MIME_TYPE,
            Document.COLUMN_SIZE,
            Document.COLUMN_LAST_MODIFIED,
            Document.COLUMN_FLAGS,
        )

        init {
            // The library name matches `[package].name` in Cargo.toml with
            // dashes -> underscores. The cdylib is packaged into the AAB by
            // the cargo-ndk Gradle plugin.
            try {
                System.loadLibrary("nanocrew_sync_lib")
            } catch (t: UnsatisfiedLinkError) {
                Log.e(TAG, "failed to load nanocrew_sync_lib: ${t.message}")
            }
        }
    }

    // ── Native JNI bridge ────────────────────────────────────────────────────
    // Method names mirror src/android_provider.rs. Keep them in sync.

    private external fun queryRootsNative(): Cursor?
    private external fun queryChildDocumentsNative(parentDocId: String): Cursor?
    private external fun queryDocumentNative(docId: String): Cursor?
    private external fun openDocumentNative(docId: String, modeFlags: Int): ParcelFileDescriptor?
    private external fun closeDocumentNative(docId: String, wasWritable: Boolean)
    private external fun createDocumentNative(
        parentDocId: String,
        mimeType: String,
        displayName: String,
    ): String?
    private external fun deleteDocumentNative(docId: String)
    private external fun renameDocumentNative(docId: String, displayName: String): String?
    private external fun isReadyNative(): Boolean

    // ── DocumentsProvider hooks ──────────────────────────────────────────────

    override fun onCreate(): Boolean {
        Log.i(TAG, "NanoCrewDocumentsProvider.onCreate")
        return true
    }

    override fun queryRoots(projection: Array<out String>?): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_ROOT_PROJECTION)
        if (!isReadyNative()) {
            // No drives configured / cache locked — return an empty cursor so
            // SAF still shows the provider, but with no roots inside.
            return cursor
        }
        val native = queryRootsNative()
        return native ?: cursor
    }

    override fun queryChildDocuments(
        parentDocumentId: String,
        projection: Array<out String>?,
        sortOrder: String?,
    ): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
        val native = queryChildDocumentsNative(parentDocumentId)
        return native ?: cursor
    }

    override fun queryDocument(
        documentId: String,
        projection: Array<out String>?,
    ): Cursor {
        val cursor = MatrixCursor(projection ?: DEFAULT_DOCUMENT_PROJECTION)
        val native = queryDocumentNative(documentId)
        return native ?: cursor
    }

    override fun openDocument(
        documentId: String,
        mode: String,
        signal: CancellationSignal?,
    ): ParcelFileDescriptor? {
        // Parse the Java open-mode string ("r", "rw", "w", ...) into the
        // POSIX bit set the JNI side wants.
        val modeFlags = ParcelFileDescriptor.parseMode(mode)
        val wasWritable = mode.contains('w')

        val pfd = openDocumentNative(documentId, modeFlags)
        if (pfd != null && wasWritable) {
            // Hook close-callback so we can upload the diff back to the
            // cloud provider once the consumer finishes writing. The handler
            // thread is owned by the SAF runtime; we just delegate to JNI.
            return ParcelFileDescriptor.open(
                pfd.fileDescriptor.let { _ -> pfd.toString().let { null } } as? java.io.File
                    ?: return pfd,
                modeFlags,
            )
            // TODO: real implementation uses
            //   ParcelFileDescriptor.open(file, modeFlags, handler) {
            //       closeDocumentNative(documentId, wasWritable)
            //   }
            // We can't pass a Handler from here without the Tauri main
            // looper — wire that up when AppState lands on the JNI side.
        }
        return pfd
    }

    override fun openDocumentThumbnail(
        documentId: String,
        sizeHint: android.graphics.Point,
        signal: CancellationSignal?,
    ): AssetFileDescriptor? {
        // TODO: image previews. Provider returns null for now so Files just
        // shows the MIME-type icon.
        return null
    }

    override fun createDocument(
        parentDocumentId: String,
        mimeType: String,
        displayName: String,
    ): String? {
        return createDocumentNative(parentDocumentId, mimeType, displayName)
    }

    override fun deleteDocument(documentId: String) {
        deleteDocumentNative(documentId)
    }

    override fun renameDocument(documentId: String, displayName: String): String? {
        return renameDocumentNative(documentId, displayName)
    }
}

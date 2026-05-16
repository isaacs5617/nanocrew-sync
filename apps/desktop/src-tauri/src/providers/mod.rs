pub mod dropbox;
pub mod ftp;
pub mod gdrive;
pub mod onedrive;
pub mod s3;
pub mod sftp;
pub mod webdav;

use async_trait::async_trait;
use bytes::Bytes;

// ── Result types ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

impl From<String> for ProviderError {
    fn from(s: String) -> Self {
        ProviderError::Other(s)
    }
}

impl From<russh::Error> for ProviderError {
    fn from(e: russh::Error) -> Self {
        ProviderError::Other(e.to_string())
    }
}

/// Metadata for a single object.
#[derive(Clone, Debug)]
pub struct FileStat {
    pub size: u64,
    /// Windows FILETIME (100-ns intervals since 1601-01-01). 0 for unknown.
    pub mtime_filetime: u64,
}

/// Result of listing a directory.
#[derive(Clone, Debug)]
pub struct ListDirResult {
    /// Immediate subdirectory names (last path component, no trailing slash).
    pub dirs: Vec<String>,
    /// (filename, stat) pairs — filename is the last path component only.
    pub files: Vec<(String, FileStat)>,
}

/// A part that has been successfully uploaded and can be referenced in
/// `complete_multipart`.
#[derive(Clone, Debug)]
pub struct CompletedPart {
    pub part_number: i32,
    pub etag: String,
}

// ── Trait ────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait CloudProvider: Send + Sync {
    /// List a single "directory" (delimited by `/`). `prefix` is the VFS-
    /// relative parent key ("" for root, "foo/bar" for a subdir).
    async fn list_dir(&self, prefix: &str) -> Result<ListDirResult, ProviderError>;

    /// Stream a directory listing, invoking `on_page` for each provider page
    /// as it arrives. The default implementation calls `list_dir` once (good
    /// enough for providers where listing isn't paginated). Providers that DO
    /// paginate (S3, Google Drive, OneDrive) override this for progressive
    /// delivery — this is what lets Explorer render entries for huge folders
    /// before the full pagination completes.
    async fn list_dir_stream(
        &self,
        prefix: &str,
        on_page: &mut (dyn FnMut(ListDirResult) + Send),
    ) -> Result<(), ProviderError> {
        let result = self.list_dir(prefix).await?;
        on_page(result);
        Ok(())
    }

    /// Fetch metadata for a single object key. Returns `Ok(None)` when the
    /// object does not exist.
    async fn stat(&self, key: &str) -> Result<Option<FileStat>, ProviderError>;

    /// Fetch a byte range `[offset, offset + length)` from an object.
    async fn get_range(
        &self,
        key: &str,
        offset: u64,
        length: u64,
    ) -> Result<Bytes, ProviderError>;

    /// Upload an object in a single request (small-file fast path).
    async fn put_object(&self, key: &str, data: Bytes) -> Result<(), ProviderError>;

    /// Delete a single object. Providers should treat a missing key as success.
    async fn delete(&self, key: &str) -> Result<(), ProviderError>;

    /// Copy an object from `from` to `to` within the same provider/bucket.
    async fn copy_object(&self, from: &str, to: &str) -> Result<(), ProviderError>;

    /// List all object keys that share a given prefix (no delimiter — walks
    /// the entire subtree). Used for directory rename.
    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, ProviderError>;

    // ── Multipart upload ──────────────────────────────────────────────────────

    /// Initiate a multipart upload. Returns the `upload_id`.
    async fn create_multipart(&self, key: &str) -> Result<String, ProviderError>;

    /// Upload a single part. Returns the ETag.
    async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: i32,
        data: Bytes,
    ) -> Result<String, ProviderError>;

    /// Finalize a multipart upload.
    async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: Vec<CompletedPart>,
    ) -> Result<(), ProviderError>;

    /// Abort a multipart upload (best-effort cleanup on error).
    async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), ProviderError>;

    // ── Capability hints ──────────────────────────────────────────────────────

    /// Preferred block size for range-GET caching. Defaults to 1 MiB.
    fn preferred_block_size(&self) -> usize {
        1024 * 1024
    }

    /// Whether the provider supports byte-range GET requests.
    fn supports_byte_ranges(&self) -> bool {
        true
    }
}

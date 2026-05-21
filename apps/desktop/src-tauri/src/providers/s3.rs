//! S3 implementation of [`CloudProvider`].
//!
//! Holds an already-constructed `aws_sdk_s3::Client` and the resolved bucket
//! name. All S3 SDK calls that `S3Fs` previously made inline are delegated here.

use async_trait::async_trait;
use aws_sdk_s3::Client;
use bytes::Bytes;

use super::{CloudProvider, CompletedPart, FileStat, ListDirResult, ProviderError};

fn unix_secs_to_filetime(secs: i64) -> u64 {
    let s = secs.max(0) as u64 + 11_644_473_600;
    s * 10_000_000
}

/// Percent-encode an S3 key for use in `x-amz-copy-source`. RFC 3986
/// unreserved chars (`A-Za-z0-9-._~`) plus `/` (kept as path separator) pass
/// through; everything else is `%XX`-encoded.
fn percent_encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.as_bytes() {
        let c = *b;
        let is_unreserved =
            c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b'/');
        if is_unreserved {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{:02X}", c));
        }
    }
    out
}

// ── S3Provider ───────────────────────────────────────────────────────────────

pub struct S3Provider {
    pub client: Client,
    pub bucket: String,
    /// Normalised subdirectory prefix (empty = root, otherwise trailing slash).
    /// All provider-relative keys are prepended with this to form real S3 keys.
    pub bucket_prefix: String,
}

impl S3Provider {
    pub fn new(client: Client, bucket: String, bucket_prefix: String) -> Self {
        Self { client, bucket, bucket_prefix }
    }

    /// Prepend `bucket_prefix` to a VFS-relative key to get the actual S3 key.
    fn abs_key(&self, rel_key: &str) -> String {
        if self.bucket_prefix.is_empty() {
            rel_key.to_string()
        } else {
            format!("{}{}", self.bucket_prefix, rel_key)
        }
    }

    /// Strip `bucket_prefix` from an absolute S3 key to recover the VFS-
    /// relative key. Returns `None` when the key does not share the prefix.
    fn rel_key<'a>(&self, abs_key: &'a str) -> Option<&'a str> {
        if self.bucket_prefix.is_empty() {
            Some(abs_key)
        } else {
            abs_key.strip_prefix(&self.bucket_prefix)
        }
    }
}

#[async_trait]
impl CloudProvider for S3Provider {
    async fn list_dir(&self, prefix: &str) -> Result<ListDirResult, ProviderError> {
        let mut acc = ListDirResult { dirs: vec![], files: vec![] };
        let mut callback = |p: ListDirResult| {
            acc.dirs.extend(p.dirs);
            acc.files.extend(p.files);
            true
        };
        self.list_dir_stream(prefix, &mut callback).await?;
        Ok(acc)
    }

    async fn list_dir_stream(
        &self,
        prefix: &str,
        on_page: &mut (dyn FnMut(ListDirResult) -> bool + Send),
    ) -> Result<(), ProviderError> {
        let s3_prefix = if prefix.is_empty() {
            self.bucket_prefix.clone()
        } else {
            format!("{}{}/", self.bucket_prefix, prefix)
        };

        let mut cont: Option<String> = None;

        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&s3_prefix)
                .delimiter("/");
            if let Some(c) = cont.as_ref() {
                req = req.continuation_token(c);
            }
            let resp = req.send().await.map_err(|e| {
                let detail = e
                    .as_service_error()
                    .map(|se| {
                        let code = se.meta().code().unwrap_or("service_error");
                        let msg = se.meta().message().unwrap_or("no message");
                        format!("{code}: {msg}")
                    })
                    .unwrap_or_else(|| e.to_string());
                ProviderError::Other(format!(
                    "list_objects_v2 prefix={s3_prefix:?}: {detail}"
                ))
            })?;

            let mut page_dirs = Vec::<String>::new();
            let mut page_files = Vec::<(String, FileStat)>::new();

            for cp in resp.common_prefixes() {
                if let Some(full) = cp.prefix() {
                    let name =
                        full.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                    if !name.is_empty() {
                        page_dirs.push(name.to_string());
                    }
                }
            }
            for obj in resp.contents() {
                let Some(full) = obj.key() else { continue };
                if full.ends_with("/.keep") || full.ends_with('/') {
                    continue;
                }
                let name = full.rsplit('/').next().unwrap_or("");
                if name.is_empty() || name == ".keep" {
                    continue;
                }
                let size = obj.size().unwrap_or(0).max(0) as u64;
                let mtime_filetime = obj
                    .last_modified()
                    .map(|d| unix_secs_to_filetime(d.secs()))
                    .unwrap_or(0);
                page_files.push((name.to_string(), FileStat { size, mtime_filetime }));
            }

            let keep_going = on_page(ListDirResult { dirs: page_dirs, files: page_files });
            if !keep_going {
                break;
            }

            match resp.next_continuation_token() {
                Some(t) => cont = Some(t.to_string()),
                None => break,
            }
        }

        Ok(())
    }

    async fn stat(&self, key: &str) -> Result<Option<FileStat>, ProviderError> {
        let abs = self.abs_key(key);
        let resp = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&abs)
            .send()
            .await;

        match resp {
            Ok(out) => {
                let size = out.content_length().unwrap_or(0).max(0) as u64;
                let mtime_filetime = out
                    .last_modified()
                    .map(|d| unix_secs_to_filetime(d.secs()))
                    .unwrap_or(0);
                Ok(Some(FileStat { size, mtime_filetime }))
            }
            Err(e) => {
                if e.as_service_error()
                    .map(|se| se.is_not_found())
                    .unwrap_or(false)
                {
                    return Ok(None);
                }
                Err(ProviderError::Other(format!("head_object {abs:?}: {e}")))
            }
        }
    }

    async fn get_range(
        &self,
        key: &str,
        offset: u64,
        length: u64,
    ) -> Result<Bytes, ProviderError> {
        if length == 0 {
            return Ok(Bytes::new());
        }
        let abs = self.abs_key(key);
        let end = offset + length - 1;
        let range = format!("bytes={}-{}", offset, end);
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&abs)
            .range(range)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("get_object {abs:?}: {e}")))?;
        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| ProviderError::Other(format!("body collect: {e}")))?
            .into_bytes();
        Ok(bytes)
    }

    async fn put_object(&self, key: &str, data: Bytes) -> Result<(), ProviderError> {
        let abs = self.abs_key(key);
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&abs)
            .body(data.into())
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("put_object {abs:?}: {e}")))?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), ProviderError> {
        let abs = self.abs_key(key);
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&abs)
            .send()
            .await
            .map_err(|e| ProviderError::Other(format!("delete_object {abs:?}: {e}")))?;
        Ok(())
    }

    async fn copy_object(&self, from: &str, to: &str) -> Result<(), ProviderError> {
        let abs_from = self.abs_key(from);
        let abs_to = self.abs_key(to);
        let copy_src = format!(
            "{}/{}",
            self.bucket,
            percent_encode_key(&abs_from)
        );
        self.client
            .copy_object()
            .bucket(&self.bucket)
            .key(&abs_to)
            .copy_source(&copy_src)
            .send()
            .await
            .map_err(|e| {
                let svc = e.as_service_error().map(|s| format!("{s:?}"));
                ProviderError::Other(format!(
                    "copy_object {abs_from:?} -> {abs_to:?}: {e:?} svc={svc:?}"
                ))
            })?;
        Ok(())
    }

    async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>, ProviderError> {
        let s3_prefix = self.abs_key(prefix);
        let mut keys: Vec<String> = Vec::new();
        let mut cont: Option<String> = None;

        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&s3_prefix);
            if let Some(c) = cont.as_ref() {
                req = req.continuation_token(c);
            }
            let resp = req.send().await.map_err(|e| {
                ProviderError::Other(format!("list_objects_v2 prefix={s3_prefix:?}: {e}"))
            })?;
            for obj in resp.contents() {
                if let Some(k) = obj.key() {
                    // Strip bucket_prefix to return provider-relative keys
                    // (the same space the caller works in).
                    if let Some(rel) = self.rel_key(k) {
                        keys.push(rel.to_string());
                    }
                }
            }
            match resp.next_continuation_token() {
                Some(t) => cont = Some(t.to_string()),
                None => break,
            }
        }

        Ok(keys)
    }

    async fn create_multipart(&self, key: &str) -> Result<String, ProviderError> {
        let abs = self.abs_key(key);
        let resp = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(&abs)
            .send()
            .await
            .map_err(|e| {
                ProviderError::Other(format!("create_multipart_upload {abs:?}: {e}"))
            })?;
        let upload_id = resp
            .upload_id()
            .ok_or_else(|| ProviderError::Other("missing upload_id".into()))?
            .to_string();
        Ok(upload_id)
    }

    async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: i32,
        data: Bytes,
    ) -> Result<String, ProviderError> {
        let abs = self.abs_key(key);
        let resp = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(&abs)
            .upload_id(upload_id)
            .part_number(part_number)
            .body(data.into())
            .send()
            .await
            .map_err(|e| {
                ProviderError::Other(format!(
                    "upload_part {abs:?} part={part_number}: {e}"
                ))
            })?;
        let etag = resp.e_tag().unwrap_or("").to_string();
        Ok(etag)
    }

    async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: Vec<CompletedPart>,
    ) -> Result<(), ProviderError> {
        let abs = self.abs_key(key);
        let sdk_parts: Vec<aws_sdk_s3::types::CompletedPart> = parts
            .into_iter()
            .map(|p| {
                aws_sdk_s3::types::CompletedPart::builder()
                    .e_tag(p.etag)
                    .part_number(p.part_number)
                    .build()
            })
            .collect();
        let completed = aws_sdk_s3::types::CompletedMultipartUpload::builder()
            .set_parts(Some(sdk_parts))
            .build();
        self.client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(&abs)
            .upload_id(upload_id)
            .multipart_upload(completed)
            .send()
            .await
            .map_err(|e| {
                ProviderError::Other(format!(
                    "complete_multipart_upload {abs:?}: {e}"
                ))
            })?;
        Ok(())
    }

    async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), ProviderError> {
        let abs = self.abs_key(key);
        self.client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(&abs)
            .upload_id(upload_id)
            .send()
            .await
            .map_err(|e| {
                ProviderError::Other(format!("abort_multipart_upload {abs:?}: {e}"))
            })?;
        Ok(())
    }
}

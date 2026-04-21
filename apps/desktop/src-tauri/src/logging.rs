//! Structured logging setup.
//!
//! Two sinks:
//! - stderr, always on at `info`, so `cargo run` / dev builds see events live.
//! - Rolling daily file under `<app_data>/logs/nanocrew-sync.log.YYYY-MM-DD`,
//!   level controlled by the `verbose_logging` pref (off → `info`, on → `debug`).
//!
//! We hold onto the `WorkerGuard` in `AppState` so the background writer
//! thread outlives the subscriber — dropping it on process exit flushes any
//! buffered lines to disk.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize the global tracing subscriber. Returns the `WorkerGuard` that
/// keeps the file-writer thread alive; store it in `AppState` so it drops at
/// process shutdown (not at the end of setup()).
///
/// Safe to call exactly once per process — a second call is a no-op and logs
/// a warning via `eprintln!`, because the global subscriber is already set.
pub fn init(log_dir: &Path, verbose: bool) -> Option<WorkerGuard> {
    // `rolling::daily` creates the directory if needed and rotates at UTC
    // midnight. Old files are never auto-deleted — that's a cleanup task we
    // can bolt on later if logs get hefty.
    let file_appender = rolling::daily(log_dir, "nanocrew-sync.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Crate-scoped filter: our own `nanocrew_sync_lib=<level>` plus a quieter
    // default for dependency chatter (aws_sdk_s3 at `info` is noisy).
    let level = if verbose { "debug" } else { "info" };
    let filter_spec = format!(
        "nanocrew_sync_lib={level},warn,aws_sdk_s3=warn,aws_config=warn,hyper=warn,h2=warn,rustls=warn"
    );

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false)
        .with_level(true);

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(true);

    // `try_init` instead of `init` so a second call (e.g. from tests) returns
    // Err rather than panicking.
    let res = tracing_subscriber::registry()
        .with(EnvFilter::try_new(&filter_spec).unwrap_or_else(|_| EnvFilter::new("info")))
        .with(file_layer)
        .with(stderr_layer)
        .try_init();

    match res {
        Ok(()) => Some(guard),
        Err(e) => {
            eprintln!("logging: subscriber already set ({e}); keeping original");
            None
        }
    }
}

//! Custom `SharedHttpClient` for the AWS SDK.
//!
//! Wires together rustls TLS + an optional corporate HTTPS proxy + an
//! optional extra root CA certificate. Built once per S3 client (per drive
//! + per `test_connection` call); the cost is negligible next to the
//! connect-and-TLS-handshake it precedes.
//!
//! ### Configuration sources
//! The caller passes the values explicitly — we don't peek into prefs here
//! so this module stays reusable from the `test_connection` path (which
//! runs before any drive is stored) as well as from `spawn_mount`.
//!
//! ### Why `build_with_connector_fn`
//! The top-level `aws_smithy_http_client::Builder` does not expose proxy
//! configuration in 1.1.x — only `ConnectorBuilder` does. The public (but
//! `#[doc(hidden)]`) `build_with_connector_fn` bridge is the supported way
//! to inject a pre-built `Connector` into a `SharedHttpClient`.
//!
//! ### Env-var fallback
//! When no explicit `ca_pem_path` is supplied we still honour `SSL_CERT_FILE`
//! (the rustls convention) so users can point at a corporate trust anchor
//! globally without per-drive settings. Native OS roots are always loaded
//! via `rustls-native-certs`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use aws_smithy_http_client::{
    proxy::ProxyConfig,
    tls::{self, rustls_provider::CryptoMode, TlsContext, TrustStore},
    Builder, Connector,
};
use aws_smithy_runtime_api::client::http::SharedHttpClient;

/// Build a `SharedHttpClient` with optional proxy + extra root CA.
///
/// * `proxy_url`   — e.g. `http://proxy.corp:8080` or `http://user:pass@host:port`.
///                   Userinfo is stripped and fed to `with_basic_auth` so
///                   credentials aren't logged in error messages.
/// * `ca_pem_path` — path to a PEM file containing one or more CA certs
///                   that should be trusted in addition to the OS store.
pub fn build(
    proxy_url: Option<&str>,
    ca_pem_path: Option<&Path>,
) -> Result<SharedHttpClient, String> {
    // ── Trust store ──────────────────────────────────────────────────────────
    // `TrustStore::default()` enables platform-native roots; `::empty()`
    // starts clean. We want native + optional extras.
    let mut trust = TrustStore::default();

    // Rustls doesn't honour SSL_CERT_FILE automatically; we do it manually so
    // users can set one env var globally (common on CI + corporate laptops).
    if let Ok(path) = std::env::var("SSL_CERT_FILE") {
        match std::fs::read(&path) {
            Ok(bytes) => trust = trust.with_pem_certificate(bytes),
            Err(e) => tracing::warn!(target: "nanocrew::http", "SSL_CERT_FILE unreadable: {e}"),
        }
    }

    if let Some(p) = ca_pem_path {
        let bytes = std::fs::read(p)
            .map_err(|e| format!("custom CA cert unreadable at {}: {e}", p.display()))?;
        trust = trust.with_pem_certificate(bytes);
    }

    let tls_ctx: TlsContext = tls::TlsContext::builder()
        .with_trust_store(trust)
        .build()
        .map_err(|e| format!("TLS context build failed: {e}"))?;

    // ── Proxy ────────────────────────────────────────────────────────────────
    let proxy_cfg: Option<ProxyConfig> = if let Some(raw) =
        proxy_url.map(str::trim).filter(|s| !s.is_empty())
    {
        let u = url::Url::parse(raw).map_err(|e| format!("proxy URL invalid: {e}"))?;
        let user = u.username().to_string();
        let pass = u.password().unwrap_or("").to_string();

        // Sanitise — drop the userinfo so it doesn't leak into tracing.
        let mut clean = u.clone();
        let _ = clean.set_username("");
        let _ = clean.set_password(None);

        let mut cfg = ProxyConfig::all(clean.as_str())
            .map_err(|e| format!("proxy URL invalid: {e}"))?;
        if !user.is_empty() {
            cfg = cfg.with_basic_auth(user, pass);
        }
        Some(cfg)
    } else {
        None
    };

    // ── Connector + SharedHttpClient bridge ──────────────────────────────────
    // `Connector` is not `Clone`, but `build_with_connector_fn` requires a
    // closure callable multiple times. Capture the config pieces (which *are*
    // `Clone`) and rebuild the `Connector` on each invocation — it's called
    // once per Smithy client, and construction is cheap next to the handshake
    // that follows.
    let provider = tls::Provider::Rustls(CryptoMode::Ring);
    Ok(Builder::new().build_with_connector_fn(move |_settings, _rc| {
        let mut cb = Connector::builder()
            .tls_provider(provider.clone())
            .tls_context(tls_ctx.clone());
        if let Some(ref pc) = proxy_cfg {
            cb = cb.proxy_config(pc.clone());
        }
        cb.build()
    }))
}

/// Convenience: read `proxy_url` + `custom_ca_pem_path` out of the `prefs`
/// table and hand back a ready-to-use `SharedHttpClient`. Empty / missing
/// values mean "no proxy" and "no extra CA" respectively — the resulting
/// client is still rustls-backed and honours OS roots + `SSL_CERT_FILE`.
pub fn build_from_prefs(db: &Mutex<Connection>) -> Result<SharedHttpClient, String> {
    let proxy = crate::commands::prefs::get(db, "proxy_url")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let ca_path: Option<PathBuf> = crate::commands::prefs::get(db, "custom_ca_pem_path")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    build(proxy.as_deref(), ca_path.as_deref())
}

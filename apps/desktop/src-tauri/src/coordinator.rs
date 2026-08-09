//! Sync-coordinator client (Wave 4: push-based cache freshness).
//!
//! The NanoCrew coordinator (see `coordinator/`) is a small Fastify + Redis
//! pub/sub service that broadcasts filesystem-change events between every
//! NanoCrew Sync client mounted on the same S3 bucket. This module is the
//! Rust side of that protocol:
//!
//!   * A background tokio task holds a `WebSocket` connection to
//!     `<base_url>/subscribe`, forwarding every `invalidate` frame the server
//!     pushes down an mpsc channel that the VFS layer drains to drop stale
//!     listing caches sub-second.
//!   * `CoordinatorClient::notify` is a fire-and-forget POST to
//!     `<base_url>/notify` that lets **this** client tell its peers about a
//!     mutation it just committed (upload / delete / rename / mkdir).
//!
//! **Fallback contract:** if `coordinator_url` is unset, the WebSocket cannot
//! connect, or a `notify` call fails, the app must still work exactly as it
//! does today. Every cache line here is guarded by TTL polling and the
//! existing background-refresh task in `winfsp_vfs`; the coordinator only
//! accelerates freshness — it is never on the critical path of a user's IO.
//!
//! **Auth:** the same RS256 license JWT the app already holds for licensing
//! is passed opaquely to the coordinator, which verifies it against the
//! matching public key. This module never inspects the token.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// Events flowing from the WebSocket task up to the VFS layer.
#[derive(Debug, Clone)]
pub enum CoordinatorEvent {
    /// A peer mutated a prefix we should invalidate. `actor` is the peer's
    /// machine id — informational only; the server has already filtered our
    /// own emissions before send.
    Invalidate { prefix: String, actor: String },
}

#[derive(Serialize)]
struct SubscribeMsg<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    bucket: &'a str,
    machine_id: &'a str,
    license_jwt: &'a str,
}

#[derive(Deserialize)]
struct InvalidateFrame {
    #[serde(rename = "type")]
    ty: String,
    // bucket is echoed but we only ever subscribe to one bucket per client, ignore.
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    actor_machine: String,
}

/// Handle passed to the VFS layer. Cloneable and thread-safe.
pub struct CoordinatorClient {
    bucket: String,
    machine_id: String,
    license_jwt: String,
    owner: String,
    base_url: String,
    http: reqwest::Client,
}

impl CoordinatorClient {
    /// Construct a client and spawn the WebSocket subscription loop on `rt`.
    ///
    /// The loop reconnects forever with capped exponential backoff
    /// (1s → 2s → 5s → 15s → 60s) on any disconnect, and never crashes the
    /// app on error — everything is logged at `warn`/`info` on the
    /// `nanocrew::coordinator` target.
    ///
    /// `event_tx` receives `CoordinatorEvent::Invalidate` for every peer
    /// mutation. `emit_status` is invoked whenever the connection state
    /// flips so the UI can show a green/red dot; it fires with `false` while
    /// reconnecting and `true` once a `subscribed` ack has been received.
    pub fn spawn(
        base_url: String,
        bucket: String,
        machine_id: String,
        license_jwt: String,
        owner: String,
        event_tx: mpsc::Sender<CoordinatorEvent>,
        emit_status: Arc<dyn Fn(bool) + Send + Sync>,
        rt: &Runtime,
    ) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let this = Arc::new(Self {
            bucket: bucket.clone(),
            machine_id: machine_id.clone(),
            license_jwt: license_jwt.clone(),
            owner,
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        });

        // Spawn the subscribe loop. Cloned scalars — nothing back-references
        // the Arc<Self>, so the task can outlive an early Arc-drop safely.
        let base_ws = ws_url_from(&this.base_url);
        let bucket_c = bucket;
        let machine_c = machine_id;
        let jwt_c = license_jwt;
        rt.spawn(async move {
            subscribe_loop(base_ws, bucket_c, machine_c, jwt_c, event_tx, emit_status).await;
        });

        this
    }

    /// Fire-and-forget POST /notify. Returns immediately; the request is
    /// driven on `self.http`'s pool. Any failure is logged at `debug` and
    /// swallowed — a coordinator outage MUST NOT block a local write.
    pub fn notify(&self, prefix: &str) {
        let url = format!("{}/notify", self.base_url);
        let body = serde_json::json!({
            "bucket": self.bucket,
            "prefix": prefix,
            "license_jwt": self.license_jwt,
            "machine_id": self.machine_id,
            "owner": self.owner,
        });
        let http = self.http.clone();
        // Detach — we do NOT await. If the runtime is torn down before this
        // completes, the task is cancelled and reqwest cleans up.
        tokio::spawn(async move {
            match http.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(target: "nanocrew::coordinator", "notify ok status={}", resp.status());
                }
                Ok(resp) => {
                    tracing::debug!(target: "nanocrew::coordinator", "notify non-2xx status={}", resp.status());
                }
                Err(e) => {
                    tracing::debug!(target: "nanocrew::coordinator", "notify failed: {e}");
                }
            }
        });
    }
}

/// Turn a base URL (`http://`, `https://`, `ws://`, `wss://`, or bare host)
/// into a WebSocket URL rooted at `/subscribe`.
fn ws_url_from(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    let with_scheme = if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        trimmed.to_string()
    } else {
        // Bare host — default to secure ws.
        format!("wss://{trimmed}")
    };
    format!("{with_scheme}/subscribe")
}

async fn subscribe_loop(
    ws_url: String,
    bucket: String,
    machine_id: String,
    license_jwt: String,
    event_tx: mpsc::Sender<CoordinatorEvent>,
    emit_status: Arc<dyn Fn(bool) + Send + Sync>,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    // Backoff schedule per task spec: 1s, 2s, 5s, 15s, 60s cap.
    const BACKOFF_STEPS: &[u64] = &[1, 2, 5, 15, 60];
    let mut step = 0usize;

    tracing::info!(target: "nanocrew::coordinator", "starting subscribe loop url={ws_url}");

    loop {
        (emit_status)(false);

        let connect_res = tokio_tungstenite::connect_async(&ws_url).await;
        let (mut ws, _resp) = match connect_res {
            Ok(pair) => pair,
            Err(e) => {
                let delay = BACKOFF_STEPS[step.min(BACKOFF_STEPS.len() - 1)];
                tracing::warn!(
                    target: "nanocrew::coordinator",
                    "ws connect failed: {e} — retry in {delay}s"
                );
                tokio::time::sleep(Duration::from_secs(delay)).await;
                step = (step + 1).min(BACKOFF_STEPS.len() - 1);
                continue;
            }
        };

        // Send the mandatory first frame.
        let sub = SubscribeMsg {
            ty: "subscribe",
            bucket: &bucket,
            machine_id: &machine_id,
            license_jwt: &license_jwt,
        };
        let sub_txt = match serde_json::to_string(&sub) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(target: "nanocrew::coordinator", "serialize subscribe: {e}");
                let _ = ws.close(None).await;
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        if let Err(e) = ws.send(Message::Text(sub_txt)).await {
            tracing::warn!(target: "nanocrew::coordinator", "send subscribe: {e}");
            let _ = ws.close(None).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        // We're connected. Reset backoff and flip status.
        step = 0;
        (emit_status)(true);
        tracing::info!(target: "nanocrew::coordinator", "ws subscribed bucket={bucket}");

        // Read loop. tungstenite auto-pongs incoming pings.
        while let Some(msg) = ws.next().await {
            match msg {
                Ok(Message::Text(txt)) => {
                    match serde_json::from_str::<InvalidateFrame>(&txt) {
                        Ok(frame) if frame.ty == "invalidate" => {
                            tracing::debug!(
                                target: "nanocrew::coordinator",
                                "peer invalidate prefix={:?} actor={}",
                                frame.prefix, frame.actor_machine,
                            );
                            let ev = CoordinatorEvent::Invalidate {
                                prefix: frame.prefix,
                                actor: frame.actor_machine,
                            };
                            if event_tx.send(ev).await.is_err() {
                                tracing::info!(
                                    target: "nanocrew::coordinator",
                                    "event receiver dropped — exiting subscribe loop"
                                );
                                let _ = ws.close(None).await;
                                (emit_status)(false);
                                return;
                            }
                        }
                        Ok(_) => { /* subscribed ack / unknown types — ignore */ }
                        Err(e) => {
                            tracing::debug!(
                                target: "nanocrew::coordinator",
                                "unparseable frame ({e}): {txt}"
                            );
                        }
                    }
                }
                Ok(Message::Binary(_)) => { /* server never sends binary; ignore */ }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => { /* handled by tungstenite */ }
                Ok(Message::Close(frame)) => {
                    tracing::info!(
                        target: "nanocrew::coordinator",
                        "ws close from server: {frame:?}"
                    );
                    break;
                }
                Ok(Message::Frame(_)) => { /* low-level, ignore */ }
                Err(e) => {
                    tracing::warn!(target: "nanocrew::coordinator", "ws read error: {e}");
                    break;
                }
            }
        }

        // Disconnected. Announce and back off before reconnecting.
        (emit_status)(false);
        let delay = BACKOFF_STEPS[step.min(BACKOFF_STEPS.len() - 1)];
        tracing::info!(
            target: "nanocrew::coordinator",
            "ws disconnected — reconnect in {delay}s"
        );
        tokio::time::sleep(Duration::from_secs(delay)).await;
        step = (step + 1).min(BACKOFF_STEPS.len() - 1);
    }
}

# NanoCrew Sync — Build Phases

> Source of truth for build progress from alpha → paid GA.
> Update checkboxes as phases land. Each phase ends with a commit + tag.

**Legend:** `[ ]` pending · `[~]` in progress · `[x]` done · `[!]` blocked/deferred

---

## Phase 0 — Ship what we have (alpha)

- [x] Tauri 2 desktop shell with React UI
- [x] Argon2id admin account, SQLite drive + credentials store
- [x] WinFsp 2.1 integration — drives mount at Windows letters
- [x] S3 read path — case-insensitive resolution, range GETs
- [x] S3 write path — temp-file spool + streaming 16 MiB multipart with 8-way concurrency
- [x] Rename (including case-only), delete, mkdir
- [x] Transfers screen with live upload progress
- [x] Activity screen (mount/unmount events)
- [x] Settings screen skeleton (most toggles marked "coming soon")
- [x] WinFsp detection + prompt-to-install
- [x] `.gitignore`, `LICENSE` (proprietary), `README.md`, `PHASES.md`
- [x] Private GitHub repo + initial push
- [x] Tag `v0.1.0-alpha` (superseded by v0.1.1 as first CI-built release)
- [x] Verify signed-MSI + NSIS-EXE build targets produce installers (v0.1.1 shipped both)

**Target:** tagged release with a working MSI and EXE installer.

---

## Phase 0.5 — Auto-update infrastructure

- [x] Ed25519 updater keypair generated (`~/.nanocrew-sync-updater.key`)
- [x] `tauri-plugin-updater` + `tauri-plugin-process` added to Cargo + package.json
- [x] Public key embedded in `tauri.conf.json`; updater endpoints set (GitHub Releases primary, `releases.nanocrew.dev` placeholder)
- [x] Updater + process permissions added to `capabilities/default.json`
- [x] Plugin wired in `lib.rs`
- [x] "Check for updates" button added to Settings → About with download progress + auto-relaunch
- [x] GitHub Actions release workflow (`.github/workflows/release.yml`) — builds signed MSI + NSIS EXE on tag push, generates `latest.json`, publishes to GitHub Release
- [x] `RELEASING.md` documents the release flow and key management
- [x] Upload `TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` to GitHub secrets
- [x] First real end-to-end update test: tag v0.1.1, Actions published, "Check for updates" from v0.1.0 → v0.1.1 install + relaunch verified
- [x] Public asset hosting decided: GitHub Releases on a public repo (repo flipped public 2026-04-21)
- [x] CI hardening: MSVC dev env + NASM + forced `link.exe`/`cl.exe` on PATH + override of broken `CC_*` env var + WinFsp SDK install with `ADDLOCAL=ALL` + space→dot rename on artifacts so `latest.json` URL matches stored asset name

**Target:** every future change reaches our own machines via "Check for updates" — no manual MSI wrangling.

---

## Phase 1 — Correctness & UX polish

- [x] 1.1 Download progress on Transfers page
  - [x] Emit `transfer_progress` from `read()` on first qualifying read (file ≥ 256 KiB)
  - [x] Per-handle `DownloadState` byte counter, throttled to 250 ms progress events
  - [x] `close()` emits terminal `state: "done"` — partial reads treated as done (not error) to avoid flooding UI with phantom failures from Windows thumbnailers / AV / editor probes
  - [~] Idle sweeper deferred — not needed unless we see stuck rows in practice
- [x] 1.2 LIST cache TTL (5 s directory cache + 5 s meta cache) — already live; invalidated on every create/delete/rename/mkdir
- [x] 1.3 Verify small-file single-PUT path (≤ 16 MiB skips MPU) — gated on `upload_id.is_none()` at finalize; MPU only starts when contiguous writes cross `PART_TARGET`
- [x] 1.4 Strip verbose `[winfsp]` logging — keep errors, drop routine traces (stripped `finalize_write begin`, `cleanup flags=`, `upload complete`; all remaining eprintln! calls are error paths)
- [x] 1.5 Replace `tokio::runtime::Builder::new_current_thread` drop-on-block pattern with a single long-lived runtime on the filesystem context — mount thread now builds one multi-thread runtime, uses it for AWS config load, and hands ownership to `S3Fs` for all subsequent IO

**Target:** every mount-to-Explorer action reports progress and stays responsive.

---

## Phase 2 — Provider breadth

- [x] 2.1 Generic `AddDriveS3Screen` — one form, presets for:
  - [x] AWS S3 (22 regions)
  - [x] Backblaze B2 (6 regions)
  - [x] Cloudflare R2 (custom endpoint flow for `<account-id>.r2.cloudflarestorage.com`)
  - [x] MinIO (self-hosted, custom endpoint)
  - [x] IDrive e2 (8 regions)
  - [x] DigitalOcean Spaces (8 regions)
  - [x] Storj (global gateway)
  - [x] Scaleway Object Storage (3 regions)
  - [x] Contabo Object Storage (3 regions)
  - [x] Oracle Cloud Object Storage (custom endpoint flow)
  - [x] Linode Object Storage (9 regions)
  - [x] Vultr Object Storage (7 regions)
- [x] 2.2 Provider picker rewrite — single form at `/add-drive/<providerId>`; Wasabi kept as the recommended preset at the top
- [x] 2.3 Region/endpoint presets per provider — `S3_PROVIDER_PRESETS` in `packages/ui/src/s3-providers.ts` is the single source of truth
- [x] 2.4 "Test connection" button — already calls `list_objects_v2 max_keys=1` on the backend (HeadBucket-equivalent but also validates list permission); errors now prettified (AccessDenied/SignatureDoesNotMatch/NoSuchBucket/DNS/timeout → human-readable)

**Target:** any S3-compatible bucket mountable in ≤ 60 seconds from "Add drive" click.

---

## Phase 3 — Security features

- [x] 3.1 Cache encryption — every on-disk cache block is encrypted with AES-256-GCM (`aes-gcm` crate). Per-drive key = `SHA-256(machine_id || cache_key_salt || "drive-{id}")`; `cache_key_salt` is 32 random bytes generated on first use and persisted to the `prefs` table (so two installs on the same machine still produce different ciphertexts). Each block gets a fresh 12-byte random nonce, prepended to the ciphertext+tag on disk: `[nonce:12][ciphertext][tag:16]`. A failed GCM decrypt drops the row + file so the next read re-fetches — silent recovery from tampering, wrong key, or pre-encryption legacy blobs. Design note: we intentionally don't derive the key from the user password, because drives (and therefore cache reads) stay live through the Windows-lock flow — a password-derived key would break reads after lock. The machine-ID + install-salt combo still protects against "cache files copied off the laptop in isolation". 3 existing cache tests pass with encryption on.
- [x] 3.2 Credential-at-rest re-encryption — DPAPI-wrap S3 secrets in SQLite (`credentials.rs` + new `dpapi.rs`; `v1:<base64>` envelope with legacy-plaintext migration on first read; `add_drive` routes through `credentials::store` and never persists the raw secret)
- [x] 3.3 Session lock — "lock app on minimize" wired via Tauri `onResized` + `isMinimized`; `verify_password` backend command; `LockScreen` UI with password prompt; drives stay mounted throughout (files remain accessible in Explorer while locked); manual Lock button on Account screen; opt-in toggle in Settings → Security. PIN fallback deferred to a later polish pass.
- [x] 3.4 Audit log — the `activity` table (append-only) and `activity::record` helper already capture mount, unmount, add_drive, update_drive, delete_drive, account_created, sign_in, sign_out, password_changed. Added the missing lock/unlock path: new `record_lock_event(locked, reason)` command in `auth.rs` writes `auth/lock` and `auth/unlock` rows with the reason tag (`"minimize"` / `"manual"`). Frontend `App.tsx` calls it on both the minimize-triggered lock and the manual Lock button, plus on unlock via `handleUnlock`. Full security trail visible in Activity screen.

**Target:** "zero plaintext at rest" claim substantiated end-to-end.

---

## Phase 4 — File Lock (parity blocker vs RaiDrive)

- [x] 4.1 Honor Windows share modes in `open()` — WinFsp's kernel driver enforces share-modes natively within a single host; we layer a per-mount `local_writers` HashSet on top so a second writer-open on this machine fast-fails with `STATUS_SHARING_VIOLATION` regardless of how it arrived. Writer access detected from `FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES | DELETE | GENERIC_WRITE | GENERIC_ALL`.
- [x] 4.2 Lockfile detection — `file_lock::classify_lockfile` recognises Office `~$*.{docx,xlsx,pptx,doc,xls,ppt}`, LibreOffice `.~lock.*#`, vim `.*.swp`, and generic `*.lock`. VFS emits `file_lock_event` Tauri events on open/create/close with `state: "lockfile_created" | "lockfile_released"` so future UI can show "being edited" banners.
- [x] 4.3 Cross-device lock coordination — sentinel objects at `.nanocrew/locks/<sha256>.json` with schema-versioned JSON payload (`v`, `key`, `machine`, `owner`, `acquired_at`, `expires_at`). Writer-open checks sentinel, rejects foreign-machine holders with `STATUS_SHARING_VIOLATION`, emits `sentinel_conflict` event. `machine_id` sourced from `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` (falls back to hostname). 15-minute TTL; heartbeat function present but unused pending a per-mount refresh task. Owner tagged from signed-in username (or `auto-mount` for startup mounts). Internal `.nanocrew/` keys bypass all lock checks to prevent self-deadlock.
- [x] 4.4 File Browser lock indicator — `commands/locks.rs` added with a `list_file_locks(drive_id)` Tauri command that enumerates `.nanocrew/locks/` in the drive's bucket, parses each sentinel JSON, filters out expired rows, and returns `FileLockEntry` rows (key, owner, machine, acquired/expires, `is_ours`). The File Browser fetches this map on drive change + refresh and renders a padlock icon next to each locked entry — red for foreign-machine locks, muted for our own; tooltip shows owner, machine prefix, and expiry time.
- [x] 4.5 "Break lock" admin action — `break_file_lock(drive_id, key)` command force-deletes the sentinel via `file_lock::release` and writes an audit row `mount/break_lock` with the acting user and the affected key (severity WARN). Frontend: a red "Break lock" item in the file context menu (only shown when a lock exists), gated by a `window.confirm` that names the lock holder.

**Target:** Office / LibreOffice / OpenOffice collaborative editing on a mounted drive works correctly across two machines.

---

## Phase 5 — Reliability plumbing

- [x] 5.1 Auto-mount on startup — `auto_mount_drives` in `lib.rs` reads `auto_mount = 1` rows from SQLite and spawns mount tasks in parallel during `tauri::Builder::setup`. Uses the `"auto-mount"` sentinel owner since no user has signed in yet.
- [x] 5.2 System tray icon — `TrayIconBuilder` in `lib.rs` with Show/Quit menu. Left-click toggles visibility; close-to-tray via `on_window_event` WindowEvent::CloseRequested → `api.prevent_close()` + `window.hide()`. Single-instance plugin refocuses instead of spawning twin tray icons.
- [x] 5.3 Start at Windows sign-in — new `commands::system::{get_autostart, set_autostart}` write to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` under value name `NanoCrewSync`. HKCU = no UAC. Settings → General "Launch at Windows sign-in" toggle wired via `AutostartRow`; includes `--hidden` arg so sign-in wakes to tray.
- [x] 5.4 Toast notifications — `tauri-plugin-notification` + `@tauri-apps/plugin-notification` wired. `useDriveNotifications(token)` hook at the AppShell level listens for `drive_status_changed` (mount / unmount / error transitions, skipping the initial status so we don't fire on load) and `transfer_progress` (upload success/fail). OS permission is requested lazily on first toast. Settings → Notifications exposes three real prefs: `notify_mount_events` (default on), `notify_errors` (default on), `notify_uploads` (default off — noisy). Drive names resolved from `list_drives` so toasts read "Wasabi bucket (Z:)" instead of "drive #3".
- [x] 5.5 Bandwidth throttle — new `throttle::RateLimiter` (async token bucket, credit-debit semantics so chunks larger than burst still work) wired into every S3 I/O boundary in `winfsp_vfs.rs`: `get_range` (download side), `upload_part` in the multipart dispatch loop, and `upload_single_put` for small files. `S3Fs` carries one `Arc<RateLimiter>` per direction; unlimited limiters short-circuit in `acquire`. New `prefs::get_rate_bps` parses MB/s pref strings (decimals + 0-means-unlimited) into bytes/sec. `MountConfig` gains `upload_rate_bps` / `download_rate_bps` fields, populated from prefs `upload_rate_mbps` / `download_rate_mbps` at both MountConfig build sites (manual mount + auto_mount_drives). Settings → Network → Bandwidth now has two real `PrefInput` fields; 3 unit tests cover unlimited / within-burst / proportional-wait. Per-drive overrides deferred — needs a drive-detail UI that doesn't exist yet.
- [x] 5.6 LRU local cache + pin — new `cache.rs` implements a per-drive on-disk range cache keyed by `sha256(object_key)` with 2-level directory fanout at `%LOCALAPPDATA%\NanoCrew\Sync\cache\drive-<id>\`, indexed by `cache_entries` + `pinned_keys` SQLite tables (schema in `db.rs`). `S3Fs::get_range` decomposes reads into 1 MiB-aligned blocks: cache hits read from disk and bump `last_access`, misses fetch from S3 and write through. A background sweeper thread per mount (started in `DiskCache::start_eviction`) evicts oldest-first once `SUM(size_bytes)` exceeds the per-drive `cache_size_gb`; rows with matching `pinned_keys` are skipped via LEFT JOIN. New Tauri commands `pin_file` / `unpin_file` / `is_file_pinned` / `list_pinned_files` in `commands/cache.rs`. `FileBrowserScreen` gains a right-click context menu with "Keep on device" / "Unpin from device" plus a green-dot indicator on pinned rows. Settings → Cache & storage replaces the COMING SOON toggle with a real `cache_enabled` pref (default on). Caches are invalidated on upload success, delete, and rename so next reader sees fresh bytes. 3 unit tests cover round-trip, invalidate, and pin-survives-eviction.
- [x] 5.7 Reconnect on network blip — AWS SDK retry config bumped from default `Standard/3 attempts` to `adaptive/8 attempts` in `mounts.rs::spawn_mount`, so transient transport errors (Wi-Fi handoff, DNS hiccup, provider throttling) retry quietly instead of surfacing as Explorer "copy failed" dialogs.

**Target:** app behaves like a polished background utility — survives reboots, sleeps, Wi-Fi drops.

---

## Phase 6 — Activity & audit

- [x] 6.1 Persist activity log to SQLite `activity` table — new table with `id, ts, kind, action, severity, drive_id, actor, target, message` + ts DESC / kind indexes. Central `commands::activity::record(...)` helper wired into `auth::{create_admin, sign_in, sign_out, change_password}` and `drives::{add_drive, remove_drive, mount_drive, unmount_drive}` plus the `lib.rs::auto_mount_drives` success/error paths, so every user-visible domain event lands in the log. Inserts also fan out a live `activity_appended` Tauri event.
- [x] 6.2 Activity screen rewrite — `ActivityScreen` now loads from `list_activity` (kinds, severity, since, limit) and live-appends via `activity_appended`. UI adds KIND chips (ALL/MOUNT/DRIVE/AUTH/FILE/SYSTEM), an ERRORS-ONLY toggle with live count, free-text filter across action/actor/target/message, Clear log (`clear_activity`) and Export CSV (client-side Blob download, RFC-4180-safe). A matching Rust `export_activity_csv(path)` command is also registered for programmatic dumps.
- [x] 6.3 Error aggregation — client-side grouping by `(kind, action, target, message)` in the Activity screen, surfaced as an opt-in "GROUP REPEATS" toggle alongside "ERRORS ONLY". Each group shows occurrence count, first + latest timestamps, a Retry button for mount failures (invokes `mount_drive` with the stored `drive_id`), and a Dismiss button. Dismissals persist in `localStorage` under `nanocrew_activity_dismissed_v1` keyed by signature + ts, so new occurrences auto-revive the group.

**Target:** an admin can reconstruct exactly what happened on a drive in the last 30 days. (All three objectives now shipped.)

---

## Phase 7 — Settings depth (wire every "coming soon")

- [x] 7.1 General — startup at sign-in already done in Phase 5.3. Added `prefs` key/value SQLite table + `commands::prefs::{get_pref, set_pref, clear_pref}` commands and a `PrefToggle` UI helper. "Start minimized to system tray" now persists as pref `start_minimized` and is honored in `lib.rs` setup (window hidden before first paint if the pref or `--hidden` argv is set). "Check for updates automatically" persists as pref `auto_update_check` (default on).
- [x] 7.2 Drives — "Auto-mount on startup" and "Read-only by default" toggles now persist as prefs `default_auto_mount` / `default_readonly`; the AddDriveS3Screen pre-selects from these on mount so the "default for new drives" row actually changes new-drive behavior. Per-drive overrides still shown as placeholder card.
- [x] 7.3 Network — HTTPS proxy + custom CA certificate now fully wired. Added `aws-smithy-http-client` (rustls-ring) + `aws-smithy-runtime-api` + `url` crates. New `src-tauri/src/http_client.rs` module builds a `SharedHttpClient` from an optional proxy URL (with stripped + re-applied basic auth so credentials don't leak into tracing) and an optional extra PEM trust anchor, on top of rustls with OS-native roots and `SSL_CERT_FILE` honoured manually. `build_from_prefs(&db)` reads prefs `proxy_url` + `custom_ca_pem_path` and hands the resulting `SharedHttpClient` to `aws_config::ConfigLoader::http_client(...)` in all four AWS SDK call sites: `mounts::spawn_mount` (live mounts), `commands::drives::test_connection`, `list_drive_objects`, and `list_buckets`. Settings → Network replaces the Proxy & TLS placeholder with two real `PrefInput` text fields (debounced save-on-change). SOCKS5/bandwidth throttle still deferred to 5.5 — aws-smithy-http-client 1.1 only exposes HTTP/HTTPS proxies.
- [x] 7.4 Cache — Settings › Cache & storage now has a real "Cache location" card. Backend: `default_cache_root()` returns `%LOCALAPPDATA%\NanoCrew\Sync\cache`; `get_cache_root()` honors the `cache_root` pref when set (non-empty) and falls back to the default; new `get_cache_root_info` command returns `(effective, default, is_custom)`. `MountConfig` gains a `cache_root: PathBuf` that `spawn_mount` joins with `drive-<id>` for per-drive isolation; both `mount_drive` and `auto_mount_drives` resolve it once from prefs. Frontend: `CacheLocationCard` shows the effective path with a DEFAULT/CUSTOM badge, a `PrefInput` bound to `cache_root` (blank = default), and an "Open folder" button. Changes apply at next mount — existing cache data isn't migrated.
- [x] 7.5 Security — "Require password after Windows lock" now persists as pref `lock_on_session_lock` (replacing the COMING SOON row). Binding it to the actual `WTS_SESSION_LOCK`/sleep events is deferred; pref is in place for Phase 7.5a.
- [x] 7.6 Notifications — `useDriveNotifications` hook in `hooks/useDriveNotifications.ts` (wired once at AppShell level in `App.tsx`) bridges backend events to native Win10/11 toasts via `@tauri-apps/plugin-notification`. Listens to `drive_status_changed` (mount/unmount success + mount errors) and `transfer_progress` (upload done/error). Gated per-category by the existing `notify_mount_events` / `notify_errors` / `notify_uploads` prefs. OS permission resolved once per session and cached. Cached `list_drives` label map keeps toasts friendly ("Backup (Z:)" instead of "drive #3").
- [x] 7.7 Advanced verbose logging — `tracing` + `tracing-subscriber` + `tracing-appender` wired in `logging.rs`; rolling daily file at `%APPDATA%\NanoCrew\Sync\logs\nanocrew-sync.log.YYYY-MM-DD`. Level gated by `verbose_logging` pref (info → debug). `WorkerGuard` lives in `AppState` so buffered writes flush on exit. Settings → Advanced exposes the toggle + "Open log folder" button. `eprintln!` calls in `auto_mount_drives` and `activity::record` migrated to structured `tracing::error!/warn!`.

**Target:** (partial) most General/Drives/Security rows are real; Network/Cache/Notifications/Advanced still show COMING SOON pending upstream work.

---

## Phase 8 — Internationalization

- [~] 8.1 `react-i18next` + `en.json` baseline — scaffold + four of the highest-traffic screens migrated: ActivityScreen, OnboardingScreen, SetupScreen, DashboardScreen. `en.json` now has ~90 keys covering tray, common buttons, activity, settings section headings, onboarding, setup, and the dashboard (incl. pluralised `subtitleOne/Other`, status labels, and error-hint fragments). `AddDriveS3Screen`, `AddDrivePickerScreen`, `SettingsScreen`, `AccountScreen`, `FileBrowserScreen`, `TransfersScreen`, `ErrorScreen`, and `LockScreen` still contain English literals — next wave.
- [ ] 8.2 Ship 6 languages (DE, FR, ES, PT-BR, JA) — deferred until 8.1 extraction is complete; adding locales before the en.json source is finalised just means churn.
- [ ] 8.3 Language picker in Settings → General — deferred (scaffold exposes `SUPPORTED_LOCALES` for the picker to consume once non-EN locales land).
- [ ] 8.4 GitHub-based contribution flow for community-translated locales — requires the locales/ directory structure to be stable first.
- [ ] 8.5 RTL layout smoke test — deferred until at least one RTL locale (AR/HE) is added.

**Target:** 6 languages at release; contributor path published for the rest. (Scaffold only; full extraction/translation is future work.)

---

## Phase 9 — Licensing & pricing gate

- [ ] 9.1 License key format — signed JWT with tier, seat count, expiry, machine limit
- [ ] 9.2 Activation flow — paste key in Settings → About → Activate
- [ ] 9.3 Machine fingerprint — hardware UUID + MAC hash; deactivate on uninstall
- [ ] 9.4 Tier enforcement:
  - [ ] Free: 1 drive, 50 GB/mo transfer, core providers
  - [ ] Personal ($49 one-time): unlimited drives, all providers, 1 PC, 1 year of updates
  - [ ] Pro ($99/yr or $199 lifetime): 3 PCs, File Lock, cache encryption, priority support
  - [ ] Team ($12/user/mo, 5-seat min): floating seats, SSO, audit log, central drive config
- [ ] 9.5 14-day full-Pro trial on first install
- [ ] 9.6 In-app upgrade CTAs (Stripe or Lemon Squeezy checkout link)

**Target:** a Free user can upgrade to Pro and see the gated features unlock in < 60 s.

### Design notes for the next session

Phase 9 is deliberately **not** scaffolded tonight — each sub-item depends on product decisions that need your input before I lock them in with code:

1. **JWT key-signing infrastructure.** We need an `ed25519` (or RS256) keypair. The public key ships embedded in the Rust binary; the private key signs licenses on a server (or a local dev tool for now). Decision needed: is the signing service a simple Node/Rust CLI you run manually, or does it live in a Cloudflare Worker / Lambda behind an admin dashboard?
2. **Machine fingerprint source.** `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` is already read by `file_lock::machine_id()` and is the obvious candidate. Hardware UUID (`wmic csproduct get uuid`) is more stable across reinstalls but needs elevation on some SKUs. Pick one.
3. **Trial start timestamp.** Will live in the `prefs` table (key: `trial_started_at`). Written on first successful sign-in after install. Straightforward — but we need the trial length locked (14 days vs 30).
4. **Enforcement gates.** The schema lists Free/Personal/Pro/Team tiers with specific feature gates (drive count, File Lock, cache encryption). Every gate is a `fn ensure_tier(state, feature) -> Result<()>` call at the command entry point. Easy to add — but the *list* of gates needs to match what's actually behind the paywall, which is a pricing decision.
5. **Payment provider.** Stripe vs Lemon Squeezy vs Paddle changes the checkout-return URL and webhook format. The licensing server design depends on this. This is fundamentally a business decision.
6. **Offline grace period.** If the machine can't reach the license server for N days, what happens? Silent downgrade to Free? Lockout? Warning banner only?

When you're ready, the landing points in code are:
- `commands::prefs` (already exists) — stash trial start / cached license JWT
- New `commands::licensing` module with `get_status`, `activate`, `deactivate`, `machine_fingerprint`
- New `licensing::verify` Rust module that parses the JWT and checks signature + expiry + machine
- UI: new "License" section in `SettingsScreen`, plus a tier badge on `AccountScreen`

I'd rather leave these hooks empty and well-documented than ship a stub that pretends to enforce anything.

---

## Phase 10 — Website + marketing

- [ ] 10.1 Landing page (Astro static site) — comparison table vs RaiDrive
- [ ] 10.2 Pricing page — 4 tiers, lifetime/annual toggle, FAQ
- [ ] 10.3 Docs site (MDX, searchable) — install, provider setup, troubleshooting
- [ ] 10.4 Release automation — GitHub Actions signs and publishes MSI + EXE to `releases.nanocrew.dev`
- [ ] 10.5 Auto-updater — Tauri updater consuming the release feed

**Target:** a prospect can land on nanocrew.dev, pick a tier, check out, and install — unattended.

---

## Phase 11 — Beta → GA

- [ ] 11.1 EV code-signing certificate acquired; CI signs every MSI + EXE
- [ ] 11.2 Closed beta — 20 invited users, feedback channel in Discord
- [ ] 11.3 Crash reporting — Sentry for Rust + JS
- [ ] 11.4 Public beta
- [ ] 11.5 GA — launch-week pricing ($39 Personal for first 90 days)
- [ ] 11.6 Post-launch: Product Hunt, r/sysadmin, r/DataHoarder, Wasabi partner listing

**Target:** paying customers. Real ones, not friends.

---

## Deferred (post-GA)

- [ ] WebDAV backend
- [ ] SFTP backend
- [ ] Tailscale detection + bypass for LAN-adjacent buckets
- [ ] Local Disk Public/Private share exposure (SMB)
- [ ] macOS port (FUSE-T / macFUSE)
- [ ] Linux port (FUSE3)

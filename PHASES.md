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

- [ ] 3.1 Cache encryption — AES-256-GCM on temp spool files, per-session key derived from machine ID + user password (deferred: needs part-aligned chunk encryption so MPU reads stay cheap; crates wired, module design TBD)
- [x] 3.2 Credential-at-rest re-encryption — DPAPI-wrap S3 secrets in SQLite (`credentials.rs` + new `dpapi.rs`; `v1:<base64>` envelope with legacy-plaintext migration on first read; `add_drive` routes through `credentials::store` and never persists the raw secret)
- [x] 3.3 Session lock — "lock app on minimize" wired via Tauri `onResized` + `isMinimized`; `verify_password` backend command; `LockScreen` UI with password prompt; drives stay mounted throughout (files remain accessible in Explorer while locked); manual Lock button on Account screen; opt-in toggle in Settings → Security. PIN fallback deferred to a later polish pass.
- [ ] 3.4 Audit log of security-sensitive actions (mount, unmount, credential add, lock)

**Target:** "zero plaintext at rest" claim substantiated end-to-end.

---

## Phase 4 — File Lock (parity blocker vs RaiDrive)

- [x] 4.1 Honor Windows share modes in `open()` — WinFsp's kernel driver enforces share-modes natively within a single host; we layer a per-mount `local_writers` HashSet on top so a second writer-open on this machine fast-fails with `STATUS_SHARING_VIOLATION` regardless of how it arrived. Writer access detected from `FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES | DELETE | GENERIC_WRITE | GENERIC_ALL`.
- [x] 4.2 Lockfile detection — `file_lock::classify_lockfile` recognises Office `~$*.{docx,xlsx,pptx,doc,xls,ppt}`, LibreOffice `.~lock.*#`, vim `.*.swp`, and generic `*.lock`. VFS emits `file_lock_event` Tauri events on open/create/close with `state: "lockfile_created" | "lockfile_released"` so future UI can show "being edited" banners.
- [x] 4.3 Cross-device lock coordination — sentinel objects at `.nanocrew/locks/<sha256>.json` with schema-versioned JSON payload (`v`, `key`, `machine`, `owner`, `acquired_at`, `expires_at`). Writer-open checks sentinel, rejects foreign-machine holders with `STATUS_SHARING_VIOLATION`, emits `sentinel_conflict` event. `machine_id` sourced from `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` (falls back to hostname). 15-minute TTL; heartbeat function present but unused pending a per-mount refresh task. Owner tagged from signed-in username (or `auto-mount` for startup mounts). Internal `.nanocrew/` keys bypass all lock checks to prevent self-deadlock.
- [ ] 4.4 File Browser lock indicator — red padlock icon on locked files, shows locker's device name
- [ ] 4.5 "Break lock" admin action for orphaned locks

**Target:** Office / LibreOffice / OpenOffice collaborative editing on a mounted drive works correctly across two machines.

---

## Phase 5 — Reliability plumbing

- [x] 5.1 Auto-mount on startup — `auto_mount_drives` in `lib.rs` reads `auto_mount = 1` rows from SQLite and spawns mount tasks in parallel during `tauri::Builder::setup`. Uses the `"auto-mount"` sentinel owner since no user has signed in yet.
- [x] 5.2 System tray icon — `TrayIconBuilder` in `lib.rs` with Show/Quit menu. Left-click toggles visibility; close-to-tray via `on_window_event` WindowEvent::CloseRequested → `api.prevent_close()` + `window.hide()`. Single-instance plugin refocuses instead of spawning twin tray icons.
- [x] 5.3 Start at Windows sign-in — new `commands::system::{get_autostart, set_autostart}` write to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` under value name `NanoCrewSync`. HKCU = no UAC. Settings → General "Launch at Windows sign-in" toggle wired via `AutostartRow`; includes `--hidden` arg so sign-in wakes to tray.
- [ ] 5.4 Toast notifications — mount/unmount, upload complete, mount error (deferred — requires tauri-plugin-notification + event translation layer)
- [ ] 5.5 Bandwidth throttle — per-drive upload/download caps (deferred — needs part-dispatch governor coupled to SQLite prefs)
- [ ] 5.6 LRU local cache + pin — keep N GB of recently-read bytes; pin files for offline; respect per-drive `cache_size_gb` (deferred — largest reliability feature; needs on-disk cache layout + eviction thread)
- [x] 5.7 Reconnect on network blip — AWS SDK retry config bumped from default `Standard/3 attempts` to `adaptive/8 attempts` in `mounts.rs::spawn_mount`, so transient transport errors (Wi-Fi handoff, DNS hiccup, provider throttling) retry quietly instead of surfacing as Explorer "copy failed" dialogs.

**Target:** app behaves like a polished background utility — survives reboots, sleeps, Wi-Fi drops.

---

## Phase 6 — Activity & audit

- [x] 6.1 Persist activity log to SQLite `activity` table — new table with `id, ts, kind, action, severity, drive_id, actor, target, message` + ts DESC / kind indexes. Central `commands::activity::record(...)` helper wired into `auth::{create_admin, sign_in, sign_out, change_password}` and `drives::{add_drive, remove_drive, mount_drive, unmount_drive}` plus the `lib.rs::auto_mount_drives` success/error paths, so every user-visible domain event lands in the log. Inserts also fan out a live `activity_appended` Tauri event.
- [x] 6.2 Activity screen rewrite — `ActivityScreen` now loads from `list_activity` (kinds, severity, since, limit) and live-appends via `activity_appended`. UI adds KIND chips (ALL/MOUNT/DRIVE/AUTH/FILE/SYSTEM), an ERRORS-ONLY toggle with live count, free-text filter across action/actor/target/message, Clear log (`clear_activity`) and Export CSV (client-side Blob download, RFC-4180-safe). A matching Rust `export_activity_csv(path)` command is also registered for programmatic dumps.
- [ ] 6.3 Error aggregation — deferred: grouping repeated errors and surfacing inline recovery actions belongs with the toast/notification work in Phase 5.4 and Phase 7.6.

**Target:** an admin can reconstruct exactly what happened on a drive in the last 30 days. (Achieved for 6.1/6.2; 6.3 deferred.)

---

## Phase 7 — Settings depth (wire every "coming soon")

- [x] 7.1 General — startup at sign-in already done in Phase 5.3. Added `prefs` key/value SQLite table + `commands::prefs::{get_pref, set_pref, clear_pref}` commands and a `PrefToggle` UI helper. "Start minimized to system tray" now persists as pref `start_minimized` and is honored in `lib.rs` setup (window hidden before first paint if the pref or `--hidden` argv is set). "Check for updates automatically" persists as pref `auto_update_check` (default on).
- [x] 7.2 Drives — "Auto-mount on startup" and "Read-only by default" toggles now persist as prefs `default_auto_mount` / `default_readonly`; the AddDriveS3Screen pre-selects from these on mount so the "default for new drives" row actually changes new-drive behavior. Per-drive overrides still shown as placeholder card.
- [ ] 7.3 Network — deferred: proxy/TLS/throttle are substantial work and belong with Phase 5.5 bandwidth throttle.
- [ ] 7.4 Cache — deferred: requires a real on-disk cache (Phase 5.6 LRU) before a location picker makes sense.
- [x] 7.5 Security — "Require password after Windows lock" now persists as pref `lock_on_session_lock` (replacing the COMING SOON row). Binding it to the actual `WTS_SESSION_LOCK`/sleep events is deferred; pref is in place for Phase 7.5a.
- [ ] 7.6 Notifications — deferred: depends on toast delivery (Phase 5.4).
- [ ] 7.7 Advanced verbose logging — deferred: needs `tracing` + `tracing-appender` wiring across crates; kept as COMING SOON for now.

**Target:** (partial) most General/Drives/Security rows are real; Network/Cache/Notifications/Advanced still show COMING SOON pending upstream work.

---

## Phase 8 — Internationalization

- [~] 8.1 `react-i18next` + `en.json` baseline — **scaffold done**: `i18next`, `react-i18next`, and `i18next-browser-languagedetector` added to `apps/desktop`; `src/i18n/index.ts` bootstraps i18n with a localStorage cache (`nanocrew_locale`) and a single `en.json` file containing ~25 baseline keys (tray, common buttons, activity, settings sections). ActivityScreen migrated as the proof-of-life consumer. Remaining strings across the app still live as English literals — extraction is the bulk of the work and is deferred.
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

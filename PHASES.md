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
- [ ] 1.2 LIST cache TTL (5 s directory cache) so remote changes become visible without remount
- [ ] 1.3 Verify small-file single-PUT path (< 16 MiB skips MPU)
- [ ] 1.4 Strip verbose `[winfsp]` logging — keep errors, drop routine traces
- [ ] 1.5 Replace `tokio::runtime::Builder::new_current_thread` drop-on-block pattern with a single long-lived runtime on the filesystem context

**Target:** every mount-to-Explorer action reports progress and stays responsive.

---

## Phase 2 — Provider breadth

- [ ] 2.1 Generic `AddDriveS3Screen` — one form, presets for:
  - [ ] AWS S3
  - [ ] Backblaze B2
  - [ ] Cloudflare R2
  - [ ] MinIO (self-hosted)
  - [ ] IDrive e2
  - [ ] DigitalOcean Spaces
  - [ ] Storj
  - [ ] Scaleway Object Storage
  - [ ] Contabo Object Storage
  - [ ] Oracle Cloud Object Storage
  - [ ] Linode Object Storage
  - [ ] Vultr Object Storage
- [ ] 2.2 Provider picker rewrite — route all S3-compatible to the generic form; Wasabi kept as a convenience preset
- [ ] 2.3 Region/endpoint presets per provider (avoid hand-typing hosts)
- [ ] 2.4 "Test connection" button — `HeadBucket` before save, surface permission / endpoint errors inline

**Target:** any S3-compatible bucket mountable in ≤ 60 seconds from "Add drive" click.

---

## Phase 3 — Security features

- [ ] 3.1 Cache encryption — AES-256-GCM on temp spool files, per-session key derived from machine ID + user password
- [ ] 3.2 Credential-at-rest re-encryption — DPAPI-wrap S3 secrets in SQLite (currently plain under FS-perms protection)
- [ ] 3.3 Session lock — "lock app on minimize" and "require password after screen lock" wired; PIN fallback
- [ ] 3.4 Audit log of security-sensitive actions (mount, unmount, credential add, lock)

**Target:** "zero plaintext at rest" claim substantiated end-to-end.

---

## Phase 4 — File Lock (parity blocker vs RaiDrive)

- [ ] 4.1 Honor Windows share modes in `open()` — reject conflicting opens with `STATUS_SHARING_VIOLATION`
- [ ] 4.2 Lockfile detection — recognize Office `~$*.docx`, LibreOffice `.~lock.*#`, `.lock` patterns
- [ ] 4.3 Cross-device lock coordination — sentinel objects in bucket `.nanocrew/locks/`, TTL heartbeat, conflict UI
- [ ] 4.4 File Browser lock indicator — red padlock icon on locked files, shows locker's device name
- [ ] 4.5 "Break lock" admin action for orphaned locks

**Target:** Office / LibreOffice / OpenOffice collaborative editing on a mounted drive works correctly across two machines.

---

## Phase 5 — Reliability plumbing

- [ ] 5.1 Auto-mount on startup — reads `auto_mount` flag, mounts flagged drives at app boot
- [ ] 5.2 System tray icon — minimize-to-tray, drive status menu, unmount/remount, quit
- [ ] 5.3 Start at Windows sign-in — registry Run key toggle
- [ ] 5.4 Toast notifications — mount/unmount, upload complete, mount error
- [ ] 5.5 Bandwidth throttle — per-drive upload/download caps (governor on part dispatch)
- [ ] 5.6 LRU local cache + pin — keep N GB of recently-read bytes; pin files for offline; respect per-drive `cache_size_gb`
- [ ] 5.7 Reconnect on network blip — exponential backoff retry on S3 transport errors

**Target:** app behaves like a polished background utility — survives reboots, sleeps, Wi-Fi drops.

---

## Phase 6 — Activity & audit

- [ ] 6.1 Persist activity log to SQLite `activity` table — mount, unmount, upload, download, rename, delete, error
- [ ] 6.2 Activity screen rewrite — filter by drive / event type / date; export CSV
- [ ] 6.3 Error aggregation — group repeated errors, show recovery actions

**Target:** an admin can reconstruct exactly what happened on a drive in the last 30 days.

---

## Phase 7 — Settings depth (wire every "coming soon")

- [ ] 7.1 General: startup at sign-in, start minimized to tray, auto-update check
- [ ] 7.2 Drives: auto-mount default, read-only default, per-drive context dialog
- [ ] 7.3 Network: upload/download limits (bytes/sec), HTTP/SOCKS5 proxy, custom CA certificate
- [ ] 7.4 Cache: auto-evict, cache location picker, max size slider
- [ ] 7.5 Security: lock on screen lock, lock on minimize
- [ ] 7.6 Notifications: 4 toggles wired to real toast delivery
- [ ] 7.7 Advanced: verbose logging → `tracing` file appender

**Target:** no `COMING SOON` pill anywhere in Settings.

---

## Phase 8 — Internationalization

- [ ] 8.1 `react-i18next` + `en.json` baseline — extract every user-facing string
- [ ] 8.2 Ship 6 languages: EN, DE, FR, ES, PT-BR, JA
- [ ] 8.3 Language picker in Settings → General (replacing the static "English (South Africa)" row)
- [ ] 8.4 GitHub-based contribution flow for community-translated locales
- [ ] 8.5 RTL layout smoke test (in preparation for AR / HE)

**Target:** 6 languages at release; contributor path published for the rest.

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

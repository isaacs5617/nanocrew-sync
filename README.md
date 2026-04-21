# NanoCrew Sync

Mount S3-compatible cloud storage (Wasabi, Amazon S3, Backblaze B2, Cloudflare R2, MinIO) as native Windows drive letters. Built on Tauri 2 and WinFsp.

> **Status:** Early access (v0.1.0-alpha). Single-user, single-machine. Paid tiers and multi-provider support land in upcoming releases — see [PHASES.md](./PHASES.md) for the roadmap.

---

## Why NanoCrew Sync

- **Real drive letters, not a sync folder.** Your bucket appears as `Z:\` in Explorer, File → Open dialogs, command line — everywhere.
- **Direct to cloud, no middleman.** TLS-only, end-to-end between your machine and your provider. We never see your data.
- **Credentials stay local.** Argon2id-hashed admin account, S3 secrets in a local SQLite vault. No phone-home, no account servers.
- **Streaming uploads with live progress.** 16 MiB multipart windows, 8-way concurrency, Explorer's progress bar reflects real S3 throughput.
- **Pay once, own it.** Lifetime licensing option on paid tiers — no subscription lock-in.

---

## Requirements

- Windows 10 1903+ / Windows 11 (x64 or ARM64)
- [WinFsp 2.1+](https://winfsp.dev/rel/) — bundled with the installer, installed automatically
- An S3-compatible bucket and credentials

---

## Install

### End users

Grab the latest signed MSI from the [Releases](../../releases) page and run it. The installer bundles WinFsp and registers NanoCrew Sync with Windows.

### From source

```bash
# Prerequisites
# - Node 20+
# - pnpm 9+
# - Rust stable (via rustup)
# - Visual Studio 2022 Build Tools with "Desktop development with C++" workload
# - WinFsp 2.1+ installed (for development; the installer bundles it for end users)

git clone git@github.com:<org>/nanocrew-sync.git
cd nanocrew-sync
pnpm install
pnpm tauri dev
```

### Build an installable MSI

```bash
pnpm tauri build
# Output: apps/desktop/src-tauri/target/release/bundle/msi/NanoCrew Sync_0.1.0_x64_en-US.msi
# Output: apps/desktop/src-tauri/target/release/bundle/nsis/NanoCrew Sync_0.1.0_x64-setup.exe
```

---

## Monorepo layout

```
app/
├── apps/
│   └── desktop/                    # Tauri 2 desktop shell
│       ├── src/                    # React UI (screens, contexts)
│       └── src-tauri/
│           ├── src/
│           │   ├── winfsp_vfs.rs   # WinFsp → S3 filesystem bridge
│           │   ├── mounts.rs       # Per-drive mount lifecycle
│           │   ├── commands/       # Tauri command handlers (auth, drives)
│           │   ├── db.rs           # SQLite drive + account store
│           │   └── credentials.rs  # S3 secret storage
│           └── tauri.conf.json
└── packages/
    └── ui/                         # Shared design system (@nanocrew/ui)
```

---

## Architecture at a glance

1. **UI** (React, Vite) calls Tauri commands to add, list, mount, unmount drives.
2. **Mounts** (`mounts.rs`) boots a `winfsp::host::FileSystemHost` per drive, each in its own thread.
3. **VFS** (`winfsp_vfs.rs`) implements WinFsp's `FileSystemContext` trait against the AWS SDK for Rust.
4. **Writes** spool to a temp file, then stream out to S3 as 16 MiB multipart parts with 8-way concurrency — paced by a semaphore so Explorer's progress bar tracks real upload throughput.
5. **Reads** issue positioned range GETs directly.

---

## Roadmap

The full multi-phase plan — from correctness polish through licensing and GA — lives in [PHASES.md](./PHASES.md). That file is the source of truth for what's built and what's next.

---

## Licensing

Proprietary. See [LICENSE](./LICENSE). End-user use is governed by the EULA shipped with each signed installer.

For commercial licensing: `legal@nanocrew.dev`.

---

## Contributing

This repository is currently closed-source with a small authorized dev circle. If you've been invited to contribute, see `CONTRIBUTING.md` (coming soon). Security issues: email `security@nanocrew.dev`.

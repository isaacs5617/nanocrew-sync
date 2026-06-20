# NanoCrew Sync — macOS Port Design (v0.3.0 beta)

This document captures the architectural plan for the macOS beta build of
NanoCrew Sync. The target VFS host is **[FUSE-T](https://github.com/macos-fuse-t/fuse-t)**
— a userspace re-implementation of FUSE that runs entirely outside the kernel
(via NFS-loopback) and therefore does **not** require a kernel extension or
SIP changes. This is the only realistic macOS option as of 2026 now that
macFUSE is kext-only on Intel and crippled on Apple Silicon.

## Goals & non-goals (this scaffolding pass)

In scope:

- Compile-clean Windows build preserved at all times.
- macOS-conditional source tree (`fuse_t_vfs.rs`, macOS keychain backend),
  feature-gated dependencies, conditional compilation in `mounts.rs`.
- macOS bundle settings in `tauri.conf.json`.
- GitHub Actions workflow that builds a `.dmg` on macOS runners.

Out of scope (follow-ups marked TODO in code):

- A *working* write path on macOS. The scaffold gets browse / open / read
  enough to verify the mount end-to-end; multipart upload, rename across
  prefixes, and `.keep` markers come in a follow-up pass.
- Apple Developer ID code signing + notarization (needs secrets).
- FUSE-T installer prompt on first launch.
- macOS-specific UI affordances (vibrancy, tray-icon template image).

## What is and isn't platform-specific

### Truly Windows-only (must be `#[cfg(target_os = "windows")]`-gated)

| File | Reason |
|------|--------|
| `src/winfsp_vfs.rs` | Implements `winfsp::filesystem::FileSystemContext`. Pulls in `winfsp`, `winfsp-sys`, `windows::Win32::*`, `widestring`, `std::os::windows::fs::FileExt`. |
| `src/dpapi.rs` | Direct `CryptProtectData` / `CryptUnprotectData` FFI. |
| Parts of `src/mounts.rs` | The `FileSystemHost`, `VolumeParams`, drive-letter normalization. |
| `winfsp_build` in `[build-dependencies]` | WinFsp headers/libs. |
| WinFsp MSI installer hooks in `tauri.conf.json` `bundle.resources` and `bundle.windows.nsis.installerHooks`. |

### Already platform-agnostic (no changes needed)

- `cache.rs` — `DiskCache` is pure Rust + `tokio` + `aes-gcm` + `rusqlite`.
- `dir_listing_cache.rs` — JSON-backed filesystem cache, no OS calls beyond
  `std::fs`.
- `file_lock.rs` — already has `#[cfg(windows)]` / `#[cfg(not(windows))]`
  branches; the non-Windows branch reads `/etc/machine-id` then falls back to
  a hash of `hostname()`. Will need a final pass to confirm the macOS branch
  uses the IOPlatformUUID or a stable Mac equivalent, but is structurally
  fine.
- `providers/` — the entire `CloudProvider` trait and all 7 backends.
- `auth.rs`, `db.rs`, `error.rs`, `http_client.rs`, `license.rs`,
  `logging.rs`, `state.rs`, `throttle.rs`, `types.rs` — pure Rust.
- `credentials.rs` — uses `dpapi` directly today. **Needs an abstraction layer**
  (see below) so the secret wrapper is `dpapi` on Windows and Keychain on
  macOS.

### New macOS-specific files

| File | Purpose |
|------|---------|
| `src/fuse_t_vfs.rs` | Implements `fuser::Filesystem`. Mirrors `winfsp_vfs::S3Fs`. Owns an `InodeTable` mapping FUSE inodes ↔ S3 keys. |
| `src/keychain.rs` | macOS Keychain wrapper using `security-framework`. Mirrors `dpapi.rs` API (`protect` / `unprotect` returning `Vec<u8>`). Generic `Service`/`Account` strings tied to the bundle identifier `dev.nanocrew.sync`. |
| `src/secret_store.rs` (new, optional) | Thin OS-abstraction layer that fans out to `dpapi` on Windows or `keychain` on macOS. `credentials.rs` could call into this instead of `dpapi` directly. Initial scaffold keeps `credentials.rs` Windows-only and adds a Keychain-based equivalent later; `credentials.rs` is `#[cfg(target_os = "windows")]`-gated for v0.3.0. |

## Conditional compilation strategy in `mounts.rs`

`mounts.rs` becomes a thin dispatcher:

```rust
#[cfg(target_os = "windows")]
mod windows_impl { /* current spawn_mount + WinFsp host */ }

#[cfg(target_os = "macos")]
mod macos_impl { /* spawn_mount that builds FUSE-T host via fuse_t_vfs */ }

#[cfg(target_os = "windows")]
pub use windows_impl::spawn_mount;
#[cfg(target_os = "macos")]
pub use macos_impl::spawn_mount;

// MountConfig / MountHandle are platform-agnostic and live at the top of
// the file. MountConfig.letter is reused on macOS as the mount-point folder
// name under ~/NanoCrew/<volume>/ (or wherever we land on for default
// mount-point base).
```

## Path conventions

Application data lives at:

- Windows: `%APPDATA%\dev.nanocrew.sync\` (already the case via
  `tauri::path::app_data_dir`).
- macOS: `~/Library/Application Support/dev.nanocrew.sync/` (Tauri already
  resolves to this on Mac via the same `app_data_dir()` call — no code
  change).

Cache root default:

- Windows: `%LOCALAPPDATA%\NanoCrew\Sync\cache` (current).
- macOS: `~/Library/Caches/dev.nanocrew.sync/cache` (TODO: switch the default
  resolver in the prefs `cache_root` fallback path).

Mount point default:

- Windows: drive letter `Z:` (current).
- macOS: `/Volumes/NanoCrew-<bucket>` or `~/NanoCrew/<bucket>`.
  Apple convention is `/Volumes` for removable-style volumes; FUSE-T
  supports both. TODO during full pass.

## Equivalent of DPAPI on macOS — the Keychain

Use the `security-framework` crate to talk to the macOS Keychain. Each drive
secret is stored as a generic password item with:

- Service: `dev.nanocrew.sync`
- Account: `drive-{drive_id}-secret_key`

The Keychain enforces the same property DPAPI does on Windows: a different
local user cannot read another user's items by default, and the encrypted
form on disk is bound to the device.

Implementation sketch (`src/keychain.rs`):

```rust
use security_framework::passwords::{
    get_generic_password, set_generic_password, delete_generic_password,
};

const SERVICE: &str = "dev.nanocrew.sync";

pub fn store(account: &str, secret: &[u8]) -> Result<(), AppError> { ... }
pub fn retrieve(account: &str) -> Result<Vec<u8>, AppError> { ... }
pub fn delete(account: &str) -> Result<(), AppError> { ... }
```

For v0.3.0 beta, `credentials.rs` is Windows-only and a parallel
`credentials_mac.rs` (or the `secret_store` abstraction) uses keychain
directly with no on-disk envelope — Keychain is the envelope. The
`drives.secret_key` column will store the Keychain *account* (e.g. the
literal string `drive-7-secret_key`) instead of an encrypted blob.

## FUSE-T mount lifecycle

1. **Install check** at app launch: `which mount_fuse-t` or stat
   `/usr/local/bin/mount_fuse-t`. If absent, surface a UI prompt with a
   link to https://github.com/macos-fuse-t/fuse-t/releases. Block mount
   attempts until installed. TODO — not implemented in scaffold.
2. **Mount**: use the `fuser` crate's `MountOption` builder to spawn the
   loopback NFS server. `fuser::Mount::new(mountpoint, options)` returns
   a session handle. Options: `fsname=NanoCrew-<bucket>`,
   `volname=NanoCrew-<bucket>`, `local`, `noapplexattr`,
   `noappledouble`. On macOS `fuser` automatically uses the FUSE-T
   transport when FUSE-T is the installed implementation (the crate
   negotiates via the userland `mount_fuse-t` binary).
3. **Unmount**: drop the `BackgroundSession` or call `fuser::unmount`. On
   macOS the equivalent shell command is `umount /Volumes/NanoCrew-…`.
4. **Stop dispatcher** is automatic — `fuser` joins the worker pool when
   the session is dropped.

## InodeTable

FUSE addresses files by 64-bit inode numbers; S3 addresses by string keys.
We maintain a bidirectional map:

```rust
pub struct InodeTable {
    next: AtomicU64,                // start at 2 (1 = root)
    by_inode: RwLock<HashMap<u64, String>>,         // inode -> key
    by_key:   RwLock<HashMap<String, u64>>,         // key -> inode
}

impl InodeTable {
    pub fn root() -> u64 { 1 }
    pub fn intern(&self, key: &str) -> u64 { /* allocate or reuse */ }
    pub fn lookup_key(&self, inode: u64) -> Option<String> { ... }
    pub fn forget(&self, inode: u64) { /* called from Filesystem::forget */ }
}
```

Eviction policy is "FUSE forget" — when the kernel sends a `forget` op the
entry is dropped, mirroring how WinFsp lets us GC `OpenFile` state on
`cleanup`.

## Things explicitly NOT done in this pass

1. **Keychain integration for credentials.** The `keychain.rs` module and
   `security-framework` dependency are present and compile, but
   `credentials.rs` is still gated to Windows; the macOS code path has a
   placeholder TODO. A follow-up change introduces a `secret_store`
   abstraction and migrates the credentials flow.
2. **FUSE-T installer prompt.** No UI work for "FUSE-T missing — install
   from …" — must be added before beta ships.
3. **macOS UI tweaks.** Window chrome (decorations: false on macOS yields
   no titlebar, which conflicts with macOS HIG — need a custom traffic-
   lights treatment), tray icon template image rendering, mount path
   chooser.
4. **Apple notarization and Developer ID signing.** Requires the user to
   create an Apple Developer account, generate a Developer ID Application
   certificate, and add the cert + altool credentials as GitHub Actions
   secrets (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
   `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`,
   `APPLE_TEAM_ID`).
5. **Universal binary builds.** `pnpm tauri build --target
   universal-apple-darwin` is configured in the workflow but separate
   `aarch64-apple-darwin` / `x86_64-apple-darwin` toolchains must be
   installed by the runner.
6. **Multipart upload on macOS.** The `fuser` write path in this scaffold
   either returns `EROFS` or writes through a single `put_object` for
   tiny files — TODO to port the streaming MPU machinery from
   `winfsp_vfs::WriteState`.
7. **`.keep` marker / empty-folder handling on macOS.** Logic is identical
   to Windows but not wired up yet.
8. **Bandwidth throttling on the macOS read path.** `RateLimiter` is
   already platform-agnostic and present — just needs to be plumbed into
   `fuse_t_vfs::read`.

## Files modified / created (this scaffold)

Created:

- `apps/desktop/src-tauri/src/fuse_t_vfs.rs`
- `apps/desktop/src-tauri/src/keychain.rs`
- `docs/macos-port-design.md` (this file)
- `.github/workflows/release-macos.yml`

Modified:

- `apps/desktop/src-tauri/Cargo.toml` (target-gated dependencies)
- `apps/desktop/src-tauri/src/lib.rs` (conditional `mod` declarations)
- `apps/desktop/src-tauri/src/mounts.rs` (cfg dispatch)
- `apps/desktop/src-tauri/tauri.conf.json` (`bundle.macOS` block)

## Validation

`cargo check --release` on Windows after all scaffolding changes must
still succeed. The macOS-only modules are gated and not compiled on
Windows, so they cannot break the Windows build even while they contain
TODOs and unimplemented match arms.

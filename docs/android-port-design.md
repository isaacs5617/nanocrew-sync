# NanoCrew Sync — Android Port Design (v0.4.0 beta)

Status: **scaffolding in progress**. This document is the source of truth for
how the Android build differs from the Windows desktop build, what's done, and
what remains.

## Goal

Ship NanoCrew Sync on Android as a **storage provider** — i.e. NanoCrew drives
appear in the Files app, Word, Excel, image pickers, share sheets, everywhere
that talks to Android's Storage Access Framework (SAF). No FUSE, no root
required.

## Why this is tractable

The desktop port already split provider IO behind the `CloudProvider` trait
(v0.2.0). All seven providers (S3, SFTP, FTP, WebDAV, GDrive, Dropbox,
OneDrive) only depend on `reqwest`, `tokio`, `serde`, and async traits — they
already compile for `aarch64-linux-android`. The Windows-only surface is the
VFS layer:

- `winfsp_vfs.rs` — WinFsp file-system dispatcher
- `mounts.rs` — drive-letter lifecycle
- `dpapi.rs` — DPAPI credential wrap
- Several `windows` crate calls in `commands/drives.rs` (sentinel SIDs, etc.)

The Android port keeps the entire provider tree, the cache, the file lock, the
license, the audit log, and the Tauri command surface (with caveats) intact.
Only the *mount* concept is replaced.

## SAF DocumentsProvider model

Instead of "mount a drive letter and serve POSIX-ish IO," on Android we
register a `DocumentsProvider` subclass. The OS calls into the provider when
the user picks a file, shares to/from NanoCrew, or browses Files. Each
NanoCrew "drive" is exposed as a root under that provider.

```
+----------------------+        SAF / DocumentsContract
| Android Files / Word | <--------------------------------+
+----------+-----------+                                  |
           |                                              |
           v                                              |
+----------------------------------------------------+    |
| NanoCrewDocumentsProvider.kt                       |    |
|  - queryRoots()       -> roots cursor              |    |
|  - queryChildDocs()   -> children cursor           |    |
|  - openDocument()     -> ParcelFileDescriptor      |    |
|  - createDocument()   -> doc_id                    |    |
|  - delete/rename/...                               |    |
+----------+-----------------------------------------+    |
           | JNI                                          |
           v                                              |
+----------------------------------------------------+    |
| android_provider.rs (Rust)                         |    |
|  - listChildrenNative                              |    |
|  - openDocumentNative -> materialize to cache,     |    |
|                          return FD                 |    |
|  - createDocumentNative / deleteNative / ...       |    |
+----------+-----------------------------------------+    |
           |                                              |
           v                                              |
+----------------------------------------------------+    |
| CloudProvider trait (S3 / SFTP / WebDAV / ...)     | ---+
| + shared cache (cache.rs) + file_lock + license    |
+----------------------------------------------------+
```

### Materialize-on-open

SAF expects `openDocument` to return a `ParcelFileDescriptor`. We can't stream
a remote object as an FD without writing it locally first. Plan: when an open
arrives, download (or hit the cache) into a private app file under
`getCacheDir()/drive-<id>/<doc-id>`, then `ParcelFileDescriptor.open(file)`
that. On `close`, if the FD was opened RW we upload the diff back via the
provider's `put_object` (or multipart for large files).

This is roughly how Google Drive's own SAF integration works.

### Drive ID → root mapping

The `drives` SQLite row already has an `id`. We expose `"<drive_id>"` as the
root id and `"<drive_id>:<key>"` as the document id. Parsing/serializing is
trivial.

## Component cfg gating

`Cargo.toml` is restructured so platform-only deps are inside
`[target.'cfg(target_os = "...")'.dependencies]` blocks:

| Crate                            | Windows | Android | Notes                                  |
|----------------------------------|---------|---------|----------------------------------------|
| `tauri`, `tokio`, `serde`        | yes     | yes     | top-level                              |
| `reqwest`, `bytes`, `async-trait`| yes     | yes     | top-level                              |
| `aws-sdk-s3` + companions        | yes     | yes     | rustls-only; runs on Android           |
| `russh`, `russh-sftp`            | yes     | yes     | pure Rust, compiles for aarch64        |
| `suppaftp` (native-tls)          | yes     | ?       | needs `rustls` feature on Android      |
| `winfsp`, `winfsp-sys`           | yes     | **no**  | Windows cfg only                       |
| `windows` crate                  | yes     | **no**  | Windows cfg only                       |
| `widestring`                     | yes     | no      | Windows cfg only                       |
| `jni`, `ndk-context`, `android_logger` | no | yes  | Android cfg only                       |
| `tauri-plugin-single-instance`   | yes     | no      | desktop-only Tauri plugin              |
| `tauri-plugin-updater`           | yes     | ?       | Android has Play Store; skip plugin    |

`mounts.rs` and `winfsp_vfs.rs` are gated to Windows. Likewise `dpapi.rs`.
`credentials.rs` keeps its public API but switches its impl between DPAPI
(Windows) and Android Keystore (Android) — Android impl is a TODO stub for
v0.4.0 scaffold.

The current `lib.rs` `run()` body wires the tray icon, single-instance, and
auto-mount loop — none of which apply on Android. A `#[cfg(mobile)]` parallel
`run()` provides the mobile entry, registering only the cross-platform
commands and the SAF provider.

## Mobile UI scope (v0.4.0)

Stripped down from the desktop UI; reuses the same React/Vite frontend:

1. **Drive list** — read-only-ish view of `drives` table
2. **Add drive** — provider-specific forms (S3, SFTP, ...) using the existing
   `add_drive`/`test_connection` commands
3. **License activation** — same `activate_license` flow
4. **Settings** — minimal: cache size cap, telemetry toggle, sign out

No drive-letter UI, no WinFsp installer check, no tray. Everything that
references "drive letter" or "mount status" is hidden behind a
`platform === 'desktop'` runtime check, set from a new Tauri command
`get_platform()` (returns `"windows" | "android"`).

The frontend gets `@media (pointer: coarse)` tap-target bumps in
`global.css` so existing components work on phones without a redesign.

## Credential storage on Android

DPAPI is Windows-only. On Android we'll use **Android Keystore** via JNI:

- Generate an `AES/GCM/NoPadding` key alias `nanocrew.cred.<drive_id>` in
  `AndroidKeyStore`
- Wrap the secret with that key, store ciphertext + IV in the existing
  `drives.secret_key` column with a `v2:` prefix
- On retrieve, peek the prefix:
  - `v1:` → DPAPI (Windows only — refuse on Android)
  - `v2:` → Keystore unwrap
  - else → legacy plaintext, migrate to `v2:`

Scaffold (v0.4.0): `credentials::store_android`/`retrieve_android` stubs that
panic / return placeholder — real implementation in v0.4.1.

## File picker / share-target

After the DocumentsProvider works, two more intents make NanoCrew a first-class
mobile citizen:

1. **`ACTION_GET_CONTENT` / `ACTION_OPEN_DOCUMENT`** — handled automatically
   by being a registered DocumentsProvider.
2. **`ACTION_SEND` (share to)** — separate `<activity>` declaration; lets
   users share photos/videos into a NanoCrew drive from Photos / browser /
   anywhere. Wires into `createDocument` + bytes stream.

Both are out-of-scope for the v0.4.0 scaffold but documented here so we don't
forget.

## Required local toolchain

To complete the Android build the developer machine needs:

- **Android Studio** (or just the cmdline-tools) for `sdkmanager`
- **Android SDK** — platform 34 (compile target) and 24 (min target)
- **Android NDK r26+** (we use `r27c`) — set `NDK_HOME` and `ANDROID_NDK_HOME`
- **Rust targets**:
  `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android`
- **`cargo-ndk`** for cleaner cross-compile invocation:
  `cargo install cargo-ndk`
- **JDK 17+** on `PATH`
- Env vars persisted: `ANDROID_HOME`, `NDK_HOME`, `JAVA_HOME`

`pnpm tauri android init` requires all of the above. If the command fails
because the SDK isn't installed, the scaffolded files in `gen/android/` will
need to be created/regenerated after install. The Rust-side scaffolding in
this branch does **not** depend on running `init` successfully.

## What this scaffold delivers (v0.4.0-pre)

- ✅ `Cargo.toml` restructured with `[target.'cfg(...)']` gating
- ✅ `mounts.rs`, `winfsp_vfs.rs`, `dpapi.rs` gated to Windows
- ✅ `lib.rs` split into Windows `run()` and `mobile_run()`
- ✅ `src/android_provider.rs` — JNI surface with TODO bodies
- ✅ `src/jni_helpers.rs` — type conversion helpers
- ✅ `gen/android/.../NanoCrewDocumentsProvider.kt` — Kotlin shim skeleton
- ✅ `gen/android/.../AndroidManifest.xml` — provider declaration + intent
- ✅ `global.css` touch-target media query
- ✅ This document

## What this scaffold does NOT do (TODO)

- ❌ Run `pnpm tauri android init` — requires Android SDK on the dev machine
- ❌ Full mobile UI redesign / responsive sweep
- ❌ Auth flow on Android (lock screen vs. BiometricPrompt — TBD)
- ❌ Real Android Keystore credential storage (stub only)
- ❌ Google Play Console setup, signing, AAB build pipeline
- ❌ `network_security_config.xml` (need cleartext exception for local MinIO
      testing)
- ❌ The materialize-on-open download loop in `openDocument` (signature exists,
      body is `todo!()`)
- ❌ Background sync / push file-change notifications via
      `ContentResolver.notifyChange()`
- ❌ Cargo target gating of `suppaftp` (likely needs `rustls` feature on
      Android — TBD when first Android `cargo build` is attempted)
- ❌ CI for Android — separate workflow needs to be added once `init` is run
- ❌ Tauri command surface trimming for mobile (currently mobile run() wires
      all the cross-platform commands; some, like `set_autostart`, are
      meaningless on Android and should be cfg'd out)

## Next steps for a follow-up branch

1. Install Android SDK + NDK on the dev machine
2. Run `pnpm tauri android init` from `apps/desktop/`
3. Resolve any `cargo check --target aarch64-linux-android` errors —
   probably `suppaftp` feature flag and one or two stragglers
4. Implement `android_provider::open_document` materialize-on-open
5. Implement Android Keystore credential wrap/unwrap
6. Mobile-friendly drive-add wizard in the React UI
7. Beta on internal devices via `adb install`
8. Closed-track release on Play Store

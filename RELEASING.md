# Releasing NanoCrew Sync

This document is for the maintainer. End users should consult the README.

## Keys

The auto-updater uses a single Ed25519 signing keypair.

- **Private key (secret):** `C:\Users\strau\.nanocrew-sync-updater.key`
  Back this up to a password manager (1Password, Bitwarden) immediately. If
  lost, every installed copy of the app becomes un-updatable and we have to
  ship a forced manual migration.
- **Public key:** embedded in `apps/desktop/src-tauri/tauri.conf.json` under
  `plugins.updater.pubkey`. Committed to the repo.

## One-time setup: GitHub secrets

The release workflow (`.github/workflows/release.yml`) needs two secrets on
the repository:

1. **`TAURI_SIGNING_PRIVATE_KEY`** — the full contents of
   `C:\Users\strau\.nanocrew-sync-updater.key`
   (including the `untrusted comment:` header and the blank lines).
2. **`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`** — the empty string `""` (the key
   was generated without a password). If you later re-generate with a
   password, update this secret.

Set them via the GitHub UI (Settings → Secrets and variables → Actions) or:

```powershell
gh secret set TAURI_SIGNING_PRIVATE_KEY            < $env:USERPROFILE\.nanocrew-sync-updater.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD -b ""
```

## Publishing a new version

1. **Bump the version** in three places (they must match):
   - `apps/desktop/src-tauri/tauri.conf.json` — `version`
   - `apps/desktop/src-tauri/Cargo.toml` — `package.version`
   - `apps/desktop/package.json` — `version`

2. **Update `PHASES.md`** — tick the boxes you just landed.

3. **Commit and tag:**
   ```bash
   git commit -am "Bump to vX.Y.Z — <one-line summary>"
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin main --follow-tags
   ```

4. **The release workflow runs automatically** on tag push. It will:
   - Build signed MSI + NSIS EXE
   - Generate the updater `.sig` files
   - Produce `latest.json` describing the new release
   - Create / update the GitHub Release at that tag with all artifacts attached

5. **Verify the updater sees it** — open an older installed build of NanoCrew
   Sync, go to Settings → About, click "Check for updates." You should see
   the new version download, install, and the app relaunch.

## Updater endpoint

The app's updater is configured with two endpoints (it tries them in order):

1. `https://github.com/isaacs5617/nanocrew-sync/releases/latest/download/latest.json`
   (the release workflow uploads `latest.json` here as a release asset)
2. `https://releases.nanocrew.dev/{{target}}/{{arch}}/{{current_version}}`
   (placeholder — a future public CDN once the marketing site lands)

**Private repo caveat:** GitHub release assets in a private repository
require authentication to download. While the repo is private, the updater
will fail for anyone who isn't logged in as a collaborator. Options:

- **Option A (fast):** Make the repo public. Simplest.
- **Option B (recommended for paid product):** Move the release assets to a
  public Cloudflare R2 bucket or GitHub Pages. Update endpoint #1 to point
  there and have the workflow also upload to that destination.
- **Option C (temporary):** Keep private, only maintainers test the updater.

For now we are in **Option C** — update flow is manual outside the circle.
Flip to A or B before the public beta.

## Rolling back

To withdraw a bad release:

1. Delete or mark as "draft" the GitHub Release for the bad tag.
2. Because the updater fetches `releases/latest/download/latest.json`, deleting
   the release makes the next-most-recent non-draft release the "latest"
   — and the updater will downgrade new checks to that.

Clients that already installed the bad version can be forced to update by
publishing a patch release vX.Y.Z+1.

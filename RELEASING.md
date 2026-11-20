# Releasing VC Sound Design

Releases are built by [`.github/workflows/release.yml`](.github/workflows/release.yml) when a
`v*` tag is pushed. It builds installers for macOS (universal `.dmg`/`.app`), Windows
(`.msi` and NSIS `.exe`) and Linux (`.deb` and `.AppImage`) and attaches them to a **draft**
GitHub release, so you can check it before publishing.

Signing and auto-update are switched on one at a time by repository secrets and variables.
With none set you still get working, unsigned installers and the app's update check is
hidden.

## Cutting a release

```bash
node scripts/bump-version.mjs 0.2.0
```

This sets the same version in `Cargo.toml`, `crates/vcsd-app/tauri.conf.json` and
`ui/package.json`. Commit, tag and push:

```bash
git commit -am "v0.2.0"
```

```bash
git tag v0.2.0 && git push && git push --tags
```

The workflow refuses a tag that doesn't match the repo version
(`node scripts/check-version.mjs v0.2.0`). When the three builds finish, edit the draft
release (add notes: they appear in the app's update dialog) and publish it. Installed copies
see the update after publishing, because `latest.json` is served from the latest *published*
release.

You can also run the workflow by hand (Actions → Release → Run workflow). That builds the
installers as run artifacts without creating a release.

## Auto-update (all platforms)

Updates are signed with a key that is separate from OS code signing. Users' apps refuse any
update not signed with it, so keep the private key safe and backed up: if you lose it,
existing installs can't update any more.

1. Generate the key pair once, on your machine:

   ```bash
   npx --prefix ui tauri signer generate -w ~/.tauri/vcsd-updater.key
   ```

2. Repository **secrets** (Settings → Secrets and variables → Actions):
   - `TAURI_SIGNING_PRIVATE_KEY`: the contents of `~/.tauri/vcsd-updater.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password
3. Repository **variable**:
   - `TAURI_UPDATER_PUBKEY`: the contents of `~/.tauri/vcsd-updater.key.pub`
   - `VCSD_UPDATE_ENDPOINT` (optional): where the app looks for `latest.json`. The default
     is `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`. It must be
     HTTPS.

**Private repositories:** release assets of a private repo need a GitHub login to download,
so installed apps can't fetch them. Either publish releases from a public repo, or copy
`latest.json` and the update bundles to your own HTTPS host and point
`VCSD_UPDATE_ENDPOINT` at it.

In the app: **☰ → Check for updates…** checks, shows the release notes, downloads and
verifies the update, and restarts after saving the project. It also checks quietly at
startup (a pill appears in the top bar when an update is ready); that can be switched off in
the same dialog. Installing is refused while playing or recording.

What updates per platform: macOS replaces the `.app` (`.app.tar.gz`); Windows runs the new
installer in passive mode; Linux replaces the `.AppImage`. `.deb` installs are updated by
installing the new `.deb`.

## macOS signing and notarisation

Needs a paid Apple Developer account and a **Developer ID Application** certificate.

Secrets:
- `APPLE_CERTIFICATE`: the certificate exported as `.p12`, base64-encoded
  (`base64 -i cert.p12 | pbcopy`)
- `APPLE_CERTIFICATE_PASSWORD`: the `.p12` export password
- `APPLE_SIGNING_IDENTITY`: e.g. `Developer ID Application: Your Name (TEAMID)`
- `APPLE_ID`, `APPLE_PASSWORD` (an [app-specific password](https://support.apple.com/102654)),
  `APPLE_TEAM_ID`: for notarisation

Without these, macOS users must right-click → Open the first time (or allow it in System
Settings → Privacy & Security).

## Windows signing

Needs an Authenticode code-signing certificate as a `.pfx`.

Secrets:
- `WINDOWS_CERTIFICATE`: the `.pfx`, base64-encoded
  (`[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))`)
- `WINDOWS_CERTIFICATE_PASSWORD`: its password

Optional variable `WINDOWS_TIMESTAMP_URL` (default `http://timestamp.digicert.com`).
Certificates that live only on a hardware token or in a cloud HSM (most new EV/OV certificates
since 2023) can't be imported this way; they need a custom `signCommand` for your provider in
`scripts/release-config.mjs`. Unsigned builds show a SmartScreen warning ("More info → Run
anyway").

## Linux

The AppImage bundles GStreamer so picture playback works on any distribution; that makes it
about 170 MB. The `.deb` is small and depends on the distribution's GStreamer packages
instead. Linux packages are not code-signed; update bundles are still verified with the
updater key.

## Testing an update locally

`scripts/release-config.mjs` accepts a plain-HTTP `localhost` endpoint (marked TEST ONLY;
signatures are still checked). Build version A with
`VCSD_UPDATE_ENDPOINT=http://127.0.0.1:8899/latest.json`, bump to version B and build again,
serve B's bundle and a `latest.json` from that port, then use **Check for updates…** in A.
Never commit the key or the generated `crates/vcsd-app/tauri.release.json`.

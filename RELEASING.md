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

Needs a paid Apple Developer account and a **Developer ID Application** certificate. The
secrets have the same names as in VC Composer's workflow, so the same values go in:

- `MACOS_CERTIFICATE_P12`: the certificate exported as `.p12`, base64-encoded
  (`base64 -i cert.p12 | pbcopy`)
- `MACOS_CERTIFICATE_PASSWORD`: the `.p12` export password
- `MACOS_SIGNING_IDENTITY`: e.g. `Developer ID Application: Your Name (TEAMID)`
- `APPLE_ID`, `APPLE_APP_PASSWORD` (an [app-specific password](https://support.apple.com/102654)),
  `APPLE_TEAM_ID`: for notarisation

Without these, macOS users must right-click → Open the first time (or allow it in System
Settings → Privacy & Security).

## Windows signing: Azure Trusted Signing

Windows installers are signed through **Azure Trusted Signing** (Microsoft now calls it
Artifact Signing): the same signing account as VC Composer and VC Writer, and the same six
secrets, so the same values go in:

| Secret | What it is |
| --- | --- |
| `AZURE_TENANT_ID` | Directory (tenant) ID of the Entra app registration |
| `AZURE_CLIENT_ID` | Application (client) ID of that registration |
| `AZURE_CLIENT_SECRET` | A client secret on that registration |
| `AZURE_SIGNING_ENDPOINT` | The signing account's regional endpoint, e.g. `https://eus.codesigning.azure.net/`. It must match the account's **Overview → Account URI**. |
| `AZURE_SIGNING_ACCOUNT` | The signing account's name |
| `AZURE_SIGNING_PROFILE` | The certificate profile's name |

How it works: Tauri signs each Windows file while it builds (the app, both installers and the
update bundles) by running `scripts/windows/sign.ps1`, which calls `signtool` with Microsoft's
signing client (`Microsoft.ArtifactSigning.Client`). The private key never leaves Microsoft:
there is no certificate file to keep. Signing during the build, rather than afterwards, is what
keeps the update bundles' updater signatures valid.

The workflow checks:
- **Before the build:** if only some of the six secrets are set, it stops and names the missing
  ones, rather than quietly building unsigned.
- **After the build:** it checks each installer with `Get-AuthenticodeSignature`, the way a
  customer's PC will, and fails if signing was set up but a file isn't validly signed.

If sign-in works but signing is refused, the app registration is missing the **Trusted Signing
Certificate Profile Signer** role on the signing account (Access control (IAM)).

**Older route, a `.pfx` certificate:** certificates issued since June 2023 keep their key in
hardware and can't be used this way. For an older one, set `WINDOWS_CERTIFICATE` (the `.pfx`,
base64: `[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))`) and
`WINDOWS_CERTIFICATE_PASSWORD`, and optionally the variable `WINDOWS_TIMESTAMP_URL` (default
`http://timestamp.digicert.com`). It is used only when the Azure secrets aren't set.

Unsigned builds show a SmartScreen warning ("More info → Run anyway").

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

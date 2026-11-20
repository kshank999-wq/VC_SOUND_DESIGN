#!/usr/bin/env node
// Writes a Tauri config overlay for release builds from environment variables,
// so the base config stays buildable (unsigned, no updater) for development.
//
//   node scripts/release-config.mjs crates/vcsd-app/tauri.release.json
//   tauri build --config tauri.release.json
//
// Env (all optional; each enables one capability):
//   TAURI_UPDATER_PUBKEY            update public key (from `tauri signer generate`) → in-app updates
//   TAURI_SIGNING_PRIVATE_KEY       its private key (read by the Tauri CLI) → signed update artifacts
//   VCSD_UPDATE_ENDPOINT            where latest.json lives (default: this repo's latest GitHub release)
//   GITHUB_REPOSITORY               owner/repo, for the default endpoint
//   WINDOWS_CERTIFICATE_THUMBPRINT  Authenticode cert in the runner's store → signed .msi/.exe
//   WINDOWS_TIMESTAMP_URL           timestamp server (default DigiCert)
// macOS signing/notarisation needs no overlay: the Tauri CLI reads APPLE_CERTIFICATE,
// APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID from the environment.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export function releaseConfig(env) {
  const v = (k) => (env[k] && String(env[k]).trim()) || null;
  const cfg = {};
  const notes = [];
  const pubkey = v("TAURI_UPDATER_PUBKEY");
  if (pubkey) {
    const repo = v("GITHUB_REPOSITORY");
    const endpoint = v("VCSD_UPDATE_ENDPOINT") ?? (repo ? `https://github.com/${repo}/releases/latest/download/latest.json` : null);
    if (!endpoint) throw new Error("TAURI_UPDATER_PUBKEY set but no VCSD_UPDATE_ENDPOINT or GITHUB_REPOSITORY");
    const url = new URL(endpoint);
    const local = ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname);
    if (url.protocol !== "https:" && !local) throw new Error(`update endpoint must be https: ${endpoint}`);
    cfg.plugins = { updater: { pubkey, endpoints: [endpoint], windows: { installMode: "passive" } } };
    if (url.protocol !== "https:") {
      // local testing only (signatures are still verified)
      cfg.plugins.updater.dangerousInsecureTransportProtocol = true;
      notes.push("updater: plain-http localhost endpoint (TEST ONLY)");
    }
    notes.push(`updater: on (${endpoint})`);
    if (v("TAURI_SIGNING_PRIVATE_KEY")) {
      cfg.bundle = { ...(cfg.bundle ?? {}), createUpdaterArtifacts: true };
      notes.push("update artifacts: signed");
    } else {
      notes.push("update artifacts: NOT produced (TAURI_SIGNING_PRIVATE_KEY missing)");
    }
  } else {
    notes.push("updater: off (no TAURI_UPDATER_PUBKEY)");
  }
  const thumb = v("WINDOWS_CERTIFICATE_THUMBPRINT");
  if (thumb) {
    cfg.bundle = {
      ...(cfg.bundle ?? {}),
      windows: { certificateThumbprint: thumb, digestAlgorithm: "sha256", timestampUrl: v("WINDOWS_TIMESTAMP_URL") ?? "http://timestamp.digicert.com" },
    };
    notes.push("windows: code signing on");
  } else {
    notes.push("windows: unsigned");
  }
  notes.push(v("APPLE_CERTIFICATE") ? `macos: signing${v("APPLE_ID") ? " + notarisation" : " (no notarisation: APPLE_ID missing)"}` : "macos: unsigned");
  return { config: cfg, notes };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const out = process.argv[2];
  if (!out) {
    console.error("usage: release-config.mjs <output.json>");
    process.exit(2);
  }
  const { config, notes } = releaseConfig(process.env);
  writeFileSync(out, JSON.stringify(config, null, 2) + "\n");
  for (const n of notes) console.log("•", n); // never prints secrets
}

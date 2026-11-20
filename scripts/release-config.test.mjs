import { test } from "node:test";
import assert from "node:assert/strict";
import { releaseConfig } from "./release-config.mjs";
import { readVersions } from "./bump-version.mjs";

test("no env: unsigned, no updater (dev-safe)", () => {
  const { config, notes } = releaseConfig({});
  assert.deepEqual(config, {});
  assert.ok(notes.includes("updater: off (no TAURI_UPDATER_PUBKEY)"));
});

test("updater with default GitHub endpoint and signed artifacts", () => {
  const { config } = releaseConfig({ TAURI_UPDATER_PUBKEY: "PUB", TAURI_SIGNING_PRIVATE_KEY: "PRIV", GITHUB_REPOSITORY: "o/r" });
  assert.deepEqual(config.plugins.updater.endpoints, ["https://github.com/o/r/releases/latest/download/latest.json"]);
  assert.equal(config.plugins.updater.pubkey, "PUB");
  assert.equal(config.bundle.createUpdaterArtifacts, true);
  assert.ok(!JSON.stringify(config).includes("PRIV"), "private key never written to the config");
});

test("pubkey without private key: updater on but no artifacts; custom endpoint; windows signing", () => {
  const { config } = releaseConfig({ TAURI_UPDATER_PUBKEY: "PUB", VCSD_UPDATE_ENDPOINT: "https://u.example/latest.json", WINDOWS_CERTIFICATE_THUMBPRINT: "AB12" });
  assert.equal(config.bundle.createUpdaterArtifacts, undefined);
  assert.deepEqual(config.plugins.updater.endpoints, ["https://u.example/latest.json"]);
  assert.equal(config.bundle.windows.certificateThumbprint, "AB12");
  assert.equal(config.bundle.windows.digestAlgorithm, "sha256");
});

test("pubkey but nowhere to fetch updates from is an error", () => {
  assert.throws(() => releaseConfig({ TAURI_UPDATER_PUBKEY: "PUB" }));
});

test("versions are in step across Cargo, Tauri and the UI", () => {
  const v = readVersions();
  assert.equal(v.cargo, v.tauri);
  assert.equal(v.tauri, v.ui);
});

test("plain http only for localhost testing", () => {
  assert.throws(() => releaseConfig({ TAURI_UPDATER_PUBKEY: "PUB", VCSD_UPDATE_ENDPOINT: "http://updates.example.com/latest.json" }), /https/);
  const { config } = releaseConfig({ TAURI_UPDATER_PUBKEY: "PUB", VCSD_UPDATE_ENDPOINT: "http://127.0.0.1:8899/latest.json" });
  assert.equal(config.plugins.updater.dangerousInsecureTransportProtocol, true);
  const { config: c2 } = releaseConfig({ TAURI_UPDATER_PUBKEY: "PUB", GITHUB_REPOSITORY: "o/r" });
  assert.equal(c2.plugins.updater.dangerousInsecureTransportProtocol, undefined);
});

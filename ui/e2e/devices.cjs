// Audio & MIDI dialog against `vcsd serve` whose ALSA config exposes virtual
// devices named "Studio Interface A/B" (see crates/vcsd-engine/tests/devices.rs).
// Usage: node e2e/devices.cjs [base-url] [screenshot-dir]
const { chromium } = require("playwright");
const assert = require("assert/strict");
(async () => {
  const BASE = process.argv[2] || "http://127.0.0.1:7878";
  const out = process.argv[3] || null;
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 1600, height: 960 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(e.message));
  await p.goto(BASE);
  await p.waitForSelector(".topbar");
  await p.click(".status .device-btn");
  await p.waitForSelector(".devices section");
  const sel = (i) => p.locator(".devices select").nth(i);
  await sel(0).selectOption("Studio Interface A");
  await p.waitForFunction(() => document.querySelector(".devices")?.textContent.includes("Active: Studio Interface A"));
  await sel(1).selectOption("Studio Interface B");
  await p.waitForTimeout(300);
  // simulated controller traffic lights the activity LED
  await p.evaluate(() => fetch("/api/midi.message", { method: "POST", body: JSON.stringify({ bytes: [0x99, 36, 110] }) }));
  await p.waitForSelector(".led.on");
  assert.ok((await p.textContent(".devices")).includes("note 36 on · vel 110 · ch 10"));
  if (out) await p.screenshot({ path: out + "/devices_dialog.png" });
  await p.click("button:has-text('Test tone')");
  // the status bar follows the active output
  await p.click(".devices button.primary:has-text('Done')");
  await p.waitForFunction(() => document.querySelector(".status .device-btn")?.textContent.includes("Studio Interface A"));
  // unplugged device shows a warning in the status bar
  await p.evaluate(() => fetch("/api/prefs.set", { method: "POST", body: JSON.stringify({ output_device: "Unplugged Box" }) }));
  await p.waitForFunction(() => document.querySelector(".status .device-btn")?.textContent.includes("⚠"));
  await p.evaluate(() => fetch("/api/prefs.set", { method: "POST", body: JSON.stringify({ output_device: "Studio Interface A" }) }));
  assert.deepEqual(errors, []);
  console.log("devices UI ok");
  await b.close();
})().catch((e) => { console.error(e); process.exit(1); });

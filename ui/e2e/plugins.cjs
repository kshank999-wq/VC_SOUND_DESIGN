// Plugin hosting in the UI, against `vcsd serve` with the test VST3 installed
// (VCSD_VST3_PATH pointing at a folder holding "VCSD Test.vst3").
// Usage: node e2e/plugins.cjs [base-url] [screenshot-dir]
const { chromium } = require("playwright");
const assert = require("assert/strict");

const BASE = process.argv[2] || "http://127.0.0.1:7878";
const OUT = process.argv[3] || null;

(async () => {
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 1600, height: 1000 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(e.message));
  const api = (cmd, args = {}) =>
    p.evaluate(async ([c, a]) => (await (await fetch("/api/" + c, { method: "POST", body: JSON.stringify(a) })).json()).ok, [cmd, args]);
  await p.goto(BASE);
  // start from the default window layout (layouts persist between runs)
  await p.evaluate(() => fetch("/api/layout.set", { method: "POST", body: JSON.stringify({ reset: true }) }));
  await p.reload();
  await p.waitForSelector(".topbar");
  const st0 = await api("state.get");
  const master = st0.buses.find((x) => !x.parent_id);
  // known start: no plugin slots anywhere
  for (const o of [...st0.tracks, ...st0.buses]) for (const i of o.inserts.filter((x) => x.type === "plugin")) await api("insert.remove", { owner_id: o.id, id: i.id });
  const slots = async (owner) => (await api("state.get"))[owner.startsWith("bus") ? "buses" : "tracks"].find((x) => x.id === owner).inserts.filter((i) => i.type === "plugin");

  // ---- master strip: + FX → browser → search → insert the delay
  await p.click(".tabs button:has-text('Mixer')");
  await p.waitForSelector(".strip.master");
  await p.locator(".strip.master button.fx-add").click();
  await p.waitForSelector(".plugin-browser .plugin-item");
  assert.ok((await p.locator(".plugin-browser .plugin-item").count()) >= 2, "test plugins listed");
  await p.fill(".plugin-browser input", "delay");
  assert.equal(await p.locator(".plugin-browser .plugin-item").count(), 1, "search narrows the list");
  if (OUT) await p.screenshot({ path: `${OUT}/plugins_browser.png` });
  await p.locator(".plugin-browser .plugin-item").first().click();
  await p.waitForSelector(".plugin-window");
  let s = await slots(master.id);
  assert.equal(s.length, 1);
  assert.equal(s[0].name, "VCSD Test Delay");
  await p.waitForSelector(".plugin-window >> text=latency 480 samples");
  assert.equal((await api("state.get")).plugins[s[0].id].loaded, true);

  // ---- drag the Gain slider to a quarter (×0.5 = −6 dB): live while dragging, recorded on release
  const slider = p.locator(".plugin-window .param-row input[type=range]").first();
  const bb = await slider.boundingBox();
  await p.mouse.move(bb.x + bb.width * 0.5, bb.y + bb.height / 2);
  await p.mouse.down();
  await p.mouse.move(bb.x + bb.width * 0.25, bb.y + bb.height / 2, { steps: 8 });
  await p.mouse.up();
  for (let i = 0; i < 40 && (await slots(master.id))[0].params["0"] === undefined; i++) await p.waitForTimeout(50);
  s = await slots(master.id);
  const v = s[0].params["0"];
  assert.ok(Math.abs(v - 0.25) < 0.06, `parameter recorded near a quarter: ${v}`);
  // the plugin's own text for the value (test gain: ×2v, in dB)
  const shown = await p.locator(".plugin-window .param-val").first().textContent();
  assert.equal(shown, `${(20 * Math.log10(2 * v)).toFixed(1)} dB`, `plugin's own display text: ${shown}`);
  const undoLabel = (await api("state.get")).undo;
  assert.equal(undoLabel, "plugin parameter");

  // ---- audible through the plugin while playing
  await api("transport.play", { from: 48000 * 2 });
  let peak = 0;
  for (let i = 0; i < 12; i++) {
    await p.waitForTimeout(60);
    peak = Math.max(peak, (await api("transport.state")).master_peak);
  }
  await api("transport.stop");
  assert.ok(peak > 0.001, `master audible through the plugin: ${peak}`);
  if (OUT) await p.screenshot({ path: `${OUT}/plugins_mixer.png` });

  // ---- bypass from the window, undo it
  await p.locator(".plugin-window button.fx-power").click();
  await p.waitForTimeout(300);
  assert.equal((await slots(master.id))[0].bypass, true);
  assert.ok(await p.locator(".strip.master .fx-slot.bypassed").count(), "strip shows bypass");
  await p.click(".plugin-window .plugin-head", { position: { x: 60, y: 10 } });
  await p.keyboard.press("Escape");
  await api("undo");
  assert.equal((await slots(master.id))[0].bypass, false);

  // ---- track inspector: add a plugin, then toggle a built-in insert: the plugin stays
  const track = st0.tracks.find((t) => t.channels === 1) ?? st0.tracks[0];
  const builtins = track.inserts.filter((i) => i.type !== "plugin" && i.type !== "compressor");
  await api("track.update", { id: track.id, inserts: builtins }); // known start
  await p.click(".tabs button:has-text('Timeline')");
  await p.locator(".track-head .drag-grip", { hasText: track.name }).first().click();
  await p.waitForSelector(".inspector >> text=+ Add plugin…");
  await p.click(".inspector button:has-text('+ Add plugin…')");
  await p.fill(".plugin-browser input", "gain");
  await p.keyboard.press("Enter"); // one match → inserted
  await p.waitForTimeout(400);
  assert.equal((await slots(track.id))[0]?.name, "VCSD Test Gain");
  await p.locator(".inspector label:has-text('Compressor') input").click(); // controlled: updates after the engine answers
  await p.waitForTimeout(300);
  const ins = (await api("state.get")).tracks.find((t) => t.id === track.id).inserts;
  assert.deepEqual(ins.map((i) => i.type), [...builtins.map((i) => i.type), "compressor", "plugin"], "built-in toggles keep plugin slots");
  if (OUT) await p.screenshot({ path: `${OUT}/plugins_inspector.png`, clip: { x: 1180, y: 40, width: 420, height: 700 } });
  // two plugin windows can be open at once
  assert.equal(await p.locator(".plugin-window").count(), 2);
  // remove from the window (buses have no inspector); undo brings it back
  await p.locator(".plugin-window", { hasText: "VCSD Test Delay" }).locator("button:has-text('Remove')").click();
  await p.waitForTimeout(300);
  assert.equal((await slots(master.id)).length, 0);
  await api("undo");
  assert.equal((await slots(master.id)).length, 1);

  // ---- a missing plugin is flagged, not lost
  await api("track.update", { id: track.id, inserts: [...ins, { type: "plugin", id: "ins_e2e_missing", plugin_id: "vst3:" + "0".repeat(32), format: "vst3", name: "Gone Verb", vendor: "", path: "/nowhere.vst3", bypass: false, state: "AAEC", params: {} }] });
  await p.waitForSelector(".inspector .fx-slot.missing");
  await p.click(".inspector .fx-slot.missing .fx-name");
  await p.waitForSelector(".plugin-window .warn-box");
  const qa = await api("qa.run");
  assert.ok(qa.some((i) => i.code === "plugin_missing" && i.message.includes("Gone Verb")));

  // ---- the Plugins… manager from the menu
  await p.click(".brand");
  await p.click(".menu >> text=Plugins…");
  await p.waitForSelector(".plugin-browser >> text=effects in");
  await p.click(".plugin-browser button:has-text('Folders…')");
  await p.waitForSelector(".plugin-folders textarea");
  await p.click(".plugin-browser button:has-text('Close')");

  // clean up
  for (const o of [track.id, master.id]) for (const i of await slots(o)) await api("insert.remove", { owner_id: o, id: i.id });
  await api("track.update", { id: track.id, inserts: builtins });
  assert.deepEqual(errors, []);
  console.log("plugin hosting ok");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

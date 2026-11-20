// End-to-end UI workflow against a running `vcsd serve <project>` (browser mode).
// Usage: node e2e/workflow.cjs [http://127.0.0.1:7878] [screenshot-dir]
// Uses the Playwright + Chromium available on the machine.
const { chromium } = require("playwright");
const assert = require("assert/strict");

const BASE = process.argv[2] || "http://127.0.0.1:7878";
const OUT = process.argv[3] || null;

(async () => {
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 1600, height: 960 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(e.message));
  const api = (cmd, args = {}) =>
    p.evaluate(async ([c, a]) => (await (await fetch("/api/" + c, { method: "POST", body: JSON.stringify(a) })).json()), [cmd, args]);
  const state = async () => (await api("state.get")).ok;
  const shot = async (n) => OUT && p.screenshot({ path: `${OUT}/${n}.png` });
  const step = (t) => console.log("•", t);

  await p.goto(BASE);

  // start from the default window layout (layouts persist between runs)

  await p.evaluate(() => fetch("/api/layout.set", { method: "POST", body: JSON.stringify({ reset: true }) }));

  await p.reload();
  await p.waitForSelector(".topbar");
  await p.waitForTimeout(800);
  const s0 = await state();
  assert.ok(s0.open, "project open");

  step("live spotting while playing (Spot mode: G hit, F x3 footsteps, Enter closes)");
  await p.click("button:has-text('Spot')");
  await api("transport.play", { from: 48000 * 30 });
  await p.waitForTimeout(400);
  await p.keyboard.press("KeyG");
  for (let i = 0; i < 3; i++) {
    await p.waitForTimeout(250);
    await p.keyboard.press("KeyF");
  }
  await p.keyboard.press("Enter");
  await p.keyboard.press("Space"); // stop
  await p.waitForTimeout(400);
  let s = await state();
  assert.equal(s.markers.length, s0.markers.length + 2, "hit + footsteps markers");
  const steps = s.markers.find((m) => m.kind === "repeated" && !m.cue_id);
  assert.equal(steps.sub_events.length, 3, "three tapped steps");
  const hit = s.markers.find((m) => m.kind === "point" && !m.cue_id);
  assert.ok(hit.start_sample > 48000 * 30 && hit.start_sample < 48000 * 31.5, "hit lands at heard position");
  await shot("10_spotted");

  step("cue chart: make cues, preview + apply populate");
  await p.click(".tabs button:has-text('Cue chart')");
  await p.click("button:has-text('Make cues from 2 markers')");
  await p.waitForTimeout(300);
  s = await state();
  assert.equal(s.cues.length, s0.cues.length + 2);
  await p.click("button:has-text('Populate placeholders')");
  await p.waitForSelector(".preview");
  await shot("11_populate_preview");
  await p.click(".preview button:has-text('Apply')");
  await p.waitForTimeout(400);
  s = await state();
  const newCue = s.cues.find((c) => c.marker_id === hit.id);
  assert.ok(s.clips.some((c) => c.cue_id === newCue.id), "placeholder placed for new hit cue");

  step("replace the new cue's sound from the library, then undo");
  await p.click(`td.code >> text=${newCue.code}`);
  await p.click(".leftdock .tabs button:has-text('Library')");
  await p.fill(".lib input", "glass smash 02");
  await p.waitForTimeout(400);
  await p.click(".lib-item >> text=glass_smash_02");
  await p.click("button:has-text('Replace (this cue)')");
  await p.waitForTimeout(400);
  s = await state();
  const glass2 = (await api("library.search", { text: "glass smash 02" })).ok[0].id;
  assert.equal(s.clips.find((c) => c.cue_id === newCue.id).asset_id, glass2);
  await shot("12_replaced");
  await p.click(".table-wrap");
  await p.keyboard.press("Control+KeyZ");
  await p.waitForTimeout(400);
  s = await state();
  assert.notEqual(s.clips.find((c) => c.cue_id === newCue.id).asset_id, glass2, "undo restored placeholder");

  step("perform 4 footsteps on pads against picture");
  const walk = s.cues.find((c) => c.description === "Greg crosses kitchen");
  await p.click(`td.code >> text=${walk.code}`);
  await p.click("button:has-text('Perform on pads')");
  await p.waitForSelector(".pads-panel");
  await p.click("button:has-text('Arm & play')");
  await p.waitForTimeout(500);
  for (let i = 0; i < 4; i++) {
    await p.keyboard.press("KeyJ");
    await p.waitForTimeout(220);
  }
  await shot("13_perform");
  await p.click("button:has-text('Keep take')");
  await p.waitForTimeout(500);
  s = await state();
  assert.ok(s.performances.some((x) => x.cue_id === walk.id), "performance take stored");
  const audible = s.clips.filter((c) => c.cue_id === walk.id && !c.muted);
  assert.equal(audible.length, 4, "4 performed clips audible, placeholders muted");

  step("timeline view + export");
  await p.click(".tabs button:has-text('Timeline')");
  await p.waitForTimeout(500);
  await shot("14_timeline");
  await p.click(".tabs button:has-text('QA & Export')");
  await p.click("button:has-text('Run QA')");
  await p.waitForTimeout(800);
  await p.click(".qa-col button.primary:has-text('Export')");
  await p.waitForSelector("text=files →", { timeout: 60000 });
  await shot("15_export");

  step("detached mixer window");
  const m = await b.newPage({ viewport: { width: 1200, height: 520 } });
  await m.goto(BASE + "/?view=mixer");
  await m.waitForSelector(".strip");
  await m.waitForTimeout(500);
  if (OUT) await m.screenshot({ path: `${OUT}/16_mixer_window.png` });
  assert.ok((await m.$$(".strip")).length >= 10);

  assert.deepEqual(errors, [], "no page errors");
  console.log("UI workflow passed");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

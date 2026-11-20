// Timeline trim and fade handles, driven with a real mouse, against
// `vcsd serve` on the demo project.
// Usage: node e2e/handles.cjs [base-url] [screenshot-dir]
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
    p.evaluate(async ([c, a]) => (await (await fetch("/api/" + c, { method: "POST", body: JSON.stringify(a) })).json()).ok, [cmd, args]);
  await p.goto(BASE);
  await p.waitForSelector(".tl-canvas");
  await p.waitForTimeout(800);

  // the door-handle placeholder instances in scene 12: short one-shots
  const st = await api("state.get");
  const cue = st.cues.find((c) => c.description === "door handle jiggle");
  let target = st.clips.filter((c) => c.cue_id === cue.id && !c.muted).sort((a, b) => a.timeline_start - b.timeline_start)[0];
  // known starting point: whole source, no fades (makes the test repeatable)
  const srcAsset = (await api("library.search", { text: "" })).find((a) => a.id === target.asset_id) || st.assets.find((a) => a.id === target.asset_id);
  await api("clip.update", { id: target.id, trim_start: target.timeline_start - target.source_offset, fade_in: 0, fade_out: 0 });
  await api("clip.update", { id: target.id, length: srcAsset.length });
  target = (await api("state.get")).clips.find((c) => c.id === target.id);
  assert.equal(target.source_offset, 0);
  // zoom in around it so the clip is wide on screen
  await api("transport.locate", { sample: target.timeline_start });
  const canvas = await p.$(".tl-canvas");
  const box = await canvas.boundingBox();
  for (let i = 0; i < 12; i++) {
    const hit0 = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), target.id);
    if (hit0 && hit0.w > 260) break;
    // anchor zoom on the clip start so it stays on screen
    const cx = hit0 ? box.x + Math.max(20, hit0.x) : box.x + box.width / 2;
    await p.mouse.move(cx, box.y + 60);
    await p.keyboard.down("Control");
    await p.mouse.wheel(0, -400);
    await p.keyboard.up("Control");
    await p.waitForTimeout(150);
  }
  // scroll so the whole clip is on screen (shift+wheel pans by pixels)
  const h0 = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), target.id);
  await p.mouse.move(box.x + 200, box.y + 60);
  await p.keyboard.down("Shift");
  await p.mouse.wheel(0, h0.x - 100);
  await p.keyboard.up("Shift");
  await p.waitForTimeout(200);
  const hit = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), target.id);
  assert.ok(hit.x > 20 && hit.x + hit.w < box.width - 20, `clip fully visible: ${JSON.stringify(hit)}`);
  assert.ok(hit && hit.w > 120, `clip visible and wide: ${JSON.stringify(hit)}`);
  const X = (x) => box.x + x;
  const Y = (y) => box.y + y;
  const clip = async () => (await api("state.get")).clips.find((c) => c.id === target.id);
  const c0 = await clip();

  // 1) fade-in handle: top band at the current fade-in position → drag right
  const fiX = hit.x + c0.fade_in / (c0.length / hit.w);
  await p.mouse.move(X(fiX), Y(hit.y + 3));
  assert.equal(await p.evaluate(() => document.querySelector(".tl-canvas").style.cursor), "ew-resize");
  await p.mouse.down();
  await p.mouse.move(X(fiX + hit.w * 0.25), Y(hit.y + 3), { steps: 6 });
  if (OUT) await p.screenshot({ path: `${OUT}/handles_fade_drag.png`, clip: { x: box.x, y: box.y, width: box.width, height: box.height } });
  await p.mouse.up();
  await p.waitForTimeout(300);
  const c1 = await clip();
  const want = Math.min(c0.fade_in + c0.length * 0.25, c0.length - c0.fade_out);
  assert.ok(Math.abs(c1.fade_in - want) < c0.length * 0.04, `fade in moved by ~25%: ${c0.fade_in} → ${c1.fade_in} of ${c0.length}`);

  // 2) right edge: trim the tail to ~60% (snaps to a frame boundary)
  let h = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), target.id);
  await p.mouse.move(X(h.x + h.w - 2), Y(h.y + h.h / 2 + 8));
  assert.equal(await p.evaluate(() => document.querySelector(".tl-canvas").style.cursor), "e-resize");
  await p.mouse.down();
  await p.mouse.move(X(h.x + h.w * 0.6), Y(h.y + h.h / 2 + 8), { steps: 6 });
  await p.mouse.up();
  await p.waitForTimeout(300);
  const c2 = await clip();
  assert.ok(Math.abs(c2.length - c1.length * 0.6) <= 2000 + c1.length * 0.05, `tail trimmed to ~60% (±1 frame): ${c1.length} → ${c2.length}`);
  assert.equal((c2.timeline_start + c2.length) % 2000, 0, "trimmed end snapped to a frame (24 fps @ 48 kHz)");
  assert.ok(c2.fade_in + c2.fade_out <= c2.length);

  // 3) left edge: trim the head in, audio stays put (source offset grows by the same amount)
  h = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), target.id);
  await p.mouse.move(X(h.x + 2), Y(h.y + h.h / 2 + 8));
  await p.mouse.down();
  await p.mouse.move(X(h.x + h.w * 0.3), Y(h.y + h.h / 2 + 8), { steps: 6 });
  await p.mouse.up();
  await p.waitForTimeout(300);
  const c3 = await clip();
  const d = c3.timeline_start - c2.timeline_start;
  assert.ok(d > 0, "head moved right");
  assert.equal(c3.source_offset - c2.source_offset, d, "audio stays in place");
  assert.equal(c3.timeline_start + c3.length, c2.timeline_start + c2.length, "end unchanged");
  if (OUT) await p.screenshot({ path: `${OUT}/handles_after.png`, clip: { x: box.x, y: box.y, width: box.width, height: box.height } });

  // 4) double-click the fade-in handle removes the fade
  h = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), target.id);
  const fx = h.x + c3.fade_in / (c3.length / h.w);
  await p.mouse.dblclick(X(fx), Y(h.y + 3));
  await p.waitForTimeout(300);
  assert.equal((await clip()).fade_in, 0, "fade removed");

  // undo steps back through the handle edits
  await p.keyboard.press("Control+KeyZ");
  await p.waitForTimeout(300);
  assert.equal((await clip()).fade_in, c3.fade_in, "undo restores the fade");

  assert.deepEqual(errors, []);
  console.log("timeline handles ok");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

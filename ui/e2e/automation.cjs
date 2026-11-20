// Automation lanes driven with a real mouse against `vcsd serve` (demo project).
// Usage: node e2e/automation.cjs [base-url] [screenshot-dir]
const { chromium } = require("playwright");
const assert = require("assert/strict");

const BASE = process.argv[2] || "http://127.0.0.1:7878";
const OUT = process.argv[3] || null;

(async () => {
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 1600, height: 960 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(e.message));
  p.on("dialog", (d) => d.accept());
  const api = (cmd, args = {}) =>
    p.evaluate(async ([c, a]) => (await (await fetch("/api/" + c, { method: "POST", body: JSON.stringify(a) })).json()).ok, [cmd, args]);
  await p.goto(BASE);
  await p.evaluate(() => localStorage.removeItem("vcsd.automationLanes"));
  await p.reload();
  await p.waitForSelector(".tl-canvas");
  await p.waitForTimeout(600);

  const st0 = await api("state.get");
  const track = st0.tracks[0];
  await api("automation.clear", { owner_id: track.id });
  const pts = async (param) =>
    ((await api("state.get")).automation || []).filter((x) => x.owner_id === track.id && x.param === param).sort((a, b) => a.sample - b.sample);

  // open the lanes from the track header
  const head = p.locator(".track-head", { hasText: track.name }).first();
  await head.locator("button:has-text('A')").click();
  await p.waitForSelector(".auto-head");
  assert.equal(await p.locator(".auto-head").count(), 2, "volume + pan lanes");
  const canvas = await (await p.$(".tl-canvas")).boundingBox();
  const vol = await p.locator(".auto-head").nth(0).boundingBox();
  const pan = await p.locator(".auto-head").nth(1).boundingBox();

  // click near the top of the volume lane, then low down further right
  await p.mouse.click(canvas.x + 150, vol.y + 2);
  await p.waitForTimeout(300);
  await p.mouse.click(canvas.x + 450, vol.y + vol.height - 8);
  await p.waitForTimeout(300);
  let v = await pts("volume_db");
  assert.equal(v.length, 2, "two breakpoints added");
  assert.ok(v[0].value > 6, `high click is loud: ${v[0].value}`);
  assert.ok(v[1].value < -45, `low click is quiet: ${v[1].value}`);
  assert.ok(v.every((x) => x.sample % 2000 === 0), "snapped to frames");
  if (OUT) await p.screenshot({ path: `${OUT}/automation_lanes.png`, clip: { x: 0, y: canvas.y - 30, width: 1300, height: canvas.height + 30 } });

  // drag the second point up with Shift held: value changes, time does not
  const hit = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), v[1].id);
  const hx = canvas.x + hit.x + hit.w / 2, hy = canvas.y + hit.y + hit.h / 2;
  await p.mouse.move(hx, hy);
  await p.mouse.down();
  await p.keyboard.down("Shift");
  await p.mouse.move(hx + 60, vol.y + vol.height / 2, { steps: 6 });
  if (OUT) await p.screenshot({ path: `${OUT}/automation_drag.png`, clip: { x: 0, y: canvas.y - 30, width: 1300, height: canvas.height + 30 } });
  await p.mouse.up();
  await p.keyboard.up("Shift");
  await p.waitForTimeout(300);
  let v2 = await pts("volume_db");
  const moved = v2.find((x) => x.id === v[1].id);
  assert.equal(moved.sample, v[1].sample, "Shift keeps the time");
  assert.ok(moved.value > -40 && moved.value < 0, `dragged to mid-lane: ${moved.value}`);

  // clicking on the line keeps its value (no jump)
  const mid = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), v[0].id);
  // the line is flat before the first point: click on it to the left of point 1
  await p.mouse.click(canvas.x + Math.max(10, mid.x - 60), canvas.y + mid.y + mid.h / 2);
  await p.waitForTimeout(300);
  v2 = await pts("volume_db");
  assert.equal(v2.length, 3);
  assert.ok(Math.abs(v2[0].value - v[0].value) < 0.05, "on-line click keeps value");

  // double-click a breakpoint deletes it
  const dh = await p.evaluate((id) => window.__vcsdHits.find((h) => h.id === id), v2[0].id);
  await p.mouse.dblclick(canvas.x + dh.x + dh.w / 2, canvas.y + dh.y + dh.h / 2);
  await p.waitForTimeout(300);
  assert.equal((await pts("volume_db")).length, 2, "deleted (the dblclick's own clicks add nothing on a point)");

  // pan lane: click on the centre line → centre value
  await p.mouse.click(canvas.x + 300, pan.y + pan.height / 2 - 1);
  await p.waitForTimeout(300);
  const pp = await pts("pan");
  assert.equal(pp.length, 1);
  assert.ok(Math.abs(pp[0].value - track.pan) < 0.05, `pan on line: ${pp[0].value}`);
  assert.match(await p.locator(".auto-head").nth(1).textContent(), /C|L|R/);

  // undo, lanes persist across reloads, clear button
  await p.keyboard.press("Control+KeyZ");
  await p.waitForTimeout(300);
  assert.equal((await pts("pan")).length, 0, "undo removes the pan point");
  await p.reload();
  await p.waitForSelector(".auto-head");
  await p.locator(".auto-head").nth(0).locator("button").click();
  await p.waitForTimeout(300);
  assert.equal((await pts("volume_db")).length, 0, "lane cleared");
  await p.locator(".track-head", { hasText: track.name }).first().locator("button:has-text('A')").click();
  await p.waitForTimeout(200);
  assert.equal(await p.locator(".auto-head").count(), 0, "lanes hidden");

  assert.deepEqual(errors, []);
  console.log("automation lanes ok");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

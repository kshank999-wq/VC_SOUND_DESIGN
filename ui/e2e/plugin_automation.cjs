// Plugin parameter automation in the UI, against `vcsd serve` with the test VST3
// installed (VCSD_VST3_PATH). Usage: node e2e/plugin_automation.cjs [base-url] [screenshot-dir]
const { chromium } = require("playwright");
const assert = require("assert/strict");

const BASE = process.argv[2] || "http://127.0.0.1:7878";
const OUT = process.argv[3] || null;

(async () => {
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 1600, height: 1000 } });
  const errors = [];
  p.on("pageerror", (e) => errors.push(e.message));
  p.on("dialog", (d) => d.accept());
  const api = (cmd, args = {}) =>
    p.evaluate(async ([c, a]) => (await (await fetch("/api/" + c, { method: "POST", body: JSON.stringify(a) })).json()).ok, [cmd, args]);
  await p.goto(BASE);
  await p.evaluate(() => fetch("/api/layout.set", { method: "POST", body: JSON.stringify({ reset: true }) }));
  await p.evaluate(() => { localStorage.removeItem("vcsd.pluginLanes"); localStorage.removeItem("vcsd.automationLanes"); });
  await p.reload();
  await p.waitForSelector(".topbar");
  await api("transport.stop"); // a known start, even after an aborted run
  let st = await api("state.get");
  for (const o of [...st.tracks, ...st.buses]) for (const i of o.inserts.filter((x) => x.type === "plugin")) await api("insert.remove", { owner_id: o.id, id: i.id });
  const master = st.buses.find((x) => !x.parent_id);
  await api("automation.mode", { owner_id: master.id, mode: "read" });
  await api("automation.clear", { owner_id: master.id });
  const gain = (await api("plugins.list")).plugins.find((x) => x.name === "VCSD Test Gain");
  const slot = (await api("insert.add", { owner_id: master.id, plugin_id: gain.id })).id;
  const lane = async () => ((await api("state.get")).automation || []).filter((x) => x.owner_id === slot && x.param === "plugin:0").sort((a, b) => a.sample - b.sample);

  // ---- the plugin panel's "A" shows the parameter's lane under the master bus
  await p.click(".tabs button:has-text('Mixer')");
  await p.locator(".strip.master .fx-name").click();
  await p.waitForSelector(".plugin-window .param-row");
  await p.locator(".plugin-window .param-row button:has-text('A')").click();
  await p.click(".plugin-window .plugin-head button[title='Close']");
  await p.click(".tabs button:has-text('Timeline')");
  const canvas = await (await p.$(".tl-canvas")).boundingBox();
  await p.mouse.move(canvas.x + 300, canvas.y + 200);
  await p.mouse.wheel(0, 4000);
  await p.waitForTimeout(300);
  const head = p.locator(".auto-head.plugin", { hasText: "VCSD Test Gain · Gain" });
  await head.waitFor();
  assert.ok(await p.locator(".auto-add select").count(), "+ Plugin lane picker under the master");

  // ---- draw two points: low at the left, high further right
  const hb = await head.boundingBox();
  await p.mouse.click(canvas.x + 150, hb.y + hb.height - 6);
  await p.waitForTimeout(250);
  await p.mouse.click(canvas.x + 650, hb.y + 6);
  await p.waitForTimeout(300);
  let pts = await lane();
  assert.equal(pts.length, 2, JSON.stringify(pts));
  assert.ok(pts[0].value < 0.1 && pts[1].value > 0.9, `low then high: ${pts.map((x) => x.value)}`);
  if (OUT) await p.screenshot({ path: `${OUT}/plugin_lane.png`, clip: { x: 0, y: hb.y - 140, width: 1300, height: 260 } });

  // ---- while playing, the plugin follows the lane (the panel shows it)
  await api("transport.play", { from: pts[1].sample + 4800 });
  await p.waitForTimeout(400);
  let shown = (await api("plugin.params", { id: slot })).params[0];
  await api("transport.stop");
  assert.ok(Math.abs(shown.value - pts[1].value) < 0.01, `plugin at the lane's value: ${shown.value} vs ${pts[1].value}`);

  // ---- writing: Latch on the master, drag the panel's slider while playing
  await api("automation.clear", { owner_id: slot });
  await api("automation.mode", { owner_id: master.id, mode: "latch" });
  await p.click(".tabs button:has-text('Mixer')");
  await p.locator(".strip.master .fx-name").click();
  await p.waitForSelector(".plugin-window .param-row");
  await api("transport.play", { from: 48000 * 2 });
  await p.waitForTimeout(300);
  const slider = p.locator(".plugin-window .param-row input[type=range]").first();
  const sb = await slider.boundingBox();
  await p.mouse.move(sb.x + sb.width * 0.5, sb.y + sb.height / 2);
  await p.mouse.down();
  for (let i = 1; i <= 10; i++) {
    await p.mouse.move(sb.x + sb.width * (0.5 - 0.035 * i), sb.y + sb.height / 2);
    await p.waitForTimeout(40);
  }
  assert.ok(await p.locator(".plugin-window .param-row.writing").count(), "the row shows it is being written");
  await p.waitForTimeout(350);
  const txt = await p.locator(".plugin-window .param-val").first().textContent();
  assert.ok(/^-\d/.test(txt) || txt.startsWith("-inf"), `value text follows the write: ${txt}`);
  if (OUT) await p.screenshot({ path: `${OUT}/plugin_write.png` });
  await p.mouse.up();
  await p.waitForTimeout(300);
  await api("transport.stop"); // stop → commit (keys stay inside the plugin window)
  await p.waitForTimeout(400);
  st = await api("state.get");
  assert.equal(st.undo, "write automation");
  pts = await lane();
  assert.ok(pts.length >= 2 && pts[pts.length - 1].value < 0.25, `latched low: ${pts.map((x) => x.value.toFixed(2))}`);
  await api("automation.mode", { owner_id: master.id, mode: "read" });

  // ---- the picker adds another parameter lane (the delay's gain, after inserting it)
  const delay = (await api("plugins.list")).plugins.find((x) => x.name === "VCSD Test Delay");
  await api("insert.add", { owner_id: master.id, plugin_id: delay.id });
  await p.click(".plugin-window .plugin-head button[title='Close']");
  await p.click(".tabs button:has-text('Timeline')");
  await p.mouse.move(canvas.x + 300, canvas.y + 200);
  await p.mouse.wheel(0, 4000);
  const sel = p.locator(".auto-add select").last();
  await sel.focus();
  await p.waitForTimeout(400);
  const opt = await sel.locator("optgroup[label='VCSD Test Delay'] option").first().getAttribute("value");
  await sel.selectOption(opt);
  await p.locator(".auto-head.plugin", { hasText: "VCSD Test Delay · Gain" }).waitFor();

  // ---- removing the plugin takes its lanes along
  await api("insert.remove", { owner_id: master.id, id: slot });
  assert.equal((await lane()).length, 0);
  await p.waitForTimeout(300);
  assert.equal(await p.locator(".auto-head.plugin", { hasText: "VCSD Test Gain" }).count(), 0);

  // clean up
  st = await api("state.get");
  for (const i of st.buses.find((x) => x.id === master.id).inserts.filter((x) => x.type === "plugin")) await api("insert.remove", { owner_id: master.id, id: i.id });
  await p.evaluate(() => { localStorage.removeItem("vcsd.pluginLanes"); localStorage.removeItem("vcsd.automationLanes"); });
  assert.deepEqual(errors, []);
  console.log("plugin automation ok");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

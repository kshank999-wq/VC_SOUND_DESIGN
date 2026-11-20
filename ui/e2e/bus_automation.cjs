// Bus and master automation lanes, driven with the mouse, verified by what
// the engine actually plays. Usage: node e2e/bus_automation.cjs [base-url] [screenshot-dir]
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
  await p.evaluate(() => localStorage.removeItem("vcsd.automationLanes"));
  await p.reload();
  await p.waitForSelector(".tl-canvas");
  await p.waitForTimeout(600);

  const st = await api("state.get");
  const master = st.buses.find((x) => !x.parent_id);
  const bg = st.buses.find((x) => x.name === "Backgrounds/Atmos");
  for (const x of [master, bg]) await api("automation.clear", { owner_id: x.id });
  const pts = async (owner) => ((await api("state.get")).automation || []).filter((x) => x.owner_id === owner);
  const peakWhilePlaying = async (key, from = 48000 * 2) => {
    await api("transport.play", { from });
    let m = 0;
    for (let i = 0; i < 12; i++) {
      await p.waitForTimeout(60);
      const t = await api("transport.state");
      m = Math.max(m, key === "master" ? t.master_peak : t.meters[key] ?? 0);
    }
    await api("transport.stop");
    return m;
  };

  // the Buses section lists every stem bus and the master
  const heads = p.locator(".bus-head");
  assert.equal(await heads.count(), st.buses.length);
  assert.match(await heads.last().textContent(), /Review Mix/);

  // scroll the timeline down to the bus rows
  const canvas = await (await p.$(".tl-canvas")).boundingBox();
  await p.mouse.move(canvas.x + 300, canvas.y + 200);
  await p.mouse.wheel(0, 2000);
  await p.waitForTimeout(200);

  const baseline = await peakWhilePlaying("master");
  // steady wind ambience (scene 14) makes the stem-bus level comparison stable
  const WIND = 48000 * 42;
  const bgBase = await peakWhilePlaying(bg.id, WIND);
  assert.ok(baseline > 0.01 && bgBase > 0.01, "audible before automation");

  // master: open lanes, click low in the volume lane → near silence
  await heads.last().locator("button:has-text('A')").click();
  await p.mouse.wheel(0, 2000);
  await p.waitForTimeout(250);
  const vol = await p.locator(".auto-head").nth(0).boundingBox();
  assert.ok(vol.y + vol.height <= canvas.y + canvas.height, "master lanes visible");
  await p.mouse.click(canvas.x + 200, vol.y + vol.height - 5);
  await p.waitForTimeout(300);
  const mp = await pts(master.id);
  assert.equal(mp.length, 1);
  assert.ok(mp[0].value < -60, `low click: ${mp[0].value}`);
  if (OUT) await p.screenshot({ path: `${OUT}/bus_automation.png`, clip: { x: 0, y: canvas.y - 30, width: 1300, height: canvas.height + 30 } });
  const quiet = await peakWhilePlaying("master");
  assert.ok(quiet < baseline / 100, `master automation silences the mix live: ${baseline.toFixed(3)} → ${quiet.toFixed(5)}`);
  await api("automation.clear", { owner_id: master.id });

  // stem bus: -12 dB via API, the bus meter follows; mixer shows the badge
  await api("automation.set", { bus_id: bg.id, param: "volume_db", sample: 0, value: -12 });
  const bgAuto = await peakWhilePlaying(bg.id, WIND);
  const delta = 20 * Math.log10(bgAuto / bgBase);
  // live peak meters are approximate; exact dB accuracy is covered by the core render test
  assert.ok(Math.abs(delta + 12) < 1.5, `stem bus about −12 dB live: ${delta.toFixed(2)} dB`);
  await p.click(".tabs button:has-text('Mixer')");
  await p.waitForTimeout(300);
  assert.equal(await p.locator(".mixer-buses .auto-badge:not(.off)").count(), 1, "badge on the automated bus only");
  if (OUT) await p.screenshot({ path: `${OUT}/bus_automation_mixer.png`, clip: { x: 270, y: canvas.y - 30, width: 1030, height: canvas.height + 30 } });
  await api("automation.clear", { owner_id: bg.id });

  assert.deepEqual(errors, []);
  console.log("bus + master automation ok");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

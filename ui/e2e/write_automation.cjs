// Write automation by moving a real mixer fader during playback (Latch),
// then check Touch returns on release. Usage: node e2e/write_automation.cjs [base-url] [screenshot-dir]
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
  await p.waitForSelector(".topbar");
  await p.click(".tabs button:has-text('Mixer')");
  await p.waitForSelector(".strip");

  const st = await api("state.get");
  const track = st.tracks.find((t) => t.bus_id === st.buses.find((x) => x.name === "Foley").id);
  await api("automation.clear", { owner_id: track.id });
  await api("track.update", { id: track.id, gain_db: 0 });
  const lane = async () => ((await api("state.get")).automation || []).filter((x) => x.owner_id === track.id && x.param === "volume_db").sort((a, b) => a.sample - b.sample);

  const strip = p.locator(".mixer-tracks .strip").first();
  assert.match(await strip.locator(".strip-name").getAttribute("title"), new RegExp(track.name.replace(/[•]/g, ".")));
  await strip.locator("select.automode-select").selectOption("latch");
  await p.waitForTimeout(300);
  assert.equal((await api("state.get")).tracks.find((t) => t.id === track.id).automation_mode, "latch");

  // play, grab the fader thumb and pull it down while picture runs
  await api("transport.play", { from: 48000 * 2 });
  await p.waitForTimeout(400);
  const thumb = await strip.locator(".fader-thumb").boundingBox();
  const fader = await strip.locator(".fader").boundingBox();
  await p.mouse.move(thumb.x + thumb.width / 2, thumb.y + thumb.height / 2);
  await p.mouse.down();
  for (let i = 1; i <= 12; i++) {
    await p.mouse.move(thumb.x + thumb.width / 2, thumb.y + thumb.height / 2 + (fader.y + fader.height * 0.55 - thumb.y) * (i / 12));
    await p.waitForTimeout(40);
  }
  assert.ok(await strip.evaluate((el) => el.classList.contains("is-writing")), "strip shows it is being written");
  if (OUT) await p.screenshot({ path: `${OUT}/write_latch_mixer.png`, clip: { x: 270, y: 440, width: 1030, height: 520 } });
  await p.mouse.up();
  const tx = await api("transport.state");
  assert.equal(tx.writing.length, 1, "latched after release");
  const latched = tx.writing[0].value;
  assert.ok(latched < -10, `fader pulled down: ${latched} dB`);
  await p.waitForTimeout(400);
  await p.keyboard.press("Space"); // stop via the UI → commit
  await p.waitForTimeout(400);
  const pts = await lane();
  assert.ok(pts.length >= 4 && pts.length < 40, `drag became a thinned set of breakpoints: ${pts.length}`);
  const last = pts[pts.length - 2]; // last before the 50 ms return ramp
  assert.ok(Math.abs(last.value - latched) < 0.01, "held at the latched value to stop");
  assert.ok(Math.abs(pts[pts.length - 1].value) < 0.01, "ramps back to the previous level");
  // the fader now follows the written automation in Read playback position
  await api("transport.locate", { sample: last.sample - 100 });
  await p.waitForTimeout(300);
  const shown = Number(await strip.locator(".mono.small").first().textContent());
  assert.ok(Math.abs(shown - latched) < 0.6, `fader displays automation: ${shown}`);

  // Touch: the lane returns on release (committed while still playing)
  await strip.locator("select.automode-select").selectOption("touch");
  await api("automation.clear", { owner_id: track.id });
  await api("transport.play", { from: 48000 * 2 });
  await p.waitForTimeout(300);
  const t2 = await strip.locator(".fader-thumb").boundingBox();
  await p.mouse.move(t2.x + t2.width / 2, t2.y + t2.height / 2);
  await p.mouse.down();
  await p.mouse.move(t2.x + t2.width / 2, t2.y + 120, { steps: 8 });
  await p.waitForTimeout(250);
  await p.mouse.up();
  await p.waitForTimeout(250);
  assert.equal((await api("transport.state")).writing.length, 0, "touch committed on release");
  assert.ok((await lane()).length >= 3);
  await api("transport.stop");

  // put the strip back
  await strip.locator("select.automode-select").selectOption("read");
  await api("automation.clear", { owner_id: track.id });
  assert.deepEqual(errors, []);
  console.log("write/latch/touch automation ok");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

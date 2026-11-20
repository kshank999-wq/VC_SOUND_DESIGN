// Persisted window layout: resize panels with the splitters, change tabs and
// the timeline view, reload, and check everything comes back.
// Usage: node e2e/layout.cjs [base-url] [screenshot-dir]
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
  await api("layout.set", { reset: true });
  await p.reload();
  await p.waitForSelector(".tl-canvas");
  await p.waitForTimeout(400);

  const w = async (sel) => (await p.locator(sel).boundingBox()).width;
  const h = async (sel) => (await p.locator(sel).boundingBox()).height;
  assert.equal(Math.round(await w("aside.left")), 270);
  assert.equal(Math.round(await w("aside.right")), 300);

  const drag = async (from, to) => {
    await p.mouse.move(from.x, from.y);
    await p.mouse.down();
    await p.mouse.move(to.x, to.y, { steps: 8 });
    await p.mouse.up();
  };
  // splitters: left panel → 360, inspector → 400, picture → ~30% of the centre
  await drag({ x: 270, y: 500 }, { x: 360, y: 500 });
  await drag({ x: 1300, y: 500 }, { x: 1200, y: 500 });
  const center = await p.locator("main.center").boundingBox();
  const split = await p.locator(".splitter.h").boundingBox();
  await drag({ x: center.x + 200, y: split.y + 3 }, { x: center.x + 200, y: center.y + center.height * 0.3 });
  await p.waitForTimeout(200);
  assert.equal(Math.round(await w("aside.left")), 360);
  assert.equal(Math.round(await w("aside.right")), 400);
  const upper = (await h("main.center .upper")) / center.height;
  assert.ok(Math.abs(upper - 0.3) < 0.02, `picture area ~30%: ${upper}`);

  // timeline view: zoom in and scroll down
  const canvas = await p.locator(".tl-canvas").boundingBox();
  await p.mouse.move(canvas.x + 300, canvas.y + 150);
  await p.keyboard.down("Control");
  await p.mouse.wheel(0, -600);
  await p.keyboard.up("Control");
  await p.mouse.wheel(0, 180);
  await p.waitForTimeout(300);
  const zoomLabel = await p.locator(".tl-corner").textContent();
  const headY = await p.locator(".tl-heads > div").evaluate((el) => el.style.transform);
  assert.notEqual(headY, "translateY(0px)", "scrolled");

  // tabs and snap
  await p.click(".leftdock .tabs button:has-text('Library')");
  await p.click(".tabs button:has-text('Cue chart')");
  await p.click("button:has-text('Snap')");
  await p.waitForTimeout(600); // debounce → engine prefs
  if (OUT) await p.screenshot({ path: `${OUT}/layout_custom.png` });

  // reload: everything restored
  await p.reload();
  await p.waitForSelector(".topbar");
  await p.waitForTimeout(400);
  assert.equal(Math.round(await w("aside.left")), 360, "left panel width restored");
  assert.equal(Math.round(await w("aside.right")), 400, "inspector width restored");
  const c2 = await p.locator("main.center").boundingBox();
  assert.ok(Math.abs((await h("main.center .upper")) / c2.height - 0.3) < 0.02, "picture/timeline split restored");
  assert.match(await p.locator(".leftdock .tabs button.active").textContent(), /Library/);
  assert.match(await p.locator(".lower .tabs button.active").textContent(), /Cue chart/);
  assert.ok(!(await p.locator("button:has-text('Snap')").evaluate((el) => el.classList.contains("on"))), "snap off restored");
  await p.click(".tabs button:has-text('Timeline')");
  await p.waitForTimeout(300);
  assert.equal(await p.locator(".tl-corner").textContent(), zoomLabel, "timeline zoom restored");
  assert.equal(await p.locator(".tl-heads > div").evaluate((el) => el.style.transform), headY, "timeline scroll restored");

  // stored in the engine's per-user prefs (not browser storage)
  const saved = await api("layout.get");
  assert.equal(saved.leftW, 360);
  assert.ok(Object.values(saved.projects).some((v) => v.vp && v.scrollY > 0));

  // double-click a splitter: that panel back to default
  await p.mouse.dblclick(360, 500);
  await p.waitForTimeout(500);
  assert.equal(Math.round(await w("aside.left")), 270);
  assert.equal((await api("layout.get")).leftW, 270);

  // menu → Reset window layout
  await p.click(".brand");
  await p.click(".menu >> text=Reset window layout");
  await p.waitForTimeout(300);
  assert.equal(Math.round(await w("aside.right")), 300);
  assert.deepEqual(await api("layout.get"), {});
  await p.reload();
  await p.waitForSelector(".topbar");
  await p.waitForTimeout(300);
  assert.equal(Math.round(await w("aside.right")), 300, "defaults after reset + reload");

  assert.deepEqual(errors, []);
  console.log("window layout ok");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

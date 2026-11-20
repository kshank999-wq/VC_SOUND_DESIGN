// Track folders in the timeline, driven with the mouse against `vcsd serve`.
// Restores every track's folder at the end. Usage: node e2e/folders.cjs [base-url] [screenshot-dir]
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
  const tracks = async () => (await api("state.get")).tracks;
  await p.goto(BASE);
  // start from the default window layout (layouts persist between runs)
  await p.evaluate(() => fetch("/api/layout.set", { method: "POST", body: JSON.stringify({ reset: true }) }));
  await p.reload();
  await p.evaluate(() => localStorage.removeItem("vcsd.collapsedFolders"));
  await p.reload();
  await p.waitForSelector(".tl-canvas");
  await p.waitForTimeout(500);

  const st0 = await api("state.get");
  const original = st0.tracks;
  // known starting layout: "Scene N/<DEPT>" from each track's scene (score etc. stay at top level)
  for (const t of original) {
    const sc = st0.scenes.find((x) => x.id === t.scene_id);
    await api("folder.move", { track_ids: [t.id], folder: sc ? `Scene ${sc.number}/${t.name.split(" • ")[0]}` : "" });
  }
  await p.waitForTimeout(400);
  const head = (name) => p.locator(".folder-head", { has: p.locator(".folder-name", { hasText: new RegExp(`📁 ${name}$`) }) }).first();
  assert.ok((await p.locator(".folder-head").count()) >= 4, "scene folders with department subfolders");
  const tracksVisible = async () => p.locator(".track-head").count();

  // collapse a scene: its tracks disappear from the header column; state persists across reload
  const before = await tracksVisible();
  await head("Scene 12").locator("button.twisty").click();
  await p.waitForTimeout(200);
  const after = await tracksVisible();
  assert.ok(after < before, `collapsed hides tracks: ${before} → ${after}`);
  if (OUT) await p.screenshot({ path: `${OUT}/folders_collapsed.png`, clip: { x: 270, y: 440, width: 1030, height: 500 } });
  await p.reload();
  await p.waitForSelector(".folder-head");
  assert.equal(await tracksVisible(), after, "collapse remembered");
  await head("Scene 12").locator("button.twisty").click();
  await p.waitForTimeout(200);

  // inline rename (double-click): subfolders follow
  await head("Scene 13").locator(".folder-name").dblclick();
  await p.keyboard.type("SC 13 kitchen");
  await p.keyboard.press("Enter");
  await p.waitForTimeout(300);
  let ts = await tracks();
  assert.ok(ts.some((t) => t.folder.startsWith("SC 13 kitchen/")) && !ts.some((t) => t.folder.startsWith("Scene 13")), "renamed with children");

  // drag a track (by its name) onto another folder
  const mover = ts.find((t) => t.folder.startsWith("Scene 14"));
  // collapse what's in between so source and target are both on screen (a collapsed folder still accepts drops)
  await head("Scene 12").locator("button.twisty").click();
  await head("SC 13 kitchen").locator("button.twisty").click();
  await p.waitForTimeout(200);
  const grip = p.locator(".track-head .drag-grip", { hasText: mover.name }).first();
  const tl = await (await p.$(".tl-canvas")).boundingBox();
  const gb = await grip.boundingBox();
  assert.ok(gb.y + gb.height < tl.y + tl.height, "track header visible for dragging");
  await grip.dragTo(head("SC 13 kitchen"));
  await head("SC 13 kitchen").locator("button.twisty").click();
  await head("Scene 12").locator("button.twisty").click();
  await p.waitForTimeout(300);
  ts = await tracks();
  assert.equal(ts.find((t) => t.id === mover.id).folder, "SC 13 kitchen", "dropped into the folder");

  // folder mute affects every track inside
  await head("SC 13 kitchen").locator("button:has-text('M')").click();
  await p.waitForTimeout(300);
  ts = await tracks();
  const inside = ts.filter((t) => t.folder === "SC 13 kitchen" || t.folder.startsWith("SC 13 kitchen/"));
  assert.ok(inside.length >= 2 && inside.every((t) => t.mute), "folder mute");
  assert.ok(ts.filter((t) => !inside.includes(t)).every((t) => !t.mute), "nothing else muted");
  await head("SC 13 kitchen").locator("button:has-text('M')").click();
  await p.waitForTimeout(300);

  // inspector: select the folder, ungroup it
  await head("SC 13 kitchen").locator(".folder-name").click();
  await p.waitForSelector(".inspector >> text=📁 Folder");
  if (OUT) await p.screenshot({ path: `${OUT}/folders_inspector.png` });
  await p.click(".inspector button:has-text('Ungroup')");
  await p.waitForTimeout(300);
  ts = await tracks();
  assert.ok(!ts.some((t) => t.folder.startsWith("SC 13 kitchen")), "ungrouped");
  // track inspector: type a new folder path
  await p.locator(".track-head .drag-grip", { hasText: mover.name }).first().click();
  const field = p.locator(".inspector input[list='vcsd-folders']");
  await field.fill("Sweeteners/Vehicles");
  await field.press("Enter");
  await p.waitForTimeout(300);
  assert.equal((await tracks()).find((t) => t.id === mover.id).folder, "Sweeteners/Vehicles");
  await p.click(".tl-canvas", { position: { x: 5, y: 5 } });
  await p.keyboard.press("Control+KeyZ");
  await p.waitForTimeout(300);
  assert.equal((await tracks()).find((t) => t.id === mover.id).folder, "", "undo restores (ungroup left it at top level)");

  // restore the demo project's folders
  // (scene folders are restored in the project's standard layout)
  for (const t of original) {
    const sc = st0.scenes.find((x) => x.id === t.scene_id);
    await api("folder.move", { track_ids: [t.id], folder: sc ? `Scene ${sc.number}/${t.name.split(" • ")[0]}` : t.folder.startsWith("Music") ? t.folder : "" });
  }
  assert.deepEqual(errors, []);
  console.log("track folders ok");
  await b.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

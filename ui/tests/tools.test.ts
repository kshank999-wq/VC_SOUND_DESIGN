import { test } from "node:test";
import assert from "node:assert/strict";
import { TOOLS, clipAction, thinStroke, toolForKey } from "../src/tools.ts";

test("six tools with distinct keys", () => {
  assert.deepEqual(TOOLS.map((t) => t.id), ["select", "trim", "fade", "split", "pencil", "hand"]);
  assert.equal(new Set(TOOLS.map((t) => t.code)).size, 6);
  assert.equal(toolForKey("KeyX"), "split");
  assert.equal(toolForKey("KeyQ"), null);
});

test("a press on a clip means what the tool says", () => {
  assert.deepEqual(clipAction("select", "move", 0.2), { zone: "move" });
  assert.deepEqual(clipAction("select", "fadeIn", 0.02), { zone: "fadeIn" });
  assert.deepEqual(clipAction("trim", "move", 0.3), { zone: "trimL" });
  assert.deepEqual(clipAction("trim", "move", 0.7), { zone: "trimR" });
  assert.deepEqual(clipAction("fade", "move", 0.1), { zone: "fadeIn" });
  assert.deepEqual(clipAction("fade", "move", 0.9), { zone: "fadeOut" });
  assert.deepEqual(clipAction("split", "move", 0.5), { split: true });
  assert.equal(clipAction("hand", "move", 0.5), null);
  assert.equal(clipAction("pencil", "move", 0.5), null);
});

test("pencil strokes thin to one point per few pixels, keeping both ends", () => {
  const pts = Array.from({ length: 20 }, (_, i) => ({ x: i, sample: i * 100, value: 20 - i }));
  const t = thinStroke(pts, 6);
  assert.deepEqual(t[0], [0, 20]);
  assert.deepEqual(t[t.length - 1], [1900, 1]);
  assert.deepEqual(t.map((p) => p[0]), [0, 600, 1200, 1800, 1900]);
  assert.deepEqual(thinStroke([]), []);
});

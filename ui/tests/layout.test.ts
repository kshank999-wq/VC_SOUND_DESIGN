import { test } from "node:test";
import assert from "node:assert/strict";
import { merge, sanitize } from "../src/layoutModel.ts";

test("sanitize keeps valid values and clamps sizes", () => {
  const l = sanitize({ leftW: 9999, rightW: 10, upperPct: 50, leftTab: "library", lowerTab: "nope", snap: false, junk: 1,
    projects: { "/p": { vp: { start: -4800, spp: 400 }, scrollY: -5 }, "/q": { vp: { start: 0, spp: 0 } } } });
  assert.deepEqual(l, { leftW: 520, rightW: 220, upperPct: 50, leftTab: "library", snap: false,
    projects: { "/p": { vp: { start: -4800, spp: 400 }, scrollY: 0 }, "/q": {} } });
  assert.deepEqual(sanitize(null), {});
  assert.deepEqual(sanitize("x"), {});
});

test("merge combines independent saves", () => {
  const a = merge({ leftW: 300, projects: { "/a": { scrollY: 10 } } }, { projects: { "/a": { vp: { start: 0, spp: 2 } }, "/b": { scrollY: 3 } } });
  assert.deepEqual(a, { leftW: 300, projects: { "/a": { scrollY: 10, vp: { start: 0, spp: 2 } }, "/b": { scrollY: 3 } } });
});

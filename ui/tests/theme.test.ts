import { test } from "node:test";
import assert from "node:assert/strict";
import { ACCENTS, DEFAULT_THEME, pushRecent, sanitizeTheme, theme } from "../src/theme.ts";
import { sanitize } from "../src/layoutModel.ts";

test("accent choices are the ones the spec's color rules allow", () => {
  assert.deepEqual(ACCENTS.map((a) => a.id), ["blue", "indigo", "magenta"]);
});

test("stored themes keep only valid values", () => {
  assert.deepEqual(sanitizeTheme({ accent: "amber", readout: "ice", density: "compact", reduceTransparency: "yes" }), { readout: "ice", density: "compact" });
  assert.deepEqual(theme(undefined), DEFAULT_THEME);
  assert.equal(theme({ accent: "magenta" }).accent, "magenta");
});

test("recent projects: newest first, no duplicates, at most 8", () => {
  let l = pushRecent(undefined, { path: "/a", name: "A", at: 1 });
  l = pushRecent(l, { path: "/b", name: "B", at: 2 });
  l = pushRecent(l, { path: "/a", name: "A", at: 3 });
  assert.deepEqual(l.map((x) => x.path), ["/a", "/b"]);
  for (let i = 0; i < 20; i++) l = pushRecent(l, { path: "/p" + i, name: "P", at: 10 + i });
  assert.equal(l.length, 8);
  assert.equal(sanitize({ recent: [{ path: "/x", name: "X", at: 5 }, { path: 3 }] }).recent?.length, 1);
});

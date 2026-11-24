import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { STAGES, STAGE_DEFAULTS, stageForKey, stageLayout, sanitizeStageLayout } from "../src/stage.ts";
import { sanitize } from "../src/layoutModel.ts";

test("six stages in workflow order, keys 1-6", () => {
  assert.deepEqual(STAGES.map((s) => s.id), ["prep", "spot", "build", "perform", "mix", "deliver"]);
  assert.equal(stageForKey("Digit1"), "prep");
  assert.equal(stageForKey("Numpad5"), "mix");
  assert.equal(stageForKey("Digit7"), null);
  assert.equal(stageForKey("KeyF"), null);
});

test("stage defaults follow the spec's layout table", () => {
  assert.equal(STAGE_DEFAULTS.spot.lanes, "full");
  assert.equal(STAGE_DEFAULTS.mix.pictureH, 0);
  assert.equal(STAGE_DEFAULTS.mix.deckTab, "mixer");
  assert.equal(STAGE_DEFAULTS.deliver.center, "qa");
  assert.equal(STAGE_DEFAULTS.deliver.deckOpen, false);
});

test("saved changes override defaults, clamped; junk ignored", () => {
  const l = stageLayout("build", { pictureH: 9999, deckTab: "mixer", lanes: "nope" as never, deckOpen: false });
  assert.equal(l.pictureH, 900);
  assert.equal(l.deckTab, "mixer");
  assert.equal(l.lanes, "folded", "invalid value falls back to the default");
  assert.equal(l.deckOpen, false);
  assert.deepEqual(sanitizeStageLayout("x"), {});
});

test("the stored layout keeps the stage and per-stage changes", () => {
  const l = sanitize({ stage: "mix", stages: { mix: { deckH: 300 }, bogus: { deckH: 1 } } });
  assert.equal(l.stage, "mix");
  assert.deepEqual(l.stages, { mix: { deckH: 300 } });
  assert.equal(sanitize({ stage: "nope" }).stage, undefined);
});

test("the app's tokens are the spec's tokens", () => {
  const a = readFileSync(new URL("../src/theme/tokens.css", import.meta.url), "utf8");
  const b = readFileSync(new URL("../../docs/spec/ui/tokens.css", import.meta.url), "utf8");
  assert.equal(a, b, "copy docs/spec/ui/tokens.css to ui/src/theme/tokens.css");
});

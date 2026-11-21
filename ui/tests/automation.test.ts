import { test } from "node:test";
import assert from "node:assert/strict";
import { formatValue, posToValue, valueAt, valueToPos, type AutoPoint } from "../src/automation.ts";

test("value ↔ position round-trips and spans the lane", () => {
  for (const db of [-96, -60, -48, -20, -6, 0, 6, 12]) assert.ok(Math.abs(posToValue("volume_db", valueToPos("volume_db", db)) - db) < 0.11, String(db));
  for (const p of [-1, -0.5, 0, 0.3, 1]) assert.equal(posToValue("pan", valueToPos("pan", p)), p);
  assert.equal(valueToPos("volume_db", -96), 0);
  assert.equal(valueToPos("volume_db", 12), 1);
  assert.equal(valueToPos("volume_db", -48), 0.1);
  assert.equal(posToValue("volume_db", 2), 12, "clamped");
});

test("interpolation matches the engine (linear, held at the ends)", () => {
  const pts: AutoPoint[] = [
    { id: "b", owner_id: "t", param: "volume_db", sample: 48000, value: -20 },
    { id: "a", owner_id: "t", param: "volume_db", sample: 0, value: 0 },
  ];
  assert.equal(valueAt(pts, -5, 0), 0);
  assert.equal(valueAt(pts, 24000, 0), -10);
  assert.equal(valueAt(pts, 96000, 0), -20);
  assert.equal(valueAt([], 5, 0.25), 0.25);
});

test("formatting", () => {
  assert.equal(formatValue("volume_db", -6), "−6.0 dB");
  assert.equal(formatValue("volume_db", 3.25), "+3.3 dB");
  assert.equal(formatValue("volume_db", -96), "−∞ dB");
  assert.equal(formatValue("pan", 0), "C");
  assert.equal(formatValue("pan", -0.3), "L 30");
  assert.equal(formatValue("pan", 1), "R 100");
});

test("plugin parameter lanes: 0–1 values, shown as percent", async () => {
  const { isPluginParam, pluginParamId, neutral } = await import("../src/automation.ts");
  assert.ok(isPluginParam("plugin:12") && !isPluginParam("pan"));
  assert.equal(pluginParamId("plugin:12"), 12);
  assert.equal(valueToPos("plugin:3", 0.25), 0.25);
  assert.equal(valueToPos("plugin:3", 7), 1, "clamped");
  assert.equal(posToValue("plugin:3", 0.12345), 0.123);
  assert.equal(formatValue("plugin:3", 0.256), "26%");
  assert.equal(neutral("plugin:3", 0.7), 0.7, "an empty lane shows the parameter's current value");
});

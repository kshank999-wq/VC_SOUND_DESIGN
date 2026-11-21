import { test } from "node:test";
import assert from "node:assert/strict";
import { aafSummary } from "../src/aafReport.ts";

const base = { composition: "Reel 1 v3", start_tc: "10:00:00:00", tracks: ["a", "b"], stereo_tracks: 1, clips: 1, markers: 0, media_linked: 2, media_written: 1, missing_media: [], notes: [] };

test("summary counts with singulars and plurals", () => {
  const l = aafSummary(base);
  assert.equal(l[0], '"Reel 1 v3" at 10:00:00:00: 2 tracks (1 stereo), 1 clip, 0 markers');
  assert.equal(l[1], "Media: 2 linked in place, 1 written to Audio/Imported/AAF");
  assert.equal(l.length, 2);
});

test("missing media and notes are listed", () => {
  const l = aafSummary({ ...base, stereo_tracks: 0, missing_media: ["music.wav not found — 1 clip"], notes: ["markers outside every scene left out (2)"] });
  assert.ok(!l[0].includes("stereo"));
  assert.deepEqual(l.slice(2), ["", "Missing media (clips left out):", "  music.wav not found — 1 clip", "", "Notes:", "  markers outside every scene left out (2)"]);
});

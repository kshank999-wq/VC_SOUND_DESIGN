import { test } from "node:test";
import assert from "node:assert/strict";
import { aafSummary, otioSummary } from "../src/aafReport.ts";

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

test("OTIO summaries name the timeline and their media folder", () => {
  const { composition: _, ...rest } = base;
  const l = otioSummary({ ...rest, timeline: "Reel 2" });
  assert.ok(l[0].startsWith('"Reel 2" at 10:00:00:00'));
  assert.equal(l[1], "Media: 2 linked in place, 1 written to Audio/Imported/OTIO");
});

test("OTIO summaries count rebuilt inserts and plugin automation", () => {
  const { composition: _, ...rest } = base;
  const l = otioSummary({ ...rest, timeline: "Reel 2", inserts: 2, plugins: 1, automated_parameters: 3 });
  assert.equal(l[2], "Inserts: 2 (1 plugin, 3 automated parameters)");
  assert.equal(otioSummary({ ...rest, timeline: "R", inserts: 0 }).length, 2);
});

test("OTIO summaries report restored buses", () => {
  const { composition: _, ...rest } = base;
  const l = otioSummary({ ...rest, timeline: "R", bus_levels: "buses", buses_restored: 3, buses_created: 1 });
  assert.equal(l[2], "Buses: 3 restored with their automation (1 created)");
  assert.equal(otioSummary({ ...rest, timeline: "R", bus_levels: "fold", buses_restored: 0 }).length, 2);
});

test("AAF summaries count rebuilt inserts before restored buses", () => {
  const l = aafSummary({ ...base, inserts: 1, plugins: 1, automated_parameters: 1, bus_levels: "buses", buses_restored: 2 });
  assert.deepEqual(l.slice(2), ["Inserts: 1 (1 plugin, 1 automated parameter)", "Buses: 2 restored with their automation"]);
  const m = aafSummary({ ...base, bus_levels: "buses", buses_restored: 0, buses_matched: 3 });
  assert.equal(m[2], "Buses: 3 already matching, left as they were");
});

test("OTIO summaries say when left/right pairs were merged", () => {
  const { composition: _, ...rest } = base;
  assert.equal(otioSummary({ ...rest, timeline: "R", stereo_pairs: 2 })[2], "Stereo: 2 left/right pairs merged into one track each");
});

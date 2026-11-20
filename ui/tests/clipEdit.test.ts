import { test } from "node:test";
import assert from "node:assert/strict";
import { applyClipDrag, clipZone } from "../src/clipEdit.ts";

const clip = { timeline_start: 48000, source_offset: 0, length: 48000, fade_in: 2400, fade_out: 4800, fill: "one_shot" as const };
const srcLen = 72000; // 1.5 s of source audio

test("zones: fade handles in the top band, trim edges, body", () => {
  const h = { x: 100, y: 50, w: 200 }; // spp = 240 → clip 48000 samples = 200 px
  assert.equal(clipZone(h, clip, 110, 52, 240), "fadeIn"); // fade_in 2400/240 = 10 px
  assert.equal(clipZone(h, clip, 280, 52, 240), "fadeOut"); // 300 - 20
  assert.equal(clipZone(h, clip, 102, 80, 240), "trimL");
  assert.equal(clipZone(h, clip, 297, 80, 240), "trimR");
  assert.equal(clipZone(h, clip, 200, 80, 240), "move");
});

test("head trim keeps audio in place and cannot pass source start", () => {
  const e = applyClipDrag(clip, "trimL", 60000, 0, srcLen);
  assert.deepEqual([e.timeline_start, e.source_offset, e.length], [60000, 12000, 36000]);
  const back = applyClipDrag({ ...clip, timeline_start: 60000, source_offset: 12000, length: 36000 }, "trimL", 0, 0, srcLen);
  assert.deepEqual([back.timeline_start, back.source_offset], [48000, 0], "clamped at source start");
  const tiny = applyClipDrag(clip, "trimL", 95990, 0, srcLen);
  assert.ok(tiny.fade_in + tiny.fade_out <= tiny.length, "fades shrink to fit");
});

test("tail trim is limited by source for one-shots, free for loops", () => {
  assert.equal(applyClipDrag(clip, "trimR", 500000, 0, srcLen).length, 72000);
  assert.equal(applyClipDrag({ ...clip, fill: "loop" }, "trimR", 500000, 0, srcLen).length, 452000);
  assert.equal(applyClipDrag(clip, "trimR", 0, 0, srcLen).length, 1, "never zero length");
});

test("fades stay within the clip and do not overlap", () => {
  assert.equal(applyClipDrag(clip, "fadeIn", 48000 + 12000, 0, srcLen).fade_in, 12000);
  assert.equal(applyClipDrag(clip, "fadeIn", 200000, 0, srcLen).fade_in, 48000 - 4800);
  assert.equal(applyClipDrag(clip, "fadeIn", 0, 0, srcLen).fade_in, 0);
  assert.equal(applyClipDrag(clip, "fadeOut", 0, 0, srcLen).fade_out, 48000 - 2400);
  assert.equal(applyClipDrag(clip, "move", 0, 2000, srcLen).timeline_start, 50000);
});

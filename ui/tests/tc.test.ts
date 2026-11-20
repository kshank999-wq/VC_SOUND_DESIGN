// Run with: node --test tests/   (Node 22.18+ strips TypeScript types natively)
import { test } from "node:test";
import assert from "node:assert/strict";
import { framesToTc, tcToFrames, tcAt, sampleAtTc, sampleAtFrame, frameAtSample, type Rate } from "../src/tc.ts";

const r24: Rate = { num: 24, den: 1, df: false, sr: 48000, startFrame: 86400 };
const df: Rate = { num: 30000, den: 1001, df: true, sr: 48000, startFrame: 0 };
const r2398: Rate = { num: 24000, den: 1001, df: false, sr: 48000, startFrame: 0 };

test("matches engine drop-frame values", () => {
  assert.equal(framesToTc(df, 1799), "00:00:59;29");
  assert.equal(framesToTc(df, 1800), "00:01:00;02");
  assert.equal(framesToTc(df, 107892), "01:00:00;00");
  assert.equal(tcToFrames(df, "01:00:00;00"), 107892);
  assert.equal(tcToFrames(df, "00:01:00;00"), null);
  for (let f = 0; f < 200000; f += 37) assert.equal(tcToFrames(df, framesToTc(df, f)), f);
});

test("non-zero start TC and sample mapping", () => {
  assert.equal(sampleAtTc(r24, "01:00:00:00"), 0);
  assert.equal(sampleAtTc(r24, "01:00:01:00"), 48000);
  assert.equal(tcAt(r24, 48000 + 1999), "01:00:01:00");
  assert.equal(tcAt(r24, 48000 + 2000), "01:00:01:01");
  assert.equal(tcAt(r24, -1), "00:59:59:23");
  assert.equal(sampleAtFrame(r2398, 1), 2002);
  for (let f = -50; f < 5000; f++) {
    const s = sampleAtFrame(df, f);
    assert.equal(frameAtSample(df, s), f);
    assert.equal(frameAtSample(df, s - 1), f - 1);
  }
  const tenH = 10 * 3600 * 24;
  assert.equal(frameAtSample(r2398, sampleAtFrame(r2398, tenH)), tenH);
});

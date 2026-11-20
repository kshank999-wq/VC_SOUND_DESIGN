// Pure clip-edit geometry for timeline handles (unit-tested in tests/clipEdit.test.ts).
import type { Clip } from "./types";

export type ClipZone = "move" | "trimL" | "trimR" | "fadeIn" | "fadeOut";

export interface ClipEdit {
  timeline_start: number;
  source_offset: number;
  length: number;
  fade_in: number;
  fade_out: number;
}

type ClipGeo = Pick<Clip, "timeline_start" | "source_offset" | "length" | "fade_in" | "fade_out" | "fill">;

const EDGE = 7; // px grab zone for trims
const FADE_BAND = 12; // px band at the clip top where fade handles live

/** Which part of a clip a point falls on: fade handles, trim edges, or body. */
export function clipZone(h: { x: number; y: number; w: number }, c: { fade_in: number; fade_out: number }, x: number, y: number, spp: number): ClipZone {
  const fi = h.x + c.fade_in / spp;
  const fo = h.x + h.w - c.fade_out / spp;
  if (y - h.y < FADE_BAND && h.w > 14) {
    if (Math.abs(x - fi) <= 6) return "fadeIn";
    if (Math.abs(x - fo) <= 6) return "fadeOut";
  }
  const edge = Math.min(EDGE, h.w / 3);
  if (x - h.x <= edge) return "trimL";
  if (h.x + h.w - x <= edge) return "trimR";
  return "move";
}

/** New clip geometry for a trim/fade/move drag ending at sample `cur`. */
export function applyClipDrag(c: ClipGeo, zone: ClipZone, cur: number, delta: number, srcLen: number): ClipEdit {
  const e: ClipEdit = { timeline_start: c.timeline_start, source_offset: c.source_offset, length: c.length, fade_in: c.fade_in, fade_out: c.fade_out };
  const end = c.timeline_start + c.length;
  const clampFades = () => {
    e.fade_in = Math.max(0, Math.min(e.fade_in, e.length));
    e.fade_out = Math.max(0, Math.min(e.fade_out, e.length - e.fade_in));
  };
  switch (zone) {
    case "move":
      e.timeline_start += delta;
      break;
    case "trimL": {
      const ns = Math.max(c.timeline_start - c.source_offset, Math.min(cur, end - 1));
      e.source_offset += ns - c.timeline_start;
      e.timeline_start = ns;
      e.length = end - ns;
      clampFades();
      break;
    }
    case "trimR": {
      const maxEnd = c.fill === "loop" ? Infinity : c.timeline_start + Math.max(c.length, srcLen - c.source_offset);
      e.length = Math.max(c.timeline_start + 1, Math.min(cur, maxEnd)) - c.timeline_start;
      clampFades();
      break;
    }
    case "fadeIn":
      e.fade_in = Math.max(0, Math.min(cur - c.timeline_start, c.length - c.fade_out));
      break;
    case "fadeOut":
      e.fade_out = Math.max(0, Math.min(end - cur, c.length - c.fade_in));
      break;
  }
  return e;
}

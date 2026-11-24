// Edit tools (spec v2.1 › Toolbar › Tools): what a press-and-drag in the timeline means.
// Pure; unit-tested.
import type { ClipZone } from "./clipEdit.ts";

export type Tool = "select" | "trim" | "fade" | "split" | "pencil" | "hand";

export const TOOLS: { id: Tool; label: string; key: string; code: string; hint: string }[] = [
  { id: "select", label: "Select", key: "V", code: "KeyV", hint: "Move clips and markers; trim at edges, fade at the top corners" },
  { id: "trim", label: "Trim", key: "T", code: "KeyT", hint: "Drag a clip to trim its nearer edge" },
  { id: "fade", label: "Fade", key: "E", code: "KeyE", hint: "Drag a clip to set its nearer fade" },
  { id: "split", label: "Split", key: "X", code: "KeyX", hint: "Click a clip to split it there" },
  { id: "pencil", label: "Pencil", key: "D", code: "KeyD", hint: "Draw automation; click a marker lane to add a marker" },
  { id: "hand", label: "Hand", key: "H", code: "KeyH", hint: "Drag to scroll the timeline" },
];

export const toolForKey = (code: string): Tool | null => TOOLS.find((t) => t.code === code)?.id ?? null;

/** What pressing a clip does under each tool: a drag zone, a split, or nothing (the tool
 *  acts elsewhere). `natural` is the Select tool's zone under the pointer; `frac` is the
 *  pointer's position across the clip (0 = left edge, 1 = right edge). */
export function clipAction(tool: Tool, natural: ClipZone, frac: number): { zone: ClipZone } | { split: true } | null {
  switch (tool) {
    case "select":
      return { zone: natural };
    case "trim":
      return { zone: frac < 0.5 ? "trimL" : "trimR" };
    case "fade":
      return { zone: frac < 0.5 ? "fadeIn" : "fadeOut" };
    case "split":
      return { split: true };
    default:
      return null;
  }
}

/** Pencil: thin a stroke to one point per `minPx` pixels of travel (always keeping the ends). */
export function thinStroke(points: { x: number; sample: number; value: number }[], minPx = 6): [number, number][] {
  const out: { x: number; sample: number; value: number }[] = [];
  for (const p of points) if (!out.length || Math.abs(p.x - out[out.length - 1].x) >= minPx) out.push(p);
  const last = points[points.length - 1];
  if (last && out[out.length - 1] !== last) out.push(last);
  return out.map((p) => [Math.round(p.sample), p.value]);
}

/** The cursor each tool shows over the timeline. */
export const TOOL_CURSOR: Record<Tool, string> = {
  select: "default",
  trim: "col-resize",
  fade: "ew-resize",
  split: "crosshair",
  pencil: "crosshair",
  hand: "grab",
};

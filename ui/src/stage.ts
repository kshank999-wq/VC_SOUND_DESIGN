// The six workflow stages and each one's default window layout (spec v2.1 › Changes in v2,
// table 2). Stages only rearrange panels; they never change project data. Pure; unit-tested.

export type Stage = "prep" | "spot" | "build" | "perform" | "mix" | "deliver";
export type DeckTab = "pads" | "library" | "mixer" | "log";
export type LeftTab = "scenes" | "identities" | "library";
export type CenterTab = "timeline" | "chart" | "qa";
/** Marker lanes: full (26 px each, labelled), folded (10 px minimap strips), hidden (one row). */
export type LaneMode = "full" | "folded" | "hidden";

export const STAGES: { id: Stage; label: string }[] = [
  { id: "prep", label: "Prep" },
  { id: "spot", label: "Spot" },
  { id: "build", label: "Build" },
  { id: "perform", label: "Perform" },
  { id: "mix", label: "Mix" },
  { id: "deliver", label: "Deliver" },
];

export interface StageLayout {
  /** Picture height above the timeline (px); 0 hides it. */
  pictureH: number;
  deckTab: DeckTab;
  /** Bottom deck height when open (px). */
  deckH: number;
  deckOpen: boolean;
  leftTab: LeftTab;
  center: CenterTab;
  lanes: LaneMode;
}

export const STAGE_DEFAULTS: Record<Stage, StageLayout> = {
  prep: { pictureH: 300, deckTab: "library", deckH: 200, deckOpen: true, leftTab: "scenes", center: "timeline", lanes: "folded" },
  spot: { pictureH: 320, deckTab: "pads", deckH: 150, deckOpen: true, leftTab: "scenes", center: "timeline", lanes: "full" },
  build: { pictureH: 170, deckTab: "library", deckH: 200, deckOpen: true, leftTab: "library", center: "timeline", lanes: "folded" },
  perform: { pictureH: 300, deckTab: "pads", deckH: 170, deckOpen: true, leftTab: "identities", center: "timeline", lanes: "folded" },
  mix: { pictureH: 0, deckTab: "mixer", deckH: 250, deckOpen: true, leftTab: "scenes", center: "timeline", lanes: "hidden" },
  deliver: { pictureH: 0, deckTab: "log", deckH: 200, deckOpen: false, leftTab: "scenes", center: "qa", lanes: "hidden" },
};

export const STAGE_LIMITS = { pictureH: [0, 900], deckH: [90, 520] } as const;
/** The deck's tab bar alone (collapsed deck). */
export const DECK_BAR = 28;

const TABS = { deckTab: ["pads", "library", "mixer", "log"], leftTab: ["scenes", "identities", "library"], center: ["timeline", "chart", "qa"], lanes: ["full", "folded", "hidden"] } as const;

export const isStage = (v: unknown): v is Stage => typeof v === "string" && STAGES.some((s) => s.id === v);

/** Keep only valid, clamped values from a stored per-stage override. */
export function sanitizeStageLayout(raw: unknown): Partial<StageLayout> {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const out: Partial<StageLayout> = {};
  const n = (v: unknown, [lo, hi]: readonly [number, number]) => (typeof v === "number" && Number.isFinite(v) ? Math.round(Math.max(lo, Math.min(hi, v))) : undefined);
  const ph = n(r.pictureH, STAGE_LIMITS.pictureH);
  const dh = n(r.deckH, STAGE_LIMITS.deckH);
  if (ph !== undefined) out.pictureH = ph;
  if (dh !== undefined) out.deckH = dh;
  if (typeof r.deckOpen === "boolean") out.deckOpen = r.deckOpen;
  for (const k of ["deckTab", "leftTab", "center", "lanes"] as const) {
    const v = r[k];
    if (typeof v === "string" && (TABS[k] as readonly string[]).includes(v)) (out as Record<string, unknown>)[k] = v;
  }
  return out;
}

/** A stage's layout: its defaults with the user's saved changes on top. */
export function stageLayout(stage: Stage, saved?: Partial<StageLayout>): StageLayout {
  return { ...STAGE_DEFAULTS[stage], ...sanitizeStageLayout(saved) };
}

/** Keys 1–6 select a stage. */
export function stageForKey(code: string): Stage | null {
  const m = /^(?:Digit|Numpad)([1-6])$/.exec(code);
  return m ? STAGES[Number(m[1]) - 1].id : null;
}

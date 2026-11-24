// Persisted window layout: shape, defaults, limits and merging (pure; unit-tested).
import { isStage, sanitizeStageLayout, type Stage, type StageLayout } from "./stage.ts";
import { sanitizeRecent, sanitizeTheme, type RecentProject, type Theme } from "./theme.ts";

export interface ProjectView {
  vp?: { start: number; spp: number };
  scrollY?: number;
}

export interface Layout {
  leftW?: number;
  rightW?: number;
  upperPct?: number;
  leftTab?: "scenes" | "identities" | "library";
  lowerTab?: "timeline" | "chart" | "mixer" | "qa";
  snap?: boolean;
  mixerOpen?: boolean;
  /** Check for app updates at startup (desktop release builds). */
  autoUpdateCheck?: boolean;
  /** The workflow stage last used, and each stage's layout changes (picture and deck sizes, tabs). */
  stage?: Stage;
  stages?: Partial<Record<Stage, Partial<StageLayout>>>;
  /** Appearance (Preferences › Appearance). */
  theme?: Partial<Theme>;
  /** Recently opened projects, newest first (Welcome screen). */
  recent?: RecentProject[];
  projects?: Record<string, ProjectView>;
}

export const DEFAULTS = { leftW: 270, rightW: 300, upperPct: 42 } as const;
export const LIMITS = { leftW: [180, 520], rightW: [220, 560], upperPct: [15, 80] } as const;

const LEFT_TABS = ["scenes", "identities", "library"];
const LOWER_TABS = ["timeline", "chart", "mixer", "qa"];

const num = (v: unknown, lo: number, hi: number): number | undefined =>
  typeof v === "number" && Number.isFinite(v) ? Math.max(lo, Math.min(hi, v)) : undefined;

/** Accept whatever was stored, keep only valid values (clamped). */
export function sanitize(raw: unknown): Layout {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const out: Layout = {};
  const lw = num(r.leftW, ...LIMITS.leftW);
  const rw = num(r.rightW, ...LIMITS.rightW);
  const up = num(r.upperPct, ...LIMITS.upperPct);
  if (lw !== undefined) out.leftW = lw;
  if (rw !== undefined) out.rightW = rw;
  if (up !== undefined) out.upperPct = up;
  if (typeof r.leftTab === "string" && LEFT_TABS.includes(r.leftTab)) out.leftTab = r.leftTab as Layout["leftTab"];
  if (typeof r.lowerTab === "string" && LOWER_TABS.includes(r.lowerTab)) out.lowerTab = r.lowerTab as Layout["lowerTab"];
  if (typeof r.snap === "boolean") out.snap = r.snap;
  if (typeof r.mixerOpen === "boolean") out.mixerOpen = r.mixerOpen;
  if (typeof r.autoUpdateCheck === "boolean") out.autoUpdateCheck = r.autoUpdateCheck;
  if (isStage(r.stage)) out.stage = r.stage;
  if (r.theme !== undefined) out.theme = sanitizeTheme(r.theme);
  if (r.recent !== undefined) out.recent = sanitizeRecent(r.recent);
  if (r.stages && typeof r.stages === "object") {
    out.stages = {};
    for (const [k, v] of Object.entries(r.stages as Record<string, unknown>)) if (isStage(k)) out.stages[k] = sanitizeStageLayout(v);
  }
  if (r.projects && typeof r.projects === "object") {
    out.projects = {};
    for (const [k, v] of Object.entries(r.projects as Record<string, Record<string, unknown>>)) {
      const pv: ProjectView = {};
      const vp = v?.vp as Record<string, unknown> | undefined;
      if (vp && Number.isFinite(vp.start) && Number.isFinite(vp.spp) && (vp.spp as number) > 0) pv.vp = { start: vp.start as number, spp: vp.spp as number };
      if (Number.isFinite(v?.scrollY)) pv.scrollY = Math.max(0, v.scrollY as number);
      out.projects[k] = pv;
    }
  }
  return out;
}

/** Deep merge (objects merge, other values replace) — mirrors the engine. */
export function merge<T extends object>(base: T, patch: Partial<T>): T {
  const out: Record<string, unknown> = { ...(base as Record<string, unknown>) };
  for (const [k, v] of Object.entries(patch as Record<string, unknown>)) {
    const cur = out[k];
    out[k] = v && typeof v === "object" && !Array.isArray(v) && cur && typeof cur === "object" ? merge(cur as object, v as object) : v;
  }
  return out as T;
}

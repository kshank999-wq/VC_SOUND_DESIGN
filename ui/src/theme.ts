// Appearance preferences (spec v2.1 › Design tokens › themeable): accent, readout color,
// density, reduced transparency. Applied as attributes / one property on <html>, where the
// tokens' derived colors are declared, so everything follows. Pure apart from applyTheme.

export const ACCENTS = [
  { id: "blue", label: "Blue", hex: "#5B8CFF" },
  { id: "indigo", label: "Indigo", hex: "#6366F1" },
  { id: "magenta", label: "Magenta", hex: "#C026D3" },
] as const;
export const READOUTS = [
  { id: "mint", label: "Mint", hex: "#7CF5D8" },
  { id: "green", label: "Green", hex: "#9BF07A" },
  { id: "amber", label: "Amber", hex: "#FFC266" },
  { id: "ice", label: "Ice", hex: "#C7D2FE" },
] as const;

export type Accent = (typeof ACCENTS)[number]["id"];
export type Readout = (typeof READOUTS)[number]["id"];

export interface Theme {
  accent: Accent;
  readout: Readout;
  density: "comfortable" | "compact";
  reduceTransparency: boolean;
}

export const DEFAULT_THEME: Theme = { accent: "blue", readout: "mint", density: "comfortable", reduceTransparency: false };

export function sanitizeTheme(raw: unknown): Partial<Theme> {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const out: Partial<Theme> = {};
  if (ACCENTS.some((a) => a.id === r.accent)) out.accent = r.accent as Accent;
  if (READOUTS.some((a) => a.id === r.readout)) out.readout = r.readout as Readout;
  if (r.density === "comfortable" || r.density === "compact") out.density = r.density;
  if (typeof r.reduceTransparency === "boolean") out.reduceTransparency = r.reduceTransparency;
  return out;
}

export const theme = (saved?: Partial<Theme>): Theme => ({ ...DEFAULT_THEME, ...sanitizeTheme(saved) });

/** Put a theme on the document root. */
export function applyTheme(t: Theme, root: HTMLElement = document.documentElement) {
  root.dataset.accent = t.accent;
  root.style.setProperty("--readout", READOUTS.find((r) => r.id === t.readout)!.hex);
  if (t.density === "compact") root.dataset.density = "compact";
  else delete root.dataset.density;
  if (t.reduceTransparency) root.setAttribute("data-reduce-transparency", "");
  else root.removeAttribute("data-reduce-transparency");
}

export interface RecentProject {
  path: string;
  name: string;
  /** Last opened, ms since the epoch. */
  at: number;
}

/** The recent-projects list with `p` first (deduplicated by path, newest first, at most `max`). */
export function pushRecent(list: RecentProject[] | undefined, p: RecentProject, max = 8): RecentProject[] {
  return [p, ...(list ?? []).filter((x) => x.path !== p.path)].slice(0, max);
}

export function sanitizeRecent(raw: unknown): RecentProject[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((x): x is RecentProject => !!x && typeof x.path === "string" && typeof x.name === "string" && Number.isFinite(x.at))
    .slice(0, 8);
}

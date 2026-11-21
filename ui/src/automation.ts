// Automation lane geometry and formatting (unit-tested in tests/automation.test.ts).
// Interpolation matches vcsd-core's mixer: linear between breakpoints, held
// flat before the first and after the last.

/** Volume, pan, or a plugin parameter (`plugin:<id>`, owner = the plugin slot, values 0–1). */
export type Param = "volume_db" | "pan" | `plugin:${number}`;

export const isPluginParam = (p: string): p is `plugin:${number}` => p.startsWith("plugin:");
export const pluginParamId = (p: string): number => Number(p.slice("plugin:".length));

export interface AutoPoint {
  id: string;
  /** Track or bus id, or a plugin slot id for plugin parameters. */
  owner_id: string;
  param: Param;
  sample: number;
  value: number;
}

export const SILENCE_DB = -96;

/** Value → vertical position 0 (bottom) .. 1 (top). Volume uses a fader-like
 *  law: -48..+12 dB fills the top 90%, -96..-48 is squeezed into the bottom 10%. */
export function valueToPos(param: Param, v: number): number {
  if (isPluginParam(param)) return Math.max(0, Math.min(1, v));
  if (param === "pan") return (Math.max(-1, Math.min(1, v)) + 1) / 2;
  const db = Math.max(SILENCE_DB, Math.min(12, v));
  return db >= -48 ? 0.1 + (0.9 * (db + 48)) / 60 : (0.1 * (db - SILENCE_DB)) / 48;
}

export function posToValue(param: Param, pos: number): number {
  const p = Math.max(0, Math.min(1, pos));
  if (isPluginParam(param)) return Math.round(p * 1000) / 1000;
  if (param === "pan") return Math.round((p * 2 - 1) * 100) / 100;
  const db = p >= 0.1 ? -48 + ((p - 0.1) / 0.9) * 60 : SILENCE_DB + (p / 0.1) * 48;
  return Math.round(db * 10) / 10;
}

/** Interpolated value at sample `s` for points of one lane. */
export function valueAt(points: AutoPoint[], s: number, fallback: number): number {
  if (!points.length) return fallback;
  const p = [...points].sort((a, b) => a.sample - b.sample);
  if (s <= p[0].sample) return p[0].value;
  const last = p[p.length - 1];
  if (s >= last.sample) return last.value;
  for (let i = 0; i + 1 < p.length; i++) {
    if (s >= p[i].sample && s < p[i + 1].sample) {
      const t = (s - p[i].sample) / Math.max(1, p[i + 1].sample - p[i].sample);
      return p[i].value + (p[i + 1].value - p[i].value) * t;
    }
  }
  return last.value;
}

export function formatValue(param: Param, v: number): string {
  if (isPluginParam(param)) return `${Math.round(Math.max(0, Math.min(1, v)) * 100)}%`;
  if (param === "pan") {
    const n = Math.round(Math.abs(v) * 100);
    return n === 0 ? "C" : `${v < 0 ? "L" : "R"} ${n}`;
  }
  if (v <= SILENCE_DB) return "−∞ dB";
  return `${v > 0 ? "+" : v < 0 ? "−" : ""}${Math.abs(v).toFixed(1)} dB`;
}

/** Value shown when a lane has no breakpoints: the track's pan, 0 dB, or the
 *  plugin parameter's current value (passed as `current`). */
export function neutral(param: Param, current: number): number {
  return param === "pan" || isPluginParam(param) ? current : 0;
}

// ---- plugin lanes the user chose to show (persisted per machine)

export const AUTO_OPEN_KEY = "vcsd.automationLanes";
const PLUGIN_LANES_KEY = "vcsd.pluginLanes";
export const LANES_EVENT = "vcsd-lanes";

function readList(key: string): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}
function writeList(key: string, v: string[]) {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {
    /* storage unavailable */
  }
}

/** Shown plugin lanes as `slot|param` keys. */
export function shownPluginLanes(): Set<string> {
  return new Set(readList(PLUGIN_LANES_KEY));
}

/** Show a plugin parameter's lane under its track/bus (and open that owner's lanes). */
export function showPluginLane(ownerId: string, slot: string, param: Param) {
  writeList(PLUGIN_LANES_KEY, [...new Set([...readList(PLUGIN_LANES_KEY), `${slot}|${param}`])]);
  writeList(AUTO_OPEN_KEY, [...new Set([...readList(AUTO_OPEN_KEY), ownerId])]);
  if (typeof window !== "undefined") window.dispatchEvent(new Event(LANES_EVENT));
}

export function hidePluginLane(slot: string, param: Param) {
  writeList(PLUGIN_LANES_KEY, readList(PLUGIN_LANES_KEY).filter((k) => k !== `${slot}|${param}`));
  if (typeof window !== "undefined") window.dispatchEvent(new Event(LANES_EVENT));
}

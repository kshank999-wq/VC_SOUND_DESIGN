// Timecode math mirrored from vcsd-core/src/timecode.rs (rational, DF/NDF).
// Integer-exact: BigInt is used where products can exceed 2^53.

export interface Rate {
  num: number;
  den: number;
  df: boolean;
  sr: number;
  startFrame: number;
}

export function nominal(r: Rate): number {
  return Math.floor((r.num + Math.floor(r.den / 2)) / r.den);
}

export function fps(r: Rate): number {
  return r.num / r.den;
}

/** Frame (relative to timeline start) containing sample s. */
export function frameAtSample(r: Rate, s: number): number {
  const n = BigInt(Math.round(s)) * BigInt(r.num);
  const d = BigInt(r.sr) * BigInt(r.den);
  let q = n / d;
  if (n % d !== 0n && n < 0n) q -= 1n;
  return Number(q);
}

/** First sample of relative frame f (ceiling division, as in the engine). */
export function sampleAtFrame(r: Rate, f: number): number {
  const n = BigInt(Math.round(f)) * BigInt(r.sr) * BigInt(r.den);
  const d = BigInt(r.num);
  let q = n / d;
  if (n % d !== 0n && n > 0n) q += 1n;
  return Number(q);
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

export function framesToTc(r: Rate, frames: number): string {
  const neg = frames < 0;
  let f = Math.abs(frames);
  const n = nominal(r);
  if (r.df) {
    const d = Math.floor(n / 15);
    const fp10 = n * 600 - d * 9;
    const fpm = n * 60 - d;
    const tens = Math.floor(f / fp10);
    const m = f % fp10;
    f += m > d ? d * 9 * tens + d * Math.floor((m - d) / fpm) : d * 9 * tens;
  }
  const ff = f % n;
  const ss = Math.floor(f / n) % 60;
  const mm = Math.floor(f / (n * 60)) % 60;
  const hh = Math.floor(f / (n * 3600));
  return `${neg ? "-" : ""}${pad(hh)}:${pad(mm)}:${pad(ss)}${r.df ? ";" : ":"}${pad(ff)}`;
}

export function tcToFrames(r: Rate, tc: string): number | null {
  const t = tc.trim();
  const neg = t.startsWith("-");
  const parts = (neg ? t.slice(1) : t).split(/[:;.]/);
  if (parts.length !== 4 || parts.some((p) => !/^\d+$/.test(p))) return null;
  const [h, m, s, f] = parts.map(Number);
  const n = nominal(r);
  if (m >= 60 || s >= 60 || f >= n) return null;
  let frames = (h * 3600 + m * 60 + s) * n + f;
  if (r.df) {
    const d = Math.floor(n / 15);
    if (s === 0 && m % 10 !== 0 && f < d) return null;
    const tm = h * 60 + m;
    frames -= d * (tm - Math.floor(tm / 10));
  }
  return neg ? -frames : frames;
}

/** Absolute timecode label at timeline sample s. */
export function tcAt(r: Rate, s: number): string {
  return framesToTc(r, r.startFrame + frameAtSample(r, s));
}

/** Timeline sample of a timecode label (null if invalid). */
export function sampleAtTc(r: Rate, tc: string): number | null {
  const f = tcToFrames(r, tc);
  return f === null ? null : sampleAtFrame(r, f - r.startFrame);
}

/** Snap a sample to the first sample of its frame. */
export function snap(r: Rate, s: number): number {
  return sampleAtFrame(r, frameAtSample(r, s));
}

export function oneFrame(r: Rate): number {
  return sampleAtFrame(r, 1);
}

export function secs(r: Rate, samples: number): string {
  return (samples / r.sr).toFixed(2) + "s";
}

import { useEffect, useRef, useState } from "react";
import type { LaneMode } from "../stage";
import { Knob, fmtGain, fmtPan } from "./Knob";
import { SegMeter } from "./Shell";
import { call } from "../api";
import type { EngineHook } from "../store";
import { catColor, laneColor, catKey, type Bus, type Clip, type Marker, type Selection, type Track } from "../types";
import { frameAtSample, sampleAtFrame, snap, tcAt, type Rate } from "../tc";
import { applyClipDrag, clipZone, type ClipEdit, type ClipZone } from "../clipEdit";
import { AUTO_OPEN_KEY, formatValue, hidePluginLane, isPluginParam, LANES_EVENT, neutral, pluginParamId, posToValue, showPluginLane, shownPluginLanes, valueAt, valueToPos, type AutoPoint, type Param } from "../automation";
import { paramsOf, paramTitle, useParamsVersion } from "../pluginParams";
import type { PluginSlot } from "../types";
import { ModeButton, type AutoMode } from "./AutoControl";
import { folderTree, parentOf } from "../folders";

export interface Viewport {
  start: number; // first visible sample
  spp: number; // samples per pixel
}

const RULER = 26;
const SCENES = 20;
/** Marker lane height per mode (spec v2.1: full 26, folded 10-px minimap strips); hidden
 *  leaves one summary row. */
const LANE_H = { full: 26, folded: 10, hidden: 0 } as const;
const HIDDEN_ROW = 18;
const TRACK = 56; // spec v2.1: track lane default 56 px
const HEAD_W = 252;

type Hit = { x: number; y: number; w: number; h: number; kind: "clip" | "marker" | "auto"; id: string };

const AUTO = 56; // automation lane height
const PARAMS: Param[] = ["volume_db", "pan"];
const PARAM_LABEL: Record<string, string> = { volume_db: "Volume", pan: "Pan" };
const ADD_H = 24; // "+ plugin lane" row

const BUS_H = 30; // bus header row height
/** Anything automation can be attached to: a track or a bus (incl. master). */
interface Owner { id: string; name: string; color: string; pan: number }
const FOLDER_H = 24; // folder header row height
const INDENT = 12; // px per folder level
type Row =
  | { kind: "folder"; path: string; name: string; depth: number; collapsed: boolean; trackIds: string[]; y: number; h: number }
  | { kind: "track"; t: Track; depth: number; y: number; h: number }
  | { kind: "bus"; b: Bus; first: boolean; y: number; h: number }
  /** `lane`: whose points these are (the owner, or a plugin slot for plugin parameters). */
  | { kind: "auto"; owner: Owner; lane: string; param: Param; label: string; plugin?: string; y: number; h: number }
  | { kind: "autoAdd"; owner: Owner; slots: PluginSlot[]; y: number; h: number };

/** What decides which plugin lanes appear. */
interface PluginLanes {
  automation: AutoPoint[];
  shown: Set<string>;
  title: (slot: string, id: number) => string;
  current: (slot: string, id: number) => number;
}

/** Track rows, each followed by its automation lanes when open (y relative to the first track). */
function buildRows(tracks: Track[], buses: Bus[], open: Set<string>, collapsed: Set<string>, pl: PluginLanes): Row[] {
  const rows: Row[] = [];
  let y = 0;
  const lanes = (owner: Owner, inserts: Track["inserts"]) => {
    if (!open.has(owner.id)) return;
    for (const param of PARAMS) {
      const label = param === "pan" && owner.id.startsWith("bus_") ? "Balance" : PARAM_LABEL[param];
      rows.push({ kind: "auto", owner, lane: owner.id, param, label, y, h: AUTO });
      y += AUTO;
    }
    // plugin parameters: those with automation, and those chosen with "+ Plugin lane"
    const slots = inserts.filter((i): i is PluginSlot => i.type === "plugin");
    for (const sl of slots) {
      const ids = new Set<number>();
      for (const p of pl.automation) if (p.owner_id === sl.id && isPluginParam(p.param)) ids.add(pluginParamId(p.param));
      for (const k of pl.shown) {
        const [slot, param] = k.split("|");
        if (slot === sl.id && isPluginParam(param)) ids.add(pluginParamId(param));
      }
      for (const id of [...ids].sort((a, b) => a - b)) {
        const param = `plugin:${id}` as Param;
        rows.push({ kind: "auto", owner: { ...owner, pan: pl.current(sl.id, id) }, lane: sl.id, param, label: `${sl.name} · ${pl.title(sl.id, id)}`, plugin: sl.name, y, h: AUTO });
        y += AUTO;
      }
    }
    if (slots.length) {
      rows.push({ kind: "autoAdd", owner, slots, y, h: ADD_H });
      y += ADD_H;
    }
  };
  for (const item of folderTree(tracks, collapsed)) {
    if (item.kind === "folder") {
      rows.push({ ...item, y, h: FOLDER_H });
      y += FOLDER_H;
      continue;
    }
    const t = item.track;
    rows.push({ kind: "track", t, depth: item.depth, y, h: TRACK });
    y += TRACK;
    lanes({ id: t.id, name: t.name, color: catColor(t.category), pan: t.pan }, t.inserts);
  }
  // buses: stem buses in mixer order, master last
  const ordered = [...buses.filter((b) => b.parent_id), ...buses.filter((b) => !b.parent_id)];
  ordered.forEach((b, i) => {
    rows.push({ kind: "bus", b, first: i === 0, y, h: BUS_H });
    y += BUS_H;
    lanes({ id: b.id, name: b.name, color: b.parent_id ? "#9AA4B5" : "#F4F7FB", pan: 0 }, b.inserts ?? []);
  });
  return rows;
}

const AUTO_KEY = AUTO_OPEN_KEY;
const FOLDER_KEY = "vcsd.collapsedFolders";
function loadSet(key: string): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(key) ?? "[]"));
  } catch {
    return new Set();
  }
}
function saveSet(key: string, v: Set<string>) {
  try {
    localStorage.setItem(key, JSON.stringify([...v]));
  } catch {
    /* storage unavailable */
  }
}
function loadOpen(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(AUTO_KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}

/** Canvas y of a value inside an automation row whose top is `top`. */
function autoY(param: Param, v: number, top: number) {
  return top + 4 + (1 - valueToPos(param, v)) * (AUTO - 9);
}
function autoVal(param: Param, y: number, top: number) {
  return posToValue(param, 1 - (y - top - 4) / (AUTO - 9));
}

const peakCache = new Map<string, { per: number; data: number[] } | "loading" | "error">();

function getPeaks(id: string) {
  const p = peakCache.get(id);
  if (p === undefined) {
    peakCache.set(id, "loading");
    call<{ per: number; data: number[] }>("asset.peaks", { id })
      .then((v) => peakCache.set(id, v))
      .catch(() => peakCache.set(id, "error"));
    return null;
  }
  return typeof p === "string" ? null : p;
}

interface Props {
  eng: EngineHook;
  vp: Viewport;
  setVp: (v: Viewport) => void;
  sel: Selection;
  setSel: (s: Selection) => void;
  snapOn: boolean;
  /** Marker lanes: full (Spot), folded strips, or hidden behind one row (Mix). */
  laneMode?: LaneMode;
  onLaneMode?: (m: LaneMode) => void;
  /** Restored vertical scroll for this project, and a callback to persist it. */
  initialScrollY?: number;
  onScrollY?: (y: number) => void;
}

interface Drag {
  kind: "clip" | "marker" | "ruler" | "loop" | "newRange" | "auto";
  autoPreview?: { sample: number; value: number };
  autoTop?: number;
  zone?: ClipZone;
  id?: string;
  x0: number;
  s0: number;
  laneId?: string;
  moved: boolean;
  delta: number;
  cur: number;
  preview?: ClipEdit;
}

export function Timeline({ eng, vp, setVp, sel, setSel, snapOn, laneMode = "full", onLaneMode, initialScrollY = 0, onScrollY }: Props) {
  const LANE = LANE_H[laneMode];
  const { view, tx, rate, run, livePosition } = eng;
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [scrollY, setScrollY] = useState(initialScrollY);
  useEffect(() => {
    onScrollY?.(scrollY);
  }, [scrollY]); // eslint-disable-line react-hooks/exhaustive-deps
  const [size, setSize] = useState({ w: 800, h: 300 });
  const hits = useRef<Hit[]>([]);
  const drag = useRef<Drag | null>(null);
  const hover = useRef<{ id: string; zone: ClipZone } | null>(null);
  const [autoOpen, setAutoOpen] = useState<Set<string>>(loadOpen);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadSet(FOLDER_KEY));
  const toggleFolder = (path: string) =>
    setCollapsed((o) => {
      const n = new Set(o);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      saveSet(FOLDER_KEY, n);
      return n;
    });
  const toggleAuto = (id: string) =>
    setAutoOpen((o) => {
      const n = new Set(o);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      try {
        localStorage.setItem(AUTO_KEY, JSON.stringify([...n]));
      } catch {
        /* storage unavailable */
      }
      return n;
    });
  const [, force] = useState(0);
  const lanes = view?.lanes ?? [];
  const tracks = view?.tracks ?? [];
  // plugin lanes: re-read when one is shown/hidden elsewhere (plugin panel)
  const [shownLanes, setShownLanes] = useState<Set<string>>(shownPluginLanes);
  useEffect(() => {
    const on = () => {
      setShownLanes(shownPluginLanes());
      setAutoOpen(loadOpen());
    };
    window.addEventListener(LANES_EVENT, on);
    return () => window.removeEventListener(LANES_EVENT, on);
  }, []);
  useParamsVersion(); // lane names arrive asynchronously
  const automation = (view?.automation ?? []) as AutoPoint[];
  const rows = buildRows(tracks, view?.buses ?? [], autoOpen, collapsed, {
    automation,
    shown: shownLanes,
    title: paramTitle,
    current: (slot, id) => paramsOf(slot)?.find((p) => p.id === id)?.value ?? 0.5,
  });
  const rowsH = rows.reduce((a, r) => a + r.h, 0);
  const lanesH = laneMode === "hidden" ? HIDDEN_ROW : lanes.length * LANE;
  const contentH = RULER + SCENES + lanesH + rowsH + 40;
  const stateRef = useRef({ view, tx, vp, sel, scrollY, size, rate, livePosition, rows });
  stateRef.current = { view, tx, vp, sel, scrollY, size, rate, livePosition, rows };

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth - HEAD_W, h: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // follow playhead while playing
  useEffect(() => {
    if (!tx?.playing) return;
    const id = setInterval(() => {
      const s = stateRef.current;
      const p = s.livePosition();
      const w = s.size.w * s.vp.spp;
      if (p > s.vp.start + w * 0.92 || p < s.vp.start) setVp({ ...s.vp, start: p - w * 0.08 });
    }, 200);
    return () => clearInterval(id);
  }, [tx?.playing, setVp]);

  const sOf = (x: number) => vp.start + x * vp.spp;
  const snapS = (s: number) => (snapOn && rate ? snap(rate, Math.round(s)) : Math.round(s));

  // ------------------------------------------------------------------ draw
  useEffect(() => {
    let raf = 0;
    const draw = () => {
      raf = requestAnimationFrame(draw);
      const cv = canvasRef.current;
      const st = stateRef.current;
      if (!cv || !st.view?.open || !st.rate) return;
      const { view, vp, sel, scrollY, size, rate } = st;
      const dpr = window.devicePixelRatio || 1;
      const W = Math.max(50, size.w);
      const H = Math.max(50, size.h);
      if (cv.width !== Math.round(W * dpr) || cv.height !== Math.round(H * dpr)) {
        cv.width = Math.round(W * dpr);
        cv.height = Math.round(H * dpr);
        cv.style.width = W + "px";
        cv.style.height = H + "px";
      }
      const ctx = cv.getContext("2d")!;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const css = getComputedStyle(document.documentElement);
      const C = (n: string) => css.getPropertyValue(n).trim() || "#888";
      ctx.fillStyle = C("--tl-bg");
      ctx.fillRect(0, 0, W, H);
      const X = (s: number) => (s - vp.start) / vp.spp;
      const end = vp.start + W * vp.spp;
      const newHits: Hit[] = [];
      const d = drag.current;

      // ---- content (scrolled)
      ctx.save();
      ctx.translate(0, -scrollY);
      let y = RULER + SCENES;
      // scene shading
      for (const sc of view.scenes) {
        if (sc.end_sample < vp.start || sc.start_sample > end) continue;
        ctx.fillStyle = C("--scene-shade");
        const i = view.scenes.indexOf(sc);
        if (i % 2 === 0) ctx.fillRect(X(sc.start_sample), y, X(sc.end_sample) - X(sc.start_sample), contentH);
      }
      // loop range
      const lr = st.tx?.loop_range;
      if (lr) {
        ctx.fillStyle = C("--accent");
        ctx.globalAlpha = 0.07;
        ctx.fillRect(X(lr[0]), 0, X(lr[1]) - X(lr[0]), contentH + RULER);
        ctx.globalAlpha = 1;
      }
      // marker lanes
      const laneY = new Map<string, number>();
      for (const l of view.lanes) {
        laneY.set(l.id, y);
        if (LANE) {
          ctx.fillStyle = C("--lane-bg");
          ctx.fillRect(0, y, W, LANE - 1);
        }
        y += LANE;
      }
      if (laneMode === "hidden") {
        ctx.fillStyle = C("--lane-bg");
        ctx.fillRect(0, y, W, HIDDEN_ROW - 1);
        y += HIDDEN_ROW;
      }
      const small = LANE < 16;
      const oneF = sampleAtFrame(rate, 1);
      const drawMarker = (m: Marker, dx: number, ghost: boolean) => {
        const ly = laneY.get(m.lane_id);
        if (ly === undefined || !LANE) return;
        const lane = view.lanes.find((l) => l.id === m.lane_id)!;
        const a = m.start_sample + dx;
        const b = Math.max(m.end_sample + dx, a + oneF);
        if (b < vp.start || a > end) return;
        const x0 = X(a);
        const w = Math.max(3, X(b) - x0);
        const selected = sel?.kind === "marker" && sel.id === m.id || (sel?.kind === "cue" && m.cue_id === sel.id);
        ctx.globalAlpha = ghost ? 0.5 : m.cue_id ? 1 : 0.75;
        ctx.fillStyle = m.color || laneColor(lane);
        if (small) {
          // folded: a minimap strip, no labels
          ctx.fillRect(m.kind === "point" ? x0 - 1 : x0, ly + 1, m.kind === "point" ? 2 : w, LANE - 2);
        } else if (m.kind === "point") {
          ctx.fillRect(x0 - 1, ly + 2, 3, LANE - 5);
          ctx.beginPath();
          ctx.moveTo(x0 - 5, ly + 2);
          ctx.lineTo(x0 + 6, ly + 2);
          ctx.lineTo(x0 + 0.5, ly + 9);
          ctx.fill();
        } else {
          ctx.fillRect(x0, ly + 3, w, LANE - 7);
        }
        ctx.fillStyle = "#05070A";
        if (!small) for (const e of m.sub_events) {
          const ex = X(e + dx);
          ctx.fillRect(ex, ly + LANE - 9, 1.5, 5);
        }
        if (selected) {
          ctx.strokeStyle = C("--sel");
          ctx.lineWidth = 2;
          ctx.strokeRect(x0 - 2, ly + 1, (m.kind === "point" ? 4 : w) + 4, LANE - 3);
        }
        ctx.globalAlpha = 1;
        if (small) {
          if (!ghost) newHits.push({ x: x0 - 3, y: ly - scrollY, w: (m.kind === "point" ? 6 : w) + 3, h: LANE, kind: "marker", id: m.id });
          return;
        }
        ctx.fillStyle = C("--tl-text");
        ctx.font = "11px var(--font)";
        const cue = m.cue_id ? view.cues.find((c) => c.id === m.cue_id) : null;
        const label = `${cue ? cue.code + " " : ""}${m.label}${m.cue_id ? "" : " (marker)"}`;
        ctx.save();
        ctx.beginPath();
        ctx.rect(x0 + (m.kind === "point" ? 8 : 3), ly, m.kind === "point" ? 240 : Math.max(0, w - 5), LANE);
        ctx.clip();
        ctx.fillStyle = m.kind === "point" ? C("--tl-text") : "#05070A";
        ctx.fillText(label, x0 + (m.kind === "point" ? 8 : 4), ly + 15);
        ctx.restore();
        if (!ghost) newHits.push({ x: x0 - 4, y: ly - scrollY, w: (m.kind === "point" ? 8 : w) + 4, h: LANE, kind: "marker", id: m.id });
      };
      for (const m of view.markers) drawMarker(m, d?.kind === "marker" && d.id === m.id ? d.delta : 0, false);
      // AI proposals (ghosts, dashed)
      for (const p of view.proposals.filter((p) => p.status === "pending")) {
        const cue = view.cues.find((c) => c.id === p.cue_id);
        const ly = cue?.lane_id ? laneY.get(cue.lane_id) : undefined;
        if (ly === undefined || !LANE) continue;
        ctx.setLineDash([4, 3]);
        ctx.strokeStyle = C("--ai");
        const a = X(p.start_sample);
        const w = Math.max(4, X(Math.max(p.end_sample, p.start_sample + oneF)) - a);
        ctx.strokeRect(a, ly + 2, w, LANE - 5);
        ctx.setLineDash([]);
        ctx.fillStyle = C("--ai");
        if (!small) {
          for (const e of p.sub_events) ctx.fillRect(X(e), ly + 3, 1.5, LANE - 7);
          ctx.font = "10px var(--font)";
          ctx.fillText(`AI ${(p.confidence * 100).toFixed(0)}%`, a + 3, ly - 1 + LANE);
        }
      }
      // new range being drawn
      if (d?.kind === "newRange" && d.laneId && laneY.has(d.laneId)) {
        ctx.fillStyle = "rgba(255,255,255,0.25)";
        const a = Math.min(d.s0, d.cur);
        ctx.fillRect(X(a), laneY.get(d.laneId)! + 3, Math.abs(X(d.cur) - X(d.s0)), LANE - 7);
      }
      // tracks
      ctx.fillStyle = C("--sep");
      ctx.fillRect(0, y - 1, W, 2);
      const trackY = new Map<string, number>();
      const tracksTop = y;
      for (const r of st.rows) {
        if (r.kind !== "track") continue;
        trackY.set(r.t.id, tracksTop + r.y);
        ctx.fillStyle = sel?.kind === "track" && sel.id === r.t.id ? C("--track-sel") : C("--track-bg");
        ctx.fillRect(0, tracksTop + r.y, W, TRACK - 1);
      }
      for (const r of st.rows) {
        if (r.kind !== "folder") continue;
        const top = tracksTop + r.y;
        ctx.fillStyle = sel?.kind === "folder" && sel.id === r.path ? C("--track-sel") : C("--folder-bg");
        ctx.fillRect(0, top, W, FOLDER_H - 1);
        if (r.collapsed) {
          // summary of everything inside: one thin bar per clip, in track colours
          const ids = new Set(r.trackIds);
          ctx.globalAlpha = 0.75;
          for (const c of view.clips) {
            if (!ids.has(c.track_id) || c.muted) continue;
            const a = X(c.timeline_start);
            const z = X(c.timeline_start + c.length);
            if (z < 0 || a > W) continue;
            const tr = view.tracks.find((t) => t.id === c.track_id)!;
            ctx.fillStyle = catColor(tr.category);
            ctx.fillRect(a, top + 7, Math.max(1.5, z - a), FOLDER_H - 14);
          }
          ctx.globalAlpha = 1;
        }
      }
      for (const r of st.rows) {
        if (r.kind !== "bus") continue;
        const top = tracksTop + r.y;
        if (r.first) {
          ctx.fillStyle = C("--sep");
          ctx.fillRect(0, top - 1, W, 2);
        }
        ctx.fillStyle = r.b.parent_id ? C("--bus-bg") : C("--master-bg");
        ctx.fillRect(0, top, W, BUS_H - 1);
      }
      const clipsByTrack = view.clips;
      for (const c of clipsByTrack) {
        const ty = trackY.get(c.track_id);
        if (ty === undefined) continue;
        const pv = d?.kind === "clip" && d.id === c.id && d.preview ? d.preview : null;
        const geo = pv ?? c;
        const a = geo.timeline_start;
        const b = a + geo.length;
        if (b < vp.start || a > end) continue;
        const track = view.tracks.find((t) => t.id === c.track_id) as Track;
        const x0 = X(a);
        const w = Math.max(2, X(b) - x0);
        const col = catColor(track.category);
        const selected = (sel?.kind === "clip" && sel.id === c.id) || (sel?.kind === "cue" && c.cue_id === sel.id);
        ctx.globalAlpha = c.muted ? 0.25 : 1;
        ctx.fillStyle = col;
        ctx.globalAlpha *= c.is_placeholder ? 0.45 : 0.85;
        roundRect(ctx, x0, ty + 3, w, TRACK - 7, 3);
        ctx.fill();
        ctx.globalAlpha = c.muted ? 0.3 : 1;
        // waveform
        const pk = getPeaks(c.asset_id);
        if (pk && w > 3) {
          ctx.fillStyle = "rgba(0,0,0,0.55)";
          const mid = ty + 3 + (TRACK - 7) / 2;
          const amp = (TRACK - 11) / 2;
          const nb = pk.data.length / 2;
          const x1 = Math.min(w, W - x0 + 2);
          for (let px = Math.max(0, -x0); px < x1; px += 1) {
            const sp = geo.source_offset + px * vp.spp;
            const srcLen = nb * pk.per;
            let sidx = c.fill === "loop" && srcLen > 0 ? geo.source_offset + ((px * vp.spp) % Math.max(1, srcLen - geo.source_offset)) : sp;
            if (sidx >= srcLen) break;
            const i0 = Math.floor(sidx / pk.per);
            const i1 = Math.max(i0 + 1, Math.floor((sidx + vp.spp) / pk.per));
            let lo = 0, hi = 0;
            for (let k = i0; k < i1 && k < nb; k++) {
              lo = Math.min(lo, pk.data[k * 2]);
              hi = Math.max(hi, pk.data[k * 2 + 1]);
            }
            const g = Math.pow(10, c.gain_db / 20);
            ctx.fillRect(x0 + px, mid - (hi / 127) * amp * g, 1, Math.max(1, ((hi - lo) / 127) * amp * g));
          }
        }
        // fades (shaded curve) and handles
        const top = ty + 3, bot = ty + TRACK - 4;
        ctx.strokeStyle = "rgba(255,255,255,0.75)";
        ctx.fillStyle = "rgba(0,0,0,0.28)";
        ctx.lineWidth = 1;
        if (geo.fade_in > 0) {
          const fx = X(a + geo.fade_in);
          ctx.beginPath();
          ctx.moveTo(x0, bot);
          ctx.quadraticCurveTo(x0 + (fx - x0) * 0.25, top, fx, top);
          ctx.lineTo(x0, top);
          ctx.closePath();
          ctx.fill();
          ctx.beginPath();
          ctx.moveTo(x0, bot);
          ctx.quadraticCurveTo(x0 + (fx - x0) * 0.25, top, fx, top);
          ctx.stroke();
        }
        if (geo.fade_out > 0) {
          const fx = X(b - geo.fade_out);
          const x1 = X(b);
          ctx.beginPath();
          ctx.moveTo(fx, top);
          ctx.quadraticCurveTo(x1 - (x1 - fx) * 0.25, top, x1, bot);
          ctx.lineTo(x1, top);
          ctx.closePath();
          ctx.fill();
          ctx.beginPath();
          ctx.moveTo(fx, top);
          ctx.quadraticCurveTo(x1 - (x1 - fx) * 0.25, top, x1, bot);
          ctx.stroke();
        }
        const hv = hover.current?.id === c.id ? hover.current.zone : null;
        const active = pv ? d!.zone : hv;
        if (w > 14 && (hv || pv || (sel?.kind === "clip" && sel.id === c.id))) {
          const handle = (hx: number, on: boolean) => {
            ctx.fillStyle = on ? C("--accent") : "rgba(255,255,255,0.85)";
            ctx.fillRect(hx - 3.5, top, 7, 7);
          };
          handle(X(a + geo.fade_in), active === "fadeIn");
          handle(X(b - geo.fade_out), active === "fadeOut");
        }
        if (active === "trimL" || active === "trimR") {
          ctx.fillStyle = C("--accent");
          ctx.fillRect(active === "trimL" ? x0 : x0 + w - 3, top, 3, bot - top);
        }
        if (pv && d!.zone !== "move") {
          const ms = (n: number) => `${Math.round((n / rate.sr) * 1000)} ms`;
          const label =
            d!.zone === "fadeIn" ? `fade in ${ms(pv.fade_in)}` :
            d!.zone === "fadeOut" ? `fade out ${ms(pv.fade_out)}` :
            d!.zone === "trimL" ? `start ${tcAt(rate, pv.timeline_start)} · ${(pv.length / rate.sr).toFixed(2)}s` :
            `end ${tcAt(rate, pv.timeline_start + pv.length)} · ${(pv.length / rate.sr).toFixed(2)}s`;
          ctx.font = "11px var(--mono)";
          const tw = ctx.measureText(label).width + 10;
          const lx = Math.min(Math.max(2, X(d!.cur) + 10), W - tw - 2);
          ctx.fillStyle = "rgba(10,10,12,0.9)";
          ctx.fillRect(lx, ty - 16, tw, 16);
          ctx.fillStyle = C("--accent");
          ctx.fillText(label, lx + 5, ty - 4);
        }
        if (selected) {
          ctx.strokeStyle = C("--sel");
          ctx.lineWidth = 2;
          roundRect(ctx, x0, ty + 3, w, TRACK - 7, 3);
          ctx.stroke();
        }
        if (w > 30) {
          const asset = view.assets.find((x) => x.id === c.asset_id);
          ctx.save();
          ctx.beginPath();
          ctx.rect(x0, ty, w - 2, TRACK);
          ctx.clip();
          ctx.fillStyle = "#05070A";
          ctx.font = "10px var(--font)";
          const tag = c.performance_event_id ? "♪ " : c.take_id ? "● " : c.is_placeholder ? "◌ " : "";
          ctx.fillText(tag + (asset?.name ?? "?") + (asset?.missing ? " [MISSING]" : ""), x0 + 4, ty + 14);
          ctx.restore();
        }
        ctx.globalAlpha = 1;
        newHits.push({ x: x0, y: ty + 3 - scrollY, w, h: TRACK - 7, kind: "clip", id: c.id });
      }
      // automation lanes
      const autoPts = (view.automation ?? []) as AutoPoint[];
      for (const r of st.rows) {
        if (r.kind !== "auto") continue;
        const top = tracksTop + r.y;
        if (top - scrollY > H || top + AUTO - scrollY < 0) continue;
        ctx.fillStyle = C("--auto-bg");
        ctx.fillRect(0, top, W, AUTO - 1);
        const refV = r.param === "pan" ? 0 : 0;
        ctx.fillStyle = "rgba(255,255,255,0.08)";
        ctx.fillRect(0, Math.round(autoY(r.param, refV, top)), W, 1);
        let pts = autoPts.filter((p) => p.owner_id === r.lane && p.param === r.param);
        if (d?.kind === "auto" && d.autoPreview && pts.some((p) => p.id === d.id)) {
          const pv = d.autoPreview;
          pts = pts.filter((p) => p.id === d.id || p.sample !== pv.sample).map((p) => (p.id === d.id ? { ...p, sample: pv.sample, value: pv.value } : p));
        }
        pts.sort((a, b) => a.sample - b.sample);
        const col = r.owner.color;
        ctx.strokeStyle = col;
        ctx.lineWidth = 1.5;
        if (!pts.length) {
          ctx.setLineDash([3, 4]);
          ctx.globalAlpha = 0.45;
          const yy = autoY(r.param, neutral(r.param, r.owner.pan), top);
          ctx.beginPath();
          ctx.moveTo(0, yy);
          ctx.lineTo(W, yy);
          ctx.stroke();
          ctx.setLineDash([]);
          ctx.globalAlpha = 1;
        } else {
          ctx.beginPath();
          ctx.moveTo(0, autoY(r.param, pts[0].value, top));
          for (const p of pts) ctx.lineTo(X(p.sample), autoY(r.param, p.value, top));
          ctx.lineTo(W, autoY(r.param, pts[pts.length - 1].value, top));
          ctx.stroke();
          ctx.globalAlpha = 0.12;
          ctx.fillStyle = col;
          ctx.lineTo(W, top + AUTO - 1);
          ctx.lineTo(0, top + AUTO - 1);
          ctx.closePath();
          ctx.fill();
          ctx.globalAlpha = 1;
          for (const p of pts) {
            const px = X(p.sample);
            if (px < -8 || px > W + 8) continue;
            const py = autoY(r.param, p.value, top);
            const active = d?.kind === "auto" && d.id === p.id;
            ctx.fillStyle = active ? C("--accent") : "#F4F7FB";
            ctx.beginPath();
            ctx.arc(px, py, active ? 5 : 3.5, 0, Math.PI * 2);
            ctx.fill();
            ctx.strokeStyle = "#111";
            ctx.lineWidth = 1;
            ctx.stroke();
            newHits.push({ x: px - 6, y: py - 6 - scrollY, w: 12, h: 12, kind: "auto", id: p.id });
          }
        }
        const wr = st.tx?.writing?.find((w) => w.owner_id === r.lane && w.param === r.param);
        if (wr && wr.value !== null) {
          // being written right now: live value at the playhead
          const wx = X(st.livePosition());
          const wy = autoY(r.param, wr.value, top);
          ctx.fillStyle = C("--rec");
          ctx.globalAlpha = 0.22;
          ctx.fillRect(0, top, W, AUTO - 1);
          ctx.globalAlpha = 1;
          ctx.fillStyle = C("--rec");
          ctx.beginPath();
          ctx.arc(wx, wy, 5, 0, Math.PI * 2);
          ctx.fill();
          ctx.font = "10px var(--mono)";
          ctx.fillText(`● ${wr.mode.toUpperCase()} ${formatValue(r.param, wr.value)}`, Math.min(wx + 8, W - 120), top + 12);
        }
        if (d?.kind === "auto" && d.autoPreview && d.autoTop === r.y) {
          const label = `${formatValue(r.param, d.autoPreview.value)} @ ${tcAt(rate, d.autoPreview.sample)}`;
          ctx.font = "11px var(--mono)";
          const tw = ctx.measureText(label).width + 10;
          const lx = Math.min(Math.max(2, X(d.autoPreview.sample) + 10), W - tw - 2);
          ctx.fillStyle = "rgba(10,10,12,0.9)";
          ctx.fillRect(lx, top + 2, tw, 16);
          ctx.fillStyle = C("--accent");
          ctx.fillText(label, lx + 5, top + 14);
        }
      }
      ctx.restore();

      // ---- fixed ruler + scene strip
      ctx.fillStyle = C("--ruler-bg");
      ctx.fillRect(0, 0, W, RULER + SCENES);
      const pxPerSec = rate.sr / vp.spp;
      const fpsN = rate.num / rate.den;
      const steps = [1 / fpsN, 2 / fpsN, 5 / fpsN, 10 / fpsN, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600];
      const step = steps.find((s) => s * pxPerSec >= 90) ?? 600;
      const stepSamples = step * rate.sr;
      ctx.font = "10px var(--mono)";
      ctx.fillStyle = C("--tl-text");
      ctx.strokeStyle = C("--tick");
      let t0 = Math.floor(vp.start / stepSamples) * stepSamples;
      for (let t = t0; t < end; t += stepSamples) {
        const s = snap(rate, t);
        const x = X(s);
        ctx.beginPath();
        ctx.moveTo(x + 0.5, RULER - 9);
        ctx.lineTo(x + 0.5, RULER);
        ctx.stroke();
        ctx.fillText(tcAt(rate, s), x + 3, 12);
        for (let k = 1; k < 5; k++) {
          const xs = X(s + (stepSamples * k) / 5);
          ctx.beginPath();
          ctx.moveTo(xs + 0.5, RULER - 4);
          ctx.lineTo(xs + 0.5, RULER);
          ctx.stroke();
        }
      }
      for (const sc of view.scenes) {
        if (sc.end_sample < vp.start || sc.start_sample > end) continue;
        const x0 = X(sc.start_sample);
        const w = X(sc.end_sample) - x0;
        ctx.fillStyle = C("--scene-chip");
        ctx.fillRect(x0 + 1, RULER + 2, w - 2, SCENES - 4);
        ctx.save();
        ctx.beginPath();
        ctx.rect(x0, RULER, w, SCENES);
        ctx.clip();
        ctx.fillStyle = C("--tl-text");
        ctx.font = "11px var(--font)";
        ctx.fillText(`${sc.number}  ${sc.title}`, Math.max(x0, 0) + 5, RULER + 14);
        ctx.restore();
      }
      if (lr) {
        ctx.fillStyle = C("--accent");
        ctx.fillRect(X(lr[0]), RULER - 4, X(lr[1]) - X(lr[0]), 4);
      }
      // ---- playhead
      const ph = X(st.livePosition());
      ctx.fillStyle = C("--playhead");
      ctx.fillRect(Math.round(ph), 0, 1.5, H);
      ctx.beginPath();
      ctx.moveTo(ph - 6, 0);
      ctx.lineTo(ph + 7, 0);
      ctx.lineTo(ph + 0.5, 8);
      ctx.fill();
      hits.current = newHits;
      (window as unknown as { __vcsdHits: Hit[] }).__vcsdHits = newHits; // for UI tests
    };
    draw();
    return () => cancelAnimationFrame(raf);
  }, [contentH]);

  // ------------------------------------------------------------------ input
  const localXY = (e: React.MouseEvent | MouseEvent) => {
    const r = canvasRef.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  /** Track/automation row under canvas y, with its top in canvas coordinates. */
  const rowAt = (y: number) => {
    const cy = y + scrollY - RULER - SCENES - lanesH;
    const r = rows.find((r) => cy >= r.y && cy < r.y + r.h);
    return r ? { row: r, top: RULER + SCENES + lanesH + r.y - scrollY } : null;
  };
  const autoPoints = (ownerId: string, param: Param) => ((view?.automation ?? []) as AutoPoint[]).filter((p) => p.owner_id === ownerId && p.param === param);

  /** Topmost hit under a point; clips get a few px of grab slop past their
   *  edges so trim handles are easy to catch. */
  const SLOP = 4;
  const hitAt = (x: number, y: number, kind?: Hit["kind"]) => {
    const under = [...hits.current].reverse().filter((h) => {
      const pad = h.kind === "clip" ? SLOP : 0;
      return (!kind || h.kind === kind) && x >= h.x - pad && x <= h.x + h.w + pad && y >= h.y && y <= h.y + h.h;
    });
    // with overlapping clips: fade handles beat trim edges, which beat bodies
    const zoneOf = (h: Hit) => {
      const c = h.kind === "clip" ? view?.clips.find((c) => c.id === h.id) : undefined;
      return c ? clipZone(h, c, x, y, vp.spp) : "move";
    };
    const rank = (z: ClipZone) => (z === "fadeIn" || z === "fadeOut" ? 0 : z === "move" ? 2 : 1);
    let best = under[0];
    for (const h of under) if (rank(zoneOf(h)) < rank(zoneOf(best))) best = h;
    return best;
  };
  const laneAt = (y: number) => {
    if (!LANE) return null;
    const cy = y + scrollY - RULER - SCENES;
    const i = Math.floor(cy / LANE);
    return i >= 0 && i < lanes.length ? lanes[i] : null;
  };

  const onDown = (e: React.MouseEvent) => {
    if (!rate || !view) return;
    const { x, y } = localXY(e);
    const s = sOf(x);
    if (y < RULER + SCENES) {
      if (e.shiftKey) drag.current = { kind: "loop", x0: x, s0: snapS(s), moved: false, delta: 0, cur: snapS(s) };
      else {
        drag.current = { kind: "ruler", x0: x, s0: s, moved: false, delta: 0, cur: s };
        call("transport.locate", { sample: snapS(s) });
      }
    } else if (hitAt(x, y, "auto") || rowAt(y)?.row.kind === "auto") {
      const ra = rowAt(y)!;
      const row = ra.row as Extract<Row, { kind: "auto" }>;
      const ph = hitAt(x, y, "auto");
      if (ph) {
        drag.current = { kind: "auto", id: ph.id, x0: x, s0: s, moved: false, delta: 0, cur: s, autoTop: row.y };
      } else {
        // add a breakpoint: on the line keep its value, elsewhere take the pointer's
        const pts = autoPoints(row.lane, row.param);
        const sample = e.altKey ? Math.round(s) : snapS(s);
        const lineV = valueAt(pts, sample, neutral(row.param, row.owner.pan));
        const value = Math.abs(autoY(row.param, lineV, ra.top) - y) <= 4 ? lineV : autoVal(row.param, y, ra.top);
        // listeners attach right away; the new point's id arrives asynchronously
        const pending: Drag = { kind: "auto", id: "", x0: x, s0: s, moved: false, delta: 0, cur: s, autoTop: row.y };
        drag.current = pending;
        run<{ id: string }>("automation.set", { owner_id: row.lane, param: row.param, sample, value }).then((r) => {
          if (r) pending.id = r.id;
        });
      }
    } else {
      const h = hitAt(x, y);
      if (h) {
        let zone: ClipZone | undefined;
        if (h.kind === "clip") {
          const c = view.clips.find((c) => c.id === h.id)!;
          zone = clipZone(h, c, x, y, vp.spp);
          setSel({ kind: "clip", id: c.id });
        } else {
          const m = view.markers.find((m) => m.id === h.id)!;
          setSel(m.cue_id ? { kind: "cue", id: m.cue_id } : { kind: "marker", id: m.id });
        }
        drag.current = { kind: h.kind, zone, id: h.id, x0: x, s0: s, moved: false, delta: 0, cur: s };
      } else {
        const lane = laneAt(y);
        if (lane) drag.current = { kind: "newRange", laneId: lane.id, x0: x, s0: snapS(s), moved: false, delta: 0, cur: snapS(s) };
        else {
          const ra = rowAt(y);
          if (ra?.row.kind === "track") setSel({ kind: "track", id: ra.row.t.id });
          else if (ra?.row.kind === "folder") setSel({ kind: "folder", id: ra.row.path });
          else setSel(null);
        }
      }
    }
    const move = (ev: MouseEvent) => {
      const dd = drag.current;
      if (!dd) return;
      const p = localXY(ev);
      const cs = sOf(p.x);
      if (Math.abs(p.x - dd.x0) > 3) dd.moved = true;
      const fade = dd.zone === "fadeIn" || dd.zone === "fadeOut";
      dd.cur = dd.kind === "ruler" || fade || ev.altKey ? Math.round(cs) : snapS(cs);
      if (dd.kind === "ruler") call("transport.locate", { sample: snapS(cs) });
      if (dd.kind === "clip" || dd.kind === "marker") {
        const base = dd.kind === "clip" ? view.clips.find((c) => c.id === dd.id)!.timeline_start : view.markers.find((m) => m.id === dd.id)!.start_sample;
        const tgt = base + (cs - dd.s0);
        dd.delta = (ev.altKey ? Math.round(tgt) : snapS(tgt)) - base;
      }
      if (dd.kind === "auto" && dd.moved && dd.id) {
        const cur = stateRef.current;
        const p = ((cur.view?.automation ?? []) as AutoPoint[]).find((q) => q.id === dd.id);
        const row = cur.rows.find((r) => r.kind === "auto" && r.y === dd.autoTop) as Extract<Row, { kind: "auto" }> | undefined;
        if (p && row) {
          const top = RULER + SCENES + lanes.length * LANE + row.y - scrollY;
          const sample = ev.shiftKey ? p.sample : Math.max(0, ev.altKey ? Math.round(cs) : snapS(cs));
          dd.autoPreview = { sample, value: autoVal(row.param, localXY(ev).y, top) };
        }
      }
      if (dd.kind === "clip" && dd.moved) {
        const c = view.clips.find((c) => c.id === dd.id)!;
        const asset = view.assets.find((x) => x.id === c.asset_id);
        const srcLen = asset ? Math.floor((asset.length * rate.sr) / asset.sample_rate) : c.length;
        dd.preview = applyClipDrag(c, dd.zone ?? "move", dd.cur, dd.delta, srcLen);
      }
      if (dd.kind === "loop" && dd.moved) call("transport.loop", { start: Math.min(dd.s0, dd.cur), end: Math.max(dd.s0, dd.cur) });
      force((n) => n + 1);
    };
    const up = async () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      const dd = drag.current;
      drag.current = null;
      if (!dd) return;
      if (dd.kind === "loop" && !dd.moved) call("transport.loop", {});
      if (dd.kind === "auto" && dd.moved && dd.id && dd.autoPreview) {
        await run("automation.update", { id: dd.id, sample: dd.autoPreview.sample, value: dd.autoPreview.value });
      }
      if (dd.kind === "clip" && dd.moved && dd.preview) {
        const c = view.clips.find((c) => c.id === dd.id)! as Clip;
        const pv = dd.preview;
        const z = dd.zone ?? "move";
        if (z === "move" && pv.timeline_start !== c.timeline_start) await run("clip.update", { id: c.id, timeline_start: pv.timeline_start });
        if (z === "trimL" && pv.timeline_start !== c.timeline_start) await run("clip.update", { id: c.id, trim_start: pv.timeline_start });
        if (z === "trimR" && pv.length !== c.length) await run("clip.update", { id: c.id, length: pv.length });
        if (z === "fadeIn" && pv.fade_in !== c.fade_in) await run("clip.update", { id: c.id, fade_in: pv.fade_in });
        if (z === "fadeOut" && pv.fade_out !== c.fade_out) await run("clip.update", { id: c.id, fade_out: pv.fade_out });
      }
      if (dd.kind === "marker" && dd.moved && dd.delta !== 0) {
        const m = view.markers.find((m) => m.id === dd.id)!;
        await run("marker.update", { id: m.id, start: m.start_sample + dd.delta, end: m.end_sample + dd.delta, sub_events: m.sub_events.map((x) => x + dd.delta) });
      }
      if (dd.kind === "newRange") {
        if (dd.moved && Math.abs(dd.cur - dd.s0) > 0) {
          const lane = lanes.find((l) => l.id === dd.laneId)!;
          const kind = catKey(lane.category) === "footsteps" ? "repeated" : "range";
          const r = await run<{ id: string }>("marker.add", { lane: lane.id, kind, start: Math.min(dd.s0, dd.cur), end: Math.max(dd.s0, dd.cur), label: lane.name.split("•").pop()!.trim().toLowerCase() });
          if (r) setSel({ kind: "marker", id: r.id });
        } else {
          call("transport.locate", { sample: dd.s0 });
        }
      }
      force((n) => n + 1);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const onDouble = async (e: React.MouseEvent) => {
    const { x, y } = localXY(e);
    const ap = hitAt(x, y, "auto");
    if (ap) {
      await run("automation.delete", { id: ap.id });
      return;
    }
    if (rowAt(y)?.row.kind === "auto") return;
    const ch = hitAt(x, y, "clip");
    if (ch && view) {
      const c = view.clips.find((c) => c.id === ch.id)!;
      const z = clipZone(ch, c, x, y, vp.spp);
      if (z === "fadeIn" || z === "fadeOut") {
        await run("clip.update", { id: c.id, [z === "fadeIn" ? "fade_in" : "fade_out"]: 0 });
        return;
      }
    }
    const lane = laneAt(y);
    if (!lane || !rate) return;
    const h = hits.current.find((h) => x >= h.x && x <= h.x + h.w && y >= h.y && y <= h.y + h.h);
    if (h) return;
    const s = snapS(sOf(x));
    const r = await run<{ id: string }>("marker.add", { lane: lane.id, kind: "point", start: s, end: s, label: lane.name.split("•").pop()!.trim().toLowerCase() });
    if (r) setSel({ kind: "marker", id: r.id });
  };

  const onHover = (e: React.MouseEvent) => {
    if (drag.current || !view) return;
    const { x, y } = localXY(e);
    if (hitAt(x, y, "auto")) {
      hover.current = null;
      if (canvasRef.current) canvasRef.current.style.cursor = "pointer";
      return;
    }
    if (rowAt(y)?.row.kind === "auto") {
      hover.current = null;
      if (canvasRef.current) canvasRef.current.style.cursor = "copy";
      return;
    }
    const h = hitAt(x, y, "clip");
    let cursor = "crosshair";
    if (h) {
      const c = view.clips.find((c) => c.id === h.id);
      const zone = c ? clipZone(h, c, x, y, vp.spp) : "move";
      hover.current = { id: h.id, zone };
      cursor = { move: "grab", trimL: "w-resize", trimR: "e-resize", fadeIn: "ew-resize", fadeOut: "ew-resize" }[zone];
    } else {
      hover.current = null;
      if (y < RULER + SCENES) cursor = "text";
    }
    if (canvasRef.current && canvasRef.current.style.cursor !== cursor) canvasRef.current.style.cursor = cursor;
  };

  const onWheel = (e: React.WheelEvent) => {
    if (!rate) return;
    if (e.ctrlKey || e.metaKey || e.altKey) {
      const { x } = localXY(e);
      const anchor = sOf(x);
      const spp = Math.min(48000, Math.max(0.5, vp.spp * Math.pow(1.0018, e.deltaY)));
      setVp({ spp, start: anchor - x * spp });
    } else if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      setVp({ ...vp, start: vp.start + (e.deltaX || e.deltaY) * vp.spp });
    } else {
      setScrollY((v) => Math.max(0, Math.min(Math.max(0, contentH - size.h), v + e.deltaY)));
    }
  };

  if (!view?.open || !rate) return null;
  return (
    <div className="timeline" ref={wrapRef} onWheel={onWheel}>
      <div className="tl-heads" style={{ width: HEAD_W }}>
        <div style={{ transform: `translateY(${-scrollY}px)` }}>
          <div className="tl-corner" style={{ height: RULER + SCENES }}>
            <span className="muted small">{frameLabel(rate, vp)}</span>
            {onLaneMode && (
              <button className="key lanes-key" title="Marker lanes: full, folded strips, or hidden"
                onClick={() => onLaneMode(laneMode === "full" ? "folded" : laneMode === "folded" ? "hidden" : "full")}>
                Lanes · {laneMode}
              </button>
            )}
          </div>
          {laneMode === "hidden" ? (
            <button className="lane-head lanes-summary" style={{ height: HIDDEN_ROW }} title="Show the marker lanes as strips" onClick={() => onLaneMode?.("folded")}>
              ▸ Marker lanes · {lanes.length} folded
            </button>
          ) : laneMode === "folded" ? lanes.map((l) => (
            <div key={l.id} className="lane-head folded" style={{ height: LANE }} title={l.name}>
              <span className="lane-strip" style={{ background: laneColor(l) }} />
            </div>
          )) : lanes.map((l) => {
            const keys = Object.entries(view.bindings).filter(([, b]) => b.lane_id === l.id).map(([k]) => k.replace("key:Key", "").replace("key:", "").replace(/^midi:\d+:/, "♪"));
            return (
              <div key={l.id} className="lane-head" style={{ height: LANE }}>
                <span className="swatch" style={{ background: laneColor(l) }} />
                <span className="ellipsis">{l.name}</span>
                {keys.length > 0 && <span className="keyhint">{keys.join(" ")}</span>}
              </div>
            );
          })}
          {rows.map((r) =>
            r.kind === "folder" ? (
              <FolderHead key={"f:" + r.path} row={r} eng={eng} selected={sel?.kind === "folder" && sel.id === r.path} onSelect={() => setSel({ kind: "folder", id: r.path })} onToggle={() => toggleFolder(r.path)} />
            ) : r.kind === "track" ? (
              <TrackHead key={r.t.id} t={r.t} depth={r.depth} eng={eng} selected={sel?.kind === "track" && sel.id === r.t.id} onSelect={() => setSel({ kind: "track", id: r.t.id })}
                autoOpen={autoOpen.has(r.t.id)} hasAuto={hasAutomation(automation, r.t.id, r.t.inserts)} onToggleAuto={() => toggleAuto(r.t.id)} />
            ) : r.kind === "bus" ? (
              <BusHead key={r.b.id} b={r.b} eng={eng} first={r.first} autoOpen={autoOpen.has(r.b.id)}
                hasAuto={hasAutomation(automation, r.b.id, r.b.inserts ?? [])} onToggleAuto={() => toggleAuto(r.b.id)} />
            ) : r.kind === "autoAdd" ? (
              <PluginLaneAdd key={"add:" + r.owner.id} owner={r.owner} slots={r.slots} />
            ) : (
              <AutoHead key={r.lane + r.param} row={r} eng={eng} points={autoPoints(r.lane, r.param)} />
            ),
          )}
        </div>
      </div>
      <canvas ref={canvasRef} className="tl-canvas" onMouseDown={onDown} onDoubleClick={onDouble} onMouseMove={onHover} onMouseLeave={() => (hover.current = null)} />
    </div>
  );
}

function frameLabel(rate: Rate, vp: Viewport) {
  const f = vp.spp / sampleAtFrame(rate, 1);
  return f < 1 ? `${(1 / f).toFixed(1)} px/frame` : `${f.toFixed(1)} frames/px`;
}

function BusHead({ b, eng, first, autoOpen, hasAuto, onToggleAuto }: { b: Bus; eng: EngineHook; first: boolean; autoOpen: boolean; hasAuto: boolean; onToggleAuto: () => void }) {
  const meter = (b.parent_id ? eng.tx?.meters[b.id] : eng.tx?.master_peak) ?? 0;
  const db = meter > 0 ? 20 * Math.log10(meter) : -90;
  const pct = Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  return (
    <div className={"bus-head" + (b.parent_id ? "" : " master") + (first ? " first" : "")} style={{ height: BUS_H }}>
      <span className="ellipsis th-name" title={b.parent_id ? "Stem bus" : "Master bus"}>{b.parent_id ? "⇢ " : "◆ "}{b.name}</span>
      <span className="small mono muted">{b.gain_db > 0 ? "+" : ""}{b.gain_db.toFixed(1)}</span>
      <div className="meter-h"><div style={{ width: pct + "%" }} className={db > -1 ? "hot" : ""} /></div>
      <button className={"tiny" + (b.mute ? " on-mute" : "")} title="Mute bus" onClick={() => eng.run("bus.update", { id: b.id, mute: !b.mute })}>M</button>
      <button className={"tiny" + (autoOpen ? " on" : "") + (hasAuto ? " has-auto" : "")} title={(autoOpen ? "Hide" : "Show") + " bus automation" + (hasAuto ? " (has automation)" : "")} onClick={onToggleAuto}>A</button>
      <ModeButton eng={eng} ownerId={b.id} mode={(b.automation_mode ?? "read") as AutoMode} />
    </div>
  );
}

function AutoHead({ row, eng, points }: { row: Extract<Row, { kind: "auto" }>; eng: EngineHook; points: AutoPoint[] }) {
  const { owner, param, lane, label } = row;
  const v = valueAt(points, eng.livePosition(), neutral(param, owner.pan));
  const plugin = isPluginParam(param);
  const what = plugin ? label : PARAM_LABEL[param].toLowerCase();
  return (
    <div className={"auto-head" + (plugin ? " plugin" : "")} style={{ height: AUTO }}>
      <span className="small ellipsis" title={label}>{label}{param === "volume_db" && <span className="muted"> (vs fader)</span>}</span>
      <span className="mono small auto-val">{formatValue(param, v)}</span>
      <span className="small muted">{points.length} pt</span>
      <button className="tiny" disabled={!points.length} title={`Clear ${what} automation`}
        onClick={() => confirm(`Remove all ${points.length} ${what} points on "${owner.name}"?`) && eng.run("automation.clear", { owner_id: lane, param })}>×</button>
      {plugin && !points.length && <button className="tiny" title="Hide this lane" onClick={() => hidePluginLane(lane, param)}>–</button>}
    </div>
  );
}

/** Whether a track/bus has any automation, including its plugins'. */
function hasAutomation(pts: AutoPoint[], owner: string, inserts: Track["inserts"]) {
  const slots = new Set(inserts.filter((i) => i.type === "plugin").map((i) => (i as PluginSlot).id));
  return pts.some((p) => p.owner_id === owner || slots.has(p.owner_id));
}

/** "+ Plugin lane": pick a parameter of one of the owner's plugins to automate. */
function PluginLaneAdd({ owner, slots }: { owner: Owner; slots: PluginSlot[] }) {
  useParamsVersion();
  return (
    <div className="auto-add" style={{ height: ADD_H }}>
      <select className="small" value="" onFocus={() => slots.forEach((s) => paramsOf(s.id))}
        onChange={(e) => {
          const [slot, param] = e.target.value.split("|");
          if (slot && param) showPluginLane(owner.id, slot, param as Param);
        }}>
        <option value="">+ Plugin lane…</option>
        {slots.map((s) => (
          <optgroup key={s.id} label={s.name}>
            {(paramsOf(s.id) ?? []).filter((p) => p.automatable && !p.read_only && !p.is_bypass).map((p) => (
              <option key={p.id} value={`${s.id}|plugin:${p.id}`}>{p.title}{p.units ? ` (${p.units})` : ""}</option>
            ))}
          </optgroup>
        ))}
      </select>
    </div>
  );
}

const DRAG_TYPE = "application/x-vcsd-track";

/** Folder header: collapse, inline rename, folder mute/solo, drop target for tracks. */
function FolderHead({ row, eng, selected, onSelect, onToggle }: { row: Extract<Row, { kind: "folder" }>; eng: EngineHook; selected: boolean; onSelect: () => void; onToggle: () => void }) {
  const [editing, setEditing] = useState(false);
  const [over, setOver] = useState(false);
  const kids = (eng.view?.tracks ?? []).filter((t) => row.trackIds.includes(t.id));
  const allMuted = kids.length > 0 && kids.every((t) => t.mute);
  const allSolo = kids.length > 0 && kids.every((t) => t.solo);
  const rename = (name: string) => {
    setEditing(false);
    const clean = name.replace(/\//g, "-").trim();
    if (!clean || clean === row.name) return;
    const parent = parentOf(row.path);
    eng.run("folder.rename", { from: row.path, to: parent ? `${parent}/${clean}` : clean });
  };
  return (
    <div
      className={"folder-head" + (selected ? " selected" : "") + (over ? " drop" : "")}
      style={{ height: FOLDER_H, paddingLeft: 4 + row.depth * INDENT }}
      onMouseDown={onSelect}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes(DRAG_TYPE)) {
          e.preventDefault();
          setOver(true);
        }
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        setOver(false);
        const id = e.dataTransfer.getData(DRAG_TYPE);
        if (id) eng.run("folder.move", { track_ids: [id], folder: row.path });
      }}
    >
      <button className="tiny twisty" title={row.collapsed ? "Expand" : "Collapse"} onClick={onToggle}>{row.collapsed ? "▸" : "▾"}</button>
      {editing ? (
        <input className="folder-name-edit" autoFocus defaultValue={row.name} onFocus={(e) => e.target.select()} onMouseDown={(e) => e.stopPropagation()} onBlur={(e) => rename(e.target.value)}
          onKeyDown={(e) => { e.stopPropagation(); if (e.key === "Enter") (e.target as HTMLInputElement).blur(); if (e.key === "Escape") setEditing(false); }} />
      ) : (
        <span className="ellipsis folder-name" title={`${row.path} — double-click to rename`} onDoubleClick={() => setEditing(true)}>📁 {row.name}</span>
      )}
      <span className="small muted">{row.trackIds.length}</span>
      <button className={"tiny" + (allMuted ? " on-mute" : "")} title="Mute everything in this folder" onClick={() => eng.run("folder.flag", { path: row.path, flag: "mute", on: !allMuted })}>M</button>
      <button className={"tiny" + (allSolo ? " on-solo" : "")} title="Solo everything in this folder" onClick={() => eng.run("folder.flag", { path: row.path, flag: "solo", on: !allSolo })}>S</button>
    </div>
  );
}

function TrackHead({ t, depth, eng, selected, onSelect, autoOpen, hasAuto, onToggleAuto }: { t: Track; depth: number; eng: EngineHook; selected: boolean; onSelect: () => void; autoOpen: boolean; hasAuto: boolean; onToggleAuto: () => void }) {
  const meter = eng.tx?.meters[t.id] ?? 0;
  const db = meter > 0 ? 20 * Math.log10(meter) : -90;
  const upd = (a: object) => eng.run("track.update", { id: t.id, ...a });
  const color = t.color ?? catColor(t.category);
  const num = (eng.view?.tracks.indexOf(t) ?? -1) + 1;
  const bus = eng.view?.buses.find((b) => b.id === t.bus_id)?.name;
  return (
    <div
      className={"track-head" + (selected ? " selected" : "")}
      style={{ height: TRACK, paddingLeft: 8 + depth * INDENT, "--trk": color } as React.CSSProperties}
      onMouseDown={onSelect}
      onDragOver={(e) => e.dataTransfer.types.includes(DRAG_TYPE) && e.preventDefault()}
      onDrop={(e) => {
        // dropping on a track moves the dragged track into that track's folder
        const id = e.dataTransfer.getData(DRAG_TYPE);
        if (id && id !== t.id) eng.run("folder.move", { track_ids: [id], folder: t.folder });
      }}
    >
      <div className="th-main">
        <div className="th-row">
          <label
            className="swatch track-color"
            style={{ background: color }}
            title={t.color ? `Track color ${t.color} (click to change, Alt-click for the category's)` : "Track color: the category's (click to choose one)"}
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              if (e.altKey) {
                e.preventDefault();
                if (t.color) upd({ color: null });
              }
            }}
          >
            <input type="color" value={t.color ?? "#808080"} aria-label="Track color" onChange={(e) => upd({ color: e.target.value })} />
          </label>
          <span className="th-num">{String(num).padStart(2, "0")}</span>
          <span
            className="ellipsis th-name drag-grip"
            title={`${t.name}\n${t.folder || "(top level)"}\nDrag onto a folder or track to move it`}
            draggable
            onDragStart={(e) => {
              e.dataTransfer.setData(DRAG_TYPE, t.id);
              e.dataTransfer.effectAllowed = "move";
            }}
          >
            {t.name}
          </span>
          {bus && <span className="th-bus ellipsis">{bus}</span>}
        </div>
        <div className="th-row">
          <button className="key th-key mute" aria-pressed={t.mute} title="Mute" onClick={() => upd({ mute: !t.mute })}>M</button>
          <button className="key th-key solo" aria-pressed={t.solo} title="Solo" onClick={() => upd({ solo: !t.solo })}>S</button>
          <button className="key th-key rec" aria-pressed={t.armed} title="Arm for recording" onClick={() => upd({ armed: !t.armed })}>●</button>
          <button className={"key th-key" + (hasAuto ? " has-auto" : "")} aria-pressed={autoOpen} title={(autoOpen ? "Hide" : "Show") + " automation lanes" + (hasAuto ? " (has automation)" : "")} onClick={onToggleAuto}>A</button>
          <ModeButton eng={eng} ownerId={t.id} mode={(t.automation_mode ?? "read") as AutoMode} />
          <span className="spacer" />
          <Knob value={t.gain_db} min={-48} max={12} reset={0} step={0.5} label={`${t.name} gain`} format={fmtGain} onCommit={(v) => upd({ gain_db: v })} />
          <Knob value={t.pan} min={-1} max={1} reset={0} step={0.01} bipolar label={`${t.name} pan`} format={fmtPan} onCommit={(v) => upd({ pan: v })} />
          <span className="th-db">{t.gain_db > 0 ? "+" : ""}{t.gain_db.toFixed(1)}</span>
        </div>
      </div>
      <SegMeter db={db} n={12} vertical />
    </div>
  );
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

export function zoomToRange(rate: Rate, a: number, b: number, width: number): Viewport {
  const span = Math.max(b - a, sampleAtFrame(rate, 24));
  const spp = (span * 1.08) / Math.max(200, width);
  return { spp, start: a - span * 0.04 };
}

export { frameAtSample };

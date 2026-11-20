import { useEffect, useRef, useState } from "react";
import { call } from "../api";
import type { EngineHook } from "../store";
import { catColor, catKey, type Clip, type Marker, type Selection, type Track } from "../types";
import { frameAtSample, sampleAtFrame, snap, tcAt, type Rate } from "../tc";
import { applyClipDrag, clipZone, type ClipEdit, type ClipZone } from "../clipEdit";

export interface Viewport {
  start: number; // first visible sample
  spp: number; // samples per pixel
}

const RULER = 26;
const SCENES = 20;
const LANE = 24;
const TRACK = 54;
const HEAD_W = 230;

type Hit = { x: number; y: number; w: number; h: number; kind: "clip" | "marker"; id: string };

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
}

interface Drag {
  kind: "clip" | "marker" | "ruler" | "loop" | "newRange";
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

export function Timeline({ eng, vp, setVp, sel, setSel, snapOn }: Props) {
  const { view, tx, rate, run, livePosition } = eng;
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [scrollY, setScrollY] = useState(0);
  const [size, setSize] = useState({ w: 800, h: 300 });
  const hits = useRef<Hit[]>([]);
  const drag = useRef<Drag | null>(null);
  const hover = useRef<{ id: string; zone: ClipZone } | null>(null);
  const [, force] = useState(0);
  const stateRef = useRef({ view, tx, vp, sel, scrollY, size, rate, livePosition });
  stateRef.current = { view, tx, vp, sel, scrollY, size, rate, livePosition };

  const lanes = view?.lanes ?? [];
  const tracks = view?.tracks ?? [];
  const contentH = RULER + SCENES + lanes.length * LANE + tracks.length * TRACK + 40;

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
        ctx.fillStyle = "rgba(255, 200, 60, 0.07)";
        ctx.fillRect(X(lr[0]), 0, X(lr[1]) - X(lr[0]), contentH + RULER);
      }
      // marker lanes
      const laneY = new Map<string, number>();
      for (const l of view.lanes) {
        laneY.set(l.id, y);
        ctx.fillStyle = C("--lane-bg");
        ctx.fillRect(0, y, W, LANE - 1);
        y += LANE;
      }
      const oneF = sampleAtFrame(rate, 1);
      const drawMarker = (m: Marker, dx: number, ghost: boolean) => {
        const ly = laneY.get(m.lane_id);
        if (ly === undefined) return;
        const lane = view.lanes.find((l) => l.id === m.lane_id)!;
        const a = m.start_sample + dx;
        const b = Math.max(m.end_sample + dx, a + oneF);
        if (b < vp.start || a > end) return;
        const x0 = X(a);
        const w = Math.max(3, X(b) - x0);
        const selected = sel?.kind === "marker" && sel.id === m.id || (sel?.kind === "cue" && m.cue_id === sel.id);
        ctx.globalAlpha = ghost ? 0.5 : m.cue_id ? 1 : 0.75;
        ctx.fillStyle = lane.color || catColor(lane.category);
        if (m.kind === "point") {
          ctx.fillRect(x0 - 1, ly + 2, 3, LANE - 5);
          ctx.beginPath();
          ctx.moveTo(x0 - 5, ly + 2);
          ctx.lineTo(x0 + 6, ly + 2);
          ctx.lineTo(x0 + 0.5, ly + 9);
          ctx.fill();
        } else {
          ctx.fillRect(x0, ly + 3, w, LANE - 7);
        }
        ctx.fillStyle = "#000";
        for (const e of m.sub_events) {
          const ex = X(e + dx);
          ctx.fillRect(ex, ly + LANE - 9, 1.5, 5);
        }
        if (selected) {
          ctx.strokeStyle = C("--sel");
          ctx.lineWidth = 2;
          ctx.strokeRect(x0 - 2, ly + 1, (m.kind === "point" ? 4 : w) + 4, LANE - 3);
        }
        ctx.globalAlpha = 1;
        ctx.fillStyle = C("--tl-text");
        ctx.font = "11px var(--font)";
        const cue = m.cue_id ? view.cues.find((c) => c.id === m.cue_id) : null;
        const label = `${cue ? cue.code + " " : ""}${m.label}${m.cue_id ? "" : " (marker)"}`;
        ctx.save();
        ctx.beginPath();
        ctx.rect(x0 + (m.kind === "point" ? 8 : 3), ly, m.kind === "point" ? 240 : Math.max(0, w - 5), LANE);
        ctx.clip();
        ctx.fillStyle = m.kind === "point" ? C("--tl-text") : "#0b0b0b";
        ctx.fillText(label, x0 + (m.kind === "point" ? 8 : 4), ly + 15);
        ctx.restore();
        if (!ghost) newHits.push({ x: x0 - 4, y: ly - scrollY, w: (m.kind === "point" ? 8 : w) + 4, h: LANE, kind: "marker", id: m.id });
      };
      for (const m of view.markers) drawMarker(m, d?.kind === "marker" && d.id === m.id ? d.delta : 0, false);
      // AI proposals (ghosts, dashed)
      for (const p of view.proposals.filter((p) => p.status === "pending")) {
        const cue = view.cues.find((c) => c.id === p.cue_id);
        const ly = cue?.lane_id ? laneY.get(cue.lane_id) : undefined;
        if (ly === undefined) continue;
        ctx.setLineDash([4, 3]);
        ctx.strokeStyle = "#ffd966";
        const a = X(p.start_sample);
        const w = Math.max(4, X(Math.max(p.end_sample, p.start_sample + oneF)) - a);
        ctx.strokeRect(a, ly + 2, w, LANE - 5);
        ctx.setLineDash([]);
        ctx.fillStyle = "#ffd966";
        for (const e of p.sub_events) ctx.fillRect(X(e), ly + 3, 1.5, LANE - 7);
        ctx.font = "10px var(--font)";
        ctx.fillText(`AI ${(p.confidence * 100).toFixed(0)}%`, a + 3, ly - 1 + LANE);
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
      for (const t of view.tracks) {
        trackY.set(t.id, y);
        ctx.fillStyle = sel?.kind === "track" && sel.id === t.id ? C("--track-sel") : C("--track-bg");
        ctx.fillRect(0, y, W, TRACK - 1);
        y += TRACK;
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
            ctx.fillStyle = on ? "#ffd24a" : "rgba(255,255,255,0.85)";
            ctx.fillRect(hx - 3.5, top, 7, 7);
          };
          handle(X(a + geo.fade_in), active === "fadeIn");
          handle(X(b - geo.fade_out), active === "fadeOut");
        }
        if (active === "trimL" || active === "trimR") {
          ctx.fillStyle = "#ffd24a";
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
          ctx.fillStyle = "#ffd24a";
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
          ctx.fillStyle = "#101010";
          ctx.font = "10px var(--font)";
          const tag = c.performance_event_id ? "♪ " : c.take_id ? "● " : c.is_placeholder ? "◌ " : "";
          ctx.fillText(tag + (asset?.name ?? "?") + (asset?.missing ? " [MISSING]" : ""), x0 + 4, ty + 14);
          ctx.restore();
        }
        ctx.globalAlpha = 1;
        newHits.push({ x: x0, y: ty + 3 - scrollY, w, h: TRACK - 7, kind: "clip", id: c.id });
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
        ctx.fillStyle = "rgba(255, 200, 60, 0.8)";
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
          const ti = Math.floor((y + scrollY - RULER - SCENES - lanes.length * LANE) / TRACK);
          if (ti >= 0 && ti < tracks.length) setSel({ kind: "track", id: tracks[ti].id });
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
          </div>
          {lanes.map((l) => {
            const keys = Object.entries(view.bindings).filter(([, b]) => b.lane_id === l.id).map(([k]) => k.replace("key:Key", "").replace("key:", "").replace(/^midi:\d+:/, "♪"));
            return (
              <div key={l.id} className="lane-head" style={{ height: LANE }}>
                <span className="swatch" style={{ background: l.color }} />
                <span className="ellipsis">{l.name}</span>
                {keys.length > 0 && <span className="keyhint">{keys.join(" ")}</span>}
              </div>
            );
          })}
          {tracks.map((t) => (
            <TrackHead key={t.id} t={t} eng={eng} selected={sel?.kind === "track" && sel.id === t.id} onSelect={() => setSel({ kind: "track", id: t.id })} />
          ))}
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

function TrackHead({ t, eng, selected, onSelect }: { t: Track; eng: EngineHook; selected: boolean; onSelect: () => void }) {
  const meter = eng.tx?.meters[t.id] ?? 0;
  const db = meter > 0 ? 20 * Math.log10(meter) : -90;
  const pct = Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  const upd = (a: object) => eng.run("track.update", { id: t.id, ...a });
  return (
    <div className={"track-head" + (selected ? " selected" : "")} style={{ height: TRACK }} onMouseDown={onSelect}>
      <div className="th-row">
        <span className="swatch" style={{ background: catColor(t.category) }} />
        <span className="ellipsis th-name" title={`${t.name}\n${t.folder}`}>{t.name}</span>
        <button className={"tiny" + (t.mute ? " on-mute" : "")} title="Mute" onClick={() => upd({ mute: !t.mute })}>M</button>
        <button className={"tiny" + (t.solo ? " on-solo" : "")} title="Solo" onClick={() => upd({ solo: !t.solo })}>S</button>
        <button className={"tiny" + (t.armed ? " on-rec" : "")} title="Arm for recording" onClick={() => upd({ armed: !t.armed })}>●</button>
      </div>
      <div className="th-row">
        <input
          className="fader-h"
          type="range" min={-48} max={12} step={0.5}
          defaultValue={t.gain_db}
          key={t.gain_db}
          title={`${t.gain_db.toFixed(1)} dB`}
          onMouseUp={(e) => upd({ gain_db: Number((e.target as HTMLInputElement).value) })}
          onKeyUp={(e) => upd({ gain_db: Number((e.target as HTMLInputElement).value) })}
        />
        <span className="small mono">{t.gain_db > 0 ? "+" : ""}{t.gain_db.toFixed(1)}</span>
        <div className="meter-h"><div style={{ width: pct + "%" }} className={db > -1 ? "hot" : ""} /></div>
      </div>
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

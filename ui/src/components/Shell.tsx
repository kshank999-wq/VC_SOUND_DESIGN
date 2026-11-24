// The window's fixed bands (spec v2.1 › Layout): top deck, toolbar, status strip.
import { useState } from "react";
import { call } from "../api";
import type { EngineHook } from "../store";
import { STAGES, type Stage } from "../stage";
import { TOOLS, type Tool } from "../tools";
import { frameAtSample, framesToTc, sampleAtTc, tcAt } from "../tc";
import type { Selection } from "../types";

/** A key (`.key` from the tokens): a real button; `on` maps to aria-pressed. */
export function Key({ on, className = "", children, ...rest }: React.ButtonHTMLAttributes<HTMLButtonElement> & { on?: boolean }) {
  return (
    <button {...rest} className={"key " + className} aria-pressed={on === undefined ? undefined : on}>
      {children}
    </button>
  );
}

/** A labelled group of keys (silkscreen label above, as in VC Composer). */
export function Group({ label, children, className = "" }: { label: string; children: React.ReactNode; className?: string }) {
  return (
    <div className={"tgroup " + className}>
      <span className="label">{label}</span>
      <div className="tgroup-keys">{children}</div>
    </div>
  );
}

const fmtDb = (peak: number) => (peak > 0 ? 20 * Math.log10(peak) : -Infinity);

/** A segmented meter: `lit` of `n` segments (green → lemon → red). */
export function SegMeter({ db, n = 30, vertical = false }: { db: number; n?: number; vertical?: boolean }) {
  // −60 … 0 dBFS across the segments
  const lit = Number.isFinite(db) ? Math.round(Math.max(0, Math.min(1, (db + 60) / 60)) * n) : 0;
  return (
    <div className={"segmeter" + (vertical ? " v" : "")} role="meter" aria-valuemin={-60} aria-valuemax={0} aria-valuenow={Number.isFinite(db) ? Math.round(db) : -60}>
      {Array.from({ length: n }, (_, i) => {
        const zone = i < n * 0.68 ? "g" : i < n * 0.88 ? "a" : "r";
        return <span key={i} className={"seg " + zone + (i < lit ? " on" : "")} />;
      })}
    </div>
  );
}

interface TopDeckProps {
  eng: EngineHook;
  sel: Selection;
  menu: React.ReactNode;
}

/** Nameplate · transport · counter · session readouts · master meter. */
export function TopDeck({ eng, sel, menu }: TopDeckProps) {
  const { view, tx, rate, run } = eng;
  const [tcEdit, setTcEdit] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  if (!view?.open || !rate) return null;
  const pos = eng.livePosition();
  const start = view.scenes[0]?.start_sample ?? 0;
  const end = Math.max(start, ...view.scenes.map((s) => s.end_sample), ...view.clips.map((c) => c.timeline_start + c.length));
  const selCue = sel?.kind === "cue" ? view.cues.find((c) => c.id === sel.id) : null;
  const recording = !!tx?.recording;
  const loop = tx?.loop_range ?? null;
  const db = fmtDb(tx?.master_peak ?? 0);
  // 35 mm feet + frames (16 frames a foot) from the first scene
  const frames = Math.max(0, frameAtSample(rate, pos - start));
  const feet = `${Math.floor(frames / 16)}+${String(frames % 16).padStart(2, "0")}`;
  const span = loop ?? (selCue ? [selCue.start_sample, selCue.end_sample] as [number, number] : null);
  return (
    <header className="topdeck">
      <div className="panel nameplate">
        <div className="logo" aria-hidden>
          <svg viewBox="0 0 24 24" width="20" height="20"><path d="M3 12h2M7 8v8M11 5v14M15 8v8M19 10v4M21 12h0" stroke="currentColor" strokeWidth="2" strokeLinecap="round" fill="none" /></svg>
        </div>
        <div className="np-text">
          <span className="label">VC Sound Design</span>
          <span className="np-name ellipsis">
            {view.project.name}
            {view.dirty && <span className="dirty" title="Unsaved changes" aria-label="Unsaved changes" />}
          </span>
        </div>
        <div className="np-menu">
          <Key aria-label="Project menu" aria-expanded={menuOpen} onClick={() => setMenuOpen(!menuOpen)}>☰</Key>
          {menuOpen && <div className="menu" onMouseLeave={() => setMenuOpen(false)} onClick={() => setMenuOpen(false)}>{menu}</div>}
        </div>
      </div>

      <div className="panel transport-panel">
        <span className="label">Transport</span>
        <div className="tkeys">
          <Key className="tkey" aria-label="To start" title="To start (Home)" onClick={() => call("transport.locate", { sample: start })}>⏮</Key>
          <Key className="tkey" on={!!tx?.playing} aria-label={tx?.playing ? "Stop" : "Play"} title="Play / stop (Space)" onClick={() => call("transport.toggle")}>{tx?.playing ? "■" : "▶"}</Key>
          <Key className="tkey" aria-label="To end" title="To end" onClick={() => call("transport.locate", { sample: end })}>⏭</Key>
          <Key className="tkey" on={!!loop} aria-label="Loop" title={loop ? "Loop on (click to clear)" : "Loop: shift-drag in the ruler"} onClick={() => call("transport.loop", {})}>⟲</Key>
          <Key className={"tkey rec"} on={recording}
            disabled={!recording && !selCue}
            aria-label={recording ? "Stop recording" : "Record a take"}
            title={recording ? "Stop recording" : selCue ? `Record a take for ${selCue.code} (2 s pre-roll)` : "Select a cue to record a take for it"}
            onClick={() => recording ? run("record.stop", {}, "Take saved") : selCue && run("record.start", { cue_id: selCue.id, from: selCue.start_sample, preroll: rate.sr * 2 }, "Recording… (2 s pre-roll)")}>●</Key>
          <Key className="tkey" aria-label="Previous frame" title="Previous frame (←)" onClick={() => call("transport.step", { frames: -1 })}>◀</Key>
          <Key className="tkey" aria-label="Next frame" title="Next frame (→)" onClick={() => call("transport.step", { frames: 1 })}>▶</Key>
        </div>
      </div>

      <div className="panel counter">
        {tcEdit === null ? (
          <button className="readout tc-main" title="Click to type a timecode" onClick={() => setTcEdit(tcAt(rate, tx?.position ?? 0))}>{tcAt(rate, pos)}</button>
        ) : (
          <input className="readout tc-main" autoFocus value={tcEdit} aria-label="Go to timecode" onChange={(e) => setTcEdit(e.target.value)}
            onKeyDown={(e) => { e.stopPropagation(); if (e.key === "Enter") { const s = sampleAtTc(rate, tcEdit); if (s !== null) call("transport.locate", { sample: s }); setTcEdit(null); } if (e.key === "Escape") setTcEdit(null); }}
            onBlur={() => setTcEdit(null)} />
        )}
        <div className="counter-sub">
          <span className="readout sm" title="Feet + frames from the first scene">{feet}</span>
          <span className="readout sm" title={loop ? "Loop range" : "Selected cue"}>IN {span ? tcAt(rate, span[0]) : "—"} · OUT {span ? tcAt(rate, span[1]) : "—"}</span>
          <span className="readout sm">DUR {span ? framesToTc(rate, frameAtSample(rate, span[1] - span[0])) : "—"}</span>
        </div>
        <span className="pill fps">{view.project.fps_label} fps</span>
      </div>

      <div className="panel session">
        <span className="label">Picture</span>
        <span className="readout sm">{view.picture ? `${view.picture.label}` : "no picture"}</span>
        <span className="label">Rate</span>
        <span className="readout sm">{view.project.sample_rate / 1000} kHz</span>
        <span className="label">Loop</span>
        <span className="readout sm">{loop ? `${tcAt(rate, loop[0])} – ${tcAt(rate, loop[1])}` : "off"}</span>
      </div>

      <div className="panel master">
        <div className="master-head">
          <span className="label">Master</span>
          <span className="readout sm">{Number.isFinite(db) && db > -89 ? db.toFixed(1) : "−∞"} dBFS</span>
        </div>
        <SegMeter db={db} />
        <span className="label dimmer">peak</span>
      </div>
    </header>
  );
}

interface ToolbarProps {
  eng: EngineHook;
  stage: Stage;
  setStage: (s: Stage) => void;
  snapOn: boolean;
  setSnapOn: (v: boolean) => void;
  spotMode: boolean;
  setSpotMode: (v: boolean) => void;
  zoom: (dir: "in" | "out" | "fit" | "sel") => void;
  canZoomSel: boolean;
  onMixer: () => void;
  onExport: () => void;
  onBindings: () => void;
  tool: Tool;
  setTool: (t: Tool) => void;
}

/** Small line icons for the tools (currentColor). */
const TOOL_ICON: Record<Tool, React.ReactNode> = {
  select: <path d="M5 3l12 8-5 1.5L9.5 18z" />,
  trim: <path d="M8 4v16M16 4v16M4 12h4M16 12h4" />,
  fade: <path d="M4 18L20 6M4 18h16" />,
  split: <><circle cx="7" cy="7" r="2.5" /><circle cx="7" cy="17" r="2.5" /><path d="M9 8.5L20 16M9 15.5L20 8" /></>,
  pencil: <path d="M5 19l1-4L16 5l3 3L9 18zM14 7l3 3" />,
  hand: <path d="M8 12V6a1.5 1.5 0 013 0v5V4.5a1.5 1.5 0 013 0V11V6a1.5 1.5 0 013 0v7a6 6 0 01-6 6h-1a6 6 0 01-5-2.7L5 13.5a1.5 1.5 0 012.5-1.6z" />,
};

/** Stage bar · snap · spot · zoom · history · Mixer / Make cues / Populate / Export. */
export function Toolbar(p: ToolbarProps) {
  const { view, tx, run } = p.eng;
  if (!view?.open) return null;
  const uncued = view.markers.filter((m) => !m.cue_id).length;
  const open = tx?.open_spots ?? [];
  return (
    <nav className="panel toolbar-band" aria-label="Toolbar">
      <Group label="Stage · 1–6" className="stages">
        {STAGES.map((s, i) => (
          <Key key={s.id} on={p.stage === s.id} className="stage-key" title={`${s.label} (${i + 1})`} onClick={() => p.setStage(s.id)}>
            <span className={"led" + (p.stage === s.id ? " on" : "")} />{s.label}
          </Key>
        ))}
      </Group>
      <Group label="Tools">
        {TOOLS.map((t) => (
          <Key key={t.id} on={p.tool === t.id} className="tool-key" aria-label={t.label} title={`${t.label} (${t.key}) — ${t.hint}`} onClick={() => p.setTool(t.id)}>
            <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">{TOOL_ICON[t.id]}</svg>
          </Key>
        ))}
      </Group>
      <Group label="Snap">
        <Key on={p.snapOn} title="Snap edits to frames (hold Alt while dragging to override)" onClick={() => p.setSnapOn(!p.snapOn)}>Frames</Key>
        <span className="readout sm">{p.snapOn ? "1 fr" : "free"}</span>
      </Group>
      <Group label="Spot">
        <Key on={p.spotMode} className="spot-key" title="Live spotting: F P G B D N M while playing · Shift+key or Enter closes" onClick={() => p.setSpotMode(!p.spotMode)}>
          <span className={"led rec" + (p.spotMode ? " on" : "")} />Spot
        </Key>
        <Key title="Spot keys and MIDI inputs" onClick={p.onBindings}>Bindings</Key>
      </Group>
      {p.spotMode && (
        <div className="spot-banner" role="status">
          <span className="led rec on" />
          SPOT — {open.length ? <>open: {open.map((k) => k.replace("key:Key", "")).join(", ")} <span className="dim">(Enter closes)</span></> : "tap F P G B D N M while playing"}
        </div>
      )}
      <Group label="Zoom">
        <Key aria-label="Zoom out" title="Zoom out (−)" onClick={() => p.zoom("out")}>−</Key>
        <Key aria-label="Zoom in" title="Zoom in (=)" onClick={() => p.zoom("in")}>+</Key>
        <Key title="Fit the project" onClick={() => p.zoom("fit")}>Fit</Key>
        <Key title="Zoom to the selection" disabled={!p.canZoomSel} onClick={() => p.zoom("sel")}>Sel</Key>
      </Group>
      <Group label="History">
        <Key aria-label="Undo" disabled={!view.undo} title={view.undo ? `Undo ${view.undo} (⌘Z)` : "Nothing to undo"} onClick={() => run("undo")}>↶</Key>
        <Key aria-label="Redo" disabled={!view.redo} title={view.redo ? `Redo ${view.redo} (⇧⌘Z)` : "Nothing to redo"} onClick={() => run("redo")}>↷</Key>
      </Group>
      <span className="spacer" />
      <div className="tb-right">
        <Key title="Mixer in the bottom deck (⌘M; again to detach)" onClick={p.onMixer}>Mixer ⧉</Key>
        <Key title={uncued ? `Make cues from ${uncued} uncued marker${uncued === 1 ? "" : "s"}` : "Every marker is a cue"} disabled={!uncued}
          onClick={() => run("cues.build", {}, `${uncued} cue${uncued === 1 ? "" : "s"} made`)}>
          Make cues {uncued > 0 && <span className="count">{uncued}</span>}
        </Key>
        <Key title="Fill every cue without sound with placeholders" onClick={() => run("populate", {}, "Placeholders populated")}>Populate</Key>
        <Key className="primary" onClick={p.onExport}>Export</Key>
      </div>
    </nav>
  );
}

/** Sample rate · frame rate · DSP · dropouts · latency · device · counts. */
export function StatusStrip({ eng, onDevices }: { eng: EngineHook; onDevices: () => void }) {
  const { view, tx, rate } = eng;
  if (!view?.open || !rate) return null;
  return (
    <footer className="status-strip">
      <span>{view.project.sample_rate / 1000} kHz</span>
      <span>{view.project.fps_label} fps</span>
      <span>DSP {(Math.min(9.99, tx?.load ?? 0) * 100).toFixed(0)}%</span>
      <span className={(tx?.dropouts ?? 0) > 0 ? "warn-text" : ""}>dropouts {tx?.dropouts ?? 0}</span>
      <span>latency {((tx?.output_latency ?? 0) / rate.sr * 1000).toFixed(1)} ms</span>
      <button className={"device" + (tx?.device_warning ? " warn-text" : "")} title={(tx?.device_warning ?? "") + " — click for Audio & MIDI"} onClick={onDevices}>
        {tx?.device}{tx?.device_warning ? " ⚠" : ""}
      </button>
      <span className="spacer" />
      <span>{view.tracks.length} tracks · {view.clips.length} clips · {view.cues.length} cues</span>
      <span className="dim ellipsis root">{view.dirty ? "Unsaved changes" : "Saved"} · {view.root}</span>
    </footer>
  );
}

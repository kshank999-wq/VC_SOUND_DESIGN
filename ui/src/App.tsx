import { useCallback, useEffect, useRef, useState } from "react";
import { call, openMixerWindow } from "./api";
import { useEngine } from "./store";
import type { Asset, Selection } from "./types";
import { sampleAtTc, tcAt } from "./tc";
import { Timeline, zoomToRange, type Viewport } from "./components/Timeline";
import { Viewer } from "./components/Viewer";
import { Inspector } from "./components/Inspector";
import { LeftDock } from "./components/LeftDock";
import { CueChart, QaExport } from "./components/Chart";
import { Mixer } from "./components/Mixer";
import { Pads } from "./components/Pads";
import { ImportDialog, Welcome, type ImportKind } from "./components/Dialogs";
import { DevicesDialog } from "./components/Devices";

export function App() {
  const eng = useEngine();
  const { view, tx, rate, run, toasts } = eng;
  const [sel, setSel] = useState<Selection>(null);
  const [libSel, setLibSel] = useState<Asset | null>(null);
  const [vp, setVp] = useState<Viewport>({ start: -4800, spp: 400 });
  const [snapOn, setSnapOn] = useState(true);
  const [spotMode, setSpotMode] = useState(false);
  const [lower, setLower] = useState<"timeline" | "chart" | "mixer" | "qa">("timeline");
  const [menu, setMenu] = useState(false);
  const [imp, setImp] = useState<ImportKind>(null);
  const [perform, setPerform] = useState<string | null>(null);
  const [tcEdit, setTcEdit] = useState<string | null>(null);
  const [devices, setDevices] = useState(false);
  const centerRef = useRef<HTMLDivElement>(null);
  const opened = useRef<string | null>(null);

  // initial zoom to the first scene when a project opens
  useEffect(() => {
    if (!view?.open || !rate || opened.current === view.root) return;
    opened.current = view.root ?? null;
    const sc = view.scenes[0];
    const w = (centerRef.current?.clientWidth ?? 1000) - 252;
    if (sc) setVp(zoomToRange(rate, sc.start_sample, sc.end_sample, w));
    setSel(null);
  }, [view?.open, view?.root, rate]); // eslint-disable-line react-hooks/exhaustive-deps

  const gotoRange = useCallback((a: number, b: number) => {
    if (!rate) return;
    setVp(zoomToRange(rate, a, b, (centerRef.current?.clientWidth ?? 1000) - 252));
    setLower("timeline");
  }, [rate]);

  // ------------------------------------------------------------ keyboard
  useEffect(() => {
    const kd = async (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) return;
      if (!view?.open || !rate) return;
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.code === "KeyZ") { e.preventDefault(); run(e.shiftKey ? "redo" : "undo"); return; }
      if (mod && e.code === "KeyY") { e.preventDefault(); run("redo"); return; }
      if (mod && e.code === "KeyS") { e.preventDefault(); run("project.save", {}, "Saved"); return; }
      if (mod) return;
      if (perform) return; // pads own the keys
      switch (e.code) {
        case "Space":
          e.preventDefault();
          call("transport.toggle");
          return;
        case "ArrowLeft":
        case "ArrowRight": {
          e.preventDefault();
          const n = (e.code === "ArrowLeft" ? -1 : 1) * (e.shiftKey ? Math.round(rate.num / rate.den) : 1);
          call("transport.step", { frames: n });
          return;
        }
        case "Home":
          call("transport.locate", { sample: view.scenes[0]?.start_sample ?? 0 });
          return;
        case "Equal": setVp((v) => ({ ...v, spp: Math.max(0.5, v.spp / 1.5) })); return;
        case "Minus": setVp((v) => ({ ...v, spp: Math.min(48000, v.spp * 1.5) })); return;
        case "Delete":
        case "Backspace":
          if (sel?.kind === "clip") run("clip.delete", { id: sel.id }).then(() => setSel(null));
          if (sel?.kind === "marker") run("marker.delete", { id: sel.id }).then(() => setSel(null));
          return;
        case "Enter":
          if (spotMode) { e.preventDefault(); run("spot.finish", {}); }
          return;
        case "Escape":
          setSel(null);
          return;
      }
      if (spotMode && view.bindings["key:" + e.code] && !e.repeat) {
        e.preventDefault();
        const key = "key:" + e.code;
        if (e.shiftKey && tx?.open_spots.includes(key)) run("spot.finish", { key });
        else run("spot.tap", { key });
      }
    };
    window.addEventListener("keydown", kd);
    return () => window.removeEventListener("keydown", kd);
  }, [view, rate, run, sel, spotMode, perform, tx?.open_spots]);

  if (!view) return <div className="boot">Connecting to engine…</div>;
  if (!view.open || !rate)
    return (
      <>
        <Welcome eng={eng} onDevices={() => setDevices(true)} />
        {devices && <DevicesDialog eng={eng} onClose={() => setDevices(false)} />}
        <Toasts toasts={toasts} />
      </>
    );

  const pos = tx?.position ?? 0;
  const db = (tx?.master_peak ?? 0) > 0 ? 20 * Math.log10(tx!.master_peak) : -90;
  return (
    <div className="app">
      <header className="topbar">
        <div className="brand" onClick={() => setMenu(!menu)}>
          ☰ <b>{view.project.name}</b>{view.dirty ? " •" : ""}
          {menu && (
            <div className="menu" onMouseLeave={() => setMenu(false)}>
              <div onClick={() => run("project.save", {}, "Saved")}>Save <kbd>⌘S</kbd></div>
              <hr />
              <div onClick={() => setImp("picture")}>Import picture…</div>
              <div onClick={() => setImp("script")}>Import Film Studio breakdown…</div>
              <div onClick={() => setImp("score")}>Import Composer score…</div>
              <div onClick={() => setImp("relink")}>Relink missing media…</div>
              <hr />
              <div onClick={() => run("project.settings", { ai_enabled: !view.project.ai_enabled }, view.project.ai_enabled ? "AI assist off" : "Local AI assist on")}>{view.project.ai_enabled ? "✓ " : ""}Local AI timing assist</div>
              <div onClick={() => openMixerWindow()}>Open mixer window</div>
              <div onClick={() => setDevices(true)}>Audio & MIDI devices…</div>
              <hr />
              <div onClick={() => run("project.close")}>Close project</div>
            </div>
          )}
        </div>
        <div className="transport">
          <button title="Previous frame (←)" onClick={() => call("transport.step", { frames: -1 })}>⏮</button>
          <button className={tx?.playing ? "on" : ""} title="Play / stop (Space)" onClick={() => call("transport.toggle")}>{tx?.playing ? "■" : "▶"}</button>
          <button title="Next frame (→)" onClick={() => call("transport.step", { frames: 1 })}>⏭</button>
          <button className={tx?.loop_range ? "on" : ""} title="Loop: shift-drag in the ruler; click to clear" onClick={() => call("transport.loop", {})}>⟲</button>
          {tx?.recording && <span className="rec-dot">● REC</span>}
        </div>
        {tcEdit === null ? (
          <div className="tc mono" title="Click to type a timecode" onClick={() => setTcEdit(tcAt(rate, pos))}>{tcAt(rate, eng.livePosition())}</div>
        ) : (
          <input className="tc mono" autoFocus value={tcEdit} onChange={(e) => setTcEdit(e.target.value)}
            onKeyDown={(e) => { e.stopPropagation(); if (e.key === "Enter") { const s = sampleAtTc(rate, tcEdit); if (s !== null) call("transport.locate", { sample: s }); setTcEdit(null); } if (e.key === "Escape") setTcEdit(null); }}
            onBlur={() => setTcEdit(null)} />
        )}
        <span className="badge">{view.project.fps_label} fps</span>
        <span className="badge">{view.project.sample_rate / 1000} kHz</span>
        <span className="badge" title="Picture version">{view.picture ? `PIC ${view.picture.label}` : "no picture"}</span>
        <span className="spacer" />
        <button className={snapOn ? "on" : ""} title="Snap edits to frames" onClick={() => setSnapOn(!snapOn)}>Snap</button>
        <button className={spotMode ? "on spot" : ""} title="Live spotting: F footsteps (repeated), P prop, G hit, B ambience range, D design, N note, M music · Shift+key or Enter closes" onClick={() => setSpotMode(!spotMode)}>Spot</button>
        <button disabled={!view.undo} title={view.undo ? `Undo ${view.undo}` : ""} onClick={() => run("undo")}>↶</button>
        <button disabled={!view.redo} title={view.redo ? `Redo ${view.redo}` : ""} onClick={() => run("redo")}>↷</button>
        <button onClick={() => openMixerWindow()} title="Detach mixer to its own window">Mixer ⧉</button>
        <button className="primary" onClick={() => setLower("qa")}>Export</button>
      </header>

      <aside className="left"><LeftDock eng={eng} setSel={setSel} libSel={libSel} setLibSel={setLibSel} gotoRange={gotoRange} /></aside>

      <main className="center" ref={centerRef}>
        <div className="upper">
          <Viewer eng={eng} />
          {perform && <Pads eng={eng} cueId={perform} onClose={() => setPerform(null)} />}
        </div>
        <div className="lower">
          <div className="tabs">
            {(["timeline", "chart", "mixer", "qa"] as const).map((t) => (
              <button key={t} className={lower === t ? "active" : ""} onClick={() => setLower(t)}>
                {{ timeline: "Timeline", chart: `Cue chart (${view.cues.length})`, mixer: "Mixer", qa: "QA & Export" }[t]}
              </button>
            ))}
            {spotMode && <span className="spot-hint">SPOT MODE — {tx?.open_spots.length ? `open: ${tx.open_spots.map((k) => k.replace("key:Key", "")).join(", ")} (Enter closes)` : "tap F P G B D N M while playing"}</span>}
          </div>
          {lower === "timeline" && <Timeline eng={eng} vp={vp} setVp={setVp} sel={sel} setSel={setSel} snapOn={snapOn} />}
          {lower === "chart" && <CueChart eng={eng} setSel={(s) => { setSel(s); }} />}
          {lower === "mixer" && <Mixer eng={eng} />}
          {lower === "qa" && <QaExport eng={eng} />}
        </div>
      </main>

      <aside className="right"><Inspector eng={eng} sel={sel} setSel={setSel} libSel={libSel} onPerform={(id) => setPerform(id)} /></aside>

      <footer className="status">
        <span className={"device-btn" + (tx?.device_warning ? " warn" : "")} title={(tx?.device_warning ?? "") + " — click for Audio & MIDI"} onClick={() => setDevices(true)}>🔈 {tx?.device}{tx?.device_warning ? " ⚠" : ""}</span>
        <span>DSP {(Math.min(9.99, tx?.load ?? 0) * 100).toFixed(0)}%</span>
        <span className={(tx?.dropouts ?? 0) > 0 ? "warn" : ""}>dropouts {tx?.dropouts ?? 0}</span>
        <span>latency {rate ? ((tx?.output_latency ?? 0) / rate.sr * 1000).toFixed(1) : 0} ms</span>
        <span className="master">master <span className="meter-h big"><span style={{ width: Math.max(0, Math.min(100, ((db + 60) / 60) * 100)) + "%" }} className={db > -1 ? "hot" : ""} /></span> {db > -89 ? db.toFixed(1) : "-∞"} dB</span>
        <span className="spacer" />
        <span>{view.tracks.length} tracks · {view.clips.length} clips · {view.cues.length} cues</span>
        <span className="muted mono">{view.root}</span>
      </footer>
      <ImportDialog kind={imp} eng={eng} onClose={() => setImp(null)} />
      {devices && <DevicesDialog eng={eng} onClose={() => setDevices(false)} />}
      <Toasts toasts={toasts} />
    </div>
  );
}

function Toasts({ toasts }: { toasts: { id: number; text: string; kind: string }[] }) {
  return <div className="toasts">{toasts.map((t) => <div key={t.id} className={"toast " + t.kind}>{t.text}</div>)}</div>;
}

export function MixerWindow() {
  const eng = useEngine();
  return (
    <div className="mixer-window">
      <div className="panel-title">Mixer — {eng.view?.open ? eng.view.project.name : ""} <span className="mono muted">{eng.rate && tcAt(eng.rate, eng.livePosition())}</span></div>
      <Mixer eng={eng} detached />
    </div>
  );
}

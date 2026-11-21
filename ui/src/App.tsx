import { useCallback, useEffect, useRef, useState } from "react";
import { call, isTauri, openMixerWindow } from "./api";
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
import { PluginBrowser, PluginEditors } from "./components/Plugins";
import { UpdatesDialog, useUpdates } from "./components/Updates";
import { getLayout, resetLayout, saveLayout } from "./layout";
import { DEFAULTS, LIMITS } from "./layoutModel";

export function App() {
  const eng = useEngine();
  const { view, tx, rate, run, toasts } = eng;
  const [sel, setSel] = useState<Selection>(null);
  const [libSel, setLibSel] = useState<Asset | null>(null);
  const L0 = getLayout();
  const [vp, setVp] = useState<Viewport>({ start: -4800, spp: 400 });
  const [snapOn, setSnapOnRaw] = useState(L0.snap ?? true);
  const setSnapOn = (v: boolean) => {
    setSnapOnRaw(v);
    saveLayout({ snap: v });
  };
  const [leftW, setLeftW] = useState<number>(L0.leftW ?? DEFAULTS.leftW);
  const [rightW, setRightW] = useState<number>(L0.rightW ?? DEFAULTS.rightW);
  const [upperPct, setUpperPct] = useState<number>(L0.upperPct ?? DEFAULTS.upperPct);
  const [spotMode, setSpotMode] = useState(false);
  const [lower, setLowerRaw] = useState<"timeline" | "chart" | "mixer" | "qa">(L0.lowerTab ?? "timeline");
  const setLower = (t: "timeline" | "chart" | "mixer" | "qa") => {
    setLowerRaw(t);
    saveLayout({ lowerTab: t });
  };
  const [menu, setMenu] = useState(false);
  const [imp, setImp] = useState<ImportKind>(null);
  const [perform, setPerform] = useState<string | null>(null);
  const [tcEdit, setTcEdit] = useState<string | null>(null);
  const [devices, setDevices] = useState(false);
  const [plugins, setPlugins] = useState(false);
  const updates = useUpdates(eng);
  const [showUpdates, setShowUpdates] = useState(false);
  const centerRef = useRef<HTMLDivElement>(null);
  const opened = useRef<string | null>(null);

  // initial zoom to the first scene when a project opens
  useEffect(() => {
    if (!view?.open || !rate || opened.current === view.root) return;
    opened.current = view.root ?? null;
    const saved = view.root ? getLayout().projects?.[view.root]?.vp : undefined;
    const sc = view.scenes[0];
    const w = (centerRef.current?.clientWidth ?? 1000) - 252;
    if (saved) setVp(saved); // where you left this project's timeline
    else if (sc) setVp(zoomToRange(rate, sc.start_sample, sc.end_sample, w));
    setSel(null);
  }, [view?.open, view?.root, rate]); // eslint-disable-line react-hooks/exhaustive-deps

  // remember the timeline view per project
  useEffect(() => {
    if (view?.open && view.root && opened.current === view.root) saveLayout({ projects: { [view.root]: { vp } } });
  }, [vp]); // eslint-disable-line react-hooks/exhaustive-deps

  // reopen the detached mixer if it was open last time (desktop only: browsers block unrequested popups)
  useEffect(() => {
    if (view?.open && getLayout().mixerOpen && isTauri) openMixerWindow();
  }, [view?.open]); // eslint-disable-line react-hooks/exhaustive-deps

  const resetWindowLayout = async () => {
    await resetLayout();
    setLeftW(DEFAULTS.leftW);
    setRightW(DEFAULTS.rightW);
    setUpperPct(DEFAULTS.upperPct);
    setLowerRaw("timeline");
    setSnapOnRaw(true);
    eng.toast("Window layout reset");
  };

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
    <div className="app" style={{ gridTemplateColumns: `${leftW}px minmax(0, 1fr) ${rightW}px` }}>
      <header className="topbar">
        <div className="brand" onClick={() => setMenu(!menu)}>
          ☰ <b>{view.project.name}</b>{view.dirty ? " •" : ""}
          {menu && (
            <div className="menu" onMouseLeave={() => setMenu(false)}>
              <div onClick={() => run("project.save", {}, "Saved")}>Save <kbd>⌘S</kbd></div>
              <hr />
              <div onClick={() => setImp("picture")}>Import picture…</div>
              <div onClick={() => setImp("aaf")}>Import AAF…</div>
              <div onClick={() => setImp("otio")}>Import OTIO…</div>
              <div onClick={() => setImp("script")}>Import Film Studio breakdown…</div>
              <div onClick={() => setImp("score")}>Import Composer score…</div>
              <div onClick={() => setImp("relink")}>Relink missing media…</div>
              <hr />
              <div onClick={() => run("project.settings", { ai_enabled: !view.project.ai_enabled }, view.project.ai_enabled ? "AI assist off" : "Local AI assist on")}>{view.project.ai_enabled ? "✓ " : ""}Local AI timing assist</div>
              <div onClick={() => openMixerWindow()}>Open mixer window</div>
              <div onClick={resetWindowLayout}>Reset window layout</div>
              {updates.enabled && <div onClick={() => setShowUpdates(true)}>Check for updates…{updates.phase === "available" ? " ●" : ""}</div>}
              {updates.info && <div className="muted small" style={{ cursor: "default" }}>Version {updates.info.version}</div>}
              <div onClick={() => setDevices(true)}>Audio & MIDI devices…</div>
              <div onClick={() => setPlugins(true)}>Plugins…</div>
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

      <aside className="left"><LeftDock eng={eng} setSel={setSel} libSel={libSel} setLibSel={setLibSel} gotoRange={gotoRange} initialTab={L0.leftTab} onTab={(t) => saveLayout({ leftTab: t })} /></aside>
      <Splitter dir="v" style={{ left: leftW - 3 }} title="Drag to resize the left panel (double-click: default)"
        onDrag={(x) => setLeftW(clamp(x, LIMITS.leftW))} onDone={() => saveLayout({ leftW })} onReset={() => { setLeftW(DEFAULTS.leftW); saveLayout({ leftW: DEFAULTS.leftW }); }} />
      <Splitter dir="v" style={{ right: rightW - 3 }} title="Drag to resize the inspector (double-click: default)"
        onDrag={(x) => setRightW(clamp(window.innerWidth - x, LIMITS.rightW))} onDone={() => saveLayout({ rightW })} onReset={() => { setRightW(DEFAULTS.rightW); saveLayout({ rightW: DEFAULTS.rightW }); }} />

      <main className="center" ref={centerRef} style={{ gridTemplateRows: `minmax(100px, ${upperPct}%) 1fr` }}>
        <Splitter dir="h" style={{ top: `calc(${upperPct}% - 3px)` }} title="Drag to resize picture / timeline (double-click: default)"
          onDrag={(_, y) => {
            const r = centerRef.current!.getBoundingClientRect();
            setUpperPct(clamp(((y - r.top) / r.height) * 100, LIMITS.upperPct));
          }}
          onDone={() => saveLayout({ upperPct })} onReset={() => { setUpperPct(DEFAULTS.upperPct); saveLayout({ upperPct: DEFAULTS.upperPct }); }} />
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
          {lower === "timeline" && (
            <Timeline key={view.root} eng={eng} vp={vp} setVp={setVp} sel={sel} setSel={setSel} snapOn={snapOn}
              initialScrollY={(view.root && getLayout().projects?.[view.root]?.scrollY) || 0}
              onScrollY={(y) => view.root && saveLayout({ projects: { [view.root]: { scrollY: y } } })} />
          )}
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
      {plugins && <PluginBrowser onClose={() => setPlugins(false)} />}
      <PluginEditors eng={eng} />
      {showUpdates && <UpdatesDialog u={updates} onClose={() => setShowUpdates(false)} />}
      {updates.phase === "available" && !showUpdates && (
        <button className="update-pill" onClick={() => setShowUpdates(true)}>Update {updates.update?.version} available</button>
      )}
      <Toasts toasts={toasts} />
    </div>
  );
}

function clamp(v: number, [lo, hi]: readonly [number, number]) {
  return Math.round(Math.max(lo, Math.min(hi, v)) * 10) / 10;
}

/** Draggable divider between panels. `onDrag` gets pointer client x/y. */
function Splitter({ dir, style, title, onDrag, onDone, onReset }: { dir: "v" | "h"; style: React.CSSProperties; title: string; onDrag: (x: number, y: number) => void; onDone: () => void; onReset: () => void }) {
  const dragging = useRef(false);
  return (
    <div
      className={`splitter ${dir}`}
      style={style}
      title={title}
      onPointerDown={(e) => {
        dragging.current = true;
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
        document.body.classList.add(dir === "v" ? "resizing-v" : "resizing-h");
      }}
      onPointerMove={(e) => dragging.current && onDrag(e.clientX, e.clientY)}
      onPointerUp={() => {
        if (!dragging.current) return;
        dragging.current = false;
        document.body.classList.remove("resizing-v", "resizing-h");
        onDone();
      }}
      onDoubleClick={onReset}
    />
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
      <PluginEditors eng={eng} />
    </div>
  );
}

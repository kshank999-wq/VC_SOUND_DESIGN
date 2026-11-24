import { useCallback, useEffect, useRef, useState } from "react";
import { call, isTauri, openMixerWindow } from "./api";
import { useEngine, type EngineHook } from "./store";
import type { Asset, EventType, Selection } from "./types";
import { tcAt } from "./tc";
import { Timeline, zoomToRange, type Viewport } from "./components/Timeline";
import { Viewer } from "./components/Viewer";
import { Inspector } from "./components/Inspector";
import { LeftDock } from "./components/LeftDock";
import { CueChart, QaExport } from "./components/Chart";
import { Mixer } from "./components/Mixer";
import { ImportDialog, Welcome, type ImportKind } from "./components/Dialogs";
import { DevicesDialog } from "./components/Devices";
import { PluginBrowser, PluginEditors } from "./components/Plugins";
import { UpdatesDialog, useUpdates } from "./components/Updates";
import { BottomDeck, mixerWasDetached } from "./components/Deck";
import { Key, StatusStrip, TopDeck, Toolbar } from "./components/Shell";
import { getLayout, resetLayout, saveLayout } from "./layout";
import { DEFAULTS, LIMITS } from "./layoutModel";
import { STAGE_LIMITS, stageForKey, stageLayout, type CenterTab, type DeckTab, type LeftTab, type Stage, type StageLayout } from "./stage";

/** Width of the timeline's track-header column (for zoom-to-fit). */
const HEADS = 252;

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

  // ---- the workflow stage and its layout (each stage remembers its own changes)
  const [stage, setStageRaw] = useState<Stage>(L0.stage ?? "spot");
  const [saved, setSaved] = useState(L0.stages ?? {});
  const lay: StageLayout = stageLayout(stage, saved[stage]);
  const setLay = (patch: Partial<StageLayout>) => {
    setSaved((s) => ({ ...s, [stage]: { ...s[stage], ...patch } }));
    saveLayout({ stages: { [stage]: patch } });
  };
  const setStage = (s: Stage) => {
    setStageRaw(s);
    saveLayout({ stage: s });
  };

  const [spotMode, setSpotMode] = useState(false);
  const [imp, setImp] = useState<ImportKind>(null);
  const [perform, setPerform] = useState<string | null>(null);
  const [devices, setDevices] = useState(false);
  const [plugins, setPlugins] = useState(false);
  const [bindings, setBindings] = useState(false);
  const [mixerDetached, setMixerDetached] = useState(() => mixerWasDetached() && isTauri);
  const updates = useUpdates(eng);
  const [showUpdates, setShowUpdates] = useState(false);
  const centerRef = useRef<HTMLDivElement>(null);
  const opened = useRef<string | null>(null);

  const timelineWidth = () => (centerRef.current?.clientWidth ?? 1000) - HEADS;

  // initial zoom to the first scene when a project opens
  useEffect(() => {
    if (!view?.open || !rate || opened.current === view.root) return;
    opened.current = view.root ?? null;
    const savedVp = view.root ? getLayout().projects?.[view.root]?.vp : undefined;
    const sc = view.scenes[0];
    if (savedVp) setVp(savedVp); // where you left this project's timeline
    else if (sc) setVp(zoomToRange(rate, sc.start_sample, sc.end_sample, timelineWidth()));
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
    setSaved({});
    setSnapOnRaw(true);
    eng.toast("Window layout reset (every stage back to its default)");
  };

  const gotoRange = useCallback((a: number, b: number) => {
    if (!rate) return;
    setVp(zoomToRange(rate, a, b, timelineWidth()));
    setLay({ center: "timeline" });
  }, [rate, stage]); // eslint-disable-line react-hooks/exhaustive-deps

  const selRange = (): [number, number] | null => {
    if (!view || !sel) return null;
    if (sel.kind === "cue") { const c = view.cues.find((x) => x.id === sel.id); return c ? [c.start_sample, Math.max(c.end_sample, c.start_sample + 1)] : null; }
    if (sel.kind === "marker") { const m = view.markers.find((x) => x.id === sel.id); return m ? [m.start_sample, Math.max(m.end_sample, m.start_sample + 1)] : null; }
    if (sel.kind === "clip") { const c = view.clips.find((x) => x.id === sel.id); return c ? [c.timeline_start, c.timeline_start + c.length] : null; }
    return null;
  };
  const zoom = (dir: "in" | "out" | "fit" | "sel") => {
    if (!view || !rate) return;
    if (dir === "in") setVp((v) => ({ ...v, spp: Math.max(0.5, v.spp / 1.5) }));
    if (dir === "out") setVp((v) => ({ ...v, spp: Math.min(48000, v.spp * 1.5) }));
    if (dir === "fit" && view.scenes.length) gotoRange(Math.min(...view.scenes.map((s) => s.start_sample)), Math.max(...view.scenes.map((s) => s.end_sample)));
    const r = dir === "sel" ? selRange() : null;
    if (r) gotoRange(r[0], r[1]);
  };

  const onMixer = () => {
    if (lay.deckTab === "mixer" && lay.deckOpen && !mixerDetached) {
      openMixerWindow();
      setMixerDetached(true);
    } else setLay({ deckTab: "mixer", deckOpen: true });
  };
  const startPerform = (id: string) => {
    setPerform(id);
    setLay({ deckTab: "pads", deckOpen: true });
  };

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
      if (mod && e.code === "KeyM") { e.preventDefault(); onMixer(); return; }
      if (mod) return;
      if (perform) return; // pads own the keys
      const st = !spotMode && !e.shiftKey && !e.altKey ? stageForKey(e.code) : null;
      if (st) { e.preventDefault(); setStage(st); return; }
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
        case "Equal": zoom("in"); return;
        case "Minus": zoom("out"); return;
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
        return;
      }
      // P: picture docked / hidden (outside Spot mode, whose keys include P)
      if (e.code === "KeyP" && !spotMode) setLay({ pictureH: lay.pictureH > 0 ? 0 : (stageLayout(stage).pictureH || 300) });
    };
    window.addEventListener("keydown", kd);
    return () => window.removeEventListener("keydown", kd);
  });

  if (!view) return <div className="boot">Connecting to engine…</div>;
  if (!view.open || !rate)
    return (
      <div className="app-bare">
        <Welcome eng={eng} onDevices={() => setDevices(true)} />
        {devices && <DevicesDialog eng={eng} onClose={() => setDevices(false)} />}
        <Toasts toasts={toasts} />
      </div>
    );

  const center: CenterTab = lay.center;
  const showPicture = center === "timeline" && lay.pictureH > 0;
  const menu = (
    <>
      <div role="menuitem" onClick={() => run("project.save", {}, "Saved")}>Save <kbd>⌘S</kbd></div>
      <hr />
      <div role="menuitem" onClick={() => setImp("picture")}>Import picture…</div>
      <div role="menuitem" onClick={() => setImp("aaf")}>Import AAF…</div>
      <div role="menuitem" onClick={() => setImp("otio")}>Import OTIO…</div>
      <div role="menuitem" onClick={() => setImp("script")}>Import Film Studio breakdown…</div>
      <div role="menuitem" onClick={() => setImp("score")}>Import Composer score…</div>
      <div role="menuitem" onClick={() => setImp("relink")}>Relink missing media…</div>
      <hr />
      <div role="menuitem" onClick={() => run("project.settings", { ai_enabled: !view.project.ai_enabled }, view.project.ai_enabled ? "AI assist off" : "Local AI assist on")}>{view.project.ai_enabled ? "✓ " : ""}Local AI timing assist</div>
      <div role="menuitem" onClick={() => { openMixerWindow(); setMixerDetached(true); }}>Open mixer window</div>
      <div role="menuitem" onClick={resetWindowLayout}>Reset window layout</div>
      {updates.enabled && <div role="menuitem" onClick={() => setShowUpdates(true)}>Check for updates…{updates.phase === "available" ? " ●" : ""}</div>}
      {updates.info && <div className="dim small" style={{ cursor: "default" }}>Version {updates.info.version}</div>}
      <div role="menuitem" onClick={() => setDevices(true)}>Audio & MIDI devices…</div>
      <div role="menuitem" onClick={() => setBindings(true)}>Spot bindings…</div>
      <div role="menuitem" onClick={() => setPlugins(true)}>Plugins…</div>
      <hr />
      <div role="menuitem" onClick={() => run("project.close")}>Close project</div>
    </>
  );

  return (
    <div className="app" data-stage={stage}>
      <TopDeck eng={eng} sel={sel} menu={menu} />
      <Toolbar eng={eng} stage={stage} setStage={setStage} snapOn={snapOn} setSnapOn={setSnapOn}
        spotMode={spotMode} setSpotMode={setSpotMode} zoom={zoom} canZoomSel={!!selRange()}
        onMixer={onMixer} onExport={() => setLay({ center: "qa" })} onBindings={() => setBindings(true)} />

      <div className="main-row">
        <aside className="panel rack" style={{ width: leftW }} aria-label="Left rack">
          <LeftDock key={stage} eng={eng} setSel={setSel} libSel={libSel} setLibSel={setLibSel} gotoRange={gotoRange}
            initialTab={lay.leftTab} onTab={(t: LeftTab) => setLay({ leftTab: t })} />
        </aside>
        <Splitter dir="v" title="Drag to resize the left rack (double-click: default)"
          onDrag={(x) => setLeftW(clamp(x - 8, LIMITS.leftW))} onDone={() => saveLayout({ leftW })} onReset={() => { setLeftW(DEFAULTS.leftW); saveLayout({ leftW: DEFAULTS.leftW }); }} />

        <main className="panel center-col" ref={centerRef}>
          <div className="center-tabs" role="tablist">
            {(["timeline", "chart", "qa"] as const).map((t) => (
              <button key={t} role="tab" aria-selected={center === t} className={"ctab" + (center === t ? " active" : "")} onClick={() => setLay({ center: t })}>
                {{ timeline: "Timeline", chart: `Cue chart (${view.cues.length})`, qa: "QA & Export" }[t]}
              </button>
            ))}
            <span className="spacer" />
            {center === "timeline" && <span className="dim small mono">{tcAt(rate, Math.max(0, vp.start))} · {vp.spp.toFixed(1)} smp/px</span>}
            {center === "timeline" && (
              <Key on={showPicture} title="Show / hide the picture (P)" onClick={() => setLay({ pictureH: lay.pictureH > 0 ? 0 : (stageLayout(stage).pictureH || 300) })}>Picture</Key>
            )}
          </div>
          {showPicture && (
            <>
              <div className="picture" style={{ height: lay.pictureH }}>
                <Viewer eng={eng} />
              </div>
              <Splitter dir="h" title="Drag to resize the picture (double-click: this stage's default)"
                onDrag={(_, y) => { const r = centerRef.current!.getBoundingClientRect(); setLay({ pictureH: clamp(y - r.top - 36, [80, STAGE_LIMITS.pictureH[1]]) }); }}
                onDone={() => {}} onReset={() => setLay({ pictureH: stageLayout(stage).pictureH || 300 })} />
            </>
          )}
          <div className="center-body">
            {center === "timeline" && (
              <Timeline key={view.root} eng={eng} vp={vp} setVp={setVp} sel={sel} setSel={setSel} snapOn={snapOn}
                laneMode={lay.lanes} onLaneMode={(m) => setLay({ lanes: m })}
                initialScrollY={(view.root && getLayout().projects?.[view.root]?.scrollY) || 0}
                onScrollY={(y) => view.root && saveLayout({ projects: { [view.root]: { scrollY: y } } })} />
            )}
            {center === "chart" && <CueChart eng={eng} setSel={(s) => { setSel(s); }} />}
            {center === "qa" && <QaExport eng={eng} />}
          </div>
        </main>

        <Splitter dir="v" title="Drag to resize the inspector (double-click: default)"
          onDrag={(x) => setRightW(clamp(window.innerWidth - x - 8, LIMITS.rightW))} onDone={() => saveLayout({ rightW })} onReset={() => { setRightW(DEFAULTS.rightW); saveLayout({ rightW: DEFAULTS.rightW }); }} />
        <aside className="panel inspector-col" style={{ width: rightW }} aria-label="Inspector">
          <Inspector eng={eng} sel={sel} setSel={setSel} libSel={libSel} onPerform={startPerform} />
        </aside>
      </div>

      {lay.deckOpen && (
        <Splitter dir="h" title="Drag to resize the bottom deck (double-click: this stage's default)"
          onDrag={(_, y) => setLay({ deckH: clamp(window.innerHeight - y - 26, STAGE_LIMITS.deckH) })}
          onDone={() => {}} onReset={() => setLay({ deckH: stageLayout(stage).deckH })} />
      )}
      <BottomDeck eng={eng} tab={lay.deckTab} open={lay.deckOpen} height={lay.deckH}
        setTab={(t: DeckTab) => setLay({ deckTab: t })} setOpen={(o) => setLay({ deckOpen: o })}
        libSel={libSel} performCue={perform} onPerformClose={() => setPerform(null)}
        mixerDetached={mixerDetached} setMixerDetached={setMixerDetached} />
      <StatusStrip eng={eng} onDevices={() => setDevices(true)} />

      <ImportDialog kind={imp} eng={eng} onClose={() => setImp(null)} />
      {devices && <DevicesDialog eng={eng} onClose={() => setDevices(false)} />}
      {bindings && <BindingsDialog eng={eng} onClose={() => setBindings(false)} />}
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
function Splitter({ dir, title, onDrag, onDone, onReset }: { dir: "v" | "h"; title: string; onDrag: (x: number, y: number) => void; onDone: () => void; onReset: () => void }) {
  const dragging = useRef(false);
  return (
    <div
      className={`splitter ${dir}`}
      role="separator"
      aria-orientation={dir === "v" ? "vertical" : "horizontal"}
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

/** Spot keys: which marker lane and kind each key spots (spot.bindings / spot.bind). */
function BindingsDialog({ eng, onClose }: { eng: EngineHook; onClose: () => void }) {
  const { view, run } = eng;
  const [newKey, setNewKey] = useState("");
  if (!view) return null;
  const entries = Object.entries(view.bindings).filter(([k]) => k.startsWith("key:")).sort((a, b) => a[0].localeCompare(b[0]));
  const bind = (key: string, b: { lane_id: string; kind: EventType; label: string; identity_id: string | null }) =>
    run("spot.bind", { key, lane: b.lane_id, kind: b.kind, label: b.label, identity_id: b.identity_id });
  const add = () => {
    const ch = newKey.trim().toUpperCase();
    if (!/^[A-Z]$/.test(ch) || !view.lanes[0]) return eng.toast("Type one letter A–Z", "error");
    bind("key:Key" + ch, { lane_id: view.lanes[0].id, kind: "point", label: view.lanes[0].name, identity_id: null }).then(() => setNewKey(""));
  };
  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal bindings" role="dialog" aria-label="Spot bindings" onClick={(e) => e.stopPropagation()}>
        <div className="panel-title">Spot bindings</div>
        <div className="small dim">In Spot mode, each key drops a marker of its kind in its lane at the playhead. Ranges and repeated runs stay open until Enter or Shift+key.</div>
        <table>
          <thead><tr><th>Key</th><th>Lane</th><th>Kind</th><th>Label</th><th /></tr></thead>
          <tbody>
            {entries.map(([k, b]) => (
              <tr key={k}>
                <td className="mono">{k.replace("key:Key", "").replace("key:", "")}</td>
                <td><select value={b.lane_id} onChange={(e) => bind(k, { ...b, lane_id: e.target.value })}>{view.lanes.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}</select></td>
                <td><select value={b.kind} onChange={(e) => bind(k, { ...b, kind: e.target.value as EventType })}>{(["point", "range", "repeated"] as const).map((x) => <option key={x}>{x}</option>)}</select></td>
                <td><input defaultValue={b.label} onBlur={(e) => e.target.value !== b.label && bind(k, { ...b, label: e.target.value })} /></td>
                <td><button className="link" onClick={() => run("spot.bind", { key: k, remove: true })}>Remove</button></td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="btns">
          <input placeholder="Letter" maxLength={1} value={newKey} onChange={(e) => setNewKey(e.target.value)} style={{ width: 70 }} onKeyDown={(e) => { e.stopPropagation(); if (e.key === "Enter") add(); }} />
          <button onClick={add}>Add key</button>
          <span className="spacer" />
          <button className="primary" onClick={onClose}>Done</button>
        </div>
      </div>
    </div>
  );
}

function Toasts({ toasts }: { toasts: { id: number; text: string; kind: string }[] }) {
  return <div className="toasts" role="status">{toasts.map((t) => <div key={t.id} className={"toast " + t.kind}>{t.text}</div>)}</div>;
}

export function MixerWindow() {
  const eng = useEngine();
  return (
    <div className="mixer-window app">
      <div className="panel-title">Mixer — {eng.view?.open ? eng.view.project.name : ""} <span className="mono dim">{eng.rate && tcAt(eng.rate, eng.livePosition())}</span></div>
      <Mixer eng={eng} detached />
      <PluginEditors eng={eng} />
    </div>
  );
}

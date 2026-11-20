import { useRef, useState } from "react";
import type { EngineHook } from "../store";
import { catColor, type Bus, type Track } from "../types";
import { ModeSelect, useAutoControl, type AutoMode } from "./AutoControl";
import { InsertRack } from "./Plugins";

function Meter({ v }: { v: number }) {
  const db = v > 0 ? 20 * Math.log10(v) : -90;
  const pct = Math.max(0, Math.min(100, ((db + 60) / 66) * 100));
  return (
    <div className="meter-v" title={db > -89 ? `${db.toFixed(1)} dBFS` : "-∞"}>
      <div style={{ height: pct + "%" }} className={db > -0.5 ? "hot" : db > -12 ? "warm" : ""} />
    </div>
  );
}

const MIN_DB = -60;
const MAX_DB = 12;
const toPos = (db: number) => (Math.max(MIN_DB, Math.min(MAX_DB, db)) - MIN_DB) / (MAX_DB - MIN_DB);

/** Vertical fader drawn in HTML (native vertical range inputs are not
 *  rendered consistently by every webview). Drag, wheel, double-click = 0 dB.
 *  Reports begin/move/end so automation can be written while it moves. */
function Fader({ db, onBegin, onMove, onEnd }: { db: number; onBegin: (v: number) => void; onMove: (v: number) => void; onEnd: (v: number) => void }) {
  const [live, setLive] = useState<number | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const v = live ?? db;
  const fromY = (y: number) => {
    const r = ref.current!.getBoundingClientRect();
    const t = 1 - Math.max(0, Math.min(1, (y - r.top) / r.height));
    return Math.round((MIN_DB + t * (MAX_DB - MIN_DB)) * 2) / 2;
  };
  return (
    <div
      className="fader"
      ref={ref}
      title={`${v.toFixed(1)} dB (double-click: 0 dB)`}
      onDoubleClick={() => onEnd(0)}
      onWheel={(e) => onEnd(Math.max(MIN_DB, Math.min(MAX_DB, db - Math.sign(e.deltaY) * 0.5)))}
      onPointerDown={(e) => {
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
        const nv = fromY(e.clientY);
        setLive(nv);
        onBegin(nv);
      }}
      onPointerMove={(e) => {
        if (live === null) return;
        const nv = fromY(e.clientY);
        setLive(nv);
        onMove(nv);
      }}
      onPointerUp={() => {
        if (live !== null) onEnd(live);
        setLive(null);
      }}
    >
      <div className="fader-zero" style={{ bottom: toPos(0) * 100 + "%" }} />
      <div className="fader-thumb" style={{ bottom: `calc(${toPos(v) * 100}% - 7px)` }} />
    </div>
  );
}

/** Horizontal pan/balance control with touch begin/move/end. */
function PanKnob({ value, onBegin, onMove, onEnd, title }: { value: number; onBegin: (v: number) => void; onMove: (v: number) => void; onEnd: (v: number) => void; title: string }) {
  const held = useRef(false);
  const [live, setLive] = useState<number | null>(null);
  const v = live ?? value;
  const read = (e: React.SyntheticEvent) => Number((e.target as HTMLInputElement).value);
  return (
    <input className="pan" type="range" min={-1} max={1} step={0.01} value={v} title={`${title} ${v === 0 ? "C" : (v < 0 ? "L " : "R ") + Math.round(Math.abs(v) * 100)}`}
      onPointerDown={(e) => { held.current = true; const nv = read(e); setLive(nv); onBegin(nv); }}
      onChange={(e) => { const nv = read(e); setLive(nv); if (held.current) onMove(nv); }}
      onPointerUp={(e) => { held.current = false; const nv = read(e); setLive(null); onEnd(nv); }}
      onDoubleClick={() => onEnd(0)} />
  );
}

/** Shown when a strip's level/pan is driven by automation (the fader alone doesn't tell you). */
function AutoBadge({ eng, id }: { eng: EngineHook; id: string }) {
  const pts = (eng.view?.automation ?? []).filter((p) => p.owner_id === id);
  if (!pts.length) return <div className="auto-badge off">&nbsp;</div>;
  const kinds = [...new Set(pts.map((p) => (p.param === "pan" ? "pan" : "vol")))].join("+");
  return <div className="auto-badge" title={`${pts.length} automation points (${kinds})`}>A {kinds}</div>;
}

function Strip({ t, eng }: { t: Track; eng: EngineHook }) {
  const upd = (a: object) => eng.run("track.update", { id: t.id, ...a });
  const mode = (t.automation_mode ?? "read") as AutoMode;
  const vol = useAutoControl(eng, t.id, "volume_db", mode, t.gain_db, true);
  const pan = useAutoControl(eng, t.id, "pan", mode, t.pan, true);
  return (
    <div className={"strip" + (vol.writing || pan.writing ? " is-writing" : "")}>
      <div className="strip-color" style={{ background: catColor(t.category) }} />
      <div className="strip-name" title={t.name}>{t.name.split(" • ").slice(1).join(" • ") || t.name}</div>
      <AutoBadge eng={eng} id={t.id} />
      <div className="small muted ellipsis">{t.folder.split("/")[0]}</div>
      <InsertRack eng={eng} ownerId={t.id} inserts={t.inserts} compact />
      <PanKnob value={pan.display} onBegin={pan.begin} onMove={pan.move} onEnd={pan.end} title="pan" />
      <div className="strip-mid">
        <Fader db={vol.display} onBegin={vol.begin} onMove={vol.move} onEnd={vol.end} />
        <Meter v={eng.tx?.meters[t.id] ?? 0} />
      </div>
      <div className="mono small">{vol.display.toFixed(1)}</div>
      <div className="btns tight">
        <button className={"tiny" + (t.mute ? " on-mute" : "")} onClick={() => upd({ mute: !t.mute })}>M</button>
        <button className={"tiny" + (t.solo ? " on-solo" : "")} onClick={() => upd({ solo: !t.solo })}>S</button>
      </div>
      <ModeSelect eng={eng} ownerId={t.id} mode={mode} />
      <select className="small" value={t.bus_id} onChange={(e) => upd({ bus_id: e.target.value })}>
        {eng.view!.buses.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
      </select>
    </div>
  );
}

function BusStrip({ b, eng }: { b: Bus; eng: EngineHook }) {
  const upd = (a: object) => eng.run("bus.update", { id: b.id, ...a });
  const mode = (b.automation_mode ?? "read") as AutoMode;
  const vol = useAutoControl(eng, b.id, "volume_db", mode, b.gain_db, false);
  const bal = useAutoControl(eng, b.id, "pan", mode, 0, false);
  return (
    <div className={"strip bus" + (b.parent_id ? "" : " master") + (vol.writing || bal.writing ? " is-writing" : "")}>
      <div className="strip-name">{b.name}</div>
      <AutoBadge eng={eng} id={b.id} />
      <div className="small muted">{b.parent_id ? "stem bus" : "master"}</div>
      <InsertRack eng={eng} ownerId={b.id} inserts={b.inserts ?? []} compact />
      <PanKnob value={bal.display} onBegin={bal.begin} onMove={bal.move} onEnd={bal.end} title="balance (automation only)" />
      <div className="strip-mid">
        <Fader db={vol.display} onBegin={vol.begin} onMove={vol.move} onEnd={vol.end} />
        <Meter v={b.parent_id ? eng.tx?.meters[b.id] ?? 0 : eng.tx?.master_peak ?? 0} />
      </div>
      <div className="mono small">{vol.display.toFixed(1)}</div>
      <button className={"tiny" + (b.mute ? " on-mute" : "")} onClick={() => upd({ mute: !b.mute })}>M</button>
      <ModeSelect eng={eng} ownerId={b.id} mode={mode} />
    </div>
  );
}

/** Full mixer: track strips grouped by bus, bus/stem strips and master.
 *  Runs docked or in its own window (second monitor). */
export function Mixer({ eng, detached }: { eng: EngineHook; detached?: boolean }) {
  const { view } = eng;
  if (!view?.open) return <div className="pad muted">No project open.</div>;
  const buses = view.buses.filter((b) => b.parent_id);
  const master = view.buses.filter((b) => !b.parent_id);
  return (
    <div className={"mixer" + (detached ? " detached" : "")}>
      <div className="mixer-tracks">
        {buses.map((b) => {
          const ts = view.tracks.filter((t) => t.bus_id === b.id);
          if (!ts.length) return null;
          return (
            <div key={b.id} className="mix-group">
              <div className="group-label">{b.name}</div>
              <div className="group-strips">{ts.map((t) => <Strip key={t.id} t={t} eng={eng} />)}</div>
            </div>
          );
        })}
      </div>
      <div className="mixer-buses">
        {buses.map((b) => <BusStrip key={b.id} b={b} eng={eng} />)}
        {master.map((b) => <BusStrip key={b.id} b={b} eng={eng} />)}
      </div>
    </div>
  );
}

import { useRef, useState } from "react";
import type { EngineHook } from "../store";
import { catColor, type Bus, type Track } from "../types";

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
 *  rendered consistently by every webview). Drag, wheel, double-click = 0 dB. */
function Fader({ db, onSet }: { db: number; onSet: (v: number) => void }) {
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
      onDoubleClick={() => onSet(0)}
      onWheel={(e) => onSet(Math.max(MIN_DB, Math.min(MAX_DB, db - Math.sign(e.deltaY) * 0.5)))}
      onPointerDown={(e) => {
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
        setLive(fromY(e.clientY));
      }}
      onPointerMove={(e) => live !== null && setLive(fromY(e.clientY))}
      onPointerUp={() => {
        if (live !== null) onSet(live);
        setLive(null);
      }}
    >
      <div className="fader-zero" style={{ bottom: toPos(0) * 100 + "%" }} />
      <div className="fader-thumb" style={{ bottom: `calc(${toPos(v) * 100}% - 7px)` }} />
    </div>
  );
}

function Strip({ t, eng }: { t: Track; eng: EngineHook }) {
  const upd = (a: object) => eng.run("track.update", { id: t.id, ...a });
  return (
    <div className="strip">
      <div className="strip-color" style={{ background: catColor(t.category) }} />
      <div className="strip-name" title={t.name}>{t.name.split(" • ").slice(1).join(" • ") || t.name}</div>
      <div className="small muted ellipsis">{t.folder.split("/")[0]}</div>
      <input className="pan" type="range" min={-1} max={1} step={0.05} defaultValue={t.pan} key={t.pan} title={`pan ${t.pan.toFixed(2)}`} onMouseUp={(e) => upd({ pan: Number((e.target as HTMLInputElement).value) })} onDoubleClick={() => upd({ pan: 0 })} />
      <div className="strip-mid">
        <Fader db={t.gain_db} onSet={(v) => upd({ gain_db: v })} />
        <Meter v={eng.tx?.meters[t.id] ?? 0} />
      </div>
      <div className="mono small">{t.gain_db.toFixed(1)}</div>
      <div className="btns tight">
        <button className={"tiny" + (t.mute ? " on-mute" : "")} onClick={() => upd({ mute: !t.mute })}>M</button>
        <button className={"tiny" + (t.solo ? " on-solo" : "")} onClick={() => upd({ solo: !t.solo })}>S</button>
      </div>
      <select className="small" value={t.bus_id} onChange={(e) => upd({ bus_id: e.target.value })}>
        {eng.view!.buses.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
      </select>
    </div>
  );
}

function BusStrip({ b, eng }: { b: Bus; eng: EngineHook }) {
  const upd = (a: object) => eng.run("bus.update", { id: b.id, ...a });
  return (
    <div className={"strip bus" + (b.parent_id ? "" : " master")}>
      <div className="strip-name">{b.name}</div>
      <div className="small muted">{b.parent_id ? "stem bus" : "master"}</div>
      <div className="strip-mid">
        <Fader db={b.gain_db} onSet={(v) => upd({ gain_db: v })} />
        <Meter v={b.parent_id ? eng.tx?.meters[b.id] ?? 0 : eng.tx?.master_peak ?? 0} />
      </div>
      <div className="mono small">{b.gain_db.toFixed(1)}</div>
      <button className={"tiny" + (b.mute ? " on-mute" : "")} onClick={() => upd({ mute: !b.mute })}>M</button>
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

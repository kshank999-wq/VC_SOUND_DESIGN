import { useEffect, useState } from "react";
import { call } from "../api";
import type { EngineHook } from "../store";

const KEYS = ["KeyJ", "KeyK", "KeyL", "Semicolon", "KeyU", "KeyI", "KeyO", "KeyP"];
const LABEL: Record<string, string> = { KeyJ: "J", KeyK: "K", KeyL: "L", Semicolon: ";", KeyU: "U", KeyI: "I", KeyO: "O", KeyP: "P" };

interface PadCfg { identity_id: string; mode: string }

/** Perform-to-picture pad grid (spec §11.1). Taps are timestamped by the
 *  engine at the heard transport position; velocity comes from where the pad
 *  is hit (top = loud) or MIDI. */
export function Pads({ eng, cueId, onClose }: { eng: EngineHook; cueId: string; onClose: () => void }) {
  const { view, tx, rate, run } = eng;
  const cue = view?.cues.find((c) => c.id === cueId);
  const [cfg, setCfg] = useState<PadCfg[]>(() => {
    const first = cue?.identity_id ?? view?.identities.find((i) => i.default_asset_id)?.id ?? "";
    return KEYS.map((_, i) => ({ identity_id: i === 0 ? first : "", mode: i === 0 ? "alternate_feet" : "round_robin" }));
  });
  const [armed, setArmed] = useState(false);
  const [flash, setFlash] = useState<number | null>(null);
  const hit = (i: number, velocity: number) => {
    if (!armed || !cfg[i].identity_id) return;
    setFlash(i);
    setTimeout(() => setFlash(null), 90);
    call("perform.trigger", { pad: "key:" + KEYS[i], velocity }).catch((e) => eng.toast(String(e), "error"));
  };
  useEffect(() => {
    const kd = (e: KeyboardEvent) => {
      if (e.repeat) return;
      const i = KEYS.indexOf(e.code);
      if (i >= 0) {
        e.preventDefault();
        e.stopPropagation();
        hit(i, e.shiftKey ? 127 : 100);
      }
    };
    window.addEventListener("keydown", kd, true);
    return () => window.removeEventListener("keydown", kd, true);
  });
  if (!view || !rate || !cue) return null;
  const arm = async () => {
    const pads = cfg.map((c, i) => ({ key: "key:" + KEYS[i], identity_id: c.identity_id, mode: c.mode, name: view.identities.find((x) => x.id === c.identity_id)?.name })).filter((p) => p.identity_id);
    if (!pads.length) return eng.toast("Assign an identity with a sound to at least one pad", "error");
    if (await run("perform.start", { cue_id: cue.id, pads, latency: tx?.output_latency ?? 0 })) {
      setArmed(true);
      await call("transport.play", { from: Math.max(0, cue.start_sample - rate.sr * 2) });
    }
  };
  return (
    <div className="pads-panel">
      <div className="toolbar">
        <b>Perform {cue.code}</b>
        <span className="small muted">{armed ? `${tx?.perf_triggers ?? 0} triggers captured · keys J K L ; U I O P (Shift = accent) or MIDI` : "Choose sounds, then Arm (2 s pre-roll)"}</span>
        <span className="spacer" />
        {!armed ? <button className="rec" onClick={arm}>Arm & play</button> : <>
          <button className="primary" onClick={async () => { await call("transport.stop"); if (await run("perform.commit", {}, "Performance take saved")) onClose(); }}>Keep take</button>
          <button onClick={async () => { await call("transport.stop"); await call("perform.cancel"); setArmed(false); }}>Discard</button>
        </>}
        <button onClick={async () => { if (armed) { await call("transport.stop"); await call("perform.cancel"); } onClose(); }}>Close</button>
      </div>
      <div className="pads">
        {cfg.map((c, i) => (
          <div key={i} className={"pad-cell" + (flash === i ? " flash" : "") + (c.identity_id ? "" : " empty")}
            onMouseDown={(e) => { const r = (e.currentTarget as HTMLDivElement).getBoundingClientRect(); hit(i, Math.round(127 - ((e.clientY - r.top) / r.height) * 90)); }}>
            <div className="pad-key">{LABEL[KEYS[i]]}</div>
            {!armed ? (
              <select value={c.identity_id} onMouseDown={(e) => e.stopPropagation()} onChange={(e) => setCfg(cfg.map((x, k) => (k === i ? { ...x, identity_id: e.target.value } : x)))}>
                <option value="">(empty)</option>
                {view.identities.filter((x) => x.default_asset_id).map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
              </select>
            ) : (
              <div className="small">{view.identities.find((x) => x.id === c.identity_id)?.name ?? ""}</div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

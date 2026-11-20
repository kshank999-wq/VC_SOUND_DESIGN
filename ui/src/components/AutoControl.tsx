// Automation modes and "write-aware" controls shared by mixer strips and
// timeline headers. In Touch/Latch/Write while the transport plays, control
// moves stream to the engine (`automation.touch`), which records them against
// the audio clock and monitors them live; otherwise moves edit the static
// fader/pan as before.
import { useRef } from "react";
import { call } from "../api";
import type { EngineHook } from "../store";
import { neutral, valueAt, type AutoPoint, type Param } from "../automation";

export type AutoMode = "read" | "touch" | "latch" | "write";
export const AUTO_MODES: AutoMode[] = ["read", "touch", "latch", "write"];
const MODE_INFO: Record<AutoMode, { short: string; title: string }> = {
  read: { short: "R", title: "Read — play automation; moves change the static setting" },
  touch: { short: "T", title: "Touch — write while held, return to existing automation on release" },
  latch: { short: "L", title: "Latch — write from first touch, hold the last value until stop" },
  write: { short: "W", title: "Write — write the control value for the whole pass (play → stop)" },
};

export function ModeButton({ eng, ownerId, mode }: { eng: EngineHook; ownerId: string; mode: AutoMode }) {
  const next = AUTO_MODES[(AUTO_MODES.indexOf(mode) + 1) % AUTO_MODES.length];
  const writing = eng.tx?.writing?.some((w) => w.owner_id === ownerId);
  return (
    <button
      className={`tiny automode m-${mode}${writing ? " writing" : ""}`}
      title={`${MODE_INFO[mode].title}\nClick: ${next}`}
      onClick={() => eng.run("automation.mode", { owner_id: ownerId, mode: next })}
    >
      {MODE_INFO[mode].short}
    </button>
  );
}

export function ModeSelect({ eng, ownerId, mode }: { eng: EngineHook; ownerId: string; mode: AutoMode }) {
  const writing = eng.tx?.writing?.some((w) => w.owner_id === ownerId);
  return (
    <select className={`small automode-select m-${mode}${writing ? " writing" : ""}`} value={mode} title={MODE_INFO[mode].title}
      onChange={(e) => eng.run("automation.mode", { owner_id: ownerId, mode: e.target.value })}>
      {AUTO_MODES.map((m) => <option key={m} value={m}>{m[0].toUpperCase() + m.slice(1)}</option>)}
    </select>
  );
}

/**
 * State and handlers for one automatable control.
 * `staticValue`: fader gain (dB) or static pan. Volume automation is stored
 * relative to the fader, so the control shows and sends gain + automation.
 */
export function useAutoControl(eng: EngineHook, ownerId: string, param: Param, mode: AutoMode, staticValue: number, isTrack: boolean) {
  const lastSent = useRef(0);
  const points = ((eng.view?.automation ?? []) as AutoPoint[]).filter((p) => p.owner_id === ownerId && p.param === param);
  const w = eng.tx?.writing?.find((x) => x.owner_id === ownerId && x.param === param);
  const pos = eng.livePosition();
  const auto = points.length ? valueAt(points, pos, neutral(param, staticValue)) : null;
  const laneValue = w?.value ?? auto;
  const display = param === "volume_db" ? staticValue + (laneValue ?? 0) : laneValue ?? staticValue;
  const capturing = mode !== "read" && !!eng.tx?.playing;
  const touch = (phase: "begin" | "move" | "end", v: number) => {
    const now = performance.now();
    if (phase === "move" && now - lastSent.current < 25) return; // ~40 Hz is plenty
    lastSent.current = now;
    call("automation.touch", { owner_id: ownerId, param: param === "pan" ? "pan" : "volume_db", value: v, phase }).catch((e) => eng.toast(String(e), "error"));
  };
  const setStatic = (v: number) => {
    if (param === "volume_db") {
      // with automation playing, the fader trims the static gain under it
      const gain = v - (auto ?? 0);
      eng.run(isTrack ? "track.update" : "bus.update", { id: ownerId, gain_db: gain });
    } else if (isTrack) {
      eng.run("track.update", { id: ownerId, pan: v });
    }
  };
  return {
    display,
    capturing,
    writing: !!w,
    begin: (v: number) => capturing && touch("begin", v),
    move: (v: number) => capturing && touch("move", v),
    end: (v: number) => (capturing ? touch("end", v) : setStatic(v)),
  };
}

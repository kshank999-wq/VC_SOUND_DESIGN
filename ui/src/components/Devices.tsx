import { useCallback, useEffect, useState } from "react";
import { call } from "../api";
import type { EngineHook } from "../store";

interface DeviceInfo { name: string; is_default: boolean; max_channels: number; rates: number[] }
interface Active {
  output: string; output_rate: number; output_warning: string | null; output_latency: number;
  midi: string | null; midi_error: string | null; midi_last: { ms_ago: number; bytes: number[] } | null;
}
interface Status { prefs: { output_device: string | null; input_device: string | null; midi_port: string | null }; active: Active; prefs_path: string | null }
interface Devices extends Status { outputs: DeviceInfo[]; inputs: DeviceInfo[]; midi_inputs: string[] }

function describeMidi(b: number[]): string {
  if (b.length < 3) return b.map((x) => x.toString(16)).join(" ");
  const ch = (b[0] & 15) + 1;
  switch (b[0] & 0xf0) {
    case 0x90: return b[2] ? `note ${b[1]} on · vel ${b[2]} · ch ${ch}` : `note ${b[1]} off · ch ${ch}`;
    case 0x80: return `note ${b[1]} off · ch ${ch}`;
    case 0xb0: return `CC ${b[1]} = ${b[2]} · ch ${ch}`;
    default: return b.map((x) => x.toString(16).padStart(2, "0")).join(" ");
  }
}

/** Audio & MIDI preferences (per computer, saved across projects). */
export function DevicesDialog({ eng, onClose }: { eng: EngineHook; onClose: () => void }) {
  const [d, setD] = useState<Devices | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const projectRate = eng.view?.open ? eng.view.project.sample_rate : null;

  const refresh = useCallback(async () => {
    const v = await call<Devices>("devices.list");
    setD(v);
    setStatus(v);
  }, []);
  useEffect(() => {
    refresh();
    const id = setInterval(() => call<Status>("devices.status").then(setStatus).catch(() => {}), 250);
    return () => clearInterval(id);
  }, [refresh]);

  const set = async (k: string, v: string) => {
    setBusy(true);
    setErr(null);
    try {
      const r = await call<Devices & { error?: string }>("prefs.set", { [k]: v || null });
      setD(r);
      setStatus(r);
      if (r.error) setErr(r.error);
    } catch (e) {
      setErr(String((e as Error).message ?? e));
    }
    setBusy(false);
  };

  const a = status?.active;
  const out = d?.outputs.find((o) => o.name === (d.prefs.output_device ?? ""));
  const midiLive = a?.midi_last && a.midi_last.ms_ago < 350;
  return (
    <div className="modal-bg" onMouseDown={onClose}>
      <div className="modal devices" onMouseDown={(e) => e.stopPropagation()}>
        <div className="panel-title">Audio & MIDI</div>
        {!d ? <div className="muted pad">Scanning devices…</div> : (
          <>
            <section>
              <h4>Output (monitoring)</h4>
              <div className="dev-row">
                <select value={d.prefs.output_device ?? ""} disabled={busy} onChange={(e) => set("output_device", e.target.value)}>
                  <option value="">System default</option>
                  {d.outputs.map((o) => <option key={o.name} value={o.name}>{o.name}{o.is_default ? " (default)" : ""}</option>)}
                  {d.prefs.output_device && !d.outputs.some((o) => o.name === d.prefs.output_device) && <option value={d.prefs.output_device}>{d.prefs.output_device} (not connected)</option>}
                </select>
                <button onClick={() => call("devices.test_tone")} title="1 kHz at −18 dBFS">Test tone</button>
              </div>
              <div className="small">
                Active: <b>{a?.output}</b>{a && a.output_latency > 0 && <> · output latency {((a.output_latency / (projectRate ?? a.output_rate)) * 1000).toFixed(1)} ms</>}
              </div>
              {out && projectRate && !out.rates.includes(projectRate) && <div className="small warn">This device does not run at {projectRate / 1000} kHz natively; playback is resampled for monitoring (exports are unaffected).</div>}
              {a?.output_warning && <div className="small warn">⚠ {a.output_warning}</div>}
            </section>
            <section>
              <h4>Input (Foley recording)</h4>
              <select value={d.prefs.input_device ?? ""} disabled={busy} onChange={(e) => set("input_device", e.target.value)}>
                <option value="">System default</option>
                {d.inputs.map((o) => <option key={o.name} value={o.name}>{o.name} · {o.max_channels} ch</option>)}
                {d.prefs.input_device && !d.inputs.some((o) => o.name === d.prefs.input_device) && <option value={d.prefs.input_device}>{d.prefs.input_device} (not connected)</option>}
              </select>
              {d.inputs.length === 0 && <div className="small warn">No audio inputs found — recording is unavailable until one is connected.</div>}
            </section>
            <section>
              <h4>MIDI input (spotting pads & performance)</h4>
              <div className="dev-row">
                <select value={d.prefs.midi_port ?? ""} disabled={busy} onChange={(e) => set("midi_port", e.target.value)}>
                  <option value="">None</option>
                  {d.midi_inputs.map((p) => <option key={p} value={p}>{p}</option>)}
                  {d.prefs.midi_port && !d.midi_inputs.includes(d.prefs.midi_port) && <option value={d.prefs.midi_port}>{d.prefs.midi_port} (not connected)</option>}
                </select>
                <span className={"led" + (midiLive ? " on" : "")} title="MIDI activity" />
              </div>
              <div className="small">
                {a?.midi ? <>Connected: <b>{a.midi}</b></> : <span className="muted">Not connected</span>}
                {a?.midi_last && <span className="muted"> · last: {describeMidi(a.midi_last.bytes)}</span>}
              </div>
              {a?.midi_error && <div className="small warn">⚠ {a.midi_error}</div>}
              {d.midi_inputs.length === 0 && <div className="small muted">No MIDI inputs found. Connect a controller and press Refresh.</div>}
              <div className="small muted">Default spotting notes (ch 10): 36 footsteps · 38 prop · 42 hit.</div>
            </section>
            {err && <div className="small err">{err}</div>}
            <div className="btns">
              <button onClick={refresh}>Refresh</button>
              <span className="spacer" />
              <span className="small muted" title={d.prefs_path ?? ""}>Saved for this computer, all projects</span>
              <button className="primary" onClick={onClose}>Done</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

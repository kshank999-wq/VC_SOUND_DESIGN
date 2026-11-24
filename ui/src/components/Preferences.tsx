// Preferences (spec v2.1 › Screens › Devices & Preferences): Appearance · Audio & MIDI ·
// Spot keys · Updates. Per computer, saved across projects.
import { useState } from "react";
import { getLayout, saveLayout } from "../layout";
import type { EngineHook } from "../store";
import { ACCENTS, READOUTS, type Theme } from "../theme";
import type { EventType } from "../types";
import { DevicesPanel } from "./Devices";
import { Key, SegMeter } from "./Shell";

export type PrefsTab = "appearance" | "audio" | "spot" | "updates";

const TABS: { id: PrefsTab; label: string }[] = [
  { id: "appearance", label: "Appearance" },
  { id: "audio", label: "Audio & MIDI" },
  { id: "spot", label: "Spot keys" },
  { id: "updates", label: "Updates" },
];

interface Props {
  eng: EngineHook;
  initial?: PrefsTab;
  theme: Theme;
  setTheme: (t: Theme) => void;
  onClose: () => void;
  /** Version and update capability (desktop builds), and a way to open the updates sheet. */
  version?: string;
  updatesEnabled: boolean;
  onCheckUpdates: () => void;
}

export function PreferencesDialog(p: Props) {
  const [tab, setTab] = useState<PrefsTab>(p.initial ?? "appearance");
  return (
    <div className="modal-bg" onMouseDown={p.onClose}>
      <div className="modal prefs" role="dialog" aria-label="Preferences" onMouseDown={(e) => e.stopPropagation()}>
        <nav className="prefs-nav" role="tablist" aria-label="Preferences sections">
          <div className="panel-title">Preferences</div>
          {TABS.map((t) => (
            <button key={t.id} role="tab" aria-selected={tab === t.id} className={"prefs-tab" + (tab === t.id ? " active" : "")} onClick={() => setTab(t.id)}>{t.label}</button>
          ))}
          <span className="spacer" />
          <div className="small dim prefs-foot">Saved for this computer, all projects</div>
        </nav>
        <div className="prefs-body">
          {tab === "appearance" && <Appearance theme={p.theme} setTheme={p.setTheme} />}
          {tab === "audio" && <DevicesPanel eng={p.eng} />}
          {tab === "spot" && <SpotKeys eng={p.eng} />}
          {tab === "updates" && <Updates {...p} />}
          <div className="btns prefs-done">
            <span className="spacer" />
            <button className="primary" onClick={p.onClose}>Done</button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Appearance({ theme, setTheme }: { theme: Theme; setTheme: (t: Theme) => void }) {
  const set = (patch: Partial<Theme>) => setTheme({ ...theme, ...patch });
  return (
    <div className="form">
      <h4>Accent</h4>
      <div className="small dim">Lit keys, selection, focus rings and the main buttons. Only accents that stay clear of the category, Mute, Solo and AI colors are offered.</div>
      <div className="swatches" role="radiogroup" aria-label="Accent color">
        {ACCENTS.map((a) => (
          <button key={a.id} role="radio" aria-checked={theme.accent === a.id} className={"swatch-choice" + (theme.accent === a.id ? " on" : "")} onClick={() => set({ accent: a.id })}>
            <span className="dot" style={{ background: a.hex }} />{a.label}
          </button>
        ))}
      </div>
      <h4>Readouts</h4>
      <div className="swatches" role="radiogroup" aria-label="Readout color">
        {READOUTS.map((r) => (
          <button key={r.id} role="radio" aria-checked={theme.readout === r.id} className={"swatch-choice" + (theme.readout === r.id ? " on" : "")} onClick={() => set({ readout: r.id })}>
            <span className="dot" style={{ background: r.hex }} />{r.label}
          </button>
        ))}
      </div>
      <h4>Density</h4>
      <div className="swatches" role="radiogroup" aria-label="Density">
        {(["comfortable", "compact"] as const).map((d) => (
          <button key={d} role="radio" aria-checked={theme.density === d} className={"swatch-choice" + (theme.density === d ? " on" : "")} onClick={() => set({ density: d })}>
            {d === "comfortable" ? "Comfortable" : "Compact (laptops)"}
          </button>
        ))}
      </div>
      <label className="check"><input type="checkbox" checked={theme.reduceTransparency} onChange={(e) => set({ reduceTransparency: e.target.checked })} /> Reduce transparency (opaque panels; also follows the system setting)</label>
      <h4>Preview</h4>
      <div className="panel prefs-preview">
        <Key on>Lit key</Key>
        <Key className="primary">Export</Key>
        <button className="key th-key mute" aria-pressed>M</button>
        <button className="key th-key solo" aria-pressed>S</button>
        <span className="readout">01:00:12:08</span>
        <div style={{ width: 160 }}><SegMeter db={-8} /></div>
      </div>
    </div>
  );
}

/** Spot keys: which marker lane and kind each key spots (spot.bindings / spot.bind). */
export function SpotKeys({ eng }: { eng: EngineHook }) {
  const { view, run } = eng;
  const [newKey, setNewKey] = useState("");
  if (!view?.open) return <div className="dim">Open a project to edit its spot keys.</div>;
  const entries = Object.entries(view.bindings).filter(([k]) => k.startsWith("key:")).sort((a, b) => a[0].localeCompare(b[0]));
  const bind = (key: string, b: { lane_id: string; kind: EventType; label: string; identity_id: string | null }) =>
    run("spot.bind", { key, lane: b.lane_id, kind: b.kind, label: b.label, identity_id: b.identity_id });
  const add = () => {
    const ch = newKey.trim().toUpperCase();
    if (!/^[A-Z]$/.test(ch) || !view.lanes[0]) return eng.toast("Type one letter A–Z", "error");
    bind("key:Key" + ch, { lane_id: view.lanes[0].id, kind: "point", label: view.lanes[0].name, identity_id: null }).then(() => setNewKey(""));
  };
  return (
    <div className="form">
      <div className="small dim">In Spot mode each key drops a marker of its kind in its lane at the playhead. Ranges and repeated runs stay open until Enter or Shift+key.</div>
      <table>
        <thead><tr><th>Key</th><th>Lane</th><th>Kind</th><th>Label</th><th /></tr></thead>
        <tbody>
          {entries.map(([k, b]) => (
            <tr key={k}>
              <td className="mono">{k.replace("key:Key", "").replace("key:", "")}</td>
              <td><select aria-label="Lane" value={b.lane_id} onChange={(e) => bind(k, { ...b, lane_id: e.target.value })}>{view.lanes.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}</select></td>
              <td><select aria-label="Kind" value={b.kind} onChange={(e) => bind(k, { ...b, kind: e.target.value as EventType })}>{(["point", "range", "repeated"] as const).map((x) => <option key={x}>{x}</option>)}</select></td>
              <td><input aria-label="Label" defaultValue={b.label} onKeyDown={(e) => e.stopPropagation()} onBlur={(e) => e.target.value !== b.label && bind(k, { ...b, label: e.target.value })} /></td>
              <td><button className="link" onClick={() => run("spot.bind", { key: k, remove: true })}>Remove</button></td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="btns">
        <input aria-label="New key letter" placeholder="Letter" maxLength={1} value={newKey} onChange={(e) => setNewKey(e.target.value)} style={{ width: 80 }} onKeyDown={(e) => { e.stopPropagation(); if (e.key === "Enter") add(); }} />
        <button onClick={add}>Add key</button>
      </div>
      <div className="small dim">MIDI pads: choose the input under Audio & MIDI. Default notes on channel 10: 36 footsteps · 38 prop · 42 hit.</div>
    </div>
  );
}

function Updates(p: Props) {
  const [auto, setAuto] = useState(getLayout().autoUpdateCheck !== false);
  return (
    <div className="form">
      <div>Version <span className="mono">{p.version ?? "—"}</span></div>
      {p.updatesEnabled ? (
        <>
          <label className="check"><input type="checkbox" checked={auto} onChange={(e) => { setAuto(e.target.checked); saveLayout({ autoUpdateCheck: e.target.checked }); }} /> Check for updates when the app starts</label>
          <div className="btns"><button onClick={p.onCheckUpdates}>Check now…</button></div>
        </>
      ) : (
        <div className="small dim">In-app updates are available in signed desktop releases. This build updates through its installer (or, in browser mode, by updating the app).</div>
      )}
    </div>
  );
}

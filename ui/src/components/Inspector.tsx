import { useEffect, useState } from "react";
import { call } from "../api";
import type { EngineHook } from "../store";
import { catLabel, type Asset, type Selection } from "../types";
import { sampleAtTc, secs, tcAt } from "../tc";

const STATUSES = ["spotted", "placeholder", "replaced", "needs_recording", "recorded", "approved", "missing", "omitted"];
const PRIORITIES = ["low", "normal", "high", "critical"];

interface Props {
  eng: EngineHook;
  sel: Selection;
  setSel: (s: Selection) => void;
  libSel: Asset | null;
  onPerform: (cueId: string) => void;
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
    </label>
  );
}

/** Text input that commits on blur/Enter. */
function Edit({ value, onCommit, mono, placeholder }: { value: string; onCommit: (v: string) => void; mono?: boolean; placeholder?: string }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      className={mono ? "mono" : ""}
      value={v}
      placeholder={placeholder}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== value && onCommit(v)}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        e.stopPropagation();
      }}
    />
  );
}

export function Inspector({ eng, sel, setSel, libSel, onPerform }: Props) {
  const { view, rate, run, tx } = eng;
  if (!view?.open || !rate) return null;
  const assetName = (id: string | null | undefined) => (id ? view.assets.find((a) => a.id === id)?.name ?? id.slice(0, 12) : "—");

  let body: React.ReactNode = <div className="muted small pad">Select a cue, marker, clip or track.</div>;

  if (sel?.kind === "cue") {
    const c = view.cues.find((x) => x.id === sel.id);
    if (c) {
      const clips = view.clips.filter((x) => x.cue_id === c.id).sort((a, b) => a.timeline_start - b.timeline_start);
      const props = view.proposals.filter((p) => p.cue_id === c.id && p.status === "pending");
      const takes = view.takes.filter((t) => t.cue_id === c.id);
      const ident = view.identities.find((i) => i.id === c.identity_id);
      const recording = !!tx?.recording;
      body = (
        <div className="pad">
          <div className="insp-title">
            <span className="code">{c.code}</span> <span className="badge">{catLabel(c.category)}</span> <span className="badge">{c.event_type}</span>
          </div>
          <Field label="Description"><Edit value={c.description} onCommit={(v) => run("cue.update", { id: c.id, description: v })} /></Field>
          <div className="row2">
            <Field label="In"><Edit mono value={tcAt(rate, c.start_sample)} onCommit={(v) => { const s = sampleAtTc(rate, v); if (s !== null) run("cue.update", { id: c.id, start: s, end: c.end_sample + (s - c.start_sample) }); }} /></Field>
            <Field label="Out"><Edit mono value={tcAt(rate, c.end_sample)} onCommit={(v) => { const s = sampleAtTc(rate, v); if (s !== null) run("cue.update", { id: c.id, start: c.start_sample, end: s }); }} /></Field>
          </div>
          <Field label="Sound identity">
            <select value={c.identity_id ?? ""} onChange={(e) => run("cue.update", { id: c.id, identity_id: e.target.value })}>
              <option value="">(none)</option>
              {view.identities.map((i) => <option key={i.id} value={i.id}>{i.name}</option>)}
            </select>
          </Field>
          <div className="row2">
            <Field label="Status">
              <select value={c.status} onChange={(e) => run("cue.update", { id: c.id, status: e.target.value })}>
                {[c.status, ...STATUSES.filter((x) => x !== c.status)].map((s) => <option key={s}>{s}</option>)}
              </select>
            </Field>
            <Field label="Priority">
              <select value={c.priority} onChange={(e) => run("cue.update", { id: c.id, priority: e.target.value })}>
                {PRIORITIES.map((s) => <option key={s}>{s}</option>)}
              </select>
            </Field>
          </div>
          <Field label="Owner"><Edit value={c.owner} placeholder="assign…" onCommit={(v) => run("cue.update", { id: c.id, owner: v })} /></Field>
          <Field label="Notes"><Edit value={c.notes} onCommit={(v) => run("cue.update", { id: c.id, notes: v })} /></Field>
          <div className="small muted">Timing: {c.timing_origin} · review {c.review_status}{c.sub_events.length ? ` · ${c.sub_events.length} sub-events` : ""}{c.pending_shift ? ` · reconform shift ${secs(rate, c.pending_shift)}` : ""}</div>

          <h4>Sound</h4>
          <div className="small">Placeholder: {assetName(c.placeholder_asset_id)}{c.asset_override_id ? <> · scene-local override: <b>{assetName(c.asset_override_id)}</b></> : null}</div>
          {ident && <div className="small muted">Identity default: {assetName(ident.default_asset_id)} ({ident.variation_mode}, {ident.variation_pool.length} variants)</div>}
          <div className="btns">
            <button disabled={!libSel} title={libSel ? `Replace with ${libSel.name}` : "Select a library sound first"} onClick={() => libSel && run("cue.replace", { id: c.id, asset_id: libSel.id, scope: "cue" }, "Replaced")}>Replace (this cue)</button>
            {c.identity_id && <button disabled={!libSel} onClick={() => libSel && run("cue.replace", { id: c.id, asset_id: libSel.id, scope: "scene" }, "Replaced in scene")}>…identity in scene</button>}
            {c.identity_id && <button disabled={!libSel} onClick={() => {
              if (!libSel) return;
              const n = view.cues.filter((x) => x.identity_id === c.identity_id && !x.asset_override_id).length;
              if (confirm(`Change "${ident?.name}" to ${libSel.name} in ${n} cues across the film? Scene-local overrides are kept.`)) run("cue.replace", { id: c.id, asset_id: libSel.id, scope: "all", confirm: true }, "Rippled");
            }}>…everywhere</button>}
            <button onClick={() => run("cue.revert", { id: c.id }, "Reverted to placeholder")}>Revert to placeholder</button>
            {clips.length === 0 && <button onClick={() => run("populate", { cue_ids: [c.id] }, "Placeholder placed")}>Populate</button>}
          </div>

          <h4>Foley</h4>
          <div className="btns">
            {!recording ? (
              <button className="rec" onClick={() => run("record.start", { cue_id: c.id, from: c.start_sample, preroll: rate.sr * 2 }, "Recording… (2 s pre-roll)")}>● Record take</button>
            ) : (
              <button className="rec on" onClick={() => run("record.stop", {}, "Take saved")}>■ Stop recording</button>
            )}
            <button onClick={() => onPerform(c.id)}>Perform on pads…</button>
          </div>
          {takes.length > 0 && (
            <div className="list">
              {takes.map((t) => <div key={t.id} className="small">Take {t.number} {t.slate && `(${t.slate})`} · {assetName(t.asset_id)} @ {tcAt(rate, t.timeline_start)}</div>)}
            </div>
          )}

          <h4>AI timing assist <span className="muted small">(suggestions only)</span></h4>
          {!view.project.ai_enabled ? (
            <div className="small muted">Off for this project. <button className="link" onClick={() => run("project.settings", { ai_enabled: true }, "Local AI assist enabled (no footage leaves this machine)")}>Enable local assist</button></div>
          ) : (
            <div className="btns">
              <button onClick={() => run("ai.run", { cue_ids: [c.id], assistant: "visual" })}>Align to picture</button>
              {c.event_type !== "point" && <button onClick={() => run("ai.run", { cue_ids: [c.id], assistant: "cadence" })}>Even cadence</button>}
            </div>
          )}
          {props.map((p) => (
            <div key={p.id} className="proposal">
              <div className="small"><b>{(p.confidence * 100).toFixed(0)}%</b> · {tcAt(rate, p.before_start)} → {tcAt(rate, p.start_sample)}{p.sub_events.length ? ` · ${p.sub_events.length} events` : ""}</div>
              <div className="small muted">{p.reason}</div>
              <div className="btns">
                <button onClick={() => run("ai.accept", { id: p.id }, "Accepted")}>Accept</button>
                <button onClick={() => run("ai.reject", { id: p.id })}>Reject</button>
              </div>
            </div>
          ))}

          <h4>Instances ({clips.length})</h4>
          <div className="list">
            {clips.map((cl) => (
              <div key={cl.id} className={"inst" + (cl.muted ? " muted" : "")} onClick={() => setSel({ kind: "clip", id: cl.id })}>
                <span className="mono small">{tcAt(rate, cl.timeline_start)}</span> <span className="small ellipsis">{assetName(cl.asset_id)}</span>
                {cl.history.length > 1 && <span className="small muted"> · {cl.history.length - 1} replaced</span>}
              </div>
            ))}
          </div>
          <div className="btns"><button className="danger" onClick={() => confirm(`Delete cue ${c.code} and its clips?`) && run("cue.delete", { id: c.id }).then(() => setSel(null))}>Delete cue</button></div>
        </div>
      );
    }
  } else if (sel?.kind === "clip") {
    const cl = view.clips.find((x) => x.id === sel.id);
    if (cl) {
      const cue = view.cues.find((c) => c.id === cl.cue_id);
      const ms = (n: number) => Math.round((n / rate.sr) * 1000);
      body = (
        <div className="pad">
          <div className="insp-title">Clip {cue && <button className="link" onClick={() => setSel({ kind: "cue", id: cue.id })}>{cue.code}</button>}</div>
          <div className="small">{assetName(cl.asset_id)} {cl.is_placeholder && <span className="badge">placeholder</span>} {cl.fill === "loop" && <span className="badge">loop</span>}</div>
          <div className="row2">
            <Field label="Start"><Edit mono value={tcAt(rate, cl.timeline_start)} onCommit={(v) => { const s = sampleAtTc(rate, v); if (s !== null) run("clip.update", { id: cl.id, timeline_start: s }); }} /></Field>
            <Field label="Length"><span className="mono small">{secs(rate, cl.length)}</span></Field>
          </div>
          <Field label={`Clip gain ${cl.gain_db.toFixed(1)} dB`}>
            <input type="range" min={-40} max={12} step={0.5} defaultValue={cl.gain_db} key={cl.gain_db} onMouseUp={(e) => run("clip.update", { id: cl.id, gain_db: Number((e.target as HTMLInputElement).value) })} />
          </Field>
          <div className="row2">
            <Field label="Fade in (ms)"><Edit mono value={String(ms(cl.fade_in))} onCommit={(v) => run("clip.update", { id: cl.id, fade_in: Math.round((Number(v) / 1000) * rate.sr) })} /></Field>
            <Field label="Fade out (ms)"><Edit mono value={String(ms(cl.fade_out))} onCommit={(v) => run("clip.update", { id: cl.id, fade_out: Math.round((Number(v) / 1000) * rate.sr) })} /></Field>
          </div>
          <div className="btns">
            <button onClick={() => run("clip.update", { id: cl.id, muted: !cl.muted })}>{cl.muted ? "Unmute" : "Mute"}</button>
            <button disabled={!libSel} onClick={() => libSel && run("clip.update", { id: cl.id, asset_id: libSel.id }, "Instance replaced")}>Replace this instance</button>
            <button onClick={() => run("clip.split", { id: cl.id, at: tx?.position ?? 0 })}>Split at playhead</button>
            <button className="danger" onClick={() => run("clip.delete", { id: cl.id }).then(() => setSel(null))}>Delete</button>
          </div>
          <h4>History</h4>
          <div className="list">
            {cl.history.map((h, i) => <div key={i} className="small">{i + 1}. {assetName(h.asset_id)} <span className="muted">({h.origin})</span></div>)}
          </div>
        </div>
      );
    }
  } else if (sel?.kind === "marker") {
    const m = view.markers.find((x) => x.id === sel.id);
    if (m) {
      body = (
        <div className="pad">
          <div className="insp-title">Marker <span className="badge">{m.kind}</span></div>
          <div className="small">{view.lanes.find((l) => l.id === m.lane_id)?.name} · {tcAt(rate, m.start_sample)} – {tcAt(rate, m.end_sample)}{m.sub_events.length ? ` · ${m.sub_events.length} taps` : ""}</div>
          <div className="small muted">Spotted but not yet a cue.</div>
          <div className="btns">
            <button className="primary" onClick={async () => {
              const r = await run<{ created: string[] }>("cues.build", {}, "Cues created");
              if (r?.created.length) setSel({ kind: "cue", id: r.created[0] });
            }}>Make cues from markers</button>
            <button className="danger" onClick={() => run("marker.delete", { id: m.id }).then(() => setSel(null))}>Delete</button>
          </div>
        </div>
      );
    }
  } else if (sel?.kind === "track") {
    const t = view.tracks.find((x) => x.id === sel.id);
    if (t) {
      const hp = t.inserts.find((i) => i.type === "high_pass") as { hz: number } | undefined;
      const comp = t.inserts.some((i) => i.type === "compressor");
      const setInserts = (hpHz: number | null, compOn: boolean) =>
        run("track.update", {
          id: t.id,
          inserts: [
            ...(hpHz ? [{ type: "high_pass", hz: hpHz, q: 0.707 }] : []),
            ...(compOn ? [{ type: "compressor", threshold_db: -18, ratio: 3, attack_ms: 5, release_ms: 80, makeup_db: 2 }] : []),
          ],
        });
      body = (
        <div className="pad">
          <div className="insp-title">Track</div>
          <Field label="Name"><Edit value={t.name} onCommit={(v) => run("track.update", { id: t.id, name: v })} /></Field>
          <div className="small muted">{t.folder} · {t.channels === 1 ? "mono" : "stereo"}</div>
          <Field label="Bus">
            <select value={t.bus_id} onChange={(e) => run("track.update", { id: t.id, bus_id: e.target.value })}>
              {view.buses.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}
            </select>
          </Field>
          <Field label={`Pan ${t.pan.toFixed(2)}`}>
            <input type="range" min={-1} max={1} step={0.05} defaultValue={t.pan} key={t.pan} onMouseUp={(e) => run("track.update", { id: t.id, pan: Number((e.target as HTMLInputElement).value) })} />
          </Field>
          <h4>Inserts</h4>
          <label className="small"><input type="checkbox" checked={!!hp} onChange={(e) => setInserts(e.target.checked ? 80 : null, comp)} /> High-pass {hp ? `${hp.hz} Hz` : ""}</label>
          <label className="small"><input type="checkbox" checked={comp} onChange={(e) => setInserts(hp?.hz ?? null, e.target.checked)} /> Compressor (−18 dB, 3:1)</label>
        </div>
      );
    }
  }

  const pos = tx?.position ?? 0;
  return (
    <div className="inspector">
      <div className="panel-title">Inspector</div>
      {body}
      <ActiveAt eng={eng} pos={pos} />
    </div>
  );
}

/** Spec §14: at any playhead position, list all active sounds incl. overlaps and buses. */
function ActiveAt({ eng, pos }: { eng: EngineHook; pos: number }) {
  const [rows, setRows] = useState<{ code: string; identity: string; description: string; start_tc: string; end_tc: string; buses: string[] }[]>([]);
  const playing = eng.tx?.playing;
  useEffect(() => {
    if (playing) return;
    let alive = true;
    call("report.active_at", { sample: pos }).then((r) => alive && setRows(r)).catch(() => {});
    return () => {
      alive = false;
    };
  }, [pos, playing, eng.view?.version]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="pad active-at">
      <h4>Active at playhead {playing && <span className="muted small">(paused to update)</span>}</h4>
      {rows.length === 0 && <div className="small muted">Nothing</div>}
      {rows.map((r, i) => (
        <div key={i} className="small">
          <b>{r.code}</b> {r.identity || r.description} <span className="mono muted">{r.start_tc}–{r.end_tc}</span> <span className="muted">{r.buses.join(", ")}</span>
        </div>
      ))}
    </div>
  );
}

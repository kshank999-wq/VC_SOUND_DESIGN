import { sampleAtTc, tcAt } from "../tc";
import { useMemo, useState } from "react";
import { call } from "../api";
import type { EngineHook } from "../store";
import type { ChartRow, QaIssue, Selection } from "../types";
import { PathField } from "./Dialogs";

type Key = keyof ChartRow;

/** Sortable digital cue list (spec §14) with chart actions. */
export function CueChart({ eng, setSel }: { eng: EngineHook; setSel: (s: Selection) => void }) {
  const { view, run } = eng;
  const [sort, setSort] = useState<{ k: Key; asc: boolean }>({ k: "start_sample", asc: true });
  const [filter, setFilter] = useState("");
  const [scene, setScene] = useState("");
  const [preview, setPreview] = useState<{ cues_populated: string[]; clips_created: number; tracks_created: string[]; warnings: { cue_code: string; message: string }[] } | null>(null);
  const [reportPath, setReportPath] = useState("");
  const rows = useMemo(() => {
    if (!view?.open) return [];
    const f = filter.toLowerCase();
    const r = view.chart.filter((x) => (!scene || x.scene === scene) && (!f || `${x.code} ${x.identity} ${x.description} ${x.track} ${x.media} ${x.owner} ${x.status}`.toLowerCase().includes(f)));
    return [...r].sort((a, b) => {
      const va = a[sort.k], vb = b[sort.k];
      const c = typeof va === "number" && typeof vb === "number" ? va - vb : String(va).localeCompare(String(vb));
      return sort.asc ? c : -c;
    });
  }, [view, sort, filter, scene]);
  if (!view?.open) return null;
  const unconverted = view.markers.filter((m) => !m.cue_id).length;
  const sceneId = view.scenes.find((s) => s.number === scene)?.id;
  const col = (k: Key, label: string) => (
    <th onClick={() => setSort((s) => ({ k, asc: s.k === k ? !s.asc : true }))}>{label}{sort.k === k ? (sort.asc ? " ▴" : " ▾") : ""}</th>
  );
  return (
    <div className="chart">
      <div className="toolbar">
        <input placeholder="Filter cues…" value={filter} onChange={(e) => setFilter(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
        <select value={scene} onChange={(e) => setScene(e.target.value)}>
          <option value="">All scenes</option>
          {view.scenes.map((s) => <option key={s.id} value={s.number}>Scene {s.number}</option>)}
        </select>
        <button disabled={!unconverted} onClick={() => run("cues.build", { scene_id: sceneId }, "Cues built")}>Make cues from {unconverted} marker{unconverted === 1 ? "" : "s"}</button>
        <button onClick={async () => setPreview(await call("populate", { scene_id: sceneId, dry_run: true }))}>Populate placeholders…</button>
        <span className="spacer" />
        <PathField value={reportPath} onChange={setReportPath} directory={false} title="Report file (.pdf .csv .json)" placeholder={`${view.root}/Reports/cue_report${scene ? "_sc" + scene : ""}.pdf`} />
        <button onClick={() => run("report.write", { path: reportPath || `${view.root}/Reports/cue_report${scene ? "_sc" + scene : ""}.pdf`, scene_id: sceneId }, "Report written")}>Write report</button>
      </div>
      {preview && (
        <div className="preview">
          <b>Preview:</b> {preview.cues_populated.length} cues → {preview.clips_created} clips, {preview.tracks_created.length} new tracks.
          {preview.warnings.map((w, i) => <div key={i} className="small warn">⚠ {w.cue_code}: {w.message}</div>)}
          <div className="btns">
            <button className="primary" onClick={async () => { await run("populate", { scene_id: sceneId }, "Placeholders populated"); setPreview(null); }}>Apply</button>
            <button onClick={() => setPreview(null)}>Cancel</button>
          </div>
        </div>
      )}
      <div className="table-wrap">
        <table>
          <thead>
            <tr>{col("code", "Cue")}{col("scene", "Sc")}{col("category", "Category")}{col("identity", "Identity")}{col("description", "Description")}{col("start_sample", "In")}{col("end_sample", "Out")}{col("duration_tc", "Dur")}{col("track", "Track")}{col("media", "Media")}{col("source_status", "Source")}{col("status", "Status")}{col("owner", "Owner")}{col("notes", "Notes")}</tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.cue_id} className={r.media_missing || r.source_status === "no media" ? "missing" : ""} onClick={() => { setSel({ kind: "cue", id: r.cue_id }); call("transport.locate", { sample: r.start_sample }); }}>
                <td className="code">{r.code}</td><td>{r.scene}</td><td>{r.category}</td><td>{r.identity}</td><td>{r.description}</td>
                <td className="mono">{r.start_tc}</td><td className="mono">{r.end_tc}</td><td className="mono">{r.duration_tc || (r.sub_events ? `${r.sub_events}×` : "pt")}</td>
                <td>{r.track}</td><td>{r.media}</td><td>{r.source_status}</td><td>{r.status}</td><td>{r.owner}</td><td>{r.notes}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function QaExport({ eng }: { eng: EngineHook }) {
  const { view, run } = eng;
  const [qa, setQa] = useState<QaIssue[] | null>(null);
  const [profile, setProfile] = useState("resolve");
  const [scene, setScene] = useState("");
  const [handles, setHandles] = useState(500);
  const [float, setFloat] = useState(false);
  const [aaf, setAaf] = useState(false);
  const [aafPrint, setAafPrint] = useState(false);
  const [otio, setOtio] = useState(false);
  const [withMuted, setWithMuted] = useState(false);
  const [result, setResult] = useState<{ dir: string; files: number; qa: QaIssue[]; aaf?: string | null; otio?: string | null } | null>(null);
  const interchange = aaf || otio;
  const [busy, setBusy] = useState(false);
  if (!view?.open) return null;
  const errs = qa?.filter((i) => i.severity === "error").length ?? 0;
  const doExport = async (force: boolean) => {
    setBusy(true);
    const r = await run<{ dir: string; files: number; qa: QaIssue[]; aaf: string | null; otio: string | null }>("export.run", { profile, scene_id: scene || undefined, handles_ms: handles, float, force, aaf, otio, aaf_print_plugins: interchange && aafPrint, include_muted: interchange && withMuted }, "Export complete");
    setBusy(false);
    if (r) setResult(r);
  };
  return (
    <div className="qa">
      <div className="qa-col">
        <div className="toolbar">
          <b>Delivery QA</b>
          <button onClick={async () => setQa(await call("qa.run", { hashes: true }))}>Run QA (verify hashes)</button>
          {qa && <span className="small">{errs} errors · {qa.filter((i) => i.severity === "warning").length} warnings · {qa.filter((i) => i.severity === "info").length} info</span>}
        </div>
        <div className="qa-list">
          {qa?.length === 0 && <div className="small ok">No issues found.</div>}
          {qa?.map((i, k) => <div key={k} className={"qa-item " + i.severity}><b>{i.severity}</b> <code>{i.code}</code> {i.message}</div>)}
        </div>
      </div>
      <div className="qa-col">
        <div className="toolbar"><b>Export</b></div>
        <div className="form">
          <label className="field"><span>Profile</span>
            <select value={profile} onChange={(e) => setProfile(e.target.value)}>
              <option value="quick">Quick review — stereo mix + cue report</option>
              <option value="resolve">Resolve / Fairlight — stems + aligned tracks + manifest</option>
              <option value="pro">Pro studio handoff — + discrete cues with handles, routing</option>
              <option value="archive">Archival — editable project + manifests</option>
            </select>
          </label>
          <div className="row3">
            <label className="field"><span>Range</span>
              <select value={scene} onChange={(e) => setScene(e.target.value)}>
                <option value="">Whole project</option>
                {view.scenes.map((s) => <option key={s.id} value={s.id}>Scene {s.number}</option>)}
              </select>
            </label>
            <label className="field"><span>Handles (ms)</span><input type="number" value={handles} onChange={(e) => setHandles(Number(e.target.value))} /></label>
            <label className="field"><span>Format</span><select value={float ? "f" : "i"} onChange={(e) => setFloat(e.target.value === "f")}><option value="i">24-bit PCM</option><option value="f">32-bit float</option></select></label>
            <label className="small aaf-opt" title="Editable timeline (clips, fades, clip gain, cue markers, timecode) linking copies of the source WAVs. Checked with an independent AAF reader; not yet verified in a specific Resolve/Fairlight version — the rendered stems stay the reference. What it can't carry is listed in AAF/aaf_report.json.">
              <input type="checkbox" checked={aaf} onChange={(e) => setAaf(e.target.checked)} /> Also write AAF (beta)
            </label>
            <label className="small aaf-opt" title="The same editable timeline as OpenTimelineIO (.otio): clips with exact source ranges, fades to and from silence as dissolves, cue markers, start timecode. OTIO has no audio mix model, so clip gain, fader, pan and automation are in its metadata. What it can't carry is listed in OTIO/otio_report.json.">
              <input type="checkbox" checked={otio} onChange={(e) => setOtio(e.target.checked)} /> Also write OTIO (beta)
            </label>
            <label className={"small aaf-opt sub" + (interchange ? "" : " muted")} title="AAF and OTIO can't carry VST3/AU plugins or insert effects. With this on, tracks that use them are rendered with them (and their parameter automation), pre-fader, and placed in the AAF/OTIO as one clip each, so they sound right in the receiving app. Their fader, volume, pan and bus levels stay editable; their clips don't.">
              <input type="checkbox" disabled={!interchange} checked={interchange && aafPrint} onChange={(e) => setAafPrint(e.target.checked)} /> Print tracks with plugins
            </label>
            <label className={"small aaf-opt sub" + (interchange ? "" : " muted")} title="Muted tracks, tracks on muted buses and muted clips are normally left out of the AAF/OTIO. With this on they're kept, silent: a zero gain around them in the AAF, disabled in the OTIO. Other apps hear silence (and can turn them up); VC Sound Design's own import brings them back muted, on their muted buses, with their levels.">
              <input type="checkbox" disabled={!interchange} checked={interchange && withMuted} onChange={(e) => setWithMuted(e.target.checked)} /> Include muted tracks (silent)
            </label>
          </div>
          <div className="btns">
            <button className="primary" disabled={busy} onClick={() => doExport(false)}>{busy ? "Exporting…" : "Export"}</button>
            <button disabled={busy} title="Export despite QA errors (they are documented in the package)" onClick={() => confirm("Export despite QA errors?") && doExport(true)}>Force export</button>
          </div>
          {result && (
            <div className="small">
              <div>✔ {result.files} files → <span className="mono">{result.dir}</span></div>
              <div>QA: {result.qa.filter((i) => i.severity === "error").length} errors, {result.qa.filter((i) => i.severity === "warning").length} warnings</div>
              {result.aaf && (
                <div className="aaf-result">
                  AAF: <span className="mono">{result.aaf}</span> · {result.qa.filter((i) => i.code === "aaf_loss").length} kinds of loss listed in <span className="mono">AAF/aaf_report.json</span>
                </div>
              )}
              {result.otio && (
                <div className="aaf-result">
                  OTIO: <span className="mono">{result.otio}</span> · {result.qa.filter((i) => i.code === "otio_loss").length} kinds of loss listed in <span className="mono">OTIO/otio_report.json</span>
                </div>
              )}
            </div>
          )}
          <h4>Previous exports</h4>
          {view.exports.map((x) => <div key={x.id} className="small mono">{x.profile} · {x.file_count} files · {x.output_dir}</div>)}
        </div>
      </div>
      <Reports eng={eng} />
    </div>
  );
}

interface SceneSummary {
  scene: string; cues: number; by_status: Record<string, number>; missing_media: string[]; overlaps: string[];
  ambience_gaps: string[]; muted_tracks: string[]; sync_warnings: string[]; pending_ai: number;
}
interface ActiveSound { code: string; identity: string; category: string; description: string; start_tc: string; end_tc: string; tracks: string[] }

/** Reports (spec v2.1 › Screens › Reports): a scene's summary, cue sheets, and what plays at a timecode. */
function Reports({ eng }: { eng: EngineHook }) {
  const { view, rate, run, tx } = eng;
  const [sceneId, setSceneId] = useState(view?.scenes[0]?.id ?? "");
  const [sum, setSum] = useState<SceneSummary | null>(null);
  const [tc, setTc] = useState("");
  const [active, setActive] = useState<ActiveSound[] | null>(null);
  const [written, setWritten] = useState<string | null>(null);
  if (!view?.open || !rate) return null;
  const sc = view.scenes.find((x) => x.id === sceneId);
  const write = async (ext: "pdf" | "csv" | "json", scope: string) => {
    const path = `${view.root}/Reports/cue_sheet${scope ? "_sc" + (view.scenes.find((x) => x.id === scope)?.number ?? "") : ""}.${ext}`;
    if (await run("report.write", { path, scene_id: scope || undefined }, `Cue sheet written (${ext.toUpperCase()})`)) setWritten(path);
  };
  const list = (label: string, items: string[]) => items.length > 0 && (
    <div className="rep-list"><span className="label">{label} · {items.length}</span>{items.map((x) => <div key={x} className="small">{x}</div>)}</div>
  );
  return (
    <div className="qa-col reports">
      <div className="toolbar"><b>Reports</b></div>
      <div className="form">
        <h4>Scene summary</h4>
        <div className="dev-row">
          <select aria-label="Scene" value={sceneId} onChange={(e) => { setSceneId(e.target.value); setSum(null); }}>
            {view.scenes.map((x) => <option key={x.id} value={x.id}>Scene {x.number} · {x.title}</option>)}
          </select>
          <button disabled={!sc} onClick={async () => setSum(await run<SceneSummary>("report.summary", { scene_id: sceneId }))}>Summarize</button>
        </div>
        {sum && (
          <div className="rep-summary">
            <div className="rep-status">
              <span className="pill">{sum.cues} cues</span>
              {Object.entries(sum.by_status).map(([k, n]) => <span key={k} className="pill">{k.replace(/_/g, " ")} {n}</span>)}
              {sum.pending_ai > 0 && <span className="pill">{sum.pending_ai} AI pending</span>}
            </div>
            {list("Missing media", sum.missing_media)}
            {list("Overlaps", sum.overlaps)}
            {list("Ambience gaps", sum.ambience_gaps)}
            {list("Muted tracks", sum.muted_tracks)}
            {list("Sync warnings", sum.sync_warnings)}
            {!sum.missing_media.length && !sum.overlaps.length && !sum.ambience_gaps.length && !sum.sync_warnings.length && <div className="small"><span className="sym ok">✓</span> Nothing to flag.</div>}
          </div>
        )}
        <h4>Cue sheet</h4>
        <div className="small dim">Every cue with timecode in/out, identity, sound, owner and status{sc ? "; choose the whole project or the scene above" : ""}.</div>
        <div className="btns">
          {(["pdf", "csv", "json"] as const).map((ext) => <button key={ext} onClick={() => write(ext, "")}>{ext.toUpperCase()} · project</button>)}
        </div>
        {sc && <div className="btns">{(["pdf", "csv", "json"] as const).map((ext) => <button key={ext} onClick={() => write(ext, sceneId)}>{ext.toUpperCase()} · scene {sc.number}</button>)}</div>}
        {written && <div className="small mono ellipsis" title={written}>→ {written}</div>}
        <h4>Active at…</h4>
        <div className="dev-row">
          <input className="mono" aria-label="Timecode" placeholder={tcAt(rate, tx?.position ?? 0)} value={tc} onChange={(e) => setTc(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
          <button onClick={async () => {
            const s = tc ? sampleAtTc(rate, tc) : tx?.position ?? 0;
            if (s === null) return eng.toast("Not a timecode", "error");
            setActive(await run<ActiveSound[]>("report.active_at", { sample: s }));
          }}>Report</button>
        </div>
        {active && (active.length ? active.map((a) => (
          <div key={a.code} className="small rep-active"><span className="code">{a.code}</span> {a.identity || a.description} <span className="dim">{a.start_tc}–{a.end_tc} · {a.category}</span></div>
        )) : <div className="small dim">Nothing plays there.</div>)}
      </div>
    </div>
  );
}

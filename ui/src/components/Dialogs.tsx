import { useEffect, useState } from "react";
import { call, isTauri, nativePick } from "../api";
import { aafSummary, otioSummary, type AafReport, type OtioReport } from "../aafReport";
import type { EngineHook } from "../store";
import { getLayout } from "../layout";

interface Entry { name: string; path: string; dir: boolean; project: boolean }

/** In-app file browser (works in both desktop and browser mode). */
export function FileBrowser({ start, directory, onPick, onClose, title }: { start?: string; directory: boolean; onPick: (p: string) => void; onClose: () => void; title: string }) {
  const [dir, setDir] = useState(start ?? "");
  const [data, setData] = useState<{ dir: string; parent: string | null; entries: Entry[] } | null>(null);
  const [err, setErr] = useState("");
  useEffect(() => {
    (async () => {
      let d = dir;
      if (!d) d = (await call<{ home: string }>("app.info")).home || "/";
      try {
        setData(await call("fs.list", { dir: d }));
        setErr("");
      } catch (e) {
        setErr(String(e));
      }
    })();
  }, [dir]);
  return (
    <div className="modal-bg" onMouseDown={onClose}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="panel-title">{title}</div>
        <div className="row2">
          <input className="mono" value={data?.dir ?? dir} onChange={(e) => setDir(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
        </div>
        {err && <div className="err small">{err}</div>}
        <div className="fb-list">
          {data?.parent && <div className="fb-item" onClick={() => setDir(data.parent!)}>⬆ ..</div>}
          {data?.entries.map((e) => (
            <div key={e.path} className={"fb-item" + (e.dir ? " dir" : "")}
              onClick={() => (e.dir ? setDir(e.path) : !directory && onPick(e.path))}>
              {e.dir ? (e.project ? "🎬 " : "📁 ") : "· "}{e.name}
              {e.dir && directory && <button className="tiny" onClick={(ev) => { ev.stopPropagation(); onPick(e.path); }}>choose</button>}
            </div>
          ))}
        </div>
        <div className="btns">
          {directory && data && <button className="primary" onClick={() => onPick(data.dir)}>Use this folder</button>}
          <button onClick={onClose}>Cancel</button>
        </div>
      </div>
    </div>
  );
}

/** Path input with a Browse button (native dialog in the desktop app). */
export function PathField({ value, onChange, directory, title, placeholder }: { value: string; onChange: (v: string) => void; directory: boolean; title: string; placeholder?: string }) {
  const [browse, setBrowse] = useState(false);
  return (
    <div className="pathfield">
      <input className="mono" value={value} placeholder={placeholder ?? (directory ? "/path/to/folder" : "/path/to/file")} onChange={(e) => onChange(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
      <button onClick={async () => {
        if (isTauri) {
          const r = await nativePick({ directory, title });
          if (r) onChange(r);
        } else setBrowse(true);
      }}>Browse…</button>
      {browse && <FileBrowser title={title} directory={directory} start={value || undefined} onClose={() => setBrowse(false)} onPick={(p) => { onChange(p); setBrowse(false); }} />}
    </div>
  );
}

/** The Welcome screen (spec v2.1 › Screens › Welcome): start, open, recent projects. */
export function Welcome({ eng, onDevices, onPrefs }: { eng: EngineHook; onDevices: () => void; onPrefs?: () => void }) {
  const [mode, setMode] = useState<"home" | "new" | "open" | "demo">("home");
  const [dir, setDir] = useState("");
  const [name, setName] = useState("My Film");
  const [fps, setFps] = useState("24");
  const [sr, setSr] = useState(48000);
  const [startTc, setStartTc] = useState("01:00:00:00");
  const [busy, setBusy] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [version, setVersion] = useState("");
  const recent = getLayout().recent ?? [];
  useEffect(() => {
    call<{ home: string; version: string }>("app.info").then((i) => {
      setDir((d) => d || i.home + "/VC_Sound_Projects/");
      setVersion(i.version);
    });
  }, []);
  const ago = (t: number) => {
    const m = Math.round((Date.now() - t) / 60000);
    return m < 1 ? "just now" : m < 60 ? `${m} min ago` : m < 1440 ? `${Math.round(m / 60)} h ago` : `${Math.round(m / 1440)} d ago`;
  };
  return (
    <div className="welcome">
      <div className="panel welcome-main">
        <div className="welcome-brand">
          <div className="logo big" aria-hidden>
            <svg viewBox="0 0 24 24" width="30" height="30"><path d="M3 12h2M7 8v8M11 5v14M15 8v8M19 10v4M21 12h0" stroke="currentColor" strokeWidth="2" strokeLinecap="round" fill="none" /></svg>
          </div>
          <div>
            <span className="label">VC Sound Design</span>
            <h1>Sound for picture</h1>
            <div className="dim">Spot · build · perform · mix · deliver</div>
          </div>
        </div>
        {mode === "home" && (
          <div className="welcome-actions">
            <button className="action primary" onClick={() => setMode("new")}><b>New project</b><span>Name, frame rate, sample rate and start timecode</span></button>
            <button className="action" onClick={() => setMode("open")}><b>Open project…</b><span>A project folder (🎬)</span></button>
            <button className="action" onClick={() => { setMode("demo"); setDir((d) => d.replace(/\/?$/, "/") + "Gravel_Road_Demo"); }}><b>Build the demo</b><span>“Gravel Road”: three scenes, cues, a performance, a take and exports</span></button>
          </div>
        )}
        {mode === "new" && (
          <div className="form">
            <h4>New project</h4>
            <label className="field"><span>Project folder</span><PathField value={dir} onChange={setDir} directory title="Project folder" /></label>
            <label className="field"><span>Name</span><input value={name} onChange={(e) => setName(e.target.value)} /></label>
            <div className="row3">
              <label className="field"><span>Frame rate</span>
                <select value={fps} onChange={(e) => setFps(e.target.value)}>
                  {["23.976", "24", "25", "29.97 NDF", "29.97 DF", "30", "50", "59.94 NDF", "59.94 DF", "60"].map((f) => <option key={f}>{f}</option>)}
                </select>
              </label>
              <label className="field"><span>Sample rate</span>
                <select value={sr} onChange={(e) => setSr(Number(e.target.value))}>{[44100, 48000, 96000].map((r) => <option key={r} value={r}>{r / 1000} kHz</option>)}</select>
              </label>
              <label className="field"><span>Start TC</span><input className="mono" value={startTc} onChange={(e) => setStartTc(e.target.value)} /></label>
            </div>
            <div className="small dim">Frame and sample rate can't change later: they set every timecode and every file.</div>
            <div className="btns">
              <button onClick={() => setMode("home")}>Back</button>
              <span className="spacer" />
              <button className="primary" disabled={busy || !name.trim()} onClick={async () => { setBusy(true); await eng.run("project.new", { dir, name, fps, sample_rate: sr, start_tc: startTc }); setBusy(false); }}>Create project</button>
            </div>
          </div>
        )}
        {mode === "open" && (
          <FileBrowser title="Open a project folder (🎬)" directory start={dir.replace(/[^/]*$/, "")} onClose={() => setMode("home")} onPick={(p) => eng.run("project.open", { dir: p })} />
        )}
        {mode === "demo" && (
          <div className="form">
            <h4>Build the demo</h4>
            <div className="small dim">Builds a 3-scene project with generated picture, a procedural starter pack, spotted cues, a performance, a recorded take, a score and two export packages.</div>
            <label className="field"><span>Folder (must not exist)</span><PathField value={dir} onChange={setDir} directory title="Demo folder" /></label>
            <div className="btns">
              <button onClick={() => setMode("home")}>Back</button>
              <span className="spacer" />
              <button className="primary" disabled={busy} onClick={async () => {
                setBusy(true);
                const r = await eng.run<{ log: string[] }>("project.demo", { dir });
                if (r) setLog(r.log);
                setBusy(false);
              }}>{busy ? "Building… (≈20 s)" : "Build demo"}</button>
            </div>
            {log.length > 0 && <pre className="log">{log.join("\n")}</pre>}
          </div>
        )}
        <div className="welcome-foot">
          <button className="link" onClick={onDevices}>Audio & MIDI…</button>
          {onPrefs && <button className="link" onClick={onPrefs}>Preferences…</button>}
          <span className="spacer" />
          <span className="dim small mono">{version && `v${version}`}</span>
        </div>
      </div>
      <div className="panel welcome-recent">
        <div className="panel-title">Recent projects</div>
        {recent.length === 0 ? (
          <div className="dim pad">Projects you open appear here.</div>
        ) : (
          <ul className="recent-list">
            {recent.map((r) => (
              <li key={r.path}>
                <button className="recent" onClick={() => eng.run("project.open", { dir: r.path })} title={r.path}>
                  <b>{r.name}</b>
                  <span className="mono ellipsis">{r.path}</span>
                  <span className="dim">{ago(r.at)}</span>
                </button>
                <button className="link small" title="Open the newer autosave instead (if there is one)" onClick={() => eng.run("project.recover", { dir: r.path }, "Recovered from autosave")}>Recover autosave</button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

export type ImportKind = "picture" | "aaf" | "otio" | "score" | "script" | "relink" | "take" | null;


/** What an AAF or OTIO import brought in (spec v2.1 › Screens › Import summary). */
export function ImportSummary({ r, kind, onRelink, onClose }: { r: AafReport | OtioReport; kind: "aaf" | "otio"; onRelink: () => void; onClose: () => void }) {
  const name = "timeline" in r ? r.timeline : r.composition;
  const lines = kind === "otio" ? otioSummary(r as OtioReport) : aafSummary(r as AafReport);
  const blank = lines.indexOf("");
  const details = lines.slice(2, blank < 0 ? undefined : blank);
  const tiles: [string, string | number, string?][] = [
    ["Tracks", r.tracks.length, r.stereo_tracks ? `${r.stereo_tracks} stereo` : undefined],
    ["Clips", r.clips],
    ["Markers", r.markers],
    ["Cues restored", r.cues ?? 0, r.linked_clips ? `${r.linked_clips} clips linked` : undefined],
    ["Scenes added", r.scenes_added ?? 0],
    ["Media", r.media_linked + r.media_written, `${r.media_linked} linked · ${r.media_written} written`],
  ];
  return (
    <div className="import-summary">
      <div className="is-head">
        <span className="label">{kind === "aaf" ? "AAF" : "OTIO"} imported</span>
        <div className="is-title">“{name}” <span className="readout sm">{r.start_tc}</span></div>
      </div>
      <div className="tiles">
        {tiles.map(([k, v, sub]) => (
          <div key={k} className="tile">
            <span className="label">{k}</span>
            <span className="tile-v">{v}</span>
            {sub && <span className="tile-sub">{sub}</span>}
          </div>
        ))}
      </div>
      {details.length > 0 && <ul className="is-list">{details.map((d) => <li key={d}>{d}</li>)}</ul>}
      {r.missing_media.length > 0 && (
        <section className="is-section bad">
          <div className="is-sec-head"><span className="label">Missing media · clips left out</span><button onClick={onRelink}>Relink…</button></div>
          <ul className="is-list">{r.missing_media.map((m) => <li key={m}><span className="sym" aria-hidden>✕</span>{m}</li>)}</ul>
        </section>
      )}
      {r.notes.length > 0 && (
        <section className="is-section">
          <div className="is-sec-head"><span className="label">Notes</span></div>
          <ul className="is-list">{r.notes.map((n) => <li key={n}><span className="sym warn" aria-hidden>⚠</span>{n}</li>)}</ul>
        </section>
      )}
      {!r.missing_media.length && !r.notes.length && <div className="is-ok"><span className="sym ok" aria-hidden>✓</span> Everything came in.</div>}
      <div className="btns"><span className="spacer" /><button className="primary" onClick={onClose}>Done</button></div>
    </div>
  );
}

export function ImportDialog({ kind, eng, onClose, cueId, onRelink }: { kind: ImportKind; eng: EngineHook; onClose: () => void; cueId?: string; onRelink?: () => void }) {
  const [path, setPath] = useState("");
  const [summary, setSummary] = useState<AafReport | OtioReport | null>(null);
  const [label, setLabel] = useState("v" + ((eng.view?.pictures.length ?? 0) + 1));
  const [tc, setTc] = useState("");
  const [buses, setBuses] = useState("auto");
  if (!kind) return null;
  const titles = { picture: "Import picture reference", aaf: "Import AAF (editor's timeline)", otio: "Import OpenTimelineIO (.otio)", score: "Import VC Composer score (.json)", script: "Import VC Film Studio breakdown (.json)", relink: "Relink missing media (search folder)", take: "Add a recorded take (.wav)" };
  const go = async () => {
    let ok = null;
    if (kind === "picture") ok = await eng.run("picture.import", { path, label, start_tc: tc || undefined }, "Picture imported");
    if (kind === "aaf") {
      const r = await eng.run<AafReport>("aaf.import", { path, at_tc: tc || undefined, buses });
      if (r) return setSummary(r);
    }
    if (kind === "otio") {
      const r = await eng.run<OtioReport>("otio.import", { path, at_tc: tc || undefined, buses });
      if (r) return setSummary(r);
    }
    if (kind === "score") ok = await eng.run("score.import", { path }, "Score imported");
    if (kind === "script") ok = await eng.run("script.import", { path }, "Breakdown imported");
    if (kind === "take" && cueId) ok = await eng.run("take.add", { path, cue_id: cueId }, "Take added");
    if (kind === "relink") {
      const r = await eng.run<{ relinked: unknown[]; still_missing: unknown[] }>("relink.run", { dirs: [path] });
      if (r) eng.toast(`Relinked ${r.relinked.length}, still missing ${r.still_missing.length}`);
      ok = r;
    }
    if (ok) onClose();
  };
  if (summary && (kind === "aaf" || kind === "otio"))
    return (
      <div className="modal-bg" onMouseDown={() => { setSummary(null); onClose(); }}>
        <div className="modal wide" role="dialog" aria-label="Import summary" onMouseDown={(e) => e.stopPropagation()}>
          <ImportSummary r={summary} kind={kind} onClose={() => { setSummary(null); onClose(); }} onRelink={() => { setSummary(null); onRelink?.(); }} />
        </div>
      </div>
    );
  return (
    <div className="modal-bg" onMouseDown={onClose}>
      <div className="modal" role="dialog" aria-label={titles[kind]} onMouseDown={(e) => e.stopPropagation()}>
        <div className="panel-title">{titles[kind]}</div>
        <PathField value={path} onChange={setPath} directory={kind === "relink"} title={titles[kind]} />
        {kind === "picture" && (
          <div className="row2">
            <label className="field"><span>Version label</span><input value={label} onChange={(e) => setLabel(e.target.value)} /></label>
            <label className="field"><span>Start TC (blank = read from file)</span><input className="mono" value={tc} placeholder="01:00:00:00" onChange={(e) => setTc(e.target.value)} /></label>
          </div>
        )}
        {(kind === "aaf" || kind === "otio") && (
          <label className="field"><span>Start TC (blank = the file's own start)</span><input className="mono" value={tc} placeholder="01:00:00:00" onChange={(e) => setTc(e.target.value)} /></label>
        )}
        {(kind === "otio" || kind === "aaf") && (
          <label className="field" title="For AAF and OTIO files from VC Sound Design, which describe their buses. Auto restores them when this project's matching buses are untouched.">
            <span>Bus levels</span>
            <select value={buses} onChange={(e) => setBuses(e.target.value)}>
              <option value="auto">Auto</option>
              <option value="buses">Restore buses and their automation</option>
              <option value="fold">Fold into the tracks</option>
            </select>
          </label>
        )}
        <div className="btns">
          <span className="spacer" />
          <button onClick={onClose}>Cancel</button>
          <button className="primary" disabled={!path} onClick={go}>Import</button>
        </div>
      </div>
    </div>
  );
}

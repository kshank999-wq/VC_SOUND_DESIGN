import { useEffect, useState } from "react";
import { call, isTauri, nativePick } from "../api";
import { aafSummary, otioSummary, type AafReport, type OtioReport } from "../aafReport";
import type { EngineHook } from "../store";

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

export function Welcome({ eng, onDevices }: { eng: EngineHook; onDevices: () => void }) {
  const [mode, setMode] = useState<"home" | "new" | "open" | "demo">("home");
  const [dir, setDir] = useState("");
  const [name, setName] = useState("My Film");
  const [fps, setFps] = useState("24");
  const [sr, setSr] = useState(48000);
  const [startTc, setStartTc] = useState("01:00:00:00");
  const [busy, setBusy] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  useEffect(() => {
    call<{ home: string }>("app.info").then((i) => setDir((d) => d || i.home + "/VC_Sound_Projects/"));
  }, []);
  return (
    <div className="welcome">
      <div className="welcome-card">
        <h1>VC Sound Design</h1>
        <div className="muted">Spot · populate · perform · record · mix · hand off</div>
        {mode === "home" && (
          <div className="welcome-actions">
            <button className="primary big" onClick={() => setMode("new")}>New project…</button>
            <button className="big" onClick={() => setMode("open")}>Open project…</button>
            <button className="big" onClick={() => { setMode("demo"); setDir((d) => d.replace(/\/?$/, "/") + "Gravel_Road_Demo"); }}>Create demo project…</button>
            <div className="welcome-foot"><button className="link" onClick={onDevices}>Audio & MIDI devices…</button></div>
          </div>
        )}
        {mode === "new" && (
          <div className="form">
            <label className="field"><span>Project folder</span><PathField value={dir} onChange={setDir} directory title="Project folder" /></label>
            <label className="field"><span>Name</span><input value={name} onChange={(e) => setName(e.target.value)} /></label>
            <div className="row3">
              <label className="field"><span>Frame rate</span>
                <select value={fps} onChange={(e) => setFps(e.target.value)}>
                  {["23.976", "24", "25", "29.97 NDF", "29.97 DF", "30", "50", "59.94 NDF", "59.94 DF", "60"].map((f) => <option key={f}>{f}</option>)}
                </select>
              </label>
              <label className="field"><span>Sample rate</span>
                <select value={sr} onChange={(e) => setSr(Number(e.target.value))}>{[44100, 48000, 96000].map((r) => <option key={r} value={r}>{r}</option>)}</select>
              </label>
              <label className="field"><span>Start TC</span><input className="mono" value={startTc} onChange={(e) => setStartTc(e.target.value)} /></label>
            </div>
            <div className="btns">
              <button className="primary" disabled={busy} onClick={async () => { setBusy(true); await eng.run("project.new", { dir, name, fps, sample_rate: sr, start_tc: startTc }); setBusy(false); }}>Create</button>
              <button onClick={() => setMode("home")}>Back</button>
            </div>
          </div>
        )}
        {mode === "open" && (
          <FileBrowser title="Open a project folder (🎬)" directory start={dir.replace(/[^/]*$/, "")} onClose={() => setMode("home")} onPick={(p) => eng.run("project.open", { dir: p })} />
        )}
        {mode === "demo" && (
          <div className="form">
            <div className="small muted">Builds a 3-scene project with generated picture, a procedural starter pack, spotted cues, a performance, a recorded take, a score and two export packages.</div>
            <label className="field"><span>Folder (must not exist)</span><PathField value={dir} onChange={setDir} directory title="Demo folder" /></label>
            <div className="btns">
              <button className="primary" disabled={busy} onClick={async () => {
                setBusy(true);
                const r = await eng.run<{ log: string[] }>("project.demo", { dir });
                if (r) setLog(r.log);
                setBusy(false);
              }}>{busy ? "Building… (≈20 s)" : "Build demo"}</button>
              <button onClick={() => setMode("home")}>Back</button>
            </div>
            {log.length > 0 && <pre className="log">{log.join("\n")}</pre>}
          </div>
        )}
      </div>
    </div>
  );
}

export type ImportKind = "picture" | "aaf" | "otio" | "score" | "script" | "relink" | "take" | null;


export function ImportDialog({ kind, eng, onClose, cueId }: { kind: ImportKind; eng: EngineHook; onClose: () => void; cueId?: string }) {
  const [path, setPath] = useState("");
  const [label, setLabel] = useState("v" + ((eng.view?.pictures.length ?? 0) + 1));
  const [tc, setTc] = useState("");
  const [log, setLog] = useState<string[]>([]);
  const [buses, setBuses] = useState("auto");
  if (!kind) return null;
  const titles = { picture: "Import picture reference", aaf: "Import AAF (editor's timeline)", otio: "Import OpenTimelineIO (.otio)", score: "Import VC Composer score (.json)", script: "Import VC Film Studio breakdown (.json)", relink: "Relink missing media (search folder)", take: "Add a recorded take (.wav)" };
  const go = async () => {
    let ok = null;
    if (kind === "picture") ok = await eng.run("picture.import", { path, label, start_tc: tc || undefined }, "Picture imported");
    if (kind === "aaf") {
      const r = await eng.run<AafReport>("aaf.import", { path, at_tc: tc || undefined, buses });
      if (r) {
        eng.toast(`AAF imported: ${r.tracks.length} tracks, ${r.clips} clips`);
        // keep the dialog open to show what was left out
        if (r.missing_media.length || r.notes.length) return setLog(aafSummary(r));
      }
      ok = r;
    }
    if (kind === "otio") {
      const r = await eng.run<OtioReport>("otio.import", { path, at_tc: tc || undefined, buses });
      if (r) {
        eng.toast(`OTIO imported: ${r.tracks.length} tracks, ${r.clips} clips`);
        if (r.missing_media.length || r.notes.length) return setLog(otioSummary(r));
      }
      ok = r;
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
  return (
    <div className="modal-bg" onMouseDown={onClose}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
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
        {log.length > 0 && <pre className="log">{log.join("\n")}</pre>}
        <div className="btns">
          {log.length > 0 ? (
            <button className="primary" onClick={onClose}>Close</button>
          ) : (
            <>
              <button className="primary" disabled={!path} onClick={go}>Import</button>
              <button onClick={onClose}>Cancel</button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

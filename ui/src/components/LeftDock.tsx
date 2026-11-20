import { useEffect, useMemo, useState } from "react";
import { call } from "../api";
import type { EngineHook } from "../store";
import { CATEGORIES, catColor, catKey, catLabel, type Asset, type Selection } from "../types";
import { sampleAtTc, secs, tcAt } from "../tc";
import { PathField } from "./Dialogs";

interface Props {
  eng: EngineHook;
  setSel: (s: Selection) => void;
  libSel: Asset | null;
  setLibSel: (a: Asset | null) => void;
  gotoRange: (a: number, b: number) => void;
}

export function LeftDock(p: Props) {
  const [tab, setTab] = useState<"scenes" | "identities" | "library">("scenes");
  return (
    <div className="leftdock">
      <div className="tabs">
        {(["scenes", "identities", "library"] as const).map((t) => (
          <button key={t} className={tab === t ? "active" : ""} onClick={() => setTab(t)}>{t[0].toUpperCase() + t.slice(1)}</button>
        ))}
      </div>
      {tab === "scenes" && <Scenes {...p} />}
      {tab === "identities" && <Identities {...p} />}
      {tab === "library" && <Library {...p} />}
    </div>
  );
}

function Scenes({ eng, setSel, gotoRange }: Props) {
  const { view, rate, run, tx } = eng;
  const [adding, setAdding] = useState(false);
  const [num, setNum] = useState("");
  const [title, setTitle] = useState("");
  const [a, setA] = useState("");
  const [z, setZ] = useState("");
  if (!view?.open || !rate) return null;
  const pos = tx?.position ?? 0;
  return (
    <div className="dock-body">
      {view.scenes.map((sc) => {
        const lines = view.script.filter((l) => l.scene_id === sc.id).sort((x, y) => x.order - y.order);
        const cues = view.cues.filter((c) => c.scene_id === sc.id);
        const here = pos >= sc.start_sample && pos < sc.end_sample;
        return (
          <div key={sc.id} className={"scene-item" + (here ? " here" : "")}>
            <div className="scene-head" onClick={() => { call("transport.locate", { sample: sc.start_sample }); gotoRange(sc.start_sample, sc.end_sample); }}>
              <b>{sc.number}</b> <span className="ellipsis">{sc.title}</span>
              <span className="mono small muted">{tcAt(rate, sc.start_sample)}</span>
            </div>
            <div className="small muted">{cues.length} cues · {secs(rate, sc.end_sample - sc.start_sample)}</div>
            {lines.map((l) => {
              const linked = cues.filter((c) => c.script_ref_id === l.id);
              return (
                <div key={l.id} className="script-line" onClick={() => linked[0] && setSel({ kind: "cue", id: linked[0].id })}>
                  {l.character && <b>{l.character}: </b>}{l.text}
                  {linked.map((c) => <span key={c.id} className="badge">{c.code}</span>)}
                </div>
              );
            })}
            {here && cues.filter((c) => c.status === "conceptual").length > 0 && (
              <div className="small muted">Script-only cues: {cues.filter((c) => c.status === "conceptual").map((c) => c.code).join(", ")}</div>
            )}
          </div>
        );
      })}
      {!adding ? (
        <button onClick={() => { setAdding(true); setA(tcAt(rate, pos)); setZ(tcAt(rate, pos + rate.sr * 30)); setNum(String(view.scenes.length + 1)); }}>+ Scene</button>
      ) : (
        <div className="form">
          <input placeholder="Number" value={num} onChange={(e) => setNum(e.target.value)} />
          <input placeholder="Title (e.g. EXT. ROAD - DAY)" value={title} onChange={(e) => setTitle(e.target.value)} />
          <div className="row2"><input className="mono" value={a} onChange={(e) => setA(e.target.value)} /><input className="mono" value={z} onChange={(e) => setZ(e.target.value)} /></div>
          <div className="btns">
            <button className="primary" onClick={async () => {
              const s0 = sampleAtTc(rate, a), s1 = sampleAtTc(rate, z);
              if (s0 === null || s1 === null) return eng.toast("Invalid timecode", "error");
              if (await run("scene.add", { number: num, title, start: s0, end: s1 })) setAdding(false);
            }}>Add</button>
            <button onClick={() => setAdding(false)}>Cancel</button>
          </div>
        </div>
      )}
    </div>
  );
}

function Identities({ eng, setSel, libSel }: Props) {
  const { view, rate, run } = eng;
  const [open, setOpen] = useState<string | null>(null);
  const [form, setForm] = useState(false);
  const [name, setName] = useState("");
  const [cat, setCat] = useState("footsteps");
  const [pool, setPool] = useState("");
  const [mode, setMode] = useState("round_robin");
  if (!view?.open || !rate) return null;
  const groups = new Map<string, typeof view.identities>();
  for (const i of view.identities) {
    const k = catKey(i.category);
    groups.set(k, [...(groups.get(k) ?? []), i]);
  }
  return (
    <div className="dock-body">
      {[...groups.entries()].map(([k, ids]) => (
        <div key={k}>
          <div className="group-title"><span className="swatch" style={{ background: catColor(k) }} />{catLabel(k)}</div>
          {ids.map((i) => {
            const cues = view.cues.filter((c) => c.identity_id === i.id).sort((a, b) => a.start_sample - b.start_sample);
            const def = view.assets.find((a) => a.id === i.default_asset_id);
            return (
              <div key={i.id} className="ident">
                <div className="ident-head" onClick={() => setOpen(open === i.id ? null : i.id)}>
                  <span>{open === i.id ? "▾" : "▸"} {i.name}</span>
                  <span className="small muted">{cues.length} cues</span>
                </div>
                {open === i.id && (
                  <div className="ident-body">
                    <div className="small muted">Sound: {def?.name ?? "(unassigned)"} · {i.variation_mode} · {i.variation_pool.length} variants</div>
                    <button disabled={!libSel} onClick={() => libSel && run("identity.assign", { id: i.id, default: libSel.id, pool: [libSel.id], mode: "locked" }, "Identity sound set")}>Assign selected library sound</button>
                    <div className="small"><b>All "{i.name}" cues</b></div>
                    {cues.map((c) => (
                      <div key={c.id} className="cue-link" onClick={() => { setSel({ kind: "cue", id: c.id }); call("transport.locate", { sample: c.start_sample }); }}>
                        <span className="code">{c.code}</span> <span className="mono small">{tcAt(rate, c.start_sample)}</span>
                        {c.asset_override_id && <span className="badge" title="scene-local override">override</span>}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      ))}
      {!form ? (
        <button onClick={() => setForm(true)}>+ Sound identity</button>
      ) : (
        <div className="form">
          <input placeholder='Name, e.g. "Greg / Boots"' value={name} onChange={(e) => setName(e.target.value)} />
          <select value={cat} onChange={(e) => setCat(e.target.value)}>{CATEGORIES.map((c) => <option key={c} value={c}>{catLabel(c)}</option>)}</select>
          <input placeholder="Variation pool: library search words (optional)" value={pool} onChange={(e) => setPool(e.target.value)} />
          <select value={mode} onChange={(e) => setMode(e.target.value)}>
            <option value="round_robin">Round-robin</option>
            <option value="alternate_feet">Alternate feet</option>
            <option value="random">Random (reproducible)</option>
            <option value="locked">Locked</option>
          </select>
          <div className="btns">
            <button className="primary" onClick={async () => { if (await run("identity.add", { name, category: cat, pool_query: pool, mode })) { setForm(false); setName(""); } }}>Create</button>
            <button onClick={() => setForm(false)}>Cancel</button>
          </div>
        </div>
      )}
    </div>
  );
}

function Library({ eng, libSel, setLibSel }: Props) {
  const { view, rate, run, tx } = eng;
  const [text, setText] = useState("");
  const [cat, setCat] = useState("");
  const [res, setRes] = useState<Asset[]>([]);
  const [importing, setImporting] = useState(false);
  const [path, setPath] = useState("");
  const version = view?.version;
  useEffect(() => {
    const t = setTimeout(() => {
      call<Asset[]>("library.search", { text, category: cat || undefined }).then(setRes).catch(() => setRes([]));
    }, 120);
    return () => clearTimeout(t);
  }, [text, cat, version]);
  const counts = useMemo(() => res.length, [res]);
  if (!view?.open || !rate) return null;
  return (
    <div className="dock-body lib">
      <input placeholder="Search sounds… (e.g. boot gravel)" value={text} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
      <select value={cat} onChange={(e) => setCat(e.target.value)}>
        <option value="">All categories</option>
        {CATEGORIES.map((c) => <option key={c} value={c}>{catLabel(c)}</option>)}
      </select>
      <div className="small muted">{counts} sounds{libSel ? ` · selected: ${libSel.name}` : ""}</div>
      <div className="lib-list">
        {res.map((a) => (
          <div key={a.id} className={"lib-item" + (libSel?.id === a.id ? " selected" : "")} onClick={() => setLibSel(a)} onDoubleClick={() => call("audition", { asset_id: a.id })}>
            <button className="tiny" title="Audition" onClick={(e) => { e.stopPropagation(); call(tx?.previewing ? "audition.stop" : "audition", { asset_id: a.id }); }}>▶</button>
            <span className="swatch" style={{ background: catColor(a.category) }} />
            <span className="ellipsis">{a.name}</span>
            <span className="small muted">{secs(rate, a.length)} {a.channels === 2 ? "st" : "mono"}</span>
            {a.kind === "placeholder" && <span className="badge">ph</span>}
            {a.missing && <span className="badge err">missing</span>}
          </div>
        ))}
      </div>
      {!importing ? (
        <div className="btns">
          <button onClick={() => setImporting(true)}>Import folder…</button>
          <button onClick={() => run("library.install_starter", {}, "Starter pack installed")}>Install starter pack</button>
        </div>
      ) : (
        <div className="form">
          <PathField value={path} onChange={setPath} directory title="Library folder or WAV" />
          <div className="btns">
            <button className="primary" onClick={async () => { const r = await run<{ imported: number }>("library.import", { path }); if (r) { eng.toast(`Indexed ${r.imported} sounds`); setImporting(false); } }}>Index</button>
            <button onClick={() => setImporting(false)}>Cancel</button>
          </div>
        </div>
      )}
    </div>
  );
}

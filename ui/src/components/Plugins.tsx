import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { call, closeNativeEditor, isTauri, openNativeEditor, openNativeEditors } from "../api";
import type { EngineHook } from "../store";
import type { Insert, PluginInfo, PluginParam, PluginSlot } from "../types";
import { showPluginLane, type AutoPoint, type Param } from "../automation";
import { storeParams } from "../pluginParams";

interface Catalog {
  plugins: PluginInfo[];
  instruments: number;
  failed: { path: string; error: string }[];
  bundles: number;
  scanned: number;
  folders: { standard: string[]; extra: string[] };
  formats: string[];
}

// ---------------------------------------------------------------- shared state

/** Scanned plugins, shared by every browser/rack in the window. */
let catalog: Catalog | null = null;
let loading: Promise<Catalog> | null = null;
const catalogSubs = new Set<() => void>();

function loadCatalog(cmd = "plugins.list", args: object = {}): Promise<Catalog> {
  loading = call<Catalog>(cmd, args)
    .then((c) => {
      catalog = c;
      catalogSubs.forEach((f) => f());
      return c;
    })
    .finally(() => {
      loading = null;
      catalogSubs.forEach((f) => f());
    });
  catalogSubs.forEach((f) => f());
  return loading;
}

function useCatalog(): { catalog: Catalog | null; busy: boolean } {
  const c = useSyncExternalStore((cb) => (catalogSubs.add(cb), () => catalogSubs.delete(cb)), () => catalog);
  const busy = useSyncExternalStore((cb) => (catalogSubs.add(cb), () => catalogSubs.delete(cb)), () => loading !== null);
  useEffect(() => {
    if (!catalog && !loading) loadCatalog().catch(() => {});
  }, []);
  return { catalog: c, busy };
}

/** Open plugin windows (slot ids), any number at once. */
let editors: string[] = [];
const editorSubs = new Set<() => void>();
// plugin windows float above the app but below modal dialogs (z 100):
// z = 60 + position in the stacking order
let stack: string[] = [];
const stackSubs = new Set<() => void>();
function raise(slotId: string) {
  if (stack[stack.length - 1] === slotId) return;
  stack = [...stack.filter((x) => x !== slotId), slotId];
  stackSubs.forEach((f) => f());
}
export function openPluginEditor(slotId: string) {
  if (!editors.includes(slotId)) {
    editors = [...editors, slotId];
    editorSubs.forEach((f) => f());
  }
  raise(slotId);
}
/** Open a plugin: its own window in the desktop app, else (or if it has none) the parameter panel. */
export async function openPlugin(eng: EngineHook, slotId: string) {
  const f = findSlot(eng, slotId);
  if (isTauri && f && eng.view?.plugins?.[slotId]?.loaded) {
    try {
      await openNativeEditor(slotId, `${f.slot.name} — ${f.ownerName}`);
      return;
    } catch (e) {
      const msg = String((e as Error)?.message ?? e);
      if (!/no editor/i.test(msg)) eng.toast(msg, "error");
    }
  }
  openPluginEditor(slotId);
}

function closePluginEditor(slotId: string) {
  closeNativeEditor(slotId);
  editors = editors.filter((x) => x !== slotId);
  stack = stack.filter((x) => x !== slotId);
  editorSubs.forEach((f) => f());
}

/** Find a plugin slot and its owner (track or bus) in the project. */
function findSlot(eng: EngineHook, id: string): { slot: PluginSlot; owner: string; ownerName: string; index: number; count: number } | null {
  const v = eng.view;
  if (!v?.open) return null;
  for (const o of [...v.tracks, ...v.buses]) {
    const i = (o.inserts ?? []).findIndex((x) => x.type === "plugin" && x.id === id);
    if (i >= 0) return { slot: o.inserts[i] as PluginSlot, owner: o.id, ownerName: o.name, index: i, count: o.inserts.length };
  }
  return null;
}

const isPlugin = (i: Insert): i is PluginSlot => i.type === "plugin";

// ---------------------------------------------------------------- insert rack

/** Plugin slots of one track or bus: status, bypass, order, remove, add. */
export function InsertRack({ eng, ownerId, inserts, compact }: { eng: EngineHook; ownerId: string; inserts: Insert[]; compact?: boolean }) {
  const [picking, setPicking] = useState(false);
  const status = eng.view?.plugins ?? {};
  const slots = inserts.map((ins, index) => ({ ins, index })).filter((x): x is { ins: PluginSlot; index: number } => isPlugin(x.ins));
  return (
    <div className={"fx-rack" + (compact ? " compact" : "")}>
      {slots.map(({ ins, index }) => {
        const st = status[ins.id];
        const missing = st && !st.loaded;
        const pos = slots.findIndex((x) => x.index === index);
        return (
          <div key={ins.id} className={"fx-slot" + (ins.bypass ? " bypassed" : "") + (missing ? " missing" : "")}>
            {!compact && (
              <button className={"tiny fx-power" + (ins.bypass ? "" : " on")} title={ins.bypass ? "Bypassed — click to enable" : "Click to bypass"}
                onClick={() => eng.run("insert.bypass", { owner_id: ownerId, id: ins.id, bypass: !ins.bypass })}>⏻</button>
            )}
            <span className="fx-name ellipsis" onClick={() => openPlugin(eng, ins.id)}
              title={`${ins.name}${ins.vendor ? " — " + ins.vendor : ""} (${ins.format.toUpperCase()})${ins.bypass ? " · bypassed" : ""}${missing ? "\n⚠ " + st.error : ""}${st?.latency ? `\nlatency ${st.latency} samples (compensated)` : ""}\nclick to open`}>
              {missing ? "⚠ " : ""}{ins.name}
            </span>
            {!compact && st?.latency ? <span className="small muted mono">{st.latency}</span> : null}
            {!compact && isTauri && <button className="tiny" title="Parameters (the app's panel)" onClick={() => openPluginEditor(ins.id)}>≡</button>}
            {!compact && (
              <>
                <button className="tiny" disabled={pos === 0} title="Move up (earlier in the chain)" onClick={() => eng.run("insert.move", { owner_id: ownerId, id: ins.id, to: slots[pos - 1].index })}>↑</button>
                <button className="tiny" disabled={pos === slots.length - 1} title="Move down" onClick={() => eng.run("insert.move", { owner_id: ownerId, id: ins.id, to: slots[pos + 1].index })}>↓</button>
              </>
            )}
            {!compact && <button className="tiny" title="Remove" onClick={() => { closePluginEditor(ins.id); eng.run("insert.remove", { owner_id: ownerId, id: ins.id }); }}>✕</button>}
          </div>
        );
      })}
      <button className="tiny fx-add" title="Insert a VST3/AU effect" onClick={() => setPicking(true)}>{compact ? "+ FX" : "+ Add plugin…"}</button>
      {picking && (
        <PluginBrowser
          onClose={() => setPicking(false)}
          onPick={async (p) => {
            setPicking(false);
            const r = await eng.run<{ id: string; status: { loaded: boolean; error: string | null } }>("insert.add", { owner_id: ownerId, plugin_id: p.id });
            if (r?.status && !r.status.loaded) eng.toast(`${p.name}: ${r.status.error}`, "error");
            else if (r) openPlugin(eng, r.id);
          }}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------- browser

/** Installed effects: search, rescan, extra folders. With `onPick`, choosing one inserts it. */
/** Plugin manager (☰ › Plugins) and insert picker (an insert slot's "+"): search, filter by
 *  format and vendor, rescan, folders, and what failed to load. */
export function PluginBrowser({ onPick, onClose }: { onPick?: (p: PluginInfo) => void; onClose: () => void }) {
  const { catalog: c, busy } = useCatalog();
  const [q, setQ] = useState("");
  const [fmt, setFmt] = useState("all");
  const [vendor, setVendor] = useState("all");
  const [view, setView] = useState<"installed" | "failed" | "folders">("installed");
  const [folders, setFolders] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const run = (p: Promise<unknown>) => { setErr(null); p.catch((e) => setErr(String(e?.message ?? e))); };
  const vendorOf = (p: PluginInfo) => p.vendor || "Unknown vendor";

  const all = c?.plugins ?? [];
  const groups = useMemo(() => {
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    const list = all.filter((p) => (fmt === "all" || p.format === fmt) && (vendor === "all" || vendorOf(p) === vendor) && words.every((w) => `${p.name} ${p.vendor} ${p.category}`.toLowerCase().includes(w)));
    const m = new Map<string, PluginInfo[]>();
    for (const p of list) m.set(vendorOf(p), [...(m.get(vendorOf(p)) ?? []), p]);
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [c, q, fmt, vendor]); // eslint-disable-line react-hooks/exhaustive-deps
  const count = groups.reduce((n, g) => n + g[1].length, 0);
  const vendors = useMemo(() => {
    const m = new Map<string, number>();
    for (const p of all) if (fmt === "all" || p.format === fmt) m.set(vendorOf(p), (m.get(vendorOf(p)) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [c, fmt]); // eslint-disable-line react-hooks/exhaustive-deps

  const list = (
    <div className="plugin-list" role="listbox" aria-label="Plugins">
      {!c && busy && <div className="dim pad">Scanning plugins… (each new plugin is opened once, in a separate process)</div>}
      {c && count === 0 && (
        <div className="dim pad">
          {all.length ? "Nothing matches." : "No effect plugins found. Install VST3 plugins in the standard folder, or add the folder they're in under Folders."}
        </div>
      )}
      {groups.map(([v, ps]) => (
        <div key={v}>
          <div className="plugin-vendor">{v}</div>
          {ps.map((p) => (
            <div key={p.id} role="option" aria-selected={false} tabIndex={onPick ? 0 : -1} className={"plugin-item" + (onPick ? " pickable" : "")}
              onClick={() => onPick?.(p)} onKeyDown={(e) => e.key === "Enter" && onPick?.(p)} title={p.path || p.id}>
              <span className="ellipsis pl-name">{p.name}</span>
              <span className="small dim ellipsis">{p.category.replace(/^Fx\|?/, "").replace(/\|/g, " · ") || "Effect"}</span>
              <span className="badge">{p.format.toUpperCase()}</span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
  const search = (
    <div className="pl-search">
      <input autoFocus aria-label="Search plugins" placeholder="Search name, vendor or type (eq, reverb…)" value={q} onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && onPick && count === 1) onPick(groups[0][1][0]); if (e.key === "Escape") onClose(); }} />
      <span className="small dim">{count} shown</span>
    </div>
  );

  return (
    <div className="modal-bg" onMouseDown={onClose}>
      <div className={"modal plugin-browser" + (onPick ? " picker" : " manager")} role="dialog" aria-label={onPick ? "Insert plugin" : "Plugins"} onMouseDown={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
        {onPick ? (
          <>
            <div className="panel-title">Insert plugin</div>
            {search}
            {list}
            <div className="btns"><span className="small dim">{all.length} effects</span><span className="spacer" /><button onClick={onClose}>Cancel</button></div>
          </>
        ) : (
          <div className="pl-manager">
            <nav className="pl-side" aria-label="Plugin views">
              <div className="panel-title">Plugins</div>
              {([["installed", `Installed · ${all.length}`], ["failed", `Failed · ${c?.failed.length ?? 0}`], ["folders", "Folders"]] as const).map(([id, label]) => (
                <button key={id} className={"prefs-tab" + (view === id ? " active" : "")} aria-pressed={view === id} onClick={() => setView(id)}>{label}</button>
              ))}
              {view === "installed" && (
                <>
                  <h4>Format</h4>
                  {["all", ...(c?.formats ?? [])].map((f) => (
                    <button key={f} className={"pl-filter" + (fmt === f ? " on" : "")} aria-pressed={fmt === f} onClick={() => { setFmt(f); setVendor("all"); }}>
                      {f === "all" ? "All formats" : f.toUpperCase()}<span className="dim">{f === "all" ? all.length : all.filter((p) => p.format === f).length}</span>
                    </button>
                  ))}
                  <h4>Vendor</h4>
                  <div className="pl-vendors">
                    <button className={"pl-filter" + (vendor === "all" ? " on" : "")} aria-pressed={vendor === "all"} onClick={() => setVendor("all")}>All vendors</button>
                    {vendors.map(([v, n]) => (
                      <button key={v} className={"pl-filter" + (vendor === v ? " on" : "")} aria-pressed={vendor === v} onClick={() => setVendor(v)}><span className="ellipsis">{v}</span><span className="dim">{n}</span></button>
                    ))}
                  </div>
                </>
              )}
            </nav>
            <div className="pl-main">
              {view === "installed" && <>{search}{list}</>}
              {view === "failed" && (
                <div className="plugin-failed">
                  {!c?.failed.length ? <div className="dim">Every plugin found loaded.</div> : c.failed.map((f) => (
                    <div key={f.path} className="pl-fail"><span className="sym" aria-hidden>✕</span><div><div className="mono small">{f.path}</div><div className="small dim">{f.error}</div></div></div>
                  ))}
                </div>
              )}
              {view === "folders" && c && (
                <div className="plugin-folders">
                  <h4>Always searched</h4>
                  {c.folders.standard.map((f) => <div key={f} className="mono small ellipsis">{f}</div>)}
                  <h4>Also search (one folder per line)</h4>
                  <textarea aria-label="Extra plugin folders" rows={5} value={folders ?? c.folders.extra.join("\n")} onChange={(e) => setFolders(e.target.value)} />
                  <div className="btns">
                    <button className="primary" disabled={busy || folders === null} onClick={() => run(loadCatalog("plugins.folders", { folders: (folders ?? "").split("\n") }).then(() => setFolders(null)))}>Save & scan</button>
                  </div>
                </div>
              )}
              {err && <div className="small" style={{ color: "var(--error-text)" }}>{err}</div>}
              <div className="btns pl-foot">
                <span className="small dim">{c ? `${all.length} effects in ${c.bundles} bundle${c.bundles === 1 ? "" : "s"}${c.instruments ? ` · ${c.instruments} instruments hidden` : ""}` : ""}</span>
                <span className="spacer" />
                <button disabled={busy} title="Look for new or updated plugins" onClick={() => run(loadCatalog("plugins.scan"))}>{busy ? "Scanning…" : "Rescan"}</button>
                <button disabled={busy} title="Open every plugin again (after a plugin update that didn't change its files' dates)" onClick={() => run(loadCatalog("plugins.scan", { force: true }))}>Full rescan</button>
                <button className="primary" onClick={onClose}>Done</button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- plugin window

interface ParamsReply { id: string; name: string; vendor: string; format: string; bypass: boolean; loaded: boolean; error: string | null; latency: number; params: PluginParam[] }

function ParamRow({ p, onBegin, onLive, onCommit, lane, writing, onShowLane }: {
  p: PluginParam;
  onBegin: (v: number) => void;
  onLive: (v: number) => void;
  onCommit: (v: number) => void;
  /** Automation points on this parameter. */
  lane: number;
  writing: boolean;
  onShowLane: () => void;
}) {
  const [live, setLive] = useState<number | null>(null);
  const v = live ?? p.value;
  const step = p.step_count > 0 ? 1 / p.step_count : 0.001;
  const commit = () => {
    if (live !== null) onCommit(live);
    setLive(null);
  };
  return (
    <div className={"param-row" + (writing ? " writing" : "")} title={p.read_only ? "read-only" : "double-click: default"}>
      <span className="param-name ellipsis">{p.title}</span>
      <input type="range" min={0} max={1} step={step} value={v} disabled={p.read_only}
        onPointerDown={(e) => onBegin(Number((e.target as HTMLInputElement).value))}
        onChange={(e) => { const nv = Number(e.target.value); setLive(nv); onLive(nv); }}
        onPointerUp={commit} onKeyUp={commit} onBlur={commit}
        onDoubleClick={() => onCommit(p.default)} />
      <span className="param-val mono ellipsis">{p.display}{p.units && !p.display.endsWith(p.units) ? " " + p.units : ""}</span>
      {p.automatable && !p.read_only ? (
        <button className={"tiny" + (lane ? " has-auto" : "")} title={lane ? `${lane} automation points — show the lane` : "Show an automation lane for this parameter"} onClick={onShowLane}>A</button>
      ) : <span />}
    </div>
  );
}

function PluginEditor({ eng, slotId, index, onClose }: { eng: EngineHook; slotId: string; index: number; onClose: () => void }) {
  const [d, setD] = useState<ParamsReply | null>(null);
  const [filter, setFilter] = useState("");
  const [pos, setPos] = useState<{ x: number; y: number }>({ x: Math.max(20, window.innerWidth - 400 - index * 28), y: 70 + index * 28 });
  const drag = useRef<{ dx: number; dy: number } | null>(null);
  const inflight = useRef(false);
  const queued = useRef<{ id: number; v: number } | null>(null);
  const found = findSlot(eng, slotId);
  const version = eng.view?.version;
  const z = 60 + Math.max(0, useSyncExternalStore((cb) => (stackSubs.add(cb), () => stackSubs.delete(cb)), () => stack.indexOf(slotId)));

  const load = () =>
    call<ParamsReply>("plugin.params", { id: slotId })
      .then((r) => {
        setD(r);
        storeParams(slotId, r.params);
      })
      .catch(() => setD(null));
  useEffect(() => {
    load();
  }, [slotId, version]); // eslint-disable-line react-hooks/exhaustive-deps

  // automation on this plugin: while playing, the controls follow it
  const lanePts = ((eng.view?.automation ?? []) as AutoPoint[]).filter((p) => p.owner_id === slotId);
  const playing = !!eng.tx?.playing;

  // Touch/Latch/Write on the plugin's track or bus while playing: moves write automation
  const ownerMode = (() => {
    const v = eng.view;
    if (!v?.open || !found) return "read";
    return (v.tracks.find((t) => t.id === found.owner)?.automation_mode ?? v.buses.find((b) => b.id === found.owner)?.automation_mode ?? "read") as string;
  })();
  const capturing = ownerMode !== "read" && playing;
  // while playing with automation (or writing it), the controls follow what the plugin plays
  const follow = playing && (lanePts.length > 0 || capturing);
  useEffect(() => {
    if (!follow) return;
    const id = setInterval(load, 150);
    return () => clearInterval(id);
  }, [follow]); // eslint-disable-line react-hooks/exhaustive-deps
  const lastTouch = useRef(0);
  const touch = (id: number, phase: "begin" | "move" | "end", v: number) => {
    const now = performance.now();
    if (phase === "move" && now - lastTouch.current < 25) return;
    lastTouch.current = now;
    call("automation.touch", { owner_id: slotId, param: `plugin:${id}`, value: v, phase }).catch((e) => eng.toast(String(e), "error"));
  };

  // the slot was removed (or undone away): close — once we've seen it (the
  // project view can lag a moment behind the insert that opened this window)
  const seen = useRef(false);
  useEffect(() => {
    if (found) seen.current = true;
    else if (seen.current && eng.view?.open) onClose();
  }, [found, eng.view?.open]); // eslint-disable-line react-hooks/exhaustive-deps

  // live values while dragging: at most one request in flight, latest value wins
  const sendLive = (id: number, v: number) => {
    queued.current = { id, v };
    if (inflight.current) return;
    const pump = async () => {
      inflight.current = true;
      while (queued.current) {
        const q = queued.current;
        queued.current = null;
        try {
          const r = await call<{ param: PluginParam | null }>("plugin.param", { id: slotId, param_id: q.id, value: q.v, commit: false });
          if (r.param) setD((x) => (x ? { ...x, params: x.params.map((p) => (p.id === r.param!.id ? { ...r.param!, value: p.value } : p)) } : x));
        } catch {
          /* shown on commit */
        }
      }
      inflight.current = false;
    };
    pump();
  };
  const commit = async (id: number, v: number) => {
    queued.current = null;
    await eng.run("plugin.param", { id: slotId, param_id: id, value: v });
    load();
  };

  const params = (d?.params ?? []).filter((p) => !p.is_bypass && (!filter || p.title.toLowerCase().includes(filter.toLowerCase())));
  const sr = eng.view?.open ? eng.view.project.sample_rate : 48000;
  return (
    <div className="plugin-window" style={{ left: pos.x, top: pos.y, zIndex: z }} onKeyDown={(e) => e.stopPropagation()} onMouseDown={() => raise(slotId)}>
      <div className="panel-title plugin-head"
        onPointerDown={(e) => { if ((e.target as HTMLElement).tagName === "BUTTON") return; (e.target as HTMLElement).setPointerCapture(e.pointerId); drag.current = { dx: e.clientX - pos.x, dy: e.clientY - pos.y }; }}
        onPointerMove={(e) => drag.current && setPos({ x: Math.max(0, Math.min(window.innerWidth - 120, e.clientX - drag.current.dx)), y: Math.max(0, Math.min(window.innerHeight - 40, e.clientY - drag.current.dy)) })}
        onPointerUp={() => (drag.current = null)}>
        <span className="ellipsis">{d?.name ?? found?.slot.name ?? "Plugin"} <span className="muted small">· {found?.ownerName}</span></span>
        <span className="spacer" />
        {found && (
          <button className={"tiny fx-power" + (found.slot.bypass ? "" : " on")} title={found.slot.bypass ? "Bypassed" : "Active — click to bypass"}
            onClick={() => eng.run("insert.bypass", { owner_id: found.owner, id: slotId, bypass: !found.slot.bypass })}>⏻</button>
        )}
        {isTauri && d?.loaded && found && (
          <button className="tiny" title="The plugin's own interface, in its own window"
            onClick={() => openNativeEditor(slotId, `${found.slot.name} — ${found.ownerName}`).catch((e) => eng.toast(String(e?.message ?? e), "error"))}>Plugin window</button>
        )}
        {found && (
          <button className="tiny" title="Remove this plugin from the chain (undoable)"
            onClick={() => { onClose(); closeNativeEditor(slotId); eng.run("insert.remove", { owner_id: found.owner, id: slotId }, `Removed ${found.slot.name}`); }}>Remove</button>
        )}
        <button className="tiny" title="Close" onClick={onClose}>✕</button>
      </div>
      <div className="plugin-body">
        <div className="small muted">
          {d?.vendor ? d.vendor + " · " : ""}{(d?.format ?? found?.slot.format ?? "").toUpperCase()}
          {d?.latency ? ` · latency ${d.latency} samples (${((d.latency / sr) * 1000).toFixed(1)} ms), compensated` : ""}
        </div>
        {d && !d.loaded && (
          <div className="warn-box small">
            ⚠ {d.error ?? "Not loaded."} Its settings are kept with the project and it passes audio through until it's available.
          </div>
        )}
        {d?.loaded && (
          <>
            {d.params.length > 10 && <input placeholder={`Filter ${d.params.length} parameters`} value={filter} onChange={(e) => setFilter(e.target.value)} />}
            <div className="param-list">
              {params.map((p) => (
                <ParamRow key={p.id} p={p}
                  onBegin={(v) => capturing && touch(p.id, "begin", v)}
                  onLive={(v) => (capturing ? touch(p.id, "move", v) : sendLive(p.id, v))}
                  onCommit={(v) => (capturing ? touch(p.id, "end", v) : commit(p.id, v))}
                  lane={lanePts.filter((x) => x.param === `plugin:${p.id}`).length}
                  writing={!!eng.tx?.writing?.some((w) => w.owner_id === slotId && w.param === `plugin:${p.id}`)}
                  onShowLane={() => found && showPluginLane(found.owner, slotId, `plugin:${p.id}` as Param)} />
              ))}
              {d.params.length === 0 && <div className="muted small">This plugin has no parameters the host can show.</div>}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** Mount once per window: renders every open plugin window. */
export function PluginEditors({ eng }: { eng: EngineHook }) {
  const ids = useSyncExternalStore((cb) => (editorSubs.add(cb), () => editorSubs.delete(cb)), () => editors);
  // plugin windows whose slot went away (removed, undone, project closed) are closed
  const version = eng.view?.version;
  useEffect(() => {
    if (!isTauri) return;
    openNativeEditors().then((open) => open.filter((id) => !findSlot(eng, id)).forEach((id) => closeNativeEditor(id)));
  }, [version, eng.view?.open]); // eslint-disable-line react-hooks/exhaustive-deps
  return <>{ids.map((id, i) => <PluginEditor key={id} eng={eng} slotId={id} index={i} onClose={() => closePluginEditor(id)} />)}</>;
}

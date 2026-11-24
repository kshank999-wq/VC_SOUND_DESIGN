// In-app updates (desktop release builds). Updates are downloaded from the
// release endpoint and verified against the update public key baked into the
// build before anything is installed; an unsigned or tampered update is refused.
import { useEffect, useState } from "react";
import { buildInfo, isTauri, type BuildInfo } from "../api";
import type { EngineHook } from "../store";
import { getLayout, saveLayout } from "../layout";

type Phase = "idle" | "checking" | "none" | "available" | "downloading" | "ready" | "error";

// The plugin's Update object (kept loosely typed to avoid importing it in browser mode).
interface Update {
  version: string;
  currentVersion: string;
  body?: string;
  date?: string;
  downloadAndInstall(cb?: (e: { event: string; data?: { contentLength?: number; chunkLength?: number } }) => void): Promise<void>;
}

export function useUpdates(eng: EngineHook) {
  const [info, setInfo] = useState<BuildInfo | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  const [update, setUpdate] = useState<Update | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ got: number; total: number | null }>({ got: 0, total: null });

  useEffect(() => {
    if (!isTauri) return;
    buildInfo().then(setInfo).catch(() => setInfo(null));
  }, []);

  const check = async (quiet = false) => {
    if (!info?.updater) return;
    setPhase("checking");
    setError(null);
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const u = (await check()) as Update | null;
      setUpdate(u);
      setPhase(u ? "available" : "none");
      if (u && quiet) eng.toast(`VC Sound Design ${u.version} is available — ☰ → Check for updates`);
    } catch (e) {
      setPhase(quiet ? "idle" : "error");
      if (!quiet) setError(String(e));
    }
  };

  // quiet check at startup (can be turned off)
  useEffect(() => {
    if (!info?.updater || getLayout().autoUpdateCheck === false) return;
    const t = setTimeout(() => check(true), 4000);
    return () => clearTimeout(t);
  }, [info?.updater]); // eslint-disable-line react-hooks/exhaustive-deps

  const install = async () => {
    if (!update) return;
    if (eng.tx?.recording || eng.tx?.playing) {
      setError("Stop playback/recording before installing an update.");
      return;
    }
    setPhase("downloading");
    setError(null);
    setProgress({ got: 0, total: null });
    try {
      await update.downloadAndInstall((ev) => {
        if (ev.event === "Started") setProgress({ got: 0, total: ev.data?.contentLength ?? null });
        if (ev.event === "Progress") setProgress((p) => ({ ...p, got: p.got + (ev.data?.chunkLength ?? 0) }));
      });
      setPhase("ready");
    } catch (e) {
      setPhase("error");
      setError(String(e));
    }
  };

  const restart = async () => {
    if (eng.view?.open) await eng.run("project.save");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  };

  return { info, phase, update, error, progress, check, install, restart, enabled: !!info?.updater };
}

export type UpdatesHook = ReturnType<typeof useUpdates>;

/** Software update sheet: what's installed, what's new, download, restart. */
export function UpdatesDialog({ u, onClose }: { u: UpdatesHook; onClose: () => void }) {
  const [auto, setAuto] = useState(getLayout().autoUpdateCheck !== false);
  useEffect(() => {
    if (u.phase === "idle") u.check();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const pct = u.progress.total ? Math.min(100, (u.progress.got / u.progress.total) * 100) : null;
  const head: Record<string, [string, string, string]> = {
    idle: ["…", "", "Checking for updates…"],
    checking: ["…", "", "Checking for updates…"],
    none: ["✓", "ok", "You're up to date"],
    available: ["↓", "accent", "A new version is available"],
    downloading: ["↓", "accent", "Downloading the update…"],
    ready: ["✓", "ok", "Ready to restart"],
    error: ["✕", "bad", "The update check didn't work"],
  };
  const [sym, tone, title] = head[u.phase] ?? head.idle;
  return (
    <div className="modal-bg" onMouseDown={onClose}>
      <div className="modal updates" role="dialog" aria-label="Software update" onMouseDown={(e) => e.stopPropagation()}>
        <div className="panel-title">Software update</div>
        <div className="up-head">
          <span className={"up-sym " + tone} aria-hidden>{sym}</span>
          <div>
            <div className="up-title">{title}</div>
            <div className="up-versions">
              <span className="label">Installed</span><span className="readout sm">{u.info?.version ?? "?"}</span>
              {u.update && <><span className="dim">→</span><span className="label">New</span><span className="readout sm">{u.update.version}</span>{u.update.date && <span className="small dim">{u.update.date.slice(0, 10)}</span>}</>}
            </div>
          </div>
        </div>
        {(u.phase === "available" || u.phase === "downloading" || u.phase === "ready") && u.update?.body && (
          <section className="up-notes"><span className="label">What's new</span><pre className="notes">{u.update.body}</pre></section>
        )}
        {u.phase === "downloading" && (
          <div className="dl" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct === null ? undefined : Math.round(pct)}>
            <div className="dl-bar"><div style={{ width: (pct ?? 30) + "%" }} className={pct === null ? "indeterminate" : ""} /></div>
            <span className="small mono">{(u.progress.got / 1048576).toFixed(1)}{u.progress.total ? ` / ${(u.progress.total / 1048576).toFixed(1)}` : ""} MB</span>
          </div>
        )}
        {u.phase === "ready" && <div className="small">Installed and verified. Restarting saves your project first.</div>}
        {u.error && <div className="small" style={{ color: "var(--error-text)" }}>{u.error}</div>}
        <label className="check"><input type="checkbox" checked={auto} onChange={(e) => { setAuto(e.target.checked); saveLayout({ autoUpdateCheck: e.target.checked }); }} /> Check for updates when the app starts</label>
        <div className="btns">
          {(u.phase === "none" || u.phase === "error") && <button onClick={() => u.check()}>Check again</button>}
          <span className="spacer" />
          <button onClick={onClose}>{u.phase === "ready" ? "Later" : "Close"}</button>
          {u.phase === "available" && <button className="primary" onClick={u.install}>Download & install</button>}
          {u.phase === "ready" && <button className="primary" onClick={u.restart}>Restart now</button>}
        </div>
      </div>
    </div>
  );
}

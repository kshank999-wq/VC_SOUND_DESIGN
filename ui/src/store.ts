import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { call } from "./api";
import type { ProjectView, TransportState } from "./types";
import type { Rate } from "./tc";

export interface Toast {
  id: number;
  text: string;
  kind: "error" | "info";
}

/** Live connection to the engine: project view + transport polled at ~30 Hz. */
export function useEngine() {
  const [view, setView] = useState<ProjectView | null>(null);
  const [tx, setTx] = useState<TransportState | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const versionRef = useRef(-1);
  const polledAt = useRef(performance.now());
  const fetching = useRef(false);

  const toast = useCallback((text: string, kind: Toast["kind"] = "info") => {
    const id = Math.random();
    setToasts((t) => [...t.slice(-3), { id, text, kind }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "error" ? 7000 : 3500);
  }, []);

  const refresh = useCallback(async () => {
    if (fetching.current) return;
    fetching.current = true;
    try {
      const v = await call<ProjectView>("state.get");
      versionRef.current = v.version;
      setView(v);
    } finally {
      fetching.current = false;
    }
  }, []);

  useEffect(() => {
    let alive = true;
    refresh();
    const tick = async () => {
      if (!alive) return;
      try {
        const t = await call<TransportState>("transport.state");
        polledAt.current = performance.now();
        setTx(t);
        if (t.version !== versionRef.current) refresh();
      } catch {
        /* engine busy or restarting */
      }
      if (alive) setTimeout(tick, 33);
    };
    tick();
    return () => {
      alive = false;
    };
  }, [refresh]);

  /** Run an engine command; errors become toasts. Returns null on failure. */
  const run = useCallback(
    async <T = any>(cmd: string, args: any = {}, okText?: string): Promise<T | null> => { // eslint-disable-line
      try {
        const r = await call<T>(cmd, args);
        if (okText) toast(okText);
        await refresh();
        return r;
      } catch (e) {
        toast(String((e as Error).message ?? e), "error");
        return null;
      }
    },
    [refresh, toast],
  );

  const rate: Rate | null = useMemo(
    () =>
      view?.open
        ? { num: view.project.fps_num, den: view.project.fps_den, df: view.project.drop_frame, sr: view.project.sample_rate, startFrame: view.project.start_frame }
        : null,
    [view?.open, view?.project?.fps_num, view?.project?.fps_den, view?.project?.drop_frame, view?.project?.sample_rate, view?.project?.start_frame],
  );

  /** Smooth playhead: last polled position + elapsed wall time while playing. */
  const livePosition = useCallback(() => {
    if (!tx || !rate) return 0;
    if (!tx.playing) return tx.position;
    return tx.position + ((performance.now() - polledAt.current) / 1000) * rate.sr;
  }, [tx, rate]);

  return { view, tx, rate, run, refresh, toast, toasts, livePosition };
}

export type EngineHook = ReturnType<typeof useEngine>;

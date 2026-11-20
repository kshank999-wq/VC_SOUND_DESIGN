// One API, two transports: Tauri IPC in the desktop app, HTTP in browser mode
// (`vcsd serve`). Both reach the same engine command dispatcher.

type Json = any; // eslint-disable-line @typescript-eslint/no-explicit-any

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let tauriInvoke: ((cmd: string, args: Json) => Promise<Json>) | null = null;
let tauriConvert: ((p: string) => string) | null = null;
let mediaBase: string | null = null;

async function loadTauri() {
  if (!isTauri || tauriInvoke) return;
  const core = await import("@tauri-apps/api/core");
  tauriInvoke = core.invoke;
  tauriConvert = core.convertFileSrc;
}

export class ApiError extends Error {}

export async function call<T = Json>(cmd: string, args: Json = {}): Promise<T> {
  if (isTauri) {
    await loadTauri();
    try {
      return (await tauriInvoke!("api", { cmd, args })) as T;
    } catch (e) {
      throw new ApiError(String(e));
    }
  }
  const res = await fetch(`/api/${cmd}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args ?? {}),
  });
  const body = await res.json();
  if ("error" in body) throw new ApiError(body.error);
  return body.ok as T;
}

/** URL the webview can load a local media file from. */
export function mediaUrl(path: string): string {
  // Desktop: the engine's loopback media server streams with Range support on
  // every webview (WebKitGTK cannot stream video from custom URI schemes).
  if (isTauri && mediaBase) return `${mediaBase}&path=${encodeURIComponent(path)}`;
  if (isTauri && tauriConvert) return tauriConvert(path);
  return `/media?path=${encodeURIComponent(path)}`;
}

export async function initApi() {
  await loadTauri();
  if (isTauri) {
    try {
      mediaBase = (await call<{ media_base: string | null }>("app.info")).media_base;
    } catch {
      mediaBase = null;
    }
  }
}

/** Open the detachable mixer (second window / second monitor). Its open
 *  state is remembered so it comes back at the next launch. */
export async function openMixerWindow() {
  const { saveLayout } = await import("./layout");
  if (isTauri) {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const existing = await WebviewWindow.getByLabel("mixer");
    if (existing) {
      await existing.setFocus();
      return;
    }
    const w = new WebviewWindow("mixer", { url: "index.html?view=mixer", title: "VC Sound Design — Mixer", width: 1100, height: 520 });
    saveLayout({ mixerOpen: true });
    w.once("tauri://destroyed", () => saveLayout({ mixerOpen: false }));
  } else {
    const w = window.open("?view=mixer", "vcsd-mixer", "width=1100,height=520");
    if (!w) return;
    saveLayout({ mixerOpen: true });
    const poll = setInterval(() => {
      if (w.closed) {
        clearInterval(poll);
        saveLayout({ mixerOpen: false });
      }
    }, 1000);
  }
}

/** Desktop: save native window size/position/maximised state on resize/move
 *  (restored automatically by the window-state plugin when windows open). */
export async function trackWindowState() {
  if (!isTauri) return;
  const { saveWindowState, StateFlags } = await import("@tauri-apps/plugin-window-state");
  let t: ReturnType<typeof setTimeout> | null = null;
  const save = () => {
    if (t) clearTimeout(t);
    t = setTimeout(() => saveWindowState(StateFlags.ALL).catch(() => {}), 400);
  };
  window.addEventListener("resize", save);
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  getCurrentWindow().onMoved(save).catch(() => {});
}

/** Native folder/file picker in the desktop app; null in browser mode. */
export async function nativePick(opts: { directory: boolean; title: string; filters?: { name: string; extensions: string[] }[] }): Promise<string | null> {
  if (!isTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: opts.directory, multiple: false, title: opts.title, filters: opts.filters });
  return typeof r === "string" ? r : null;
}

export interface BuildInfo {
  version: string;
  /** Built with an update signing key: in-app updates available. */
  updater: boolean;
}

/** Version and update capability of the running app. */
export async function buildInfo(): Promise<BuildInfo> {
  if (isTauri) {
    await loadTauri();
    return (await tauriInvoke!("build_info", {})) as BuildInfo;
  }
  const info = await call<{ version: string }>("app.info");
  return { version: info.version, updater: false };
}

/** Desktop only: a plugin's own editor in a native window. Rejects if the plugin
 *  has no editor or the window system can't host one (the caller falls back). */
export async function openNativeEditor(slot: string, title: string): Promise<void> {
  if (!isTauri) throw new ApiError("plugin windows need the desktop app");
  await loadTauri();
  await tauriInvoke!("plugin_editor_open", { slot, title });
}
export async function closeNativeEditor(slot: string): Promise<void> {
  if (!isTauri) return;
  await loadTauri();
  await tauriInvoke!("plugin_editor_close", { slot }).catch(() => {});
}
export async function openNativeEditors(): Promise<string[]> {
  if (!isTauri) return [];
  await loadTauri();
  return ((await tauriInvoke!("plugin_editors_open", {}).catch(() => [])) as string[]) ?? [];
}

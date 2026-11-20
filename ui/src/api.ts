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

/** Open the detachable mixer (second window / second monitor). */
export async function openMixerWindow() {
  if (isTauri) {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const existing = await WebviewWindow.getByLabel("mixer");
    if (existing) {
      await existing.setFocus();
      return;
    }
    new WebviewWindow("mixer", { url: "index.html?view=mixer", title: "VC Sound Design — Mixer", width: 1100, height: 520 });
  } else {
    window.open("?view=mixer", "vcsd-mixer", "width=1100,height=520");
  }
}

/** Native folder/file picker in the desktop app; null in browser mode. */
export async function nativePick(opts: { directory: boolean; title: string; filters?: { name: string; extensions: string[] }[] }): Promise<string | null> {
  if (!isTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: opts.directory, multiple: false, title: opts.title, filters: opts.filters });
  return typeof r === "string" ? r : null;
}

// Window layout persistence through the engine's per-user prefs (shared by the
// desktop app and browser mode; survives webview cache resets).
import { call } from "./api";
import { merge, sanitize, type Layout } from "./layoutModel";

let current: Layout = {};
let pending: Layout = {};
let timer: ReturnType<typeof setTimeout> | null = null;

export async function loadLayout(): Promise<Layout> {
  try {
    current = sanitize(await call("layout.get"));
  } catch {
    current = {};
  }
  return current;
}

export function getLayout(): Layout {
  return current;
}

/** Save part of the layout (debounced; parts merge). */
export function saveLayout(patch: Layout) {
  current = merge(current, patch);
  pending = merge(pending, patch);
  if (timer) clearTimeout(timer);
  timer = setTimeout(flushLayout, 350);
}

export function flushLayout() {
  if (timer) clearTimeout(timer);
  timer = null;
  const patch = pending;
  pending = {};
  if (Object.keys(patch).length) call("layout.set", { patch }).catch(() => {});
}

export async function resetLayout() {
  if (timer) clearTimeout(timer);
  pending = {};
  current = {};
  await call("layout.set", { reset: true });
}

if (typeof window !== "undefined") window.addEventListener("beforeunload", flushLayout);

// Plugin parameter lists by slot, shared by the timeline (lane names) and the
// plugin panel. Loaded on demand; re-rendering components subscribe.
import { useSyncExternalStore } from "react";
import { call } from "./api";
import type { PluginParam } from "./types";

const cache = new Map<string, PluginParam[]>();
const loading = new Set<string>();
const subs = new Set<() => void>();
let version = 0;

function notify() {
  version++;
  subs.forEach((f) => f());
}

/** Parameters of a plugin slot (undefined until loaded; starts loading). */
export function paramsOf(slot: string): PluginParam[] | undefined {
  if (!cache.has(slot) && !loading.has(slot)) {
    loading.add(slot);
    call<{ params: PluginParam[] }>("plugin.params", { id: slot })
      .then((r) => cache.set(slot, r.params ?? []))
      .catch(() => cache.set(slot, []))
      .finally(() => {
        loading.delete(slot);
        notify();
      });
  }
  return cache.get(slot);
}

export function paramTitle(slot: string, id: number): string {
  return paramsOf(slot)?.find((p) => p.id === id)?.title ?? `#${id}`;
}

/** Fresh values from the plugin panel (or after a change). */
export function storeParams(slot: string, params: PluginParam[]) {
  cache.set(slot, params);
  notify();
}

/** Re-render when parameter lists arrive. */
export function useParamsVersion(): number {
  return useSyncExternalStore((cb) => (subs.add(cb), () => subs.delete(cb)), () => version);
}

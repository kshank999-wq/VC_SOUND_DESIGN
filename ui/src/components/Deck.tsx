// The bottom deck (spec v2.1 › Regions › Bottom deck): Pads · Library browser · Mixer · Log.
// One placement for the mixer: here, detachable into its own window.
import { useEffect, useMemo, useState } from "react";
import { call, openMixerWindow } from "../api";
import { getLayout } from "../layout";
import type { EngineHook } from "../store";
import type { DeckTab } from "../stage";
import { laneColor, type Asset } from "../types";
import { Mixer } from "./Mixer";
import { Pads } from "./Pads";
import { Key } from "./Shell";

const TABS: { id: DeckTab; label: string }[] = [
  { id: "pads", label: "Pads" },
  { id: "library", label: "Library browser" },
  { id: "mixer", label: "Mixer" },
  { id: "log", label: "Log" },
];

interface DeckProps {
  eng: EngineHook;
  tab: DeckTab;
  open: boolean;
  height: number;
  setTab: (t: DeckTab) => void;
  setOpen: (o: boolean) => void;
  libSel: Asset | null;
  performCue: string | null;
  onPerformClose: () => void;
  /** The mixer is in its own window right now. */
  mixerDetached: boolean;
  setMixerDetached: (d: boolean) => void;
}

export function BottomDeck(p: DeckProps) {
  const { view } = p.eng;
  if (!view?.open) return null;
  const pick = (t: DeckTab) => {
    if (t === p.tab && p.open) p.setOpen(false);
    else {
      p.setTab(t);
      p.setOpen(true);
    }
  };
  return (
    <section className={"panel deck" + (p.open ? "" : " collapsed")} style={{ height: p.open ? p.height : undefined }} aria-label="Bottom deck">
      <div className="deck-tabs" role="tablist">
        {TABS.map((t) => (
          <button key={t.id} role="tab" aria-selected={p.open && p.tab === t.id} className={"deck-tab" + (p.open && p.tab === t.id ? " active" : "")} onClick={() => pick(t.id)}>
            {t.label}
            {t.id === "pads" && p.performCue && <span className="led rec on" />}
          </button>
        ))}
        <span className="spacer" />
        <span className="deck-hint">{hint(p)}</span>
        {p.tab === "mixer" && p.open && !p.mixerDetached && (
          <Key title="Detach the mixer into its own window" onClick={() => { openMixerWindow(); p.setMixerDetached(true); }}>⧉ Detach</Key>
        )}
        <Key aria-label={p.open ? "Collapse the deck" : "Open the deck"} aria-expanded={p.open} onClick={() => p.setOpen(!p.open)}>{p.open ? "▾" : "▴"}</Key>
      </div>
      {p.open && (
        <div className="deck-body">
          {p.tab === "pads" && (p.performCue ? <Pads eng={p.eng} cueId={p.performCue} onClose={p.onPerformClose} /> : <SpotPads eng={p.eng} />)}
          {p.tab === "library" && <LibraryBrowser eng={p.eng} asset={p.libSel} />}
          {p.tab === "mixer" && (p.mixerDetached ? (
            <div className="deck-empty">
              The mixer is in its own window.
              <Key onClick={() => p.setMixerDetached(false)}>Bring it back here</Key>
            </div>
          ) : <Mixer eng={p.eng} />)}
          {p.tab === "log" && <Log eng={p.eng} />}
        </div>
      )}
    </section>
  );
}

function hint(p: DeckProps): string {
  const v = p.eng.view!;
  if (p.tab === "pads") return p.performCue ? "Perform pads · J K L ; U I O P (Shift = accent)" : "Spot keys · tap while picture plays";
  if (p.tab === "mixer") return `${v.tracks.length} tracks · ${v.buses.length} buses`;
  if (p.tab === "library") return p.libSel ? p.libSel.name : "Select a sound in the Library";
  return "Engine log";
}

const KEY_LABEL = (k: string) => k.replace(/^key:(Key|Digit)?/, "");

/** The spot keys as pads (Spot stage): what each key spots, whether a range is open. */
function SpotPads({ eng }: { eng: EngineHook }) {
  const { view, tx, run } = eng;
  const keys = useMemo(() => Object.entries(view?.bindings ?? {}).filter(([k]) => k.startsWith("key:")).sort((a, b) => a[0].localeCompare(b[0])), [view?.bindings]);
  if (!view) return null;
  const open = tx?.open_spots ?? [];
  const order = "FPGBDNM";
  keys.sort((a, b) => (order.indexOf(KEY_LABEL(a[0])) + 99) % 99 - (order.indexOf(KEY_LABEL(b[0])) + 99) % 99);
  return (
    <div className="pad-grid">
      {keys.map(([k, b]) => {
        const lane = view.lanes.find((l) => l.id === b.lane_id);
        const color = lane ? laneColor(lane) : "var(--text-dim)";
        const isOpen = open.includes(k);
        return (
          <button key={k} className={"pad" + (isOpen ? " open" : "")} style={{ "--pad": color } as React.CSSProperties}
            title={`Spot ${b.label} (${KEY_LABEL(k)}) at the playhead`} onClick={() => run("spot.tap", { key: k })}>
            <span className="pad-key">{KEY_LABEL(k)}</span>
            {isOpen && <span className="pad-open">open</span>}
            <span className="pad-name">{lane?.name ?? b.label}</span>
            <span className="label">{b.kind === "point" ? "point" : b.kind}</span>
          </button>
        );
      })}
      <button className="pad close" title="Close every open range (Enter)" disabled={!open.length} onClick={() => run("spot.finish", {})}>
        <span className="pad-key">⏎</span>
        <span className="pad-name">Close range</span>
        <span className="label">Shift+key also closes</span>
      </button>
    </div>
  );
}

/** The selected sound: waveform, audition, details. */
function LibraryBrowser({ eng, asset }: { eng: EngineHook; asset: Asset | null }) {
  const { tx, rate } = eng;
  const [peaks, setPeaks] = useState<{ per: number; data: number[] } | null>(null);
  useEffect(() => {
    setPeaks(null);
    if (asset && !asset.missing) call<{ per: number; data: number[] }>("asset.peaks", { id: asset.id }).then(setPeaks).catch(() => setPeaks(null));
  }, [asset?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!asset) return <div className="deck-empty">Select a sound in the Library (left rack) to see and audition it here.</div>;
  const bars = 240;
  const path = peaks ? waveformPath(peaks.data, bars) : "";
  const secs = asset.length / (asset.sample_rate || rate?.sr || 48000);
  return (
    <div className="libbrowser">
      <div className="wave-well">
        {peaks ? (
          <svg viewBox={`0 0 ${bars} 100`} preserveAspectRatio="none" className="wave"><path d={path} /></svg>
        ) : <span className="dim">{asset.missing ? "Media missing" : "Loading waveform…"}</span>}
      </div>
      <div className="lib-meta">
        <div className="lib-title">{asset.name}</div>
        <div className="lib-facts">
          <span className="readout sm">{secs.toFixed(2)} s</span>
          <span className="pill">{asset.sample_rate / 1000} kHz</span>
          <span className="pill">{asset.channels === 1 ? "mono" : asset.channels === 2 ? "stereo" : `${asset.channels} ch`}</span>
          {asset.loopable && <span className="pill">loopable</span>}
          {asset.license && <span className="pill" title="License">{asset.license}</span>}
        </div>
        <div className="lib-tags dim">{asset.tags.join(" · ")}</div>
        <div className="btns">
          <Key on={!!tx?.previewing} onClick={() => call(tx?.previewing ? "audition.stop" : "audition", { asset_id: asset.id })}>{tx?.previewing ? "■ Stop" : "▶ Audition"}</Key>
        </div>
      </div>
    </div>
  );
}

/** Min/max pairs → a filled waveform path in a 0–100 tall box. */
export function waveformPath(data: number[], bars: number): string {
  const n = Math.floor(data.length / 2);
  if (!n) return "";
  const step = n / bars;
  const top: string[] = [];
  const bot: string[] = [];
  for (let i = 0; i < bars; i++) {
    let lo = 0;
    let hi = 0;
    for (let k = Math.floor(i * step); k < Math.max(Math.floor(i * step) + 1, Math.floor((i + 1) * step)) && k < n; k++) {
      lo = Math.min(lo, data[k * 2]);
      hi = Math.max(hi, data[k * 2 + 1]);
    }
    top.push(`${i},${(50 - hi * 48).toFixed(1)}`);
    bot.unshift(`${i},${(50 - lo * 48).toFixed(1)}`);
  }
  return `M${top.join("L")}L${bot.join("L")}Z`;
}

function Log({ eng }: { eng: EngineHook }) {
  const [lines, setLines] = useState<string[]>([]);
  useEffect(() => {
    let alive = true;
    const load = () => call<string[]>("log.get").then((l) => alive && setLines(l)).catch(() => {});
    load();
    const t = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [eng.view?.version]);
  return <pre className="deck-log">{lines.length ? lines.join("\n") : "Nothing logged yet."}</pre>;
}

/** Whether the mixer was left detached last time. */
export const mixerWasDetached = () => !!getLayout().mixerOpen;

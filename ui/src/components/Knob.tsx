// A small rotary knob (spec v2.1 › Components): drag vertically (Shift = fine), double-click
// resets, arrow keys step. The engine gets one value when the gesture ends (one undo step).
import { useEffect, useRef, useState } from "react";

interface KnobProps {
  value: number;
  min: number;
  max: number;
  /** Double-click value. */
  reset: number;
  step: number;
  label: string;
  /** Shown in the tooltip and to screen readers. */
  format: (v: number) => string;
  onCommit: (v: number) => void;
  /** Live gesture (automation writing): called on grab and on every move; onCommit ends it. */
  onBegin?: (v: number) => void;
  onMove?: (v: number) => void;
  size?: number;
  /** Draw the arc from the centre (pan) rather than from the minimum (gain). */
  bipolar?: boolean;
  color?: string;
}

const SWEEP = 270; // degrees, −135° … +135°

export function Knob({ value, min, max, reset, step, label, format, onCommit, onBegin, onMove, size = 24, bipolar = false, color }: KnobProps) {
  const [live, setLive] = useState<number | null>(null);
  const drag = useRef<{ y: number; v: number } | null>(null);
  const v = live ?? value;
  useEffect(() => setLive(null), [value]);
  const clamp = (x: number) => Math.max(min, Math.min(max, Math.round(x / step) * step));
  const frac = (x: number) => (x - min) / (max - min);
  const ang = (x: number) => -SWEEP / 2 + frac(x) * SWEEP;
  const r = size / 2 - 2.5;
  const c = size / 2;
  const pt = (deg: number) => {
    const a = ((deg - 90) * Math.PI) / 180;
    return [c + r * Math.cos(a), c + r * Math.sin(a)];
  };
  const arc = (a0: number, a1: number) => {
    const [x0, y0] = pt(a0);
    const [x1, y1] = pt(a1);
    return `M${x0.toFixed(2)},${y0.toFixed(2)}A${r},${r} 0 ${Math.abs(a1 - a0) > 180 ? 1 : 0} ${a1 > a0 ? 1 : 0} ${x1.toFixed(2)},${y1.toFixed(2)}`;
  };
  const from = bipolar ? 0 : -SWEEP / 2;
  const to = ang(v);
  const [px, py] = pt(to);
  return (
    <span
      className="knob"
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={Math.round(v * 100) / 100}
      aria-valuetext={format(v)}
      title={`${label}: ${format(v)} (drag; Shift = fine; double-click resets)`}
      style={{ width: size, height: size, ...(color ? { "--knob": color } : {}) } as React.CSSProperties}
      onMouseDown={(e) => e.stopPropagation()}
      onPointerDown={(e) => {
        e.stopPropagation();
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        drag.current = { y: e.clientY, v };
        onBegin?.(v);
      }}
      onPointerMove={(e) => {
        if (!drag.current) return;
        const per = (max - min) / (e.shiftKey ? 600 : 150); // value per pixel
        const nv = clamp(drag.current.v + (drag.current.y - e.clientY) * per);
        setLive(nv);
        onMove?.(nv);
      }}
      onPointerUp={() => {
        if (!drag.current) return;
        drag.current = null;
        // a live gesture always ends (automation); a plain one commits only a change
        if (onBegin) onCommit(live ?? value);
        else if (live !== null && live !== value) onCommit(live);
      }}
      onDoubleClick={() => onCommit(reset)}
      onKeyDown={(e) => {
        const d = e.key === "ArrowUp" || e.key === "ArrowRight" ? 1 : e.key === "ArrowDown" || e.key === "ArrowLeft" ? -1 : 0;
        if (!d) return;
        e.preventDefault();
        e.stopPropagation();
        onCommit(clamp(v + d * step * (e.shiftKey ? 1 : 2)));
      }}
    >
      <svg width={size} height={size} aria-hidden>
        <path d={arc(-SWEEP / 2, SWEEP / 2)} className="knob-track" />
        {Math.abs(to - from) > 0.5 && <path d={from < to ? arc(from, to) : arc(to, from)} className="knob-arc" />}
        <circle cx={c} cy={c} r={r - 3.5} className="knob-cap" />
        <line x1={c} y1={c} x2={c + (px - c) * 0.62} y2={c + (py - c) * 0.62} className="knob-ptr" />
      </svg>
    </span>
  );
}

export const fmtGain = (v: number) => `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`;
export const fmtPan = (v: number) => (Math.abs(v) < 0.005 ? "C" : `${v < 0 ? "L" : "R"}${Math.round(Math.abs(v) * 100)}`);

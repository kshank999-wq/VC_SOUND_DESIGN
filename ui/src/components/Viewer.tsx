import { useEffect, useRef, useState } from "react";
import { mediaUrl } from "../api";
import type { EngineHook } from "../store";
import { sampleAtFrame, tcAt } from "../tc";

/** Picture viewer. The audio engine clock is the master: the video element
 *  is slaved to the transport position (re-seeked when drift > 2 frames,
 *  parked mid-frame when stopped so frame stepping is exact). */
export function Viewer({ eng }: { eng: EngineHook }) {
  const { view, tx, rate, livePosition, run } = eng;
  const vid = useRef<HTMLVideoElement>(null);
  const [proxy, setProxy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [refAudio, setRefAudio] = useState(false);
  const [tc, setTc] = useState("");
  const pic = view?.picture;
  const src = pic ? mediaUrl(proxy ?? pic.path) : null;

  useEffect(() => {
    setErr(null);
    setProxy(null);
  }, [pic?.id]);

  useEffect(() => {
    let raf = 0;
    const loop = () => {
      raf = requestAnimationFrame(loop);
      const v = vid.current;
      if (!rate || !pic) return;
      const pos = livePosition();
      setTc(tcAt(rate, pos));
      if (!v || v.readyState < 1) return;
      const fps = rate.num / rate.den;
      const picStart = sampleAtFrame(rate, pic.start_frame - rate.startFrame);
      const t = (pos - picStart) / rate.sr;
      const playing = tx?.playing ?? false;
      if (playing) {
        // Clock follower: the engine is master. Small drift is corrected by
        // nudging playbackRate (no visible jumps); only large jumps re-seek.
        const drift = t - v.currentTime; // > 0: picture is behind the audio
        if (v.paused) {
          v.currentTime = Math.max(0, t + 0.04);
          v.playbackRate = 1;
          v.play().catch(() => {});
        } else if (Math.abs(drift) > 0.5) {
          v.currentTime = Math.max(0, t + 0.08);
        } else {
          const rate = Math.max(0.85, Math.min(1.15, 1 + drift * 3));
          if (Math.abs(v.playbackRate - rate) > 0.002) v.playbackRate = rate;
        }
      } else {
        if (!v.paused) v.pause();
        v.playbackRate = 1;
        const target = Math.max(0, t + 0.5 / fps);
        if (Math.abs(v.currentTime - target) > 0.25 / fps) v.currentTime = target;
      }
    };
    loop();
    return () => cancelAnimationFrame(raf);
  }, [rate, pic, tx?.playing, livePosition]);

  if (!view?.open || !rate) return null;
  const scene = view.scenes.find((s) => {
    const p = tx?.position ?? 0;
    return p >= s.start_sample && p < s.end_sample;
  });
  const p = tx?.position ?? 0;
  const one = sampleAtFrame(rate, 1);
  const active = view.cues.filter((c) => c.status !== "conceptual" && p >= c.start_sample && p < Math.max(c.end_sample, c.start_sample + one * 6));

  return (
    <div className="viewer">
      {src ? (
        <video
          ref={vid}
          src={src}
          muted={!refAudio}
          preload="auto"
          playsInline
          onError={() => setErr("This picture format can't play in the viewer.")}
          onLoadedData={() => setErr(null)}
        />
      ) : (
        <div className="no-picture">
          <div>No picture imported</div>
          <div className="small muted">Import a reference cut from the Project menu</div>
        </div>
      )}
      <div className="burnin mono">{tc}</div>
      {scene && <div className="scene-badge">SC {scene.number} · {scene.title}</div>}
      <div className="active-cues">
        {active.slice(0, 6).map((c) => (
          <div key={c.id} className="small">{c.code} {c.description}</div>
        ))}
      </div>
      {err && (
        <div className="viewer-err">
          {err}
          <button onClick={async () => {
            setErr("Making an H.264 proxy…");
            const r = await run<{ path: string }>("picture.proxy");
            if (r) setProxy(r.path);
            else setErr("Proxy failed (is ffmpeg installed?)");
          }}>Make playback proxy</button>
        </div>
      )}
      <label className="refaudio small"><input type="checkbox" checked={refAudio} onChange={(e) => setRefAudio(e.target.checked)} /> ref audio</label>
    </div>
  );
}

//! Mixer model and deterministic sample-accurate render engine (spec §12).
//!
//! The same `Mixer::render_*` code path serves block-based transport playback
//! and offline export, so what you hear is what you deliver. Audio is mixed in
//! f32; positions are integer samples.

use crate::error::{invalid, Result};
use crate::ids::new_id;
use crate::model::*;
use crate::state::ProjectState;
use crate::wav::AudioBuffer;
use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Decoded-audio cache shareable between mixers/threads (e.g. the realtime
/// render thread rebuilds its mixer on every edit but keeps decoded media).
pub type SourceCache = Arc<Mutex<HashMap<String, Arc<AudioBuffer>>>>;

pub const MASTER_BUS: &str = "Review Mix";

pub fn create_default_buses(s: &mut ProjectState) {
    let master = new_id("bus");
    s.buses.insert(
        master.clone(),
        Bus {
            id: master.clone(),
            name: MASTER_BUS.into(),
            order: 100,
            parent_id: None,
            gain_db: 0.0,
            mute: false,
            stem: false,
            inserts: vec![],
            automation_mode: AutoMode::Read,
        },
    );
    for (i, name) in ["Dialogue Reference", "Foley", "Hard FX", "Backgrounds/Atmos", "Design FX", "Music/Score"].iter().enumerate() {
        let id = new_id("bus");
        s.buses.insert(
            id.clone(),
            Bus {
                id,
                name: (*name).into(),
                order: i as u32,
                parent_id: Some(master.clone()),
                gain_db: 0.0,
                mute: false,
                stem: true,
                inserts: vec![],
                automation_mode: AutoMode::Read,
            },
        );
    }
}

pub fn master_bus(s: &ProjectState) -> Result<&Bus> {
    s.buses
        .values()
        .find(|b| b.parent_id.is_none() && b.name == MASTER_BUS)
        .or_else(|| s.buses.values().find(|b| b.parent_id.is_none()))
        .ok_or_else(|| invalid("no master bus"))
}

pub fn add_bus(s: &mut ProjectState, name: &str, parent: Option<&str>) -> Result<String> {
    let parent = match parent {
        Some(p) => s.buses.get(p).map(|b| b.id.clone()).ok_or_else(|| crate::error::not_found("bus", p))?,
        None => master_bus(s)?.id.clone(),
    };
    let id = new_id("bus");
    let order = s.buses.len() as u32;
    s.buses.insert(
        id.clone(),
        Bus {
            id: id.clone(),
            name: name.into(),
            order,
            parent_id: Some(parent),
            gain_db: 0.0,
            mute: false,
            stem: true,
            inserts: vec![],
            automation_mode: AutoMode::Read,
        },
    );
    Ok(id)
}

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Constant-power pan law: pan -1 (L) .. +1 (R) → (gain L, gain R), -3 dB centre.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * PI;
    (p.cos(), p.sin())
}

// ---------------------------------------------------------------- DSP

#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    fn new(kind: &Insert, sr: f32) -> Option<Biquad> {
        let (hz, q, db, k) = match *kind {
            Insert::HighPass { hz, q } => (hz, q, 0.0, 0),
            Insert::LowPass { hz, q } => (hz, q, 0.0, 1),
            Insert::Peak { hz, q, db } => (hz, q, db, 2),
            _ => return None,
        };
        let w0 = 2.0 * PI * hz.clamp(10.0, sr * 0.49) / sr;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q.max(0.05));
        let a = 10f32.powf(db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match k {
            0 => ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            1 => ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            _ => (1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a),
        };
        Some(Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 })
    }
}

#[derive(Debug, Clone, Default)]
struct InsertState {
    z: Vec<[f32; 4]>, // per channel x1 x2 y1 y2
    env: f32,
}

/// Runs hosted (VST3/AU) plugins for the mixer. Lives outside the core so
/// offline tools keep working without one: plugin slots then pass audio
/// through and the render reports it.
pub trait PluginHost: Send + Sync {
    /// Process `buf` (interleaved, `buf.channels`) in place. `pos` is the
    /// timeline sample of the first frame; a jump in `pos` resets the plugin.
    /// `automation` holds the slot's automated parameters over this block.
    fn process(&self, slot: &PluginInsert, buf: &mut AudioBuffer, pos: i64, automation: &[ParamCurve]) -> std::result::Result<(), String>;
    /// Latency the plugin reports, in samples (compensated by reading ahead).
    fn latency(&self, slot: &PluginInsert) -> i64;
    /// Where the slot's stream continues from (None: fresh or unknown). Used to
    /// pre-roll a latent chain after a jump so its first output isn't silence.
    fn continues_at(&self, _slot: &PluginInsert) -> Option<i64> {
        None
    }
}

/// One plugin parameter's automation over a processed block: `(frame offset,
/// normalised value)` points, the first at offset 0, linear in between.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamCurve {
    pub id: u32,
    pub points: Vec<(u32, f64)>,
}

/// One automated plugin parameter: its id and sorted breakpoints.
type PluginLane<'a> = (u32, Vec<&'a AutomationPoint>);

/// Value of a sorted breakpoint list at `s` (linear, held at the ends).
fn lane_value(pts: &[&AutomationPoint], s: i64) -> f32 {
    let i = pts.partition_point(|p| p.sample <= s);
    match (i.checked_sub(1).map(|k| pts[k]), pts.get(i)) {
        (None, Some(b)) => b.value,
        (Some(a), None) => a.value,
        (Some(a), Some(b)) => a.value + (b.value - a.value) * ((s - a.sample) as f32 / (b.sample - a.sample).max(1) as f32),
        (None, None) => 0.0,
    }
}

/// Shareable handle to a [`PluginHost`] (e.g. in [`crate::export::ExportOptions`]).
#[derive(Clone)]
pub struct PluginHostRef(pub Arc<dyn PluginHost>);

impl std::fmt::Debug for PluginHostRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginHost")
    }
}

fn process_inserts(
    inserts: &[Insert],
    states: &mut Vec<InsertState>,
    buf: &mut AudioBuffer,
    pos: i64,
    host: Option<&dyn PluginHost>,
    curves: &dyn Fn(&str) -> Vec<ParamCurve>,
    warn: &dyn Fn(String),
) {
    let sr = buf.sample_rate as f32;
    let ch = buf.channels as usize;
    if states.len() != inserts.len() {
        *states = vec![InsertState::default(); inserts.len()];
    }
    for (ins, st) in inserts.iter().zip(states.iter_mut()) {
        match ins {
            Insert::Plugin(p) => {
                if p.bypass {
                    continue;
                }
                match host {
                    Some(h) => {
                        if let Err(e) = h.process(p, buf, pos, &curves(&p.id)) {
                            warn(format!("plugin '{}' passed through: {e}", p.name));
                        }
                    }
                    None => warn(format!("plugin '{}' passed through: plugins are not available in this render", p.name)),
                }
            }
            Insert::Gain { db } => {
                let g = db_to_lin(*db);
                buf.data.iter_mut().for_each(|v| *v *= g);
            }
            Insert::Compressor { threshold_db, ratio, attack_ms, release_ms, makeup_db } => {
                let att = (-1.0 / (attack_ms.max(0.01) * 0.001 * sr)).exp();
                let rel = (-1.0 / (release_ms.max(0.01) * 0.001 * sr)).exp();
                let mk = db_to_lin(*makeup_db);
                for f in 0..buf.frames() {
                    let peak = (0..ch).map(|c| buf.data[f * ch + c].abs()).fold(0.0, f32::max);
                    let coef = if peak > st.env { att } else { rel };
                    st.env = coef * st.env + (1.0 - coef) * peak;
                    let lvl = 20.0 * st.env.max(1e-9).log10();
                    let over = lvl - threshold_db;
                    let gr = if over > 0.0 { -over * (1.0 - 1.0 / ratio.max(1.0)) } else { 0.0 };
                    let g = db_to_lin(gr) * mk;
                    for c in 0..ch {
                        buf.data[f * ch + c] *= g;
                    }
                }
            }
            _ => {
                let Some(bq) = Biquad::new(ins, sr) else { continue };
                if st.z.len() != ch {
                    st.z = vec![[0.0; 4]; ch];
                }
                for f in 0..buf.frames() {
                    for c in 0..ch {
                        let z = &mut st.z[c];
                        let x = buf.data[f * ch + c];
                        let y = bq.b0 * x + bq.b1 * z[0] + bq.b2 * z[1] - bq.a1 * z[2] - bq.a2 * z[3];
                        z[1] = z[0];
                        z[0] = x;
                        z[3] = z[2];
                        z[2] = y;
                        buf.data[f * ch + c] = y;
                    }
                }
            }
        }
    }
}

/// Linear-interpolation sample-rate conversion (flagged in QA as not bit-accurate).
pub fn resample_linear(b: &AudioBuffer, sr: u32) -> AudioBuffer {
    if b.sample_rate == sr {
        return b.clone();
    }
    let ch = b.channels as usize;
    let n = b.frames();
    let out_n = ((n as u64 * sr as u64) / b.sample_rate as u64) as usize;
    let ratio = b.sample_rate as f64 / sr as f64;
    let mut out = AudioBuffer::silent(sr, b.channels, out_n);
    for i in 0..out_n {
        let pos = i as f64 * ratio;
        let i0 = pos.floor() as usize;
        let fr = (pos - i0 as f64) as f32;
        for c in 0..ch {
            let a = b.data[i0.min(n - 1) * ch + c];
            let z = b.data[(i0 + 1).min(n - 1) * ch + c];
            out.data[i * ch + c] = a + (z - a) * fr;
        }
    }
    out
}

// ---------------------------------------------------------------- render

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Skip clips whose asset license marks it as a temp reference (spec §15.2).
    pub exclude_temp_refs: bool,
    /// Honour track solo (playback); exports normally ignore solo.
    pub honor_solo: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { exclude_temp_refs: false, honor_solo: true }
    }
}

pub struct Mixer<'a> {
    pub s: &'a ProjectState,
    pub opts: RenderOptions,
    cache: SourceCache,
    /// Peak level of the last rendered block per track/bus id (for meters).
    pub meters: RefCell<HashMap<String, f32>>,
    /// Live automation values `(owner_id, param) → value` that override the
    /// stored lane while a write pass is touching it (monitoring).
    pub overrides: RefCell<HashMap<(String, AutomationParam), f32>>,
    dsp: RefCell<HashMap<String, Vec<InsertState>>>,
    pub warnings: RefCell<Vec<String>>,
    /// Optional path remapping root for relocated media (unused when None).
    pub media_override: Option<&'a Path>,
    /// Hosted plugins (None: plugin slots pass audio through).
    pub plugins: Option<Arc<dyn PluginHost>>,
    /// Plugin parameter lanes by slot id: (parameter, sorted breakpoints).
    plugin_lanes: std::cell::OnceCell<HashMap<String, Vec<PluginLane<'a>>>>,
}

/// Breakpoint lookup for monotonically increasing sample positions: linear
/// interpolation between points, held flat before the first / after the last.
/// Amortised O(1) per sample (the cursor only moves forward within a block).
struct AutoCursor<'p> {
    points: Vec<&'p AutomationPoint>,
    i: usize,
    default: f32,
    /// Live value overriding the lane (automation being written right now).
    live: Option<f32>,
}

impl<'p> AutoCursor<'p> {
    fn new(mut points: Vec<&'p AutomationPoint>, default: f32) -> Self {
        points.sort_by_key(|p| (p.sample, p.id.clone()));
        Self { points, i: 0, default, live: None }
    }
    fn is_empty(&self) -> bool {
        self.points.is_empty() && self.live.is_none()
    }
    fn at(&mut self, s: i64) -> f32 {
        if let Some(v) = self.live {
            return v;
        }
        let p = &self.points;
        if p.is_empty() {
            return self.default;
        }
        if s <= p[0].sample {
            return p[0].value;
        }
        if s >= p[p.len() - 1].sample {
            return p[p.len() - 1].value;
        }
        if self.i >= p.len() - 1 || p[self.i].sample > s {
            self.i = 0;
        }
        while self.i + 1 < p.len() && p[self.i + 1].sample <= s {
            self.i += 1;
        }
        let (a, b) = (p[self.i], p[self.i + 1]);
        let t = (s - a.sample) as f32 / (b.sample - a.sample).max(1) as f32;
        a.value + (b.value - a.value) * t
    }
}

impl<'a> Mixer<'a> {
    pub fn new(s: &'a ProjectState, opts: RenderOptions) -> Self {
        Self {
            s,
            opts,
            cache: Arc::new(Mutex::new(HashMap::new())),
            meters: RefCell::new(HashMap::new()),
            overrides: RefCell::new(HashMap::new()),
            dsp: RefCell::new(HashMap::new()),
            warnings: RefCell::new(vec![]),
            media_override: None,
            plugins: None,
            plugin_lanes: std::cell::OnceCell::new(),
        }
    }

    /// Automation curves for a plugin slot over `[pos, pos + frames)`.
    pub fn plugin_curves(&self, slot_id: &str, pos: i64, frames: usize) -> Vec<ParamCurve> {
        let lanes = self.plugin_lanes.get_or_init(|| {
            let mut m: HashMap<String, HashMap<u32, Vec<&'a AutomationPoint>>> = HashMap::new();
            for p in self.s.automation.values() {
                if let AutomationParam::Plugin(id) = p.param {
                    m.entry(p.owner_id.clone()).or_default().entry(id).or_default().push(p);
                }
            }
            m.into_iter()
                .map(|(slot, by)| {
                    let mut v: Vec<(u32, Vec<&AutomationPoint>)> = by
                        .into_iter()
                        .map(|(id, mut pts)| {
                            pts.sort_by_key(|p| (p.sample, p.id.clone()));
                            (id, pts)
                        })
                        .collect();
                    v.sort_by_key(|x| x.0);
                    (slot, v)
                })
                .collect()
        });
        let ov = self.overrides.borrow();
        let live = |id: u32| ov.get(&(slot_id.to_string(), AutomationParam::Plugin(id))).copied();
        let mut out: Vec<ParamCurve> = vec![];
        let end = pos + frames.max(1) as i64 - 1;
        for (id, pts) in lanes.get(slot_id).map(|v| v.as_slice()).unwrap_or(&[]) {
            if let Some(v) = live(*id) {
                out.push(ParamCurve { id: *id, points: vec![(0, v as f64)] });
                continue;
            }
            let mut c = vec![(0u32, lane_value(pts, pos) as f64)];
            let from = pts.partition_point(|p| p.sample <= pos);
            for p in &pts[from..] {
                if p.sample >= end {
                    break;
                }
                c.push(((p.sample - pos) as u32, p.value as f64));
            }
            if end > pos {
                c.push(((end - pos) as u32, lane_value(pts, end) as f64));
            }
            c.dedup_by(|b, a| {
                a.0 == b.0 && {
                    a.1 = b.1;
                    true
                }
            });
            out.push(ParamCurve { id: *id, points: c });
        }
        // a parameter being written for the first time has no lane yet
        for ((o, p), v) in ov.iter() {
            if let (true, AutomationParam::Plugin(id)) = (o == slot_id, p) {
                if !out.iter().any(|c| c.id == *id) {
                    out.push(ParamCurve { id: *id, points: vec![(0, *v as f64)] });
                }
            }
        }
        out
    }

    /// Process plugin inserts with `host`.
    pub fn with_plugins(mut self, host: Option<Arc<dyn PluginHost>>) -> Self {
        self.plugins = host;
        self
    }

    /// Total latency of the active plugins in an insert chain.
    pub fn chain_latency(&self, inserts: &[Insert]) -> i64 {
        let Some(h) = &self.plugins else { return 0 };
        inserts.iter().filter_map(Insert::as_plugin).filter(|p| !p.bypass).map(|p| h.latency(p).max(0)).sum()
    }

    /// A latent chain that doesn't continue from `pos` must be pre-rolled.
    fn needs_preroll(&self, inserts: &[Insert], pos: i64) -> bool {
        let Some(h) = &self.plugins else { return false };
        inserts.iter().filter_map(Insert::as_plugin).find(|p| !p.bypass).is_some_and(|p| h.continues_at(p) != Some(pos))
    }

    fn run_inserts(&self, key: String, inserts: &[Insert], buf: &mut AudioBuffer, pos: i64) {
        let frames = buf.frames();
        let mut dsp = self.dsp.borrow_mut();
        let st = dsp.entry(key).or_default();
        process_inserts(inserts, st, buf, pos, self.plugins.as_deref(), &|slot| self.plugin_curves(slot, pos, frames), &|w| self.warn(w));
    }

    /// Clear DSP state (call on locate / transport jump).
    pub fn reset(&self) {
        self.dsp.borrow_mut().clear();
    }

    fn warn(&self, w: String) {
        let mut v = self.warnings.borrow_mut();
        if !v.contains(&w) {
            v.push(w);
        }
    }

    /// Mixer sharing an existing decoded-audio cache.
    pub fn with_cache(s: &'a ProjectState, opts: RenderOptions, cache: SourceCache) -> Self {
        let mut m = Self::new(s, opts);
        m.cache = cache;
        m
    }

    pub fn source(&self, asset_id: &str) -> Result<Arc<AudioBuffer>> {
        if let Some(b) = self.cache.lock().unwrap().get(asset_id) {
            return Ok(b.clone());
        }
        let a = self.s.asset(asset_id)?;
        let (_, mut buf) =
            crate::wav::read(Path::new(&a.path)).map_err(|e| invalid(format!("asset '{}' unreadable ({}): {e}", a.name, a.path)))?;
        if buf.sample_rate != self.s.project.sample_rate {
            self.warn(format!(
                "sample-rate conversion applied to '{}' ({} → {} Hz, linear)",
                a.name, buf.sample_rate, self.s.project.sample_rate
            ));
            buf = resample_linear(&buf, self.s.project.sample_rate);
        }
        let rc = Arc::new(buf);
        self.cache.lock().unwrap().insert(asset_id.to_string(), rc.clone());
        Ok(rc)
    }

    fn clip_audible(&self, c: &ClipInstance) -> bool {
        if c.muted {
            return false;
        }
        if self.opts.exclude_temp_refs {
            if let Some(a) = self.s.assets.get(&c.asset_id) {
                if a.license.as_ref().is_some_and(|l| l.temp_reference) {
                    return false;
                }
            }
        }
        true
    }

    /// Mix one clip into `out` (track-channel buffer starting at `start`).
    pub fn render_clip_into(&self, c: &ClipInstance, out: &mut AudioBuffer, start: i64) -> Result<()> {
        let frames = out.frames() as i64;
        let a = c.timeline_start.max(start);
        let z = c.end().min(start + frames);
        if a >= z {
            return Ok(());
        }
        let src = self.source(&c.asset_id)?;
        let sn = src.frames() as i64;
        let sch = src.channels as usize;
        let och = out.channels as usize;
        let g = db_to_lin(c.gain_db);
        let loop_len = (sn - c.source_offset).max(0);
        let xf = (loop_len / 4).min(self.s.project.sample_rate as i64 / 20);
        let period = (loop_len - xf).max(1);
        for t in a..z {
            let p = t - c.timeline_start;
            let mut env = g;
            if c.fade_in > 0 && p < c.fade_in {
                env *= p as f32 / c.fade_in as f32;
            }
            let rem = c.length - p;
            if c.fade_out > 0 && rem <= c.fade_out {
                env *= (rem - 1).max(0) as f32 / c.fade_out as f32;
            }
            let oi = (t - start) as usize * och;
            let fetch = |idx: i64, ch: usize| -> f32 {
                if idx >= 0 && idx < sn {
                    src.data[idx as usize * sch + ch % sch]
                } else {
                    0.0
                }
            };
            for oc in 0..och {
                let v = match c.fill {
                    FillMode::OneShot => {
                        let idx = c.source_offset + p;
                        if sch > 1 && och == 1 {
                            (0..sch).map(|k| fetch(idx, k)).sum::<f32>() / sch as f32
                        } else {
                            fetch(idx, oc)
                        }
                    }
                    FillMode::Loop => {
                        if loop_len == 0 {
                            0.0
                        } else {
                            let k = p % period;
                            let get = |idx: i64| {
                                if sch > 1 && och == 1 {
                                    (0..sch).map(|q| fetch(idx, q)).sum::<f32>() / sch as f32
                                } else {
                                    fetch(idx, oc)
                                }
                            };
                            if p >= period && k < xf {
                                let w = k as f32 / xf as f32;
                                get(c.source_offset + k) * w + get(c.source_offset + period + k) * (1.0 - w)
                            } else {
                                get(c.source_offset + k)
                            }
                        }
                    }
                };
                out.data[oi + oc] += v * env;
            }
        }
        Ok(())
    }

    fn clips_on(&self, track_id: &str, start: i64, frames: i64) -> Vec<&'a ClipInstance> {
        let mut v: Vec<&ClipInstance> = self
            .s
            .clips
            .values()
            .filter(|c| c.track_id == track_id && c.timeline_start < start + frames && c.end() > start && self.clip_audible(c))
            .collect();
        v.sort_by(|a, b| (a.timeline_start, &a.id).cmp(&(b.timeline_start, &b.id)));
        v
    }

    /// Track "print": clips + inserts + fader/automation, pre-pan, at the
    /// track's own channel count. Used for consolidated track deliverables.
    pub fn render_track_print(&self, track_id: &str, start: i64, frames: usize) -> Result<AudioBuffer> {
        let t = self.s.track(track_id)?;
        let mut buf = AudioBuffer::silent(self.s.project.sample_rate, t.channels.max(1), frames);
        if let Some(fz) = t.frozen_asset_id.as_ref().and_then(|f| self.s.assets.get(f)) {
            let pos = fz.time_reference.map(|r| r as i64 - self.s.tb().origin_since_midnight()).unwrap_or(0);
            let clip = ClipInstance {
                id: "frozen".into(),
                track_id: t.id.clone(),
                cue_id: None,
                asset_id: fz.id.clone(),
                timeline_start: pos,
                source_offset: 0,
                length: fz.length_samples,
                gain_db: 0.0,
                fade_in: 0,
                fade_out: 0,
                fill: FillMode::OneShot,
                muted: false,
                take_id: None,
                performance_event_id: None,
                sub_index: None,
                is_placeholder: false,
                history: vec![],
            };
            self.render_clip_into(&clip, &mut buf, start)?;
            return Ok(buf);
        }
        // plugin delay compensation: feed the chain `lat` samples early so its
        // output lines up with `start` (the timeline can be read ahead freely)
        let lat = self.chain_latency(&t.inserts);
        if lat > 0 && self.needs_preroll(&t.inserts, start + lat) {
            // fill the chain's delay with the audio just before (output discarded)
            let mut pre = AudioBuffer::silent(buf.sample_rate, buf.channels, lat as usize);
            for c in self.clips_on(track_id, start, lat) {
                self.render_clip_into(c, &mut pre, start)?;
            }
            self.run_inserts(format!("t:{track_id}"), &t.inserts, &mut pre, start);
        }
        for c in self.clips_on(track_id, start + lat, frames as i64) {
            self.render_clip_into(c, &mut buf, start + lat)?;
        }
        self.run_inserts(format!("t:{track_id}"), &t.inserts, &mut buf, start + lat);
        let mut vol = self.cursor(track_id, AutomationParam::VolumeDb, 0.0);
        let ch = buf.channels as usize;
        let base = db_to_lin(t.gain_db);
        if vol.is_empty() {
            if (base - 1.0).abs() > 1e-9 {
                buf.data.iter_mut().for_each(|v| *v *= base);
            }
        } else {
            for f in 0..frames {
                let db = vol.at(start + f as i64);
                let g = if db <= -96.0 { 0.0 } else { base * db_to_lin(db) };
                for c in 0..ch {
                    buf.data[f * ch + c] *= g;
                }
            }
        }
        Ok(buf)
    }

    fn track_audible(&self, t: &Track) -> bool {
        if t.mute {
            return false;
        }
        if self.opts.honor_solo && self.s.tracks.values().any(|x| x.solo) {
            return t.solo;
        }
        true
    }

    /// Stereo output of a track after pan (and pan automation).
    pub fn render_track_stereo(&self, track_id: &str, start: i64, frames: usize) -> Result<AudioBuffer> {
        let t = self.s.track(track_id)?;
        let print = self.render_track_print(track_id, start, frames)?;
        let mut out = AudioBuffer::silent(self.s.project.sample_rate, 2, frames);
        let mut pans = self.cursor(track_id, AutomationParam::Pan, t.pan);
        let ch = print.channels as usize;
        for f in 0..frames {
            let pan = pans.at(start + f as i64);
            let (gl, gr) = pan_gains(pan);
            if ch == 1 {
                let v = print.data[f];
                out.data[f * 2] = v * gl;
                out.data[f * 2 + 1] = v * gr;
            } else {
                // balance: centre = unity on both sides
                let (bl, br) = if pan <= 0.0 { (1.0, 1.0 + pan) } else { (1.0 - pan, 1.0) };
                out.data[f * 2] = print.data[f * ch] * bl;
                out.data[f * 2 + 1] = print.data[f * ch + 1] * br;
            }
        }
        self.meters.borrow_mut().insert(track_id.to_string(), out.peak());
        Ok(out)
    }

    /// Stereo bus output: routed tracks + child buses, then inserts and gain.
    pub fn render_bus(&self, bus_id: &str, start: i64, frames: usize) -> Result<AudioBuffer> {
        let bus = self.s.buses.get(bus_id).ok_or_else(|| crate::error::not_found("bus", bus_id))?;
        let mut out = AudioBuffer::silent(self.s.project.sample_rate, 2, frames);
        if bus.mute {
            return Ok(out);
        }
        // inputs are rendered `lat` early so the bus plugins' output lines up
        let lat = self.chain_latency(&bus.inserts);
        let mut tracks: Vec<&Track> = self.s.tracks.values().filter(|t| t.bus_id == bus_id && self.track_audible(t)).collect();
        tracks.sort_by_key(|t| (t.order, t.id.clone()));
        let mut kids: Vec<&Bus> = self.s.buses.values().filter(|b| b.parent_id.as_deref() == Some(bus_id)).collect();
        kids.sort_by_key(|b| (b.order, b.id.clone()));
        let inputs = |at: i64, n: usize, out: &mut AudioBuffer| -> Result<()> {
            for t in &tracks {
                out.add(&self.render_track_stereo(&t.id, at, n)?);
            }
            for k in &kids {
                out.add(&self.render_bus(&k.id, at, n)?);
            }
            Ok(())
        };
        if lat > 0 && self.needs_preroll(&bus.inserts, start + lat) {
            let mut pre = AudioBuffer::silent(self.s.project.sample_rate, 2, lat as usize);
            inputs(start, lat as usize, &mut pre)?;
            self.run_inserts(format!("b:{bus_id}"), &bus.inserts, &mut pre, start);
        }
        inputs(start + lat, frames, &mut out)?;
        self.run_inserts(format!("b:{bus_id}"), &bus.inserts, &mut out, start + lat);
        let g = db_to_lin(bus.gain_db);
        let mut vol = self.cursor(bus_id, AutomationParam::VolumeDb, 0.0);
        let mut pan = self.cursor(bus_id, AutomationParam::Pan, 0.0);
        if vol.is_empty() && pan.is_empty() {
            if (g - 1.0).abs() > 1e-9 {
                out.data.iter_mut().for_each(|v| *v *= g);
            }
        } else {
            // bus automation: volume relative to the bus fader, pan as stereo balance
            for f in 0..frames {
                let t = start + f as i64;
                let db = vol.at(t);
                let gv = if db <= -96.0 { 0.0 } else { g * db_to_lin(db) };
                let p = pan.at(t);
                let (bl, br) = if p <= 0.0 { (1.0, 1.0 + p) } else { (1.0 - p, 1.0) };
                out.data[f * 2] *= gv * bl;
                out.data[f * 2 + 1] *= gv * br;
            }
        }
        self.meters.borrow_mut().insert(bus_id.to_string(), out.peak());
        Ok(out)
    }

    fn cursor(&self, owner_id: &str, param: AutomationParam, default: f32) -> AutoCursor<'a> {
        let mut c = AutoCursor::new(self.s.automation.values().filter(|p| p.owner_id == owner_id && p.param == param).collect(), default);
        c.live = self.overrides.borrow().get(&(owner_id.to_string(), param)).copied();
        c
    }

    pub fn render_master(&self, start: i64, frames: usize) -> Result<AudioBuffer> {
        let m = master_bus(self.s)?.id.clone();
        self.render_bus(&m, start, frames)
    }
}

/// Extent of all audible clips: (first start, last end).
pub fn timeline_extent(s: &ProjectState) -> (i64, i64) {
    let a = s.clips.values().map(|c| c.timeline_start).min().unwrap_or(0);
    let z = s.clips.values().map(|c| c.end()).max().unwrap_or(0);
    (a, z)
}

/// Freeze (render-in-place) a track to `Audio/Rendered`, BWF-stamped.
pub fn freeze_track(s: &mut ProjectState, root: &Path, track_id: &str) -> Result<String> {
    let (a, z) = {
        let clips: Vec<&ClipInstance> = s.clips.values().filter(|c| c.track_id == track_id).collect();
        if clips.is_empty() {
            return Err(invalid("track has no clips to freeze"));
        }
        (clips.iter().map(|c| c.timeline_start).min().unwrap(), clips.iter().map(|c| c.end()).max().unwrap())
    };
    let name = crate::layout::safe_name(&s.track(track_id)?.name);
    let buf = Mixer::new(s, RenderOptions { honor_solo: false, ..Default::default() }).render_track_print(track_id, a, (z - a) as usize)?;
    let path = root.join("Audio/Rendered").join(format!("{name}_frozen_{}.wav", &crate::ids::new_id("f")[2..10]));
    let bext = crate::wav::Bext {
        description: format!("Frozen track {name}"),
        originator: "VC Sound Design".into(),
        time_reference: (s.tb().origin_since_midnight() + a) as u64,
        ..Default::default()
    };
    crate::wav::write(&path, &buf, crate::wav::SampleFormat::Float32, Some(&bext))?;
    let aid = crate::library::import_wav(s, &path, crate::library::AssetMeta { kind: AssetKind::Rendered, ..Default::default() })?;
    s.tracks.get_mut(track_id).unwrap().frozen_asset_id = Some(aid.clone());
    Ok(aid)
}

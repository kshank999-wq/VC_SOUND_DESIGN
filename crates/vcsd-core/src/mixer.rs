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
use std::rc::Rc;

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
        Bus { id: id.clone(), name: name.into(), order, parent_id: Some(parent), gain_db: 0.0, mute: false, stem: true, inserts: vec![] },
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

fn process_inserts(inserts: &[Insert], states: &mut Vec<InsertState>, buf: &mut AudioBuffer) {
    let sr = buf.sample_rate as f32;
    let ch = buf.channels as usize;
    if states.len() != inserts.len() {
        *states = vec![InsertState::default(); inserts.len()];
    }
    for (ins, st) in inserts.iter().zip(states.iter_mut()) {
        match ins {
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
    cache: RefCell<HashMap<String, Rc<AudioBuffer>>>,
    dsp: RefCell<HashMap<String, Vec<InsertState>>>,
    pub warnings: RefCell<Vec<String>>,
    /// Optional path remapping root for relocated media (unused when None).
    pub media_override: Option<&'a Path>,
}

fn automation_value(points: &[&AutomationPoint], s: i64, default: f32) -> f32 {
    if points.is_empty() {
        return default;
    }
    if s <= points[0].sample {
        return points[0].value;
    }
    for w in points.windows(2) {
        if s >= w[0].sample && s < w[1].sample {
            let t = (s - w[0].sample) as f32 / (w[1].sample - w[0].sample) as f32;
            return w[0].value + (w[1].value - w[0].value) * t;
        }
    }
    points[points.len() - 1].value
}

impl<'a> Mixer<'a> {
    pub fn new(s: &'a ProjectState, opts: RenderOptions) -> Self {
        Self {
            s,
            opts,
            cache: RefCell::new(HashMap::new()),
            dsp: RefCell::new(HashMap::new()),
            warnings: RefCell::new(vec![]),
            media_override: None,
        }
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

    pub fn source(&self, asset_id: &str) -> Result<Rc<AudioBuffer>> {
        if let Some(b) = self.cache.borrow().get(asset_id) {
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
        let rc = Rc::new(buf);
        self.cache.borrow_mut().insert(asset_id.to_string(), rc.clone());
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
        for c in self.clips_on(track_id, start, frames as i64) {
            self.render_clip_into(c, &mut buf, start)?;
        }
        {
            let mut dsp = self.dsp.borrow_mut();
            let st = dsp.entry(format!("t:{track_id}")).or_default();
            process_inserts(&t.inserts, st, &mut buf);
        }
        let mut vol: Vec<&AutomationPoint> =
            self.s.automation.values().filter(|p| p.track_id == track_id && p.param == AutomationParam::VolumeDb).collect();
        vol.sort_by_key(|p| p.sample);
        let ch = buf.channels as usize;
        let base = db_to_lin(t.gain_db);
        if vol.is_empty() {
            if (base - 1.0).abs() > 1e-9 {
                buf.data.iter_mut().for_each(|v| *v *= base);
            }
        } else {
            for f in 0..frames {
                let g = base * db_to_lin(automation_value(&vol, start + f as i64, 0.0));
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
        let mut pans: Vec<&AutomationPoint> =
            self.s.automation.values().filter(|p| p.track_id == track_id && p.param == AutomationParam::Pan).collect();
        pans.sort_by_key(|p| p.sample);
        let ch = print.channels as usize;
        for f in 0..frames {
            let pan = automation_value(&pans, start + f as i64, t.pan);
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
        Ok(out)
    }

    /// Stereo bus output: routed tracks + child buses, then inserts and gain.
    pub fn render_bus(&self, bus_id: &str, start: i64, frames: usize) -> Result<AudioBuffer> {
        let bus = self.s.buses.get(bus_id).ok_or_else(|| crate::error::not_found("bus", bus_id))?;
        let mut out = AudioBuffer::silent(self.s.project.sample_rate, 2, frames);
        if bus.mute {
            return Ok(out);
        }
        let mut tracks: Vec<&Track> = self.s.tracks.values().filter(|t| t.bus_id == bus_id && self.track_audible(t)).collect();
        tracks.sort_by_key(|t| (t.order, t.id.clone()));
        for t in tracks {
            out.add(&self.render_track_stereo(&t.id, start, frames)?);
        }
        let mut kids: Vec<&Bus> = self.s.buses.values().filter(|b| b.parent_id.as_deref() == Some(bus_id)).collect();
        kids.sort_by_key(|b| (b.order, b.id.clone()));
        for k in kids {
            out.add(&self.render_bus(&k.id, start, frames)?);
        }
        {
            let mut dsp = self.dsp.borrow_mut();
            let st = dsp.entry(format!("b:{bus_id}")).or_default();
            process_inserts(&bus.inserts, st, &mut out);
        }
        let g = db_to_lin(bus.gain_db);
        if (g - 1.0).abs() > 1e-9 {
            out.data.iter_mut().for_each(|v| *v *= g);
        }
        Ok(out)
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

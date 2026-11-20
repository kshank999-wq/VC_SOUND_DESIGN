//! Perform-to-picture: map one-shots to keys/MIDI/touch pads, capture
//! timecode-linked triggers while picture plays, and render them into
//! editable clips. Raw triggers are retained for re-rendering (spec §11.1).

use crate::error::{invalid, not_found, Result};
use crate::ids::{new_id, now_unix};
use crate::library::stable_hash;
use crate::model::*;
use crate::spotting::MidiNote;
use crate::state::ProjectState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Pad {
    pub name: String,
    /// Variant pool (left foot for AlternateFeet).
    pub assets: Vec<String>,
    #[serde(default)]
    pub alt_assets: Vec<String>,
    #[serde(default)]
    pub mode: VariationMode,
    #[serde(default)]
    pub gain_db: f32,
}

/// Keys are `key:<name>` (computer keyboard), `midi:<ch>:<note>` or `pad:<n>` (touch grid).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PadMap {
    pub pads: BTreeMap<String, Pad>,
}

impl PadMap {
    pub fn set(&mut self, key: &str, pad: Pad) {
        self.pads.insert(key.to_string(), pad);
    }

    /// Build a pad from a sound identity's variation pools (shoe/surface preset).
    pub fn from_identity(ident: &SoundIdentity) -> Pad {
        let mut assets = ident.variation_pool.clone();
        if assets.is_empty() {
            assets.extend(ident.default_asset_id.clone());
        }
        Pad { name: ident.name.clone(), assets, alt_assets: ident.alt_pool.clone(), mode: ident.variation_mode, gain_db: 0.0 }
    }

    fn pick(&self, key: &str, n: usize, seed: &str) -> Option<String> {
        let p = self.pads.get(key)?;
        if p.assets.is_empty() {
            return None;
        }
        Some(match p.mode {
            VariationMode::Locked => p.assets[0].clone(),
            VariationMode::RoundRobin => p.assets[n % p.assets.len()].clone(),
            VariationMode::Random => p.assets[(stable_hash(&format!("{seed}:{n}")) % p.assets.len() as u64) as usize].clone(),
            VariationMode::AlternateFeet if p.alt_assets.is_empty() => p.assets[n % p.assets.len()].clone(),
            VariationMode::AlternateFeet => {
                let side = if n % 2 == 1 { &p.alt_assets } else { &p.assets };
                side[(n / 2) % side.len()].clone()
            }
        })
    }
}

/// Velocity (1..127) to gain in dB: 127 → 0 dB, 64 → ~-6 dB, floor -40 dB.
pub fn velocity_db(v: u8) -> f32 {
    if v == 0 {
        -40.0
    } else {
        (20.0 * (v as f32 / 127.0).log10()).max(-40.0)
    }
}

/// Captures triggers during playback. Quantization is off by default.
#[derive(Debug, Clone)]
pub struct PerformanceRecorder {
    pub track_id: String,
    pub cue_id: Option<String>,
    pub pad_map: PadMap,
    /// Input latency to subtract from captured positions, in samples.
    pub latency_compensation: i64,
    pub quantize_frames: Option<u32>,
    events: Vec<(String, i64, u8)>,
}

impl PerformanceRecorder {
    pub fn new(track_id: &str, cue_id: Option<&str>, pad_map: PadMap) -> Self {
        Self {
            track_id: track_id.into(),
            cue_id: cue_id.map(|c| c.into()),
            pad_map,
            latency_compensation: 0,
            quantize_frames: None,
            events: vec![],
        }
    }

    pub fn trigger(&mut self, pad: &str, sample: i64, velocity: u8) -> Result<()> {
        if !self.pad_map.pads.contains_key(pad) {
            return Err(not_found("pad", pad));
        }
        self.events.push((pad.into(), sample, velocity.clamp(1, 127)));
        Ok(())
    }

    /// Feed a raw MIDI message; note-ons on mapped pads become triggers.
    pub fn midi(&mut self, msg: &[u8], sample: i64) -> Result<bool> {
        match MidiNote::parse(msg) {
            Some(n) if n.velocity > 0 && self.pad_map.pads.contains_key(&n.key()) => {
                self.trigger(&n.key(), sample, n.velocity)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Store the performance as a new take and render it into clips. Earlier
    /// takes on the same track/cue are muted (kept for comparison).
    pub fn commit(self, s: &mut ProjectState) -> Result<String> {
        s.track(&self.track_id)?;
        if self.events.is_empty() {
            return Err(invalid("no triggers captured"));
        }
        let take =
            s.performances.values().filter(|p| p.track_id == self.track_id && p.cue_id == self.cue_id).map(|p| p.take).max().unwrap_or(0)
                + 1;
        let pid = new_id("prf");
        s.performances.insert(
            pid.clone(),
            Performance {
                id: pid.clone(),
                track_id: self.track_id.clone(),
                cue_id: self.cue_id.clone(),
                take,
                pad_map: self.pad_map,
                latency_compensation: self.latency_compensation,
                quantize_frames: self.quantize_frames,
                created_at: now_unix(),
            },
        );
        let mut evs = self.events;
        evs.sort_by_key(|e| e.1);
        for (i, (pad, raw, vel)) in evs.into_iter().enumerate() {
            let id = new_id("pev");
            s.performance_events.insert(
                id.clone(),
                PerformanceEvent { id, performance_id: pid.clone(), pad, raw_sample: raw, sample: raw, velocity: vel, index: i as u32 },
            );
        }
        render_performance(s, &pid)?;
        select_take(s, &pid)?;
        if let Some(c) = s.performances[&pid].cue_id.clone() {
            let cue = s.cue_mut(&c)?;
            cue.timing_origin = TimingOrigin::MidiPerformed;
            cue.status = CueStatus::Recorded;
        }
        Ok(pid)
    }
}

fn quantize(s: &ProjectState, sample: i64, frames: u32) -> i64 {
    let tb = s.tb();
    let f = tb.frame_at(sample) - tb.start_frame;
    let q = frames.max(1) as i64;
    let lo = f.div_euclid(q) * q;
    let a = tb.sample_at_frame(tb.start_frame + lo);
    let b = tb.sample_at_frame(tb.start_frame + lo + q);
    if sample - a <= b - sample {
        a
    } else {
        b
    }
}

/// (Re)render a performance's triggers into clips. Deterministic: same
/// triggers + pad map always produce identical clips.
pub fn render_performance(s: &mut ProjectState, perf_id: &str) -> Result<usize> {
    let perf = s.performances.get(perf_id).ok_or_else(|| not_found("performance", perf_id))?.clone();
    let mut events: Vec<PerformanceEvent> = s.performance_events.values().filter(|e| e.performance_id == perf_id).cloned().collect();
    events.sort_by_key(|e| e.index);
    let ev_ids: Vec<String> = events.iter().map(|e| e.id.clone()).collect();
    s.clips.retain(|_, c| c.performance_event_id.as_ref().is_none_or(|e| !ev_ids.contains(e)));
    let mut per_pad: BTreeMap<String, usize> = BTreeMap::new();
    let mut n = 0;
    for e in events {
        let mut pos = e.raw_sample - perf.latency_compensation;
        if let Some(q) = perf.quantize_frames {
            pos = quantize(s, pos, q);
        }
        s.performance_events.get_mut(&e.id).unwrap().sample = pos;
        let k = per_pad.entry(e.pad.clone()).or_insert(0);
        let Some(aid) = perf.pad_map.pick(&e.pad, *k, perf_id) else { continue };
        *k += 1;
        let a = s.asset(&aid)?.clone();
        let pad_gain = perf.pad_map.pads.get(&e.pad).map(|p| p.gain_db).unwrap_or(0.0);
        let id = new_id("clp");
        s.clips.insert(
            id.clone(),
            ClipInstance {
                id,
                track_id: perf.track_id.clone(),
                cue_id: perf.cue_id.clone(),
                asset_id: a.id.clone(),
                timeline_start: pos,
                source_offset: 0,
                length: a.length_samples.max(1),
                gain_db: pad_gain + velocity_db(e.velocity),
                fade_in: 0,
                fade_out: (a.sample_rate as i64 / 200).min(a.length_samples / 4),
                fill: FillMode::OneShot,
                muted: false,
                take_id: None,
                performance_event_id: Some(e.id.clone()),
                sub_index: Some(e.index),
                is_placeholder: false,
                history: vec![AssetHistoryEntry { asset_id: a.id, origin: format!("performance:take{}", perf.take), at: now_unix() }],
            },
        );
        n += 1;
    }
    Ok(n)
}

/// Re-render a stored performance with a different pad map (e.g. new shoes).
pub fn rerender_with(s: &mut ProjectState, perf_id: &str, pad_map: PadMap) -> Result<usize> {
    s.performances.get_mut(perf_id).ok_or_else(|| not_found("performance", perf_id))?.pad_map = pad_map;
    render_performance(s, perf_id)
}

/// Make one performance take audible; mute other takes (and placeholder clips)
/// for the same cue/track.
pub fn select_take(s: &mut ProjectState, perf_id: &str) -> Result<()> {
    let perf = s.performances.get(perf_id).ok_or_else(|| not_found("performance", perf_id))?.clone();
    let mine: Vec<String> = s.performance_events.values().filter(|e| e.performance_id == perf_id).map(|e| e.id.clone()).collect();
    for c in s.clips.values_mut().filter(|c| c.track_id == perf.track_id && c.cue_id == perf.cue_id) {
        c.muted = match &c.performance_event_id {
            Some(e) => !mine.contains(e),
            None => perf.cue_id.is_some(), // placeholders of this cue are superseded
        };
    }
    Ok(())
}

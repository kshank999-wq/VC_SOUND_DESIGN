//! OTIO export (Tier-2 interchange, spec §15.2): the editable timeline as
//! OpenTimelineIO JSON (`.otio`), for editors and conform tools that read it.
//!
//! - One audio track per project track (OTIO tracks carry no channel count: a
//!   stereo track is one track of stereo clips, `channels` in its metadata);
//!   overlapping clips split onto extra tracks, like the AAF.
//! - Every clip links its media (`ExternalReference`, absolute `file://` URL)
//!   with its source range. Times are in samples (rate = the sample rate), so
//!   every position is exact; the timeline's start timecode is
//!   `global_start_time` at the project frame rate.
//! - Fades next to silence become `SMPTE_Dissolve` transitions to and from the
//!   gap (the usual OTIO fade). Fades between butted clips can't be written
//!   that way, so they are only in the clip's metadata.
//! - Cues are markers on the timeline, coloured by category.
//! - Printed tracks (optional, shared with the AAF) are one clip of their render.
//!
//! OTIO has no audio mix model. Levels and pan follow the convention of the
//! OpenTimelineIO AAF adapter: AAF's Mono Audio Gain and Mono Audio Pan as
//! `Effect`s on the track (fader × volume automation, pan or pan automation) and
//! clip (clip gain), constant or keyframed. The raw values also go into
//! `metadata.vcsd`. Losses are listed in the [`OtioReport`].

use crate::aaf::{bus_muted, clip_pieces, db_amp, gain_points, lane_db, trim, url_for, AafMedia, Losses, Piece};
use crate::error::Result;
use crate::model::*;
use crate::state::ProjectState;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct OtioOptions {
    pub name: String,
    /// Timeline range (project samples); OTIO time 0 is `start`.
    pub start: i64,
    pub end: i64,
    pub exclude_temp_refs: bool,
    /// Tracks rendered with their plugins and insert effects, by track id.
    pub prints: BTreeMap<String, AafMedia>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OtioReport {
    pub file: String,
    pub timeline: String,
    pub tracks: usize,
    pub clips: usize,
    /// Fades written as transitions to or from silence.
    pub fade_transitions: usize,
    pub markers: usize,
    pub media_files: usize,
    pub printed_tracks: usize,
    pub losses: Vec<String>,
}

fn rt(value: f64, rate: f64) -> Value {
    json!({ "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": value })
}

fn range(start: i64, duration: i64, rate: f64) -> Value {
    json!({ "OTIO_SCHEMA": "TimeRange.1", "duration": rt(duration as f64, rate), "start_time": rt(start as f64, rate) })
}

fn gap(len: i64, rate: f64) -> Value {
    json!({
        "OTIO_SCHEMA": "Gap.1", "metadata": {}, "name": "", "source_range": range(0, len, rate),
        "effects": [], "markers": [], "enabled": true,
    })
}

/// A fade to or from silence: a dissolve against the neighbouring gap (or track edge).
fn fade(fade_in: i64, fade_out: i64, rate: f64) -> Value {
    json!({
        "OTIO_SCHEMA": "Transition.1",
        "metadata": { "vcsd": { "fade": if fade_in > 0 { "in" } else { "out" } } },
        "name": if fade_in > 0 { "Fade In" } else { "Fade Out" },
        "in_offset": rt(fade_out as f64, rate),
        "out_offset": rt(fade_in as f64, rate),
        "transition_type": "SMPTE_Dissolve",
    })
}

fn colour(c: &Category) -> &'static str {
    match c {
        Category::Footsteps | Category::Props => "YELLOW",
        Category::HardFx => "RED",
        Category::Background => "GREEN",
        Category::DesignFx => "MAGENTA",
        Category::Dialogue => "BLUE",
        Category::Music => "PURPLE",
        Category::Custom(_) => "CYAN",
    }
}

fn lane(s: &ProjectState, owner: &str, param: AutomationParam, start: i64) -> Vec<(i64, f32)> {
    let mut v: Vec<(i64, f32)> =
        s.automation.values().filter(|p| p.owner_id == owner && p.param == param).map(|p| (p.sample - start, p.value)).collect();
    v.sort_by_key(|p| p.0);
    v
}

/// A linear gain as the AAF adapter writes a constant AAF Rational ("n/d").
fn rational_str(g: f64) -> String {
    format!("{}/1000000", (g * 1e6).round().clamp(0.0, i32::MAX as f64) as i64)
}

/// A level as OTIO tools carry AAF's Mono Audio Gain (the encoding of the
/// OpenTimelineIO AAF adapter, `simplify=True`): an `Effect` whose
/// `metadata.AAF.Parameters.Amplitude` is a constant ("n/d" linear gain) or
/// keyframes `[fraction of the item's length, linear gain]`, linear in amplitude.
fn gain_effect(amplitude: Value, vcsd: Value) -> Value {
    json!({
        "OTIO_SCHEMA": "Effect.1",
        "metadata": {
            "AAF": {
                "Operation": { "Name": "Mono Audio Gain", "Identification": "9d2ea894-0968-11d3-8a38-0050040ef7d2" },
                "Parameters": { "Amplitude": amplitude },
            },
            "vcsd": vcsd,
        },
        "name": "Mono Audio Gain",
        "effect_name": "Mono Audio Gain",
    })
}

/// The track's pan as AAF's Mono Audio Pan, in the same encoding as the gain
/// (`Pan`: 0 left, 0.5 centre, 1 right). The pan lane is linear in its value, as
/// AAF keyframes are, so its breakpoints are the keyframes, exactly. On a stereo
/// track the value is the mixer's balance (`metadata.vcsd.law`). None when centred.
fn track_pan(s: &ProjectState, t: &Track, start: i64, end: i64) -> Option<Value> {
    let pts = lane(s, &t.id, AutomationParam::Pan, 0);
    let law = if t.channels >= 2 { "balance" } else { "constant_power_-3dB" };
    let vcsd = json!({ "param": "pan", "pan": t.pan, "automated": !pts.is_empty(), "law": law });
    let effect = |pan: Value| {
        json!({
            "OTIO_SCHEMA": "Effect.1",
            "metadata": {
                "AAF": {
                    "Operation": { "Name": "Mono Audio Pan", "Identification": "9d2ea893-0968-11d3-8a38-0050040ef7d2" },
                    "Parameters": { "Pan": pan },
                },
                "vcsd": vcsd,
            },
            "name": "Mono Audio Pan",
            "effect_name": "Mono Audio Pan",
        })
    };
    let to_param = |p: f64| (p.clamp(-1.0, 1.0) + 1.0) / 2.0;
    if pts.is_empty() {
        return (t.pan.abs() > 1e-4).then(|| effect(json!(rational_str(to_param(t.pan as f64)))));
    }
    let last = end - 1;
    let mut keys = vec![start];
    keys.extend(pts.iter().map(|p| p.0).filter(|&x| x > start && x < last));
    keys.push(last);
    keys.dedup();
    let len = (end - start).max(1) as f64;
    let kf: Vec<Value> =
        keys.iter().map(|&x| json!([(x - start) as f64 / len, (to_param(lane_db(&pts, x)) * 1e6).round() / 1e6])).collect();
    Some(effect(json!({
        "_aaf_keyframed_property": true,
        "keyframe_values": kf,
        "keyframe_interpolation": "Linear",
        "keyframe_baked_values": null,
    })))
}

/// The track's level over `[start, end)`: its fader times its volume lane, as the
/// mixer plays it. Keyframes are fitted (linear amplitude) within 0.1 dB of the
/// lane, which ramps in dB. None at unity.
fn track_volume(s: &ProjectState, t: &Track, start: i64, end: i64) -> Option<Value> {
    let pts = lane(s, &t.id, AutomationParam::VolumeDb, 0);
    let fader = db_amp(t.gain_db as f64);
    let vcsd = json!({ "param": "volume_db", "fader_db": t.gain_db, "automated": !pts.is_empty() });
    if pts.is_empty() {
        return (t.gain_db.abs() > 1e-4).then(|| gain_effect(json!(rational_str(fader)), vcsd));
    }
    let keys: Vec<i64> = pts.iter().map(|p| p.0).collect();
    let len = (end - start).max(1) as f64;
    let kf: Vec<Value> = gain_points(&keys, start, end, &|x| fader * db_amp(lane_db(&pts, x)))
        .into_iter()
        .map(|(x, g)| json!([(x - start) as f64 / len, (g * 1e6).round() / 1e6]))
        .collect();
    Some(gain_effect(
        json!({
            "_aaf_keyframed_property": true,
            "keyframe_values": kf,
            "keyframe_interpolation": "Linear",
            "keyframe_baked_values": null,
        }),
        vcsd,
    ))
}

/// Build the OTIO timeline and write it to `out`. `media` maps asset ids to the linked WAVs.
pub fn write(s: &ProjectState, out: &Path, media: &BTreeMap<String, AafMedia>, o: &OtioOptions) -> Result<OtioReport> {
    let sr = s.project.sample_rate as f64;
    let (start, end) = (o.start, o.end.max(o.start + 1));
    let len = end - start;
    let print_key = |tid: &str| format!("print:{tid}");
    let mut media = media.clone();
    for (tid, m) in &o.prints {
        media.insert(print_key(tid), m.clone());
    }
    let printed = |t: &str| o.prints.contains_key(t);
    let mut losses = Losses::default();
    let mut report = OtioReport { timeline: o.name.clone(), printed_tracks: o.prints.len(), ..Default::default() };

    // ---- what OTIO can't hold
    let pans = |t: &Track| t.pan.abs() > 1e-4 || s.automation.values().any(|p| p.owner_id == t.id && p.param == AutomationParam::Pan);
    losses.add(
        "pan uses the receiving app's pan law (VC Sound Design: −3 dB constant power on mono tracks, balance on stereo tracks)",
        s.tracks.values().filter(|t| !t.mute && pans(t)).count(),
    );
    losses.add("bus routing, levels and automation not carried", s.buses.values().filter(|b| b.parent_id.is_some()).count());
    let inserts: usize =
        s.tracks.values().filter(|t| !printed(&t.id)).map(|t| t.inserts.len()).chain(s.buses.values().map(|b| b.inserts.len())).sum();
    losses.add("insert effects and plugins not carried (rendered in stems and mix)", inserts);
    losses.add("tracks printed with their plugins and insert effects: one rendered clip each, not editable clip by clip", o.prints.len());

    let mut tracks: Vec<&Track> = s.tracks.values().collect();
    tracks.sort_by(|a, b| (&a.folder, a.order, &a.name).cmp(&(&b.folder, b.order, &b.name)));
    let mut children = vec![];
    for t in tracks {
        if bus_muted(s, &t.bus_id) {
            losses.add("tracks on muted buses left out", 1);
            continue;
        }
        if t.mute {
            losses.add("muted tracks left out", 1);
            continue;
        }
        let mut pieces: Vec<Piece> = vec![];
        if printed(&t.id) {
            pieces.push(Piece { at: start, len, source: 0, fade_in: 0, fade_out: 0, gain: 1.0, asset: print_key(&t.id) });
        } else {
            let mut clips: Vec<&ClipInstance> = s.clips.values().filter(|c| c.track_id == t.id).collect();
            clips.sort_by(|a, b| (a.timeline_start, &a.id).cmp(&(b.timeline_start, &b.id)));
            for c in clips {
                if c.muted {
                    losses.add("muted clips left out", 1);
                    continue;
                }
                let Some(a) = s.assets.get(&c.asset_id) else { continue };
                if o.exclude_temp_refs && a.license.as_ref().is_some_and(|l| l.temp_reference) {
                    losses.add("temp-reference clips left out", 1);
                    continue;
                }
                if !media.contains_key(&c.asset_id) {
                    losses.add("clips whose media could not be linked left out", 1);
                    continue;
                }
                // the track fader stays on the track (in its metadata), not in clip gain
                let mut l = Losses::default();
                pieces.extend(clip_pieces(s, c, 0.0, &mut l).into_iter().filter_map(|p| trim(p, start, end)));
                if l.list().iter().any(|x| x.starts_with("looped")) {
                    losses.add("looped clips: loop crossfades not carried, repeats are butt-spliced", 1);
                }
            }
        }
        if pieces.is_empty() {
            continue;
        }
        // overlapping clips → extra tracks (an OTIO track is a sequence)
        let mut lanes: Vec<Vec<Piece>> = vec![];
        for p in pieces {
            match lanes.iter_mut().find(|l| l.last().is_none_or(|q| q.at + q.len <= p.at)) {
                Some(l) => l.push(p),
                None => lanes.push(vec![p]),
            }
        }
        if lanes.len() > 1 {
            losses.add("overlapping clips split onto extra tracks", lanes.len() - 1);
        }
        let points = |param| {
            let v: Vec<Value> = lane(s, &t.id, param, start).into_iter().map(|(x, v)| json!([x, v])).collect();
            v
        };
        let bus = s.buses.get(&t.bus_id).map(|b| b.name.clone()).unwrap_or_default();
        // the track's level (fader × volume automation) as a gain effect on each of its layers
        let volume = track_volume(s, t, start, end);
        let pan = track_pan(s, t, start, end);
        for (li, lane_pieces) in lanes.iter().enumerate() {
            let mut items = vec![];
            let mut pos = start;
            for (k, p) in lane_pieces.iter().enumerate() {
                let silence_before = p.at > pos || (k == 0 && p.at == start);
                if p.at > pos {
                    items.push(gap(p.at - pos, sr));
                }
                if p.fade_in > 0 {
                    if silence_before {
                        items.push(fade(p.fade_in, 0, sr));
                        report.fade_transitions += 1;
                    } else {
                        losses.add("fades between butted clips carried as clip metadata only", 1);
                    }
                }
                let m = &media[&p.asset];
                let file_name = m.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                let asset = s.assets.get(&p.asset);
                let name = if printed(&t.id) {
                    format!("{} (printed)", t.name)
                } else {
                    asset.map(|a| a.name.clone()).unwrap_or_else(|| file_name.clone())
                };
                let gain_db = if p.gain > 0.0 { 20.0 * (p.gain as f64).log10() } else { -96.0 };
                items.push(json!({
                    "OTIO_SCHEMA": "Clip.2",
                    "metadata": { "vcsd": {
                        "gain_db": (gain_db * 1000.0).round() / 1000.0,
                        "fade_in": p.fade_in, "fade_out": p.fade_out,
                        "asset_id": if printed(&t.id) { Value::Null } else { json!(p.asset) },
                    } },
                    "name": name,
                    "source_range": range(p.source, p.len, sr),
                    "effects": if (p.gain - 1.0).abs() > 1e-6 {
                        json!([gain_effect(json!(rational_str(p.gain as f64)), json!({ "param": "clip_gain_db", "gain_db": (gain_db * 1000.0).round() / 1000.0 }))])
                    } else {
                        json!([])
                    },
                    "markers": [], "enabled": true,
                    "media_references": { "DEFAULT_MEDIA": {
                        "OTIO_SCHEMA": "ExternalReference.1",
                        "metadata": { "vcsd": { "channels": m.channels } },
                        "name": file_name,
                        "available_range": range(0, m.frames, m.sample_rate as f64),
                        "available_image_bounds": null,
                        "target_url": url_for(&m.path),
                    } },
                    "active_media_reference_key": "DEFAULT_MEDIA",
                }));
                report.clips += 1;
                pos = p.at + p.len;
                if p.fade_out > 0 {
                    let butted = lane_pieces.get(k + 1).is_some_and(|q| q.at == pos);
                    if butted {
                        losses.add("fades between butted clips carried as clip metadata only", 1);
                    } else {
                        items.push(fade(0, p.fade_out, sr));
                        report.fade_transitions += 1;
                    }
                }
            }
            if end > pos {
                items.push(gap(end - pos, sr));
            }
            let mut name = t.name.clone();
            if li > 0 {
                name.push_str(&format!(" ({})", li + 1));
            }
            children.push(json!({
                "OTIO_SCHEMA": "Track.1",
                "metadata": { "vcsd": {
                    "track_id": t.id, "layer": li + 1, "channels": t.channels, "category": t.category.display(),
                    "bus": bus, "gain_db": t.gain_db, "pan": t.pan, "printed": printed(&t.id),
                    // automation: [offset in samples from the timeline start, value]
                    "volume_db": points(AutomationParam::VolumeDb),
                    "pan_automation": points(AutomationParam::Pan),
                } },
                "name": name,
                "source_range": null, "effects": volume.iter().chain(&pan).collect::<Vec<_>>(), "markers": [], "enabled": true,
                "children": items,
                "kind": "Audio",
            }));
        }
    }

    // ---- cues → timeline markers
    let mut cues: Vec<&Cue> = s.cues.values().filter(|c| c.start_sample >= start && c.start_sample < end).collect();
    cues.sort_by(|a, b| (a.start_sample, &a.code).cmp(&(b.start_sample, &b.code)));
    let markers: Vec<Value> = cues
        .iter()
        .map(|c| {
            json!({
                "OTIO_SCHEMA": "Marker.2",
                "metadata": { "vcsd": { "cue_id": c.id, "code": c.code, "category": c.category.display() } },
                "name": c.code,
                "color": colour(&c.category),
                "marked_range": range(c.start_sample - start, (c.end_sample.min(end) - c.start_sample).max(0), sr),
                "comment": c.description,
            })
        })
        .collect();
    report.markers = markers.len();
    report.tracks = children.len();
    report.media_files = children
        .iter()
        .flat_map(|t| t["children"].as_array().cloned().unwrap_or_default())
        .filter_map(|c| c["media_references"]["DEFAULT_MEDIA"]["target_url"].as_str().map(String::from))
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    let fr = s.project.frame_rate;
    let tb = s.tb();
    let timeline = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "metadata": { "vcsd": {
            "project": s.project.name, "sample_rate": s.project.sample_rate, "frame_rate": fr.label(),
            "start_tc": tb.tc_at(start), "drop_frame": fr.drop_frame,
        } },
        "name": o.name,
        "global_start_time": rt(tb.frame_at(start) as f64, fr.as_f64()),
        "tracks": {
            "OTIO_SCHEMA": "Stack.1", "metadata": {}, "name": "tracks", "source_range": null,
            "effects": [], "markers": markers, "enabled": true, "children": children,
        },
    });
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(out, serde_json::to_string_pretty(&timeline)?)?;
    report.file = out.display().to_string();
    report.losses = losses.list();
    Ok(report)
}

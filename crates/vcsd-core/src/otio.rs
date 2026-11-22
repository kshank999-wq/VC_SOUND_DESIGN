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

use crate::aaf::{bus_muted, clip_pieces, track_level, trim, url_for, AafMedia, Curve, Losses, Piece};
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
    /// Keep muted tracks, tracks on muted buses and muted clips, disabled
    /// (`enabled: false`, which editors play as muted).
    pub include_muted: bool,
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

/// A constant or keyframed AAF parameter value (keyframes: `[fraction of the
/// range, value]`, as the AAF adapter writes them).
fn aaf_value(c: &Curve, start: i64, end: i64) -> Value {
    match c {
        Curve::Const(v) => json!(rational_str(*v)),
        Curve::Points(p) => {
            let len = (end - start).max(1) as f64;
            json!({
                "_aaf_keyframed_property": true,
                "keyframe_values": p.iter().map(|&(x, v)| json!([(x - start) as f64 / len, (v * 1e6).round() / 1e6])).collect::<Vec<_>>(),
                "keyframe_interpolation": "Linear",
                "keyframe_baked_values": null,
            })
        }
    }
}

/// The track's level and pan as AAF-style effects, with its buses folded in
/// (OTIO has no buses): Mono Audio Gain = fader × volume lane × every bus level
/// down to the master, fitted (linear amplitude) within 0.1 dB of the mixer;
/// Mono Audio Pan (`Pan`: 0 left, 0.5 centre, 1 right) = the track's pan, or on a
/// stereo track its balance, with the buses' balance folded in. A pan lane with
/// no bus balance is written as its breakpoints, exactly. Also returns whether
/// bus levels were folded in.
/// A plugin's automation lane over `[start, end)` as keyframes `[fraction of the
/// range, normalised value]`: plugin lanes ramp linearly between their points,
/// so the breakpoints (plus the range's ends) are exact.
fn plugin_keyframes(pts: &[(i64, f32)], start: i64, end: i64) -> Vec<Value> {
    let last = end - 1;
    let mut keys = vec![start];
    keys.extend(pts.iter().map(|p| p.0).filter(|&x| x > start && x < last));
    keys.push(last);
    keys.dedup();
    let len = (end - start).max(1) as f64;
    keys.iter().map(|&x| json!([(x - start) as f64 / len, (crate::aaf::lane_db(pts, x) * 1e6).round() / 1e6])).collect()
}

/// A track's inserts, in signal order, as OTIO effects. OTIO effects are
/// generic, so each one is described in full in `metadata.vcsd.insert` (the
/// project's own insert: built-in settings, or the plugin's id, format, saved
/// state and parameter values), and a plugin's parameter automation in
/// `metadata.vcsd.automation`: per parameter id, keyframes (as above) and the
/// raw points `[offset in samples from the range start, value]`. Other apps see
/// the effects by name; VC Sound Design rebuilds them.
fn insert_effects(s: &ProjectState, t: &Track, start: i64, end: i64) -> Vec<Value> {
    t.inserts
        .iter()
        .map(|ins| {
            let (name, kind, automation) = match ins {
                Insert::Plugin(p) => {
                    let mut lanes: BTreeMap<u32, Vec<(i64, f32)>> = BTreeMap::new();
                    for a in s.automation.values().filter(|a| a.owner_id == p.id) {
                        if let AutomationParam::Plugin(id) = a.param {
                            lanes.entry(id).or_default().push((a.sample, a.value));
                        }
                    }
                    let auto: serde_json::Map<String, Value> = lanes
                        .into_iter()
                        .map(|(id, mut pts)| {
                            pts.sort_by_key(|x| x.0);
                            let v = json!({
                                "keyframe_values": plugin_keyframes(&pts, start, end),
                                "keyframe_interpolation": "Linear",
                                "points": pts.iter().map(|&(x, v)| json!([x - start, v])).collect::<Vec<_>>(),
                            });
                            (id.to_string(), v)
                        })
                        .collect();
                    let kind = match p.format.as_str() {
                        "au" => "Audio Unit".to_string(),
                        f => format!("{} plugin", f.to_ascii_uppercase()),
                    };
                    (p.name.clone(), kind, Value::Object(auto))
                }
                Insert::Gain { .. } => ("Gain".into(), "VC Sound Design Gain".into(), json!({})),
                Insert::HighPass { .. } => ("High Pass".into(), "VC Sound Design High Pass".into(), json!({})),
                Insert::LowPass { .. } => ("Low Pass".into(), "VC Sound Design Low Pass".into(), json!({})),
                Insert::Peak { .. } => ("Peak EQ".into(), "VC Sound Design Peak EQ".into(), json!({})),
                Insert::Compressor { .. } => ("Compressor".into(), "VC Sound Design Compressor".into(), json!({})),
            };
            json!({
                "OTIO_SCHEMA": "Effect.1",
                "metadata": { "vcsd": { "insert": ins, "automation": automation } },
                "name": name,
                "effect_name": kind,
            })
        })
        .collect()
}

fn track_effects(s: &ProjectState, t: &Track, start: i64, end: i64) -> (Vec<Value>, bool) {
    let (gain, pan, folded) = track_level(s, t, start, end);
    let vcsd = |param: &str| {
        json!({
            "param": param,
            "bus_folded": folded,
            "law": if t.channels >= 2 { "balance" } else { "constant_power_-3dB" },
        })
    };
    let mut out = vec![];
    if let Some(g) = gain {
        out.push(gain_effect(aaf_value(&g, start, end), vcsd("volume")));
    }
    if let Some(p) = pan {
        out.push(json!({
            "OTIO_SCHEMA": "Effect.1",
            "metadata": {
                "AAF": {
                    "Operation": { "Name": "Mono Audio Pan", "Identification": "9d2ea893-0968-11d3-8a38-0050040ef7d2" },
                    "Parameters": { "Pan": aaf_value(&p, start, end) },
                },
                "vcsd": vcsd("pan"),
            },
            "name": "Mono Audio Pan",
            "effect_name": "Mono Audio Pan",
        }));
    }
    (out, folded)
}

/// The parameter automation of the plugins among `inserts`, by slot id and
/// parameter id: points `[offset in samples from the range start, value]`.
pub(crate) fn plugin_automation(s: &ProjectState, inserts: &[Insert], start: i64) -> Value {
    let mut plugins = serde_json::Map::new();
    for p in inserts.iter().filter_map(Insert::as_plugin) {
        let mut lanes: BTreeMap<u32, Vec<Value>> = BTreeMap::new();
        for a in s.automation.values().filter(|a| a.owner_id == p.id) {
            if let AutomationParam::Plugin(id) = a.param {
                lanes.entry(id).or_default().push(json!([a.sample - start, a.value]));
            }
        }
        for l in lanes.values_mut() {
            l.sort_by_key(|x| x[0].as_i64());
        }
        plugins.insert(p.id.clone(), json!(lanes.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<BTreeMap<_, _>>()));
    }
    Value::Object(plugins)
}

/// The bus tree, for VC Sound Design to rebuild (OTIO has no buses): each bus
/// with its parent, level, mute, inserts (as in the project), volume and balance
/// automation, and its plugins' parameter automation — points as
/// `[offset in samples from the range start, value]`.
pub(crate) fn bus_tree(s: &ProjectState, start: i64) -> Value {
    let points = |owner: &str, param: AutomationParam| -> Vec<Value> {
        lane(s, owner, param, start).into_iter().map(|(x, v)| json!([x, v])).collect()
    };
    let mut buses: Vec<&Bus> = s.buses.values().collect();
    buses.sort_by(|a, b| (a.order, &a.name).cmp(&(b.order, &b.name)));
    let v: Vec<Value> = buses
        .into_iter()
        .map(|b| {
            let plugins = plugin_automation(s, &b.inserts, start);
            json!({
                "id": b.id, "name": b.name, "parent": b.parent_id, "order": b.order,
                "gain_db": b.gain_db, "mute": b.mute, "stem": b.stem, "inserts": b.inserts,
                "volume_db": points(&b.id, AutomationParam::VolumeDb),
                "pan_automation": points(&b.id, AutomationParam::Pan),
                "plugin_automation": plugins,
            })
        })
        .collect();
    json!(v)
}

/// "#rrggbb" as OTIO's `Color` (r, g, b in 0–1). Each channel n is written as
/// (n + 0.25) / 255: readers that truncate (OpenTimelineIO's own `to_hex`: n/255
/// can come back as n − 1) and readers that round both get n back.
fn otio_color(hex: &str) -> Option<Value> {
    let h = parse_hex_color(hex)?;
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map(|v| ((v as f64 + 0.25) / 255.0).min(1.0)).ok();
    Some(json!({ "OTIO_SCHEMA": "Color.1", "name": "", "r": c(1)?, "g": c(3)?, "b": c(5)?, "a": 1.0 }))
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
    losses.add(
        "bus routing and bus automation in VC Sound Design metadata only: other apps get bus levels folded into each track's gain and pan effects",
        s.buses.values().filter(|b| b.parent_id.is_some()).count(),
    );
    let inserts: usize = s.tracks.values().filter(|t| !t.mute && !printed(&t.id)).map(|t| t.inserts.len()).sum();
    losses.add(
        "track plugins and insert effects described as OTIO effects (with their state and automation) that other apps won't apply: print tracks with plugins to hear them there",
        inserts,
    );
    losses.add(
        "insert effects and plugins on buses in VC Sound Design metadata only (other apps don't apply them; they are in the stems and mix)",
        s.buses.values().map(|b| b.inserts.len()).sum(),
    );
    losses.add("tracks printed with their plugins and insert effects: one rendered clip each, not editable clip by clip", o.prints.len());
    losses.add(
        "solo is monitoring state, not applied: every track exported audible (the solo flags are in VC Sound Design metadata)",
        s.tracks.values().filter(|t| t.solo).count(),
    );

    let mut tracks: Vec<&Track> = s.tracks.values().collect();
    tracks.sort_by(|a, b| (&a.folder, a.order, &a.name).cmp(&(&b.folder, b.order, &b.name)));
    let mut children = vec![];
    for t in tracks {
        let bus_off = bus_muted(s, &t.bus_id);
        if bus_off && !o.include_muted {
            losses.add("tracks on muted buses left out", 1);
            continue;
        }
        if t.mute && !o.include_muted {
            losses.add("muted tracks left out", 1);
            continue;
        }
        // kept on request, silent: disabled (its own mute, or its bus's, in our metadata)
        let silenced = bus_off || t.mute;
        if silenced {
            losses.add("muted tracks and tracks on muted buses included, disabled", 1);
        }
        let mut pieces: Vec<Piece> = vec![];
        if printed(&t.id) {
            pieces.push(Piece { at: start, len, source: 0, fade_in: 0, fade_out: 0, gain: 1.0, asset: print_key(&t.id), muted: false });
        } else {
            let mut clips: Vec<&ClipInstance> = s.clips.values().filter(|c| c.track_id == t.id).collect();
            clips.sort_by(|a, b| (a.timeline_start, &a.id).cmp(&(b.timeline_start, &b.id)));
            for c in clips {
                if c.muted && !o.include_muted {
                    losses.add("muted clips left out", 1);
                    continue;
                }
                if c.muted {
                    losses.add("muted clips included, disabled", 1);
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
        let (levels, bus_folded) = track_effects(s, t, start, end);
        // inserts first (they're pre-fader), unless the track is printed with them
        let mut effects = if printed(&t.id) { vec![] } else { insert_effects(s, t, start, end) };
        effects.extend(levels);
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
                    "markers": [], "enabled": !p.muted,
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
            let mut track = json!({
                "OTIO_SCHEMA": "Track.1",
                "metadata": { "vcsd": {
                    "track_id": t.id, "layer": li + 1, "channels": t.channels, "category": t.category.display(),
                    "bus": bus, "gain_db": t.gain_db, "pan": t.pan, "printed": printed(&t.id),
                    // true: the effects carry bus levels too (what the track sounds like), the raw values don't
                    "bus_folded": bus_folded,
                    // the track's own mute (a disabled track may only be on a muted bus)
                    "mute": t.mute,
                    // monitoring state: every track is exported audible; this restores the flag
                    "solo": t.solo,
                    // where it sits in the project: its folder and position there
                    "folder": t.folder, "order": t.order,
                    "color": t.color,
                    "bus_id": t.bus_id,
                    // automation: [offset in samples from the timeline start, value]
                    "volume_db": points(AutomationParam::VolumeDb),
                    "pan_automation": points(AutomationParam::Pan),
                } },
                "name": name,
                "source_range": null, "effects": effects.clone(), "markers": [], "enabled": !silenced,
                "children": items,
                "kind": "Audio",
            });
            // the track's own color as OTIO's Color (OpenTimelineIO 0.18; older readers keep it as an extra field)
            if let Some(c) = t.color.as_deref().and_then(otio_color) {
                track["color"] = c;
            }
            children.push(track);
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
            "buses": bus_tree(s, start),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_channel_value_survives_truncating_and_rounding_readers() {
        for n in 0..=255u32 {
            let c = otio_color(&format!("#{n:02x}{n:02x}{n:02x}")).unwrap();
            let v = c["r"].as_f64().unwrap() * 255.0;
            assert_eq!(v as u32, n, "truncated");
            assert_eq!(v.round() as u32, n, "rounded");
        }
    }
}

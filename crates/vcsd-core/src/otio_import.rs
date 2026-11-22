//! OTIO import (spec §15.2): bring an OpenTimelineIO timeline into the project.
//!
//! Reads `.otio` JSON (OpenTimelineIO 0.12 and later: `Clip.1` and `Clip.2`,
//! a `Timeline` or the first one in a `SerializableCollection`) and rebuilds
//! its audio tracks as new tracks, in a folder for the import:
//!
//! - clips at their positions (any rate: times are converted to samples), with
//!   their source ranges; when the media's available range starts at a
//!   timecode (BWF media in editors), the offset into the file is relative to it;
//! - transitions as fades: against a gap a fade in or out, between clips a
//!   crossfade (both clips extend into their handles, as OTIO defines it);
//! - nested stacks (compound clips) as extra tracks, trimmed to their range;
//! - disabled tracks and clips come in muted; video tracks are ignored;
//! - markers on the timeline, tracks and clips become spotting markers in an
//!   "OTIO markers" lane (those inside a scene);
//! - VC Sound Design's own `metadata.vcsd` (clip gain and fades, track fader,
//!   pan, channels, volume and pan automation) where present, so an OTIO
//!   export comes back as it was.
//!
//! Media: `file://` URLs, absolute paths, or paths relative to the `.otio`.
//! WAV/BWF is used where it is (searched for by name when moved), AIFF and MXF
//! are converted like the AAF import's, other sample rates converted (linear),
//! into the import's media folder. What can't be brought in is listed in the
//! [`OtioImportReport`].

use crate::aaf_import::{
    add_point, aiff_to_wav, decode_url, ext_of, file_name_of, guess_category, index_dir, marker_lane, mxf_to_wav, simplify,
};
use crate::error::{invalid, Result};
use crate::ids::new_id;
use crate::layout::safe_name;
use crate::library::{import_wav, stable_hash, AssetMeta};
use crate::model::*;
use crate::state::ProjectState;
use crate::wav::{self, SampleFormat};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct OtioImportOptions {
    /// Where converted media are written.
    pub media_dir: PathBuf,
    /// Folders searched (recursively, by file name) for media that moved;
    /// the `.otio`'s own folder is always searched.
    pub search_dirs: Vec<PathBuf>,
    /// Timeline sample of the timeline's start; `None` = its `global_start_time`.
    pub at: Option<i64>,
    /// Track folder for the new tracks; `None` = "OTIO/<timeline>".
    pub folder: Option<String>,
    /// What to do with the buses our own export describes.
    pub buses: BusLevels,
}

/// Bus levels in an OTIO from VC Sound Design (`metadata.vcsd.buses`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BusLevels {
    /// Restore the buses when the project's matching buses are untouched (unity,
    /// unmuted, no automation or inserts) or missing; otherwise fold.
    #[default]
    Auto,
    /// Restore the buses: match by name under the same parent (or create them),
    /// set their level, mute and inserts, and replace their automation within
    /// the imported range; tracks keep their own levels and are routed to them.
    Buses,
    /// Keep the project's buses as they are; each track carries what its buses
    /// did (the folded gain and pan effects).
    Fold,
}

impl BusLevels {
    pub fn parse(s: &str) -> Option<BusLevels> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(BusLevels::Auto),
            "buses" | "restore" => Some(BusLevels::Buses),
            "fold" => Some(BusLevels::Fold),
            _ => None,
        }
    }
}

impl OtioImportOptions {
    /// Defaults for a project at `root`: media written to `Audio/Imported/OTIO/<file name>`.
    pub fn for_project(root: &Path, file: &Path) -> OtioImportOptions {
        let stem = file.file_stem().map(|s| safe_name(&s.to_string_lossy())).filter(|s| !s.is_empty()).unwrap_or_else(|| "OTIO".into());
        OtioImportOptions { media_dir: root.join("Audio/Imported/OTIO").join(stem), ..Default::default() }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OtioImportReport {
    pub timeline: String,
    /// Timecode of the timeline's start, as placed.
    pub start_tc: String,
    pub tracks: Vec<String>,
    pub stereo_tracks: usize,
    pub clips: usize,
    pub markers: usize,
    pub automated_tracks: usize,
    /// Inserts rebuilt (built-in effects and plugins), plugins among them, and
    /// plugin parameters that came with automation.
    pub inserts: usize,
    pub plugins: usize,
    pub automated_parameters: usize,
    /// How bus levels came in: "buses" (restored) or "fold" (in the tracks).
    pub bus_levels: String,
    /// Buses whose levels and automation were restored, and how many were created.
    pub buses_restored: usize,
    pub buses_created: usize,
    pub media_linked: usize,
    pub media_written: usize,
    /// Clips left out because their media couldn't be found or read: why, per file.
    pub missing_media: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Default)]
struct Notes(BTreeMap<String, usize>);

impl Notes {
    fn add(&mut self, what: impl Into<String>, n: usize) {
        if n > 0 {
            *self.0.entry(what.into()).or_default() += n;
        }
    }
}

// ---------------------------------------------------------------- reading

fn schema(v: &Value) -> &str {
    v["OTIO_SCHEMA"].as_str().unwrap_or("").split('.').next().unwrap_or("")
}

/// A RationalTime in seconds.
fn secs(v: &Value) -> Option<f64> {
    let rate = v["rate"].as_f64()?;
    (rate > 0.0).then(|| v["value"].as_f64().unwrap_or(0.0) / rate)
}

/// A TimeRange as (start, duration) in seconds.
fn range(v: &Value) -> Option<(f64, f64)> {
    Some((secs(&v["start_time"])?, secs(&v["duration"])?))
}

#[derive(Debug, Clone)]
struct Item {
    /// Seconds from the timeline start.
    at: f64,
    len: f64,
    /// Seconds into the media (relative to its available range's start).
    source: f64,
    url: Option<String>,
    /// Why there's no usable media reference.
    missing: Option<String>,
    fade_in: f64,
    fade_out: f64,
    muted: bool,
    vcsd: Value,
    effects: Vec<Value>,
}

#[derive(Debug, Clone)]
struct OLane {
    name: String,
    items: Vec<Item>,
    vcsd: Value,
    muted: bool,
    effects: Vec<Value>,
    /// The track's own range on the timeline (seconds), for its effects' keyframes.
    start: f64,
    len: f64,
}

/// A level parameter over time (seconds from the timeline start).
#[derive(Debug, Clone)]
enum Level {
    Const(f64),
    Points(Vec<(f64, f64)>),
}

impl Level {
    fn at(&self, t: f64) -> f64 {
        match self {
            Level::Const(v) => *v,
            Level::Points(p) => {
                let i = p.partition_point(|x| x.0 <= t);
                match (i.checked_sub(1).map(|k| p[k]), p.get(i)) {
                    (Some(a), Some(b)) if b.0 > a.0 => a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0),
                    (Some(a), _) => a.1,
                    (None, Some(b)) => b.1,
                    (None, None) => 1.0,
                }
            }
        }
    }
}

/// A number as OTIO tools write AAF values: a number, or a rational string "n/d".
fn number(v: &Value) -> Option<f64> {
    if let Some(x) = v.as_f64() {
        return Some(x);
    }
    let s = v.as_str()?.trim();
    match s.split_once('/') {
        Some((n, d)) => Some(n.trim().parse::<f64>().ok()? / d.trim().parse::<f64>().ok().filter(|d| *d != 0.0)?),
        None => s.parse().ok(),
    }
}

/// The buses the imported tracks feed (and their parents), restored from our
/// export's `metadata.vcsd.buses` per `mode`. Returns exported bus id → project
/// bus id when restored, `None` when the tracks carry the levels instead.
#[allow(clippy::too_many_arguments)]
fn restore_buses(
    s: &mut ProjectState,
    exported: &[Value],
    lanes: &[OLane],
    mode: BusLevels,
    base: i64,
    len: i64,
    from_meta: &dyn Fn(&Value) -> Option<i64>,
    notes: &mut Notes,
    report: &mut OtioImportReport,
) -> Option<HashMap<String, String>> {
    if mode == BusLevels::Fold || exported.is_empty() {
        return None;
    }
    let by_id: HashMap<&str, &Value> = exported.iter().filter_map(|b| Some((b["id"].as_str()?, b))).collect();
    // the buses the tracks use, with every bus above them, parents first
    let mut used: Vec<&str> = vec![];
    for l in lanes {
        let mut chain = vec![];
        let mut cur = l.vcsd["bus_id"].as_str();
        while let Some(id) = cur.filter(|id| by_id.contains_key(id) && !chain.contains(id)) {
            chain.push(id);
            cur = by_id[id]["parent"].as_str();
        }
        for id in chain.into_iter().rev() {
            if !used.contains(&id) {
                used.push(id);
            }
        }
    }
    if used.is_empty() {
        return None;
    }
    // match by name under the same (matched) parent; the root is the project's master
    let master = crate::mixer::master_bus(s).ok()?.id.clone();
    let mut plan: Vec<(&str, Option<String>)> = vec![];
    let mut matched: HashMap<&str, Option<String>> = HashMap::new();
    for &id in &used {
        let b = by_id[id];
        let target = match b["parent"].as_str() {
            None => Some(master.clone()),
            Some(parent) => matched.get(parent).cloned().flatten().and_then(|p| {
                s.buses
                    .values()
                    .find(|x| x.parent_id.as_deref() == Some(p.as_str()) && Some(x.name.as_str()) == b["name"].as_str())
                    .map(|x| x.id.clone())
            }),
        };
        matched.insert(id, target.clone());
        plan.push((id, target));
    }
    let touched = |bid: &str| {
        let bus = &s.buses[bid];
        let slots: Vec<&str> = bus.inserts.iter().filter_map(Insert::as_plugin).map(|p| p.id.as_str()).collect();
        bus.gain_db.abs() > 1e-6
            || bus.mute
            || !bus.inserts.is_empty()
            || s.automation.values().any(|a| a.owner_id == bid || slots.contains(&a.owner_id.as_str()))
    };
    if mode == BusLevels::Auto {
        let busy: Vec<String> =
            plan.iter().filter_map(|(_, t)| t.as_deref()).filter(|t| touched(t)).map(|t| s.buses[t].name.clone()).collect();
        if !busy.is_empty() {
            notes.add(
                format!(
                    "bus levels folded into the tracks: the project's buses have their own levels ({}); import with buses to restore them",
                    busy.join(", ")
                ),
                1,
            );
            return None;
        }
    }
    let mut map: HashMap<String, String> = HashMap::new();
    for (id, target) in plan {
        let b = by_id[id];
        let bid = match target {
            Some(t) => t,
            None => {
                let parent = b["parent"].as_str().and_then(|p| map.get(p)).cloned();
                let name = b["name"].as_str().unwrap_or("Bus");
                let Ok(new) = crate::mixer::add_bus(s, name, parent.as_deref()) else { continue };
                if let Some(bus) = s.buses.get_mut(&new) {
                    bus.stem = b["stem"].as_bool().unwrap_or(true);
                }
                report.buses_created += 1;
                new
            }
        };
        // level, mute, inserts (plugins under new slot ids, with their automation)
        let mut slot_map: HashMap<String, String> = HashMap::new();
        let inserts: Vec<Insert> = b["inserts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| serde_json::from_value::<Insert>(i.clone()).ok())
            .map(|i| match i {
                Insert::Plugin(mut p) => {
                    let new = new_id("ins");
                    slot_map.insert(std::mem::replace(&mut p.id, new), p.id.clone());
                    Insert::Plugin(p)
                }
                other => other,
            })
            .collect();
        let old_slots: Vec<String> = s.buses[&bid].inserts.iter().filter_map(Insert::as_plugin).map(|p| p.id.clone()).collect();
        {
            let bus = s.buses.get_mut(&bid).unwrap();
            bus.gain_db = b["gain_db"].as_f64().unwrap_or(0.0) as f32;
            bus.mute = b["mute"].as_bool().unwrap_or(false);
            bus.inserts = inserts;
        }
        s.automation.retain(|_, a| !old_slots.contains(&a.owner_id));
        // the bus's lanes within the imported range are the file's
        let (lo, hi) = (base, base + len);
        s.automation.retain(|_, a| {
            !(a.owner_id == bid && matches!(a.param, AutomationParam::VolumeDb | AutomationParam::Pan) && a.sample >= lo && a.sample <= hi)
        });
        for (key, param) in [("volume_db", AutomationParam::VolumeDb), ("pan_automation", AutomationParam::Pan)] {
            for p in b[key].as_array().into_iter().flatten() {
                if let (Some(t), Some(v)) = (from_meta(&p[0]), number(&p[1])) {
                    add_point(s, &bid, param, base + t, crate::editing::clamp_automation(param, v as f32));
                }
            }
        }
        for (old, lanes) in b["plugin_automation"].as_object().into_iter().flatten() {
            let Some(slot) = slot_map.get(old) else { continue };
            for (pid, pts) in lanes.as_object().into_iter().flatten() {
                let Ok(pid) = pid.parse::<u32>() else { continue };
                for p in pts.as_array().into_iter().flatten() {
                    if let (Some(t), Some(v)) = (from_meta(&p[0]), number(&p[1])) {
                        add_point(s, slot, AutomationParam::Plugin(pid), base + t, (v as f32).clamp(0.0, 1.0));
                    }
                }
            }
        }
        report.buses_restored += 1;
        map.insert(id.to_string(), bid);
    }
    Some(map)
}

/// One plugin parameter's automation from our OTIO description: its raw
/// `points` ([offset in samples of the exporting project's rate from the timeline
/// start, value]) when present, else its `keyframe_values` ([fraction of the
/// track's range, value]) — what's left when a tool keeps keyframes only. Values
/// are normalised 0–1. "Constant" (step) interpolation becomes holds; Bezier and
/// cubic are read as linear.
fn plugin_lane(
    auto: &Value,
    from_meta: &dyn Fn(&Value) -> Option<i64>,
    smp: &dyn Fn(f64) -> i64,
    track_start: f64,
    track_len: f64,
    notes: &mut Notes,
) -> Vec<(i64, f32)> {
    let mut pts: Vec<(i64, f32)> = match auto["points"].as_array() {
        Some(p) => p.iter().filter_map(|x| Some((from_meta(&x[0])?, number(&x[1])? as f32))).collect(),
        None => auto["keyframe_values"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| Some((smp(track_start + number(&k[0])? * track_len), number(&k[1])? as f32)))
            .collect(),
    };
    pts.sort_by_key(|p| p.0);
    pts.dedup_by_key(|p| p.0);
    for p in &mut pts {
        p.1 = p.1.clamp(0.0, 1.0);
    }
    let interp = auto["keyframe_interpolation"].as_str().unwrap_or("Linear").to_ascii_lowercase();
    match interp.as_str() {
        "linear" | "" => pts,
        "constant" | "step" | "hold" => {
            // each value held until the sample before the next
            let mut out = vec![];
            for (i, &(t, v)) in pts.iter().enumerate() {
                out.push((t, v));
                if let Some(&(next, nv)) = pts.get(i + 1) {
                    if next - 1 > t && nv != v {
                        out.push((next - 1, v));
                    }
                }
            }
            out
        }
        _ => {
            notes.add("plugin automation with curved interpolation read as linear", 1);
            pts
        }
    }
}

/// An AAF-style level effect (the OpenTimelineIO AAF adapter's encoding): Mono
/// Audio Gain (`Amplitude`, linear) or Mono Audio Pan (`Pan`, 0 left … 1 right),
/// constant or keyframed over `[t0, t0 + len)` (keyframe times are fractions of it).
/// Returns (is gain, its curve).
fn level_effect(e: &Value, t0: f64, len: f64) -> Option<(bool, Level)> {
    let aaf = &e["metadata"]["AAF"];
    let names = [e["effect_name"].as_str(), e["name"].as_str(), aaf["Operation"]["Name"].as_str()];
    let is = |n: &str| names.iter().flatten().any(|x| x.eq_ignore_ascii_case(n));
    let (gain, param) = if is("Mono Audio Gain") {
        (true, "Amplitude")
    } else if is("Mono Audio Pan") {
        (false, "Pan")
    } else {
        return None;
    };
    let p = &aaf["Parameters"][param];
    if let Some(kf) = p["keyframe_values"].as_array() {
        let mut pts: Vec<(f64, f64)> = kf.iter().filter_map(|k| Some((t0 + number(&k[0])? * len, number(&k[1])?))).collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        return (!pts.is_empty()).then_some((gain, Level::Points(pts)));
    }
    number(p).map(|v| (gain, Level::Const(v)))
}

struct Reader<'a> {
    notes: &'a mut Notes,
    markers: Vec<(f64, f64, String)>,
}

impl Reader<'_> {
    fn markers_of(&mut self, v: &Value, shift: f64) {
        for m in v["markers"].as_array().into_iter().flatten() {
            let Some((a, d)) = range(&m["marked_range"]) else { continue };
            let name = m["name"].as_str().unwrap_or("").trim().to_string();
            let comment = m["comment"].as_str().unwrap_or("").trim().to_string();
            let label = match (name.is_empty(), comment.is_empty()) {
                (false, false) => format!("{name} — {comment}"),
                (false, true) => name,
                (true, false) => comment,
                (true, true) => "OTIO marker".into(),
            };
            self.markers.push((a + shift, d, label));
        }
    }

    /// One track's items, positioned from `offset` (seconds), kept to `window`.
    fn track(&mut self, t: &Value, name: String, offset: f64, window: Option<(f64, f64)>, lanes: &mut Vec<OLane>) {
        // track time starts at its source range, when it has one
        let offset = offset - range(&t["source_range"]).map(|r| r.0).unwrap_or(0.0);
        self.markers_of(t, offset);
        let children: Vec<&Value> = t["children"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        let mut items: Vec<Option<Item>> = vec![];
        let mut transitions: Vec<(usize, f64, f64)> = vec![];
        let mut pos = offset;
        for c in children {
            match schema(c) {
                "Transition" => {
                    let (i, o) = (secs(&c["in_offset"]).unwrap_or(0.0), secs(&c["out_offset"]).unwrap_or(0.0));
                    let kind = c["transition_type"].as_str().unwrap_or("");
                    if !kind.is_empty() && kind != "SMPTE_Dissolve" {
                        self.notes.add("transitions other than dissolves read as fades", 1);
                    }
                    transitions.push((items.len(), i, o));
                }
                "Clip" => {
                    let media = match c.get("media_references") {
                        Some(refs) => refs.get(c["active_media_reference_key"].as_str().unwrap_or("DEFAULT_MEDIA")).cloned(),
                        None => c.get("media_reference").cloned(),
                    }
                    .unwrap_or(Value::Null);
                    let avail = range(&media["available_range"]);
                    let Some((src, len)) = range(&c["source_range"]).or(avail) else {
                        self.notes.add("clips without a source or available range left out", 1);
                        continue;
                    };
                    let (url, missing) = match schema(&media) {
                        "ExternalReference" => match media["target_url"].as_str().filter(|u| !u.is_empty()) {
                            Some(u) => (Some(u.to_string()), None),
                            None => (None, Some("a clip's media reference has no URL".to_string())),
                        },
                        "MissingReference" | "" => {
                            let n = c["name"].as_str().unwrap_or("clip");
                            (None, Some(format!("{n}: media offline in the OTIO (missing reference)")))
                        }
                        other => (None, Some(format!("{other} media (not audio) can't be imported"))),
                    };
                    let item = Item {
                        at: pos,
                        len,
                        source: src - avail.map(|a| a.0).unwrap_or(0.0),
                        url,
                        missing,
                        fade_in: 0.0,
                        fade_out: 0.0,
                        muted: c["enabled"].as_bool() == Some(false),
                        vcsd: c["metadata"]["vcsd"].clone(),
                        effects: c["effects"].as_array().cloned().unwrap_or_default(),
                    };
                    self.markers_of(c, pos - src);
                    items.push(Some(item));
                    pos += len;
                }
                "Stack" => {
                    // a nested stack (compound clip): its audio tracks as extra lanes
                    let (s0, len) = range(&c["source_range"]).unwrap_or((0.0, 0.0));
                    let mut inner = vec![];
                    for (k, sub) in c["children"].as_array().into_iter().flatten().enumerate() {
                        if schema(sub) == "Track" && is_audio(sub) {
                            let n = sub["name"].as_str().filter(|n| !n.is_empty()).map(String::from).unwrap_or(format!("{}", k + 1));
                            self.track(sub, format!("{name} / {n}"), pos - s0, Some((pos, pos + len)), &mut inner);
                        }
                    }
                    lanes.extend(inner);
                    items.push(None);
                    pos += len;
                }
                _ => {
                    // gaps (and anything else with a duration) just take time
                    if schema(c) != "Gap" {
                        self.notes.add(format!("{} items left out", schema(c)), 1);
                    }
                    pos += range(&c["source_range"]).map(|r| r.1).unwrap_or(0.0);
                    items.push(None);
                }
            }
        }
        // transitions: the clip before extends past the cut, the one after starts before it
        for (k, i, o) in transitions {
            if let Some(Some(a)) = k.checked_sub(1).and_then(|p| items.get_mut(p)) {
                a.len += o;
                a.fade_out = i + o;
            }
            if let Some(Some(b)) = items.get_mut(k) {
                b.at -= i;
                b.source -= i;
                b.len += i;
                b.fade_in = i + o;
            }
        }
        let mut out = vec![];
        for mut it in items.into_iter().flatten() {
            if it.source < 0.0 {
                // not enough media before the clip for its transition
                self.notes.add("transitions longer than the media's handles shortened", 1);
                let d = -it.source;
                it.at += d;
                it.len -= d;
                it.source = 0.0;
                it.fade_in = (it.fade_in - d).max(0.0);
            }
            if let Some((w0, w1)) = window {
                if it.at < w0 {
                    let d = w0 - it.at;
                    it.at = w0;
                    it.source += d;
                    it.len -= d;
                    it.fade_in = (it.fade_in - d).max(0.0);
                }
                if it.at + it.len > w1 {
                    it.len = w1 - it.at;
                    it.fade_out = it.fade_out.min(it.len);
                }
            }
            if it.len > 0.0 {
                out.push(it);
            }
        }
        if !out.is_empty() {
            let muted = t["enabled"].as_bool() == Some(false);
            lanes.insert(
                0,
                OLane {
                    name,
                    items: out,
                    vcsd: t["metadata"]["vcsd"].clone(),
                    muted,
                    effects: t["effects"].as_array().cloned().unwrap_or_default(),
                    start: offset,
                    len: pos - offset,
                },
            );
        }
    }
}

fn is_audio(t: &Value) -> bool {
    t["kind"].as_str().is_some_and(|k| k.eq_ignore_ascii_case("audio"))
}

// ---------------------------------------------------------------- media

struct Media<'a> {
    o: &'a OtioImportOptions,
    dir: PathBuf,
    sr: u32,
    index: Option<HashMap<String, PathBuf>>,
    cache: HashMap<String, std::result::Result<(PathBuf, bool), String>>,
}

impl Media<'_> {
    fn locate(&mut self, url: &str) -> Option<PathBuf> {
        let p = if url.contains("://") { PathBuf::from(decode_url(url)) } else { PathBuf::from(url) };
        let p = if p.is_relative() { self.dir.join(p) } else { p };
        if p.is_file() {
            return Some(p);
        }
        let name = file_name_of(&decode_url(url));
        if name.is_empty() {
            return None;
        }
        if self.index.is_none() {
            let mut idx = HashMap::new();
            for d in self.o.search_dirs.iter().chain(std::iter::once(&self.dir)) {
                index_dir(d, 0, &mut idx);
            }
            self.index = Some(idx);
        }
        self.index.as_ref().unwrap().get(&name.to_lowercase()).cloned()
    }

    /// A WAV at the project rate for `url`, and whether it was written here.
    fn get(&mut self, url: &str) -> std::result::Result<(PathBuf, bool), String> {
        if let Some(r) = self.cache.get(url) {
            return r.clone();
        }
        let r = self.load(url);
        self.cache.insert(url.to_string(), r.clone());
        r
    }

    fn load(&mut self, url: &str) -> std::result::Result<(PathBuf, bool), String> {
        let shown = decode_url(url);
        let path = self.locate(url).ok_or_else(|| format!("{} not found ({shown})", file_name_of(&shown)))?;
        let stem = path.file_stem().map(|s| safe_name(&s.to_string_lossy())).unwrap_or_default();
        let out = |suffix: &str| self.o.media_dir.join(format!("{stem}_{:08x}{suffix}.wav", stable_hash(url) as u32));
        let (wav_path, mut written) = match ext_of(&path).as_str() {
            "wav" | "bwf" | "wave" => (path.clone(), false),
            "aif" | "aiff" | "aifc" | "mxf" => {
                let o = out("");
                if !o.is_file() {
                    std::fs::create_dir_all(&self.o.media_dir).map_err(|e| e.to_string())?;
                    let b = std::fs::read(&path).map_err(|e| e.to_string())?;
                    if ext_of(&path) == "mxf" { mxf_to_wav(&b, None, &o) } else { aiff_to_wav(&b, &o) }
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                }
                (o, true)
            }
            e => return Err(format!("{}: .{e} media can't be read (WAV, AIFF and MXF can)", path.display())),
        };
        let info = wav::read_info(&wav_path).map_err(|e| format!("{}: {e}", path.display()))?;
        if info.sample_rate == self.sr {
            return Ok((wav_path, written));
        }
        let o = out(&format!("_{}", self.sr));
        if !o.is_file() {
            let (_, buf) = wav::read(&wav_path).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&self.o.media_dir).map_err(|e| e.to_string())?;
            wav::write(&o, &crate::mixer::resample_linear(&buf, self.sr), SampleFormat::Int24, None).map_err(|e| e.to_string())?;
        }
        written = true;
        Ok((o, written))
    }
}

// ---------------------------------------------------------------- import

/// Import the OTIO file's timeline into the project as new tracks.
pub fn import(s: &mut ProjectState, path: &Path, o: &OtioImportOptions) -> Result<OtioImportReport> {
    let text = std::fs::read_to_string(path)?;
    let root: Value = serde_json::from_str(&text).map_err(|e| invalid(format!("{} is not an OTIO file ({e})", path.display())))?;
    let tl = match schema(&root) {
        "Timeline" => root.clone(),
        "SerializableCollection" => root["children"]
            .as_array()
            .and_then(|c| c.iter().find(|x| schema(x) == "Timeline").cloned())
            .ok_or_else(|| invalid("the OTIO collection has no timeline"))?,
        other => return Err(invalid(format!("not an OTIO timeline ({})", if other.is_empty() { "no OTIO_SCHEMA" } else { other }))),
    };
    let name = tl["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("OTIO").to_string();
    let psr = s.project.sample_rate;
    let smp = |sec: f64| (sec * psr as f64).round() as i64;
    let mut notes = Notes::default();

    // ---- placement: global_start_time
    let base = match (o.at, secs(&tl["global_start_time"])) {
        (Some(at), _) => at,
        (None, Some(t)) => smp(t) - s.tb().origin_since_midnight(),
        (None, None) => 0,
    };
    // our own metadata is in samples of the exporting project's rate
    let meta_sr = tl["metadata"]["vcsd"]["sample_rate"].as_f64().unwrap_or(psr as f64);
    let from_meta = |v: &Value| v.as_f64().map(|x| (x * psr as f64 / meta_sr).round() as i64);

    // ---- tracks
    let stack = &tl["tracks"];
    let mut reader = Reader { notes: &mut notes, markers: vec![] };
    reader.markers_of(stack, 0.0);
    let mut lanes = vec![];
    for (k, t) in stack["children"].as_array().into_iter().flatten().enumerate() {
        if schema(t) != "Track" {
            continue;
        }
        if !is_audio(t) {
            continue;
        }
        let n = t["name"].as_str().filter(|n| !n.trim().is_empty()).map(String::from).unwrap_or(format!("A{}", k + 1));
        let mut mine = vec![];
        reader.track(t, n, 0.0, None, &mut mine);
        lanes.extend(mine);
    }
    let markers = std::mem::take(&mut reader.markers);
    drop(reader);

    let mut media = Media { o, dir: path.parent().map(Path::to_path_buf).unwrap_or_default(), sr: psr, index: None, cache: HashMap::new() };
    let folder = o.folder.clone().unwrap_or_else(|| format!("OTIO/{}", name.replace('/', " & ")));
    let mut report = OtioImportReport { timeline: name.clone(), start_tc: s.tb().tc_at(base), ..Default::default() };
    let mut order = s.tracks.values().map(|t| t.order + 1).max().unwrap_or(0);
    let mut assets: HashMap<PathBuf, String> = HashMap::new();
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    let license =
        License { name: "Editorial media (OTIO)".into(), redistributable: true, source: "editorial".into(), ..Default::default() };
    // ---- buses (our own export): restore them, or let the tracks carry them
    let exported: Vec<Value> = tl["metadata"]["vcsd"]["buses"].as_array().cloned().unwrap_or_default();
    let end_t = lanes.iter().map(|l| l.start + l.len).fold(0.0f64, f64::max);
    let bus_map = restore_buses(s, &exported, &lanes, o.buses, base, smp(end_t), &from_meta, &mut notes, &mut report);
    report.bus_levels = if bus_map.is_some() { "buses".into() } else { "fold".into() };
    let mut levels_found = false;
    for lane in lanes {
        let tid = new_id("trk");
        let mut clips = vec![];
        let mut chans = 1u16;
        // levels from AAF-style effects (when our own metadata isn't there): (gain?, start, end, curve)
        let mut levels: Vec<(bool, f64, f64, Level)> = vec![];
        for it in &lane.items {
            let got = match (&it.url, &it.missing) {
                (Some(u), _) => media.get(u),
                (None, Some(why)) => Err(why.clone()),
                (None, None) => Err("no media".into()),
            };
            let (file, _) = match got {
                Ok(x) => x,
                Err(e) => {
                    *missing.entry(e).or_default() += 1;
                    continue;
                }
            };
            let asset_id = match assets.get(&file) {
                Some(id) => id.clone(),
                None => {
                    let meta = AssetMeta { kind: AssetKind::Imported, license: Some(license.clone()), ..Default::default() };
                    let id = import_wav(s, &file, meta)?;
                    assets.insert(file.clone(), id.clone());
                    id
                }
            };
            let ach = s.assets[&asset_id].channels;
            if ach > 2 {
                notes.add("multichannel media on stereo tracks (its first two channels play)", 1);
            }
            chans = chans.max(ach.min(2));
            let v = &it.vcsd;
            let len = smp(it.len);
            // clip gain: ours, else constant gain effects (keyframed ones go to the track's lane)
            let mut clip_gain = v["gain_db"].as_f64();
            if clip_gain.is_none() {
                let mut amp = 1.0;
                for e in &it.effects {
                    match level_effect(e, it.at, it.len) {
                        Some((true, Level::Const(g))) => amp *= g,
                        Some((gain, curve)) => levels.push((gain, it.at, it.at + it.len, curve)),
                        None => notes.add("clip effects other than gain left out", 1),
                    }
                }
                if (amp - 1.0f64).abs() > 1e-9 {
                    levels_found = true;
                    clip_gain = Some(crate::aaf_import::amp_db(amp));
                }
            }
            clips.push(ClipInstance {
                id: new_id("clp"),
                track_id: tid.clone(),
                cue_id: None,
                asset_id,
                timeline_start: base + smp(it.at),
                source_offset: smp(it.source),
                length: len,
                gain_db: clip_gain.unwrap_or(0.0) as f32,
                fade_in: from_meta(&v["fade_in"]).unwrap_or_else(|| smp(it.fade_in)).clamp(0, len),
                fade_out: from_meta(&v["fade_out"]).unwrap_or_else(|| smp(it.fade_out)).clamp(0, len),
                fill: FillMode::OneShot,
                muted: it.muted,
                take_id: None,
                performance_event_id: None,
                sub_index: None,
                is_placeholder: false,
                history: vec![],
            });
        }
        if clips.is_empty() {
            continue;
        }
        let tv = &lane.vcsd;
        // our raw track values, unless the buses were folded into the effects (then
        // the effects are what the track sounded like; this project's buses are its own)
        // restored buses: the track's own values, routed to its bus
        let routed = bus_map.as_ref().and_then(|m| tv["bus_id"].as_str().and_then(|b| m.get(b))).cloned();
        let raw = tv.is_object() && (routed.is_some() || tv["bus_folded"].as_bool() != Some(true));
        // inserts described by our own export: rebuilt in order, plugins with new slot ids
        let mut inserts = vec![];
        let mut plugin_points: Vec<(String, u32, i64, f32)> = vec![];
        for e in &lane.effects {
            let d = &e["metadata"]["vcsd"];
            if d["insert"].is_null() {
                continue;
            }
            match serde_json::from_value::<Insert>(d["insert"].clone()) {
                Ok(Insert::Plugin(mut p)) => {
                    p.id = new_id("ins");
                    // OTIO's own switch: a disabled effect is a bypassed plugin
                    if e["enabled"].as_bool() == Some(false) {
                        p.bypass = true;
                    }
                    for (id, auto) in d["automation"].as_object().into_iter().flatten() {
                        let Ok(id) = id.trim().parse::<u32>() else {
                            notes.add("plugin automation for parameters without a numeric id left out", 1);
                            continue;
                        };
                        let pts = plugin_lane(auto, &from_meta, &smp, lane.start, lane.len, &mut notes);
                        report.automated_parameters += !pts.is_empty() as usize;
                        plugin_points.extend(pts.into_iter().map(|(t, v)| (p.id.clone(), id, t, v)));
                    }
                    report.plugins += 1;
                    report.inserts += 1;
                    inserts.push(Insert::Plugin(p));
                }
                Ok(other) => {
                    report.inserts += 1;
                    inserts.push(other);
                }
                Err(_) => notes.add("insert effects that couldn't be read left out", 1),
            }
        }
        if !raw {
            for e in lane.effects.iter().filter(|e| e["metadata"]["vcsd"]["insert"].is_null()) {
                match level_effect(e, lane.start, lane.len) {
                    Some((gain, curve)) => levels.push((gain, lane.start, lane.start + lane.len, curve)),
                    None => notes.add("track effects other than gain and pan left out", 1),
                }
            }
        }
        levels_found |= tv.is_object() || !levels.is_empty();
        let channels = tv["channels"].as_u64().map(|c| c.clamp(1, 2) as u16).unwrap_or(chans);
        let category = guess_category(&lane.name);
        let bus_id = match (&routed, &category) {
            (Some(b), _) => b.clone(),
            (None, Category::Custom(_)) => {
                s.buses.values().find(|b| b.name == crate::mixer::MASTER_BUS).map(|b| b.id.clone()).unwrap_or_default()
            }
            (None, cat) => crate::builder::bus_for_category(s, cat),
        };
        s.tracks.insert(
            tid.clone(),
            Track {
                id: tid.clone(),
                name: lane.name.clone(),
                scene_id: None,
                identity_id: None,
                category,
                bus_id,
                channels,
                order,
                folder: folder.clone(),
                gain_db: if raw { tv["gain_db"].as_f64().unwrap_or(0.0) as f32 } else { 0.0 },
                pan: if raw { tv["pan"].as_f64().unwrap_or(0.0).clamp(-1.0, 1.0) as f32 } else { 0.0 },
                mute: lane.muted,
                solo: false,
                armed: false,
                frozen_asset_id: None,
                inserts,
                automation_mode: AutoMode::Read,
            },
        );
        order += 1;
        // automation from our own metadata: [offset in samples from the timeline start, value]
        let mut automated = false;
        for (key, param) in [("volume_db", AutomationParam::VolumeDb), ("pan_automation", AutomationParam::Pan)] {
            if !raw {
                continue;
            }
            let pts: Vec<(i64, f64)> =
                tv[key].as_array().into_iter().flatten().filter_map(|p| Some((from_meta(&p[0])?, p[1].as_f64()?))).collect();
            for (t, v) in simplify(&pts, 1e-6) {
                add_point(s, &tid, param, base + t, crate::editing::clamp_automation(param, v as f32));
                automated |= param == AutomationParam::VolumeDb;
            }
        }
        // AAF-style levels: gains multiply, the last pan wins; fitted to dB / pan lanes
        if !levels.is_empty() {
            let at = |t: i64| t as f64 / psr as f64;
            let covers = |l: &(bool, f64, f64, Level), t: f64| l.1 <= t && t < l.2;
            let gain = |t: i64| levels.iter().filter(|l| l.0 && covers(l, at(t))).map(|l| l.3.at(at(t))).product::<f64>();
            let has_pan = levels.iter().any(|l| !l.0);
            let pan = |t: i64| levels.iter().rev().find(|l| !l.0 && covers(l, at(t))).map(|l| l.3.at(at(t)) * 2.0 - 1.0).unwrap_or(0.0);
            let mut keys = vec![];
            for l in &levels {
                keys.extend([smp(l.1) - 1, smp(l.1), smp(l.2) - 1, smp(l.2)]);
                if let Level::Points(p) = &l.3 {
                    keys.extend(p.iter().map(|x| smp(x.0)));
                }
            }
            // the lanes only matter where the clips play
            let lo = lane.items.iter().map(|i| smp(i.at)).min().unwrap_or(0);
            let hi = lane.items.iter().map(|i| smp(i.at + i.len)).max().unwrap_or(lo + 1).max(lo + 1);
            let vol = simplify(&crate::aaf::fit(&keys, lo, hi, &|t| crate::aaf_import::amp_db(gain(t)), &|a, b| (a - b).abs() > 0.1), 0.02);
            let flat = |p: &[(i64, f64)], tol: f64| p.iter().all(|x| (x.1 - p[0].1).abs() < tol);
            let tr = s.tracks.get_mut(&tid).unwrap();
            if flat(&vol, 0.01) {
                tr.gain_db = vol[0].1 as f32;
            } else {
                for (t, v) in &vol {
                    add_point(
                        s,
                        &tid,
                        AutomationParam::VolumeDb,
                        base + t,
                        crate::editing::clamp_automation(AutomationParam::VolumeDb, *v as f32),
                    );
                }
                automated = true;
            }
            if has_pan {
                let pans = simplify(&crate::aaf::fit(&keys, lo, hi, &|t| pan(t).clamp(-1.0, 1.0), &|a, b| (a - b).abs() > 0.002), 0.0005);
                if flat(&pans, 0.002) {
                    s.tracks.get_mut(&tid).unwrap().pan = pans[0].1 as f32;
                } else {
                    for (t, v) in &pans {
                        add_point(s, &tid, AutomationParam::Pan, base + t, *v as f32);
                    }
                }
            }
        }
        for (slot, id, t, v) in plugin_points {
            add_point(s, &slot, AutomationParam::Plugin(id), base + t, crate::editing::clamp_automation(AutomationParam::Plugin(id), v));
        }
        report.automated_tracks += automated as usize;
        report.stereo_tracks += (channels == 2) as usize;
        report.clips += clips.len();
        for c in clips {
            s.clips.insert(c.id.clone(), c);
        }
        report.tracks.push(tid);
    }
    if !levels_found && !tl["metadata"]["vcsd"].is_object() {
        notes.add(
            "no levels or pan in the OTIO (no gain effects or VC Sound Design metadata): tracks at unity, centred",
            report.tracks.len(),
        );
    }
    report.media_linked = media.cache.values().filter(|r| matches!(r, Ok((_, false)))).count();
    report.media_written = media.cache.values().filter(|r| matches!(r, Ok((_, true)))).count();
    report.missing_media = missing.into_iter().map(|(k, n)| format!("{k} — {n} clip{}", if n == 1 { "" } else { "s" })).collect();

    // ---- markers → spotting markers (inside scenes)
    for (at, dur, label) in markers {
        let a = base + smp(at);
        let len = smp(dur).max(0);
        if s.scene_at(a).is_none() {
            notes.add("markers outside every scene left out (add scenes, then import again)", 1);
            continue;
        }
        let lane = marker_lane(s, "OTIO markers", "otio");
        let nm = crate::spotting::NewMarker {
            lane_id: lane,
            scene_id: None,
            kind: if len > 0 { EventType::Range } else { EventType::Point },
            start: a,
            end: a + len,
            label,
            identity_id: None,
            sub_events: vec![],
            origin: TimingOrigin::Imported,
        };
        if crate::spotting::add_marker(s, nm).is_ok() {
            report.markers += 1;
        }
    }
    report.notes = notes.0.iter().map(|(k, n)| format!("{k} ({n})")).collect();
    if report.tracks.is_empty() && report.markers == 0 {
        let why = if report.missing_media.is_empty() { String::new() } else { format!(": {}", report.missing_media.join("; ")) };
        return Err(invalid(format!("nothing to import from '{name}'{why}")));
    }
    Ok(report)
}

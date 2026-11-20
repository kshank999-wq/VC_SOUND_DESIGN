//! Auto-track builder: populate placeholder audio at cue positions (spec §8.3).

use crate::error::Result;
use crate::ids::{new_id, now_unix};
use crate::library::{default_placeholder, pick_variation};
use crate::model::*;
use crate::state::ProjectState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrackPolicy {
    /// One track per identity per scene (default; avoids feature-length lanes).
    #[default]
    IdentityPerScene,
    /// One track per category per scene.
    CategoryPerScene,
    /// One track per identity for the whole project.
    IdentityGlobal,
}

#[derive(Debug, Clone, Default)]
pub struct PopulateOptions {
    pub policy: TrackPolicy,
    pub scene_id: Option<String>,
    pub cue_ids: Option<Vec<String>>,
    /// Re-place cues that already have clips. Only placeholder clips are
    /// replaced; custom/recorded clips are always kept.
    pub repopulate: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FitWarning {
    pub cue_id: String,
    pub cue_code: String,
    pub message: String,
}

#[derive(Debug, Default, Serialize)]
pub struct PopulateReport {
    pub cues_populated: Vec<String>,
    pub tracks_created: Vec<String>,
    pub clips_created: usize,
    pub warnings: Vec<FitWarning>,
}

/// Default footstep cadence (seconds) used to infer steps in an untapped range.
pub const INFERRED_STEP_SECS: f64 = 0.5;

pub fn bus_for_category(s: &ProjectState, cat: &Category) -> String {
    let name = cat.default_bus();
    s.buses
        .values()
        .find(|b| b.name == name)
        .or_else(|| s.buses.values().find(|b| b.name == "Review Mix"))
        .map(|b| b.id.clone())
        .unwrap_or_default()
}

pub fn track_label(cat: &Category, ident: Option<&SoundIdentity>, fallback: &str) -> String {
    let mut parts = vec![cat.prefix()];
    match ident {
        Some(i) => parts.extend(i.label_parts()),
        None => {
            let f = fallback.trim();
            parts.push(if f.is_empty() { cat.display() } else { f.to_string() });
        }
    }
    parts.join(" • ")
}

/// Find or create the track a cue belongs to under `policy`.
pub fn ensure_track(s: &mut ProjectState, cue: &Cue, policy: TrackPolicy) -> Result<(String, bool)> {
    let ident = cue.identity_id.as_ref().and_then(|i| s.identities.get(i)).cloned();
    let (scene_key, ident_key): (Option<String>, Option<String>) = match policy {
        TrackPolicy::IdentityPerScene => (Some(cue.scene_id.clone()), cue.identity_id.clone()),
        TrackPolicy::CategoryPerScene => (Some(cue.scene_id.clone()), None),
        TrackPolicy::IdentityGlobal => (None, cue.identity_id.clone()),
    };
    // Cues without identity under identity policies group by category+description.
    let fallback = match (&ident_key, policy) {
        (None, TrackPolicy::CategoryPerScene) => cue.category.display(),
        (None, _) => cue.description.clone(),
        _ => String::new(),
    };
    let name = track_label(&cue.category, if ident_key.is_some() { ident.as_ref() } else { None }, &fallback);
    if let Some(t) =
        s.tracks.values().find(|t| t.scene_id == scene_key && t.identity_id == ident_key && t.category == cue.category && t.name == name)
    {
        return Ok((t.id.clone(), false));
    }
    let scene_no = s.scenes.get(&cue.scene_id).map(|x| x.number.clone()).unwrap_or_default();
    let bus_id = ident.as_ref().and_then(|i| i.bus_id.clone()).unwrap_or_else(|| bus_for_category(s, &cue.category));
    let folder = match scene_key {
        Some(_) => format!("Scene {}/{}", scene_no, cue.category.default_bus()),
        None => format!("Global/{}", cue.category.default_bus()),
    };
    let order = s.tracks.values().map(|t| t.order + 1).max().unwrap_or(0);
    let id = new_id("trk");
    s.tracks.insert(
        id.clone(),
        Track {
            id: id.clone(),
            name,
            scene_id: scene_key,
            identity_id: ident_key,
            category: cue.category.clone(),
            bus_id,
            channels: cue.category.default_channels(),
            order,
            folder,
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            armed: false,
            frozen_asset_id: None,
            inserts: vec![],
            automation_mode: AutoMode::Read,
        },
    );
    Ok((id, true))
}

/// Asset for occurrence `index` of a cue: local override > identity variation >
/// identity default > best category placeholder.
pub fn resolve_asset(s: &ProjectState, cue: &Cue, index: usize) -> Option<String> {
    if let Some(a) = &cue.asset_override_id {
        return Some(a.clone());
    }
    if let Some(i) = cue.identity_id.as_ref().and_then(|i| s.identities.get(i)) {
        if let Some(a) = pick_variation(i, index, &cue.id) {
            return Some(a);
        }
        return default_placeholder(s, &cue.category, &format!("{} {}", i.name, cue.description));
    }
    default_placeholder(s, &cue.category, &cue.description)
}

fn warn(r: &mut PopulateReport, cue: &Cue, msg: String) {
    r.warnings.push(FitWarning { cue_id: cue.id.clone(), cue_code: cue.code.clone(), message: msg });
}

fn new_clip(track: &str, cue: &Cue, a: &Asset, start: i64, length: i64, fill: FillMode, sub: Option<u32>) -> ClipInstance {
    let fade = (a.sample_rate as i64 / 200).min(length / 4); // 5 ms safety fades
    ClipInstance {
        id: new_id("clp"),
        track_id: track.to_string(),
        cue_id: Some(cue.id.clone()),
        asset_id: a.id.clone(),
        timeline_start: start,
        source_offset: 0,
        length,
        gain_db: 0.0,
        fade_in: if fill == FillMode::Loop { fade * 4 } else { 0 },
        fade_out: fade,
        fill,
        muted: false,
        take_id: None,
        performance_event_id: None,
        sub_index: sub,
        is_placeholder: true,
        history: vec![AssetHistoryEntry { asset_id: a.id.clone(), origin: "placeholder".into(), at: now_unix() }],
    }
}

/// Populate placeholders for the selected cues. Never touches custom clips.
pub fn populate(s: &mut ProjectState, opts: &PopulateOptions) -> Result<PopulateReport> {
    let mut rep = PopulateReport::default();
    let sr = s.project.sample_rate as i64;
    let cue_ids: Vec<String> = s
        .sorted_cues(opts.scene_id.as_deref())
        .into_iter()
        .filter(|c| opts.cue_ids.as_ref().is_none_or(|ids| ids.contains(&c.id)))
        .filter(|c| !matches!(c.status, CueStatus::Conceptual | CueStatus::Omitted))
        .map(|c| c.id.clone())
        .collect();
    for cid in cue_ids {
        let existing: Vec<(String, bool)> = s.clips_for_cue(&cid).iter().map(|c| (c.id.clone(), c.is_placeholder)).collect();
        if !existing.is_empty() {
            if !opts.repopulate {
                continue;
            }
            if existing.iter().any(|(_, p)| !p) {
                let cue = s.cue(&cid)?.clone();
                warn(&mut rep, &cue, "has custom/recorded clips; kept as-is (not repopulated)".into());
                continue;
            }
            for (id, _) in existing {
                s.clips.remove(&id);
            }
        }
        let mut cue = s.cue(&cid)?.clone();
        let (track, created) = ensure_track(s, &cue, opts.policy)?;
        if created {
            rep.tracks_created.push(track.clone());
        }
        // Footstep ranges and untapped repeated events get inferred steps.
        let mut events = cue.sub_events.clone();
        let repeated = cue.event_type == EventType::Repeated || (cue.event_type == EventType::Range && cue.category == Category::Footsteps);
        if repeated && events.is_empty() {
            let step = (INFERRED_STEP_SECS * sr as f64) as i64;
            let mut t = cue.start_sample;
            while t < cue.end_sample.max(cue.start_sample + 1) {
                events.push(t);
                t += step;
            }
            warn(
                &mut rep,
                &cue,
                format!("{} steps inferred at {INFERRED_STEP_SECS}s cadence; tap or run timing assist to refine", events.len()),
            );
            let c = s.cue_mut(&cid)?;
            c.sub_events = events.clone();
            if c.timing_origin == TimingOrigin::Manual {
                c.timing_origin = TimingOrigin::Inferred;
            }
            cue = c.clone();
        }
        let mut clips = vec![];
        if repeated {
            for (i, &t) in events.iter().enumerate() {
                let Some(aid) = resolve_asset(s, &cue, i) else { continue };
                let a = s.asset(&aid)?;
                clips.push(new_clip(&track, &cue, a, t, a.length_samples.max(1), FillMode::OneShot, Some(i as u32)));
            }
        } else if let Some(aid) = resolve_asset(s, &cue, 0) {
            let a = s.asset(&aid)?.clone();
            let range = cue.end_sample - cue.start_sample;
            if cue.event_type == EventType::Point || range <= 0 {
                clips.push(new_clip(&track, &cue, &a, cue.start_sample, a.length_samples.max(1), FillMode::OneShot, None));
            } else if a.loopable || cue.category.loops_naturally() {
                clips.push(new_clip(&track, &cue, &a, cue.start_sample, range, FillMode::Loop, None));
                if a.length_samples < sr {
                    warn(&mut rep, &cue, format!("loop source '{}' is under 1s; may sound repetitive", a.name));
                }
            } else if a.length_samples >= range {
                clips.push(new_clip(&track, &cue, &a, cue.start_sample, range, FillMode::OneShot, None));
                if a.length_samples > range * 2 {
                    warn(
                        &mut rep,
                        &cue,
                        format!(
                            "'{}' trimmed from {:.2}s to range {:.2}s",
                            a.name,
                            a.length_samples as f64 / sr as f64,
                            range as f64 / sr as f64
                        ),
                    );
                }
            } else {
                clips.push(new_clip(&track, &cue, &a, cue.start_sample, a.length_samples, FillMode::OneShot, None));
                if (a.length_samples as f64) < range as f64 * 0.75 {
                    warn(
                        &mut rep,
                        &cue,
                        format!(
                            "'{}' ({:.2}s) is shorter than range ({:.2}s); remainder is silence — poor fit",
                            a.name,
                            a.length_samples as f64 / sr as f64,
                            range as f64 / sr as f64
                        ),
                    );
                }
            }
        }
        let c = s.cue_mut(&cid)?;
        if clips.is_empty() {
            c.status = CueStatus::Missing;
            warn(&mut rep, &cue, "no placeholder asset available for this cue (assign an identity sound or import a pack)".into());
            continue;
        }
        c.placeholder_asset_id = Some(clips[0].asset_id.clone());
        if matches!(c.status, CueStatus::Spotted | CueStatus::Missing) {
            c.status = CueStatus::Placeholder;
        }
        rep.clips_created += clips.len();
        for cl in clips {
            s.clips.insert(cl.id.clone(), cl);
        }
        rep.cues_populated.push(cid);
    }
    Ok(rep)
}

/// Ghost preview: what `populate` would do, without changing the project.
pub fn preview(s: &ProjectState, opts: &PopulateOptions) -> Result<PopulateReport> {
    let mut c = s.clone();
    populate(&mut c, opts)
}

//! Non-destructive editing: asset replacement with history, revert to
//! placeholder, clip edits and recorded takes (spec §8.3, §9, §11.2, §12).

use crate::error::{invalid, not_found, Error, Result};
use crate::ids::{new_id, now_unix};
use crate::library::{import_wav, AssetMeta};
use crate::model::*;
use crate::state::ProjectState;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceScope {
    /// Only this cue (scene-local override; the identity is unchanged).
    Cue,
    /// Every cue of the same identity in the same scene.
    IdentityInScene,
    /// Change the identity's default everywhere (ripple). Cues with local
    /// overrides stay isolated. Requires confirmation.
    IdentityEverywhere,
}

fn set_clip_asset(s: &mut ProjectState, clip_id: &str, asset_id: &str, origin: &str, keep_length: bool) -> Result<()> {
    let a = s.asset(asset_id)?.clone();
    let c = s.clip_mut(clip_id)?;
    if c.asset_id == asset_id {
        return Ok(());
    }
    c.asset_id = a.id.clone();
    c.source_offset = 0;
    if !keep_length {
        c.length = a.length_samples.max(1);
    }
    // A deliberate user choice is custom work even if the sound comes from a
    // placeholder pack; only a revert restores placeholder status.
    c.is_placeholder = origin.starts_with("revert:placeholder");
    c.history.push(AssetHistoryEntry { asset_id: a.id, origin: origin.into(), at: now_unix() });
    Ok(())
}

fn apply_to_cue_clips(s: &mut ProjectState, cue_id: &str, asset_id: &str, origin: &str) -> Result<()> {
    let clips: Vec<(String, bool)> = s.clips_for_cue(cue_id).iter().map(|c| (c.id.clone(), c.fill == FillMode::Loop)).collect();
    for (cid, is_loop) in clips {
        set_clip_asset(s, &cid, asset_id, origin, is_loop)?;
    }
    s.cue_mut(cue_id)?.status = CueStatus::Replaced;
    Ok(())
}

/// Replace the sound of a cue, keeping its timing and identity. Returns the
/// IDs of every cue that changed.
pub fn replace_cue_asset(
    s: &mut ProjectState,
    cue_id: &str,
    asset_id: &str,
    scope: ReplaceScope,
    confirm_ripple: bool,
) -> Result<Vec<String>> {
    s.asset(asset_id)?;
    let cue = s.cue(cue_id)?.clone();
    let origin = format!("replace:{scope:?}");
    let targets: Vec<String> = match scope {
        ReplaceScope::Cue => vec![cue.id.clone()],
        ReplaceScope::IdentityInScene | ReplaceScope::IdentityEverywhere => {
            let iid = cue.identity_id.clone().ok_or_else(|| invalid("cue has no sound identity"))?;
            s.cues
                .values()
                .filter(|c| c.identity_id.as_deref() == Some(iid.as_str()))
                .filter(|c| scope != ReplaceScope::IdentityInScene || c.scene_id == cue.scene_id)
                .filter(|c| scope != ReplaceScope::IdentityEverywhere || c.asset_override_id.is_none() || c.id == cue.id)
                .map(|c| c.id.clone())
                .collect()
        }
    };
    if scope == ReplaceScope::IdentityEverywhere {
        if !confirm_ripple {
            return Err(Error::Conflict(format!("ripple would change {} cues across the film; confirm to proceed", targets.len())));
        }
        let iid = cue.identity_id.clone().unwrap();
        let i = s.identities.get_mut(&iid).ok_or_else(|| not_found("identity", &iid))?;
        i.default_asset_id = Some(asset_id.into());
        i.variation_mode = VariationMode::Locked;
    }
    for t in &targets {
        if scope != ReplaceScope::IdentityEverywhere {
            s.cue_mut(t)?.asset_override_id = Some(asset_id.into());
        }
        apply_to_cue_clips(s, t, asset_id, &origin)?;
    }
    Ok(targets)
}

/// Replace a single instance (one clip of a repeated cue, for example).
pub fn replace_clip_asset(s: &mut ProjectState, clip_id: &str, asset_id: &str) -> Result<()> {
    let keep = s.clips.get(clip_id).ok_or_else(|| not_found("clip", clip_id))?.fill == FillMode::Loop;
    set_clip_asset(s, clip_id, asset_id, "replace:clip", keep)?;
    if let Some(cid) = s.clips[clip_id].cue_id.clone() {
        let c = s.cue_mut(&cid)?;
        if c.status == CueStatus::Placeholder {
            c.status = CueStatus::Replaced;
        }
    }
    Ok(())
}

/// Revert a cue's clips to their original placeholder. Takes and the full
/// replacement history are retained.
pub fn revert_to_placeholder(s: &mut ProjectState, cue_id: &str) -> Result<()> {
    let clips: Vec<(String, Option<String>, bool)> = s
        .clips_for_cue(cue_id)
        .iter()
        .map(|c| (c.id.clone(), c.history.iter().find(|h| h.origin == "placeholder").map(|h| h.asset_id.clone()), c.fill == FillMode::Loop))
        .collect();
    for (cid, ph, keep) in clips {
        if let Some(ph) = ph {
            set_clip_asset(s, &cid, &ph, "revert:placeholder", keep)?;
            let c = s.clip_mut(&cid)?;
            c.take_id = None;
            c.muted = false;
        }
    }
    let c = s.cue_mut(cue_id)?;
    c.asset_override_id = None;
    c.status = CueStatus::Placeholder;
    Ok(())
}

// ---------------------------------------------------------------- clip edits

pub fn move_clip(s: &mut ProjectState, clip_id: &str, new_start: i64) -> Result<()> {
    s.clip_mut(clip_id)?.timeline_start = new_start;
    Ok(())
}

/// Fades never exceed the clip: fade-in first, fade-out gets the remainder.
fn clamp_fades(c: &mut ClipInstance) {
    c.fade_in = c.fade_in.clamp(0, c.length);
    c.fade_out = c.fade_out.clamp(0, c.length - c.fade_in);
}

/// Source length of a clip's asset in project samples.
pub fn source_len(s: &ProjectState, c: &ClipInstance) -> Result<i64> {
    let a = s.asset(&c.asset_id)?;
    Ok((a.length_samples as i128 * s.project.sample_rate as i128 / a.sample_rate.max(1) as i128) as i64)
}

/// Trim the head: the clip starts later/earlier while audio stays in place.
pub fn trim_start(s: &mut ProjectState, clip_id: &str, new_start: i64) -> Result<()> {
    let c = s.clip_mut(clip_id)?;
    let delta = new_start - c.timeline_start;
    if delta >= c.length || c.source_offset + delta < 0 {
        return Err(invalid("trim exceeds clip/source bounds"));
    }
    c.timeline_start = new_start;
    c.source_offset += delta;
    c.length -= delta;
    clamp_fades(c);
    Ok(())
}

/// Trim the tail. One-shot clips cannot extend past the end of their
/// source audio; looped clips can be any length.
pub fn trim_end(s: &mut ProjectState, clip_id: &str, new_length: i64) -> Result<()> {
    if new_length <= 0 {
        return Err(invalid("clip length must be positive"));
    }
    let c = s.clips.get(clip_id).ok_or_else(|| not_found("clip", clip_id))?.clone();
    if c.fill == FillMode::OneShot && new_length > c.length {
        let avail = source_len(s, &c)? - c.source_offset;
        if new_length > avail {
            return Err(invalid(format!("only {avail} samples of source audio remain after the clip start")));
        }
    }
    let c = s.clip_mut(clip_id)?;
    c.length = new_length;
    clamp_fades(c);
    Ok(())
}

/// Slip source audio under a fixed clip window.
pub fn slip(s: &mut ProjectState, clip_id: &str, delta: i64) -> Result<()> {
    let c = s.clip_mut(clip_id)?;
    if c.source_offset + delta < 0 {
        return Err(invalid("cannot slip before source start"));
    }
    c.source_offset += delta;
    Ok(())
}

/// Split a clip at timeline sample `at`; returns the new right-hand clip ID.
pub fn split(s: &mut ProjectState, clip_id: &str, at: i64) -> Result<String> {
    let c = s.clips.get(clip_id).ok_or_else(|| not_found("clip", clip_id))?.clone();
    if at <= c.timeline_start || at >= c.end() {
        return Err(invalid("split point must be inside the clip"));
    }
    let left_len = at - c.timeline_start;
    let mut right = c.clone();
    right.id = new_id("clp");
    right.timeline_start = at;
    right.source_offset = c.source_offset + left_len;
    right.length = c.length - left_len;
    right.fade_in = 0;
    let l = s.clip_mut(clip_id)?;
    l.length = left_len;
    l.fade_out = 0;
    let rid = right.id.clone();
    s.clips.insert(rid.clone(), right);
    Ok(rid)
}

pub fn set_fades(s: &mut ProjectState, clip_id: &str, fade_in: i64, fade_out: i64) -> Result<()> {
    let c = s.clip_mut(clip_id)?;
    if fade_in < 0 || fade_out < 0 || fade_in + fade_out > c.length {
        return Err(invalid("fades must fit within the clip"));
    }
    c.fade_in = fade_in;
    c.fade_out = fade_out;
    Ok(())
}

pub fn set_clip_gain(s: &mut ProjectState, clip_id: &str, db: f32) -> Result<()> {
    s.clip_mut(clip_id)?.gain_db = db;
    Ok(())
}

/// Crossfade two overlapping clips on the same track across their overlap.
pub fn crossfade(s: &mut ProjectState, a: &str, b: &str) -> Result<i64> {
    let ca = s.clips.get(a).ok_or_else(|| not_found("clip", a))?.clone();
    let cb = s.clips.get(b).ok_or_else(|| not_found("clip", b))?.clone();
    let (first, second) = if ca.timeline_start <= cb.timeline_start { (ca, cb) } else { (cb, ca) };
    if first.track_id != second.track_id {
        return Err(invalid("crossfade requires clips on the same track"));
    }
    let overlap = first.end() - second.timeline_start;
    if overlap <= 0 {
        return Err(invalid("clips do not overlap"));
    }
    s.clip_mut(&first.id)?.fade_out = overlap.min(first.length);
    s.clip_mut(&second.id)?.fade_in = overlap.min(second.length);
    Ok(overlap)
}

pub fn delete_clip(s: &mut ProjectState, clip_id: &str) -> Result<()> {
    s.clips.remove(clip_id).map(|_| ()).ok_or_else(|| not_found("clip", clip_id))
}

/// Legal range for an automation value: volume is dB relative to the track
/// fader (-96 dB is treated as silence), pan is -1 (L) .. +1 (R), plugin
/// parameters are normalised 0..1.
pub fn clamp_automation(param: AutomationParam, v: f32) -> f32 {
    let v = if v.is_finite() { v } else { 0.0 };
    match param {
        AutomationParam::VolumeDb => v.clamp(-96.0, 12.0),
        AutomationParam::Pan => v.clamp(-1.0, 1.0),
        AutomationParam::Plugin(_) => v.clamp(0.0, 1.0),
    }
}

/// The plugin slot with this id, and the track or bus it sits on.
pub fn plugin_slot<'a>(s: &'a ProjectState, slot_id: &str) -> Option<(&'a PluginInsert, &'a str)> {
    let find = |ins: &'a [Insert], owner: &'a str| ins.iter().filter_map(Insert::as_plugin).find(|p| p.id == slot_id).map(|p| (p, owner));
    s.tracks.values().find_map(|t| find(&t.inserts, &t.id)).or_else(|| s.buses.values().find_map(|b| find(&b.inserts, &b.id)))
}

fn check_owner(s: &ProjectState, owner_id: &str) -> Result<()> {
    if s.tracks.contains_key(owner_id) || s.buses.contains_key(owner_id) || plugin_slot(s, owner_id).is_some() {
        Ok(())
    } else {
        Err(not_found("track, bus or plugin", owner_id))
    }
}

/// Add a breakpoint on a track or bus (or update the one already at exactly `sample`).
pub fn set_automation(s: &mut ProjectState, owner_id: &str, param: AutomationParam, sample: i64, value: f32) -> Result<String> {
    check_owner(s, owner_id)?;
    let value = clamp_automation(param, value);
    if let Some(p) = s.automation.values_mut().find(|p| p.owner_id == owner_id && p.param == param && p.sample == sample) {
        p.value = value;
        return Ok(p.id.clone());
    }
    let id = new_id("aut");
    s.automation.insert(id.clone(), AutomationPoint { id: id.clone(), owner_id: owner_id.into(), param, sample, value });
    Ok(id)
}

/// Move a breakpoint in time and/or value. If it lands exactly on another
/// breakpoint of the same lane, that one is replaced (no duplicates).
pub fn move_automation(s: &mut ProjectState, id: &str, sample: i64, value: f32) -> Result<()> {
    let p = s.automation.get(id).ok_or_else(|| not_found("automation point", id))?.clone();
    s.automation.retain(|k, q| k == id || !(q.owner_id == p.owner_id && q.param == p.param && q.sample == sample));
    let q = s.automation.get_mut(id).unwrap();
    q.sample = sample;
    q.value = clamp_automation(p.param, value);
    Ok(())
}

pub fn delete_automation(s: &mut ProjectState, id: &str) -> Result<()> {
    s.automation.remove(id).map(|_| ()).ok_or_else(|| not_found("automation point", id))
}

/// Draw a lane with the pencil: the breakpoints between the stroke's first and last sample
/// are replaced by the stroke's points (sorted; values clamped). Returns how many points the
/// lane holds in that span afterwards.
pub fn draw_automation(s: &mut ProjectState, owner_id: &str, param: AutomationParam, points: &[(i64, f32)]) -> Result<usize> {
    check_owner(s, owner_id)?;
    if points.is_empty() {
        return Ok(0);
    }
    let mut pts = points.to_vec();
    pts.sort_by_key(|p| p.0);
    pts.dedup_by_key(|p| p.0);
    let (a, b) = (pts[0].0, pts[pts.len() - 1].0);
    s.automation.retain(|_, p| !(p.owner_id == owner_id && p.param == param && p.sample >= a && p.sample <= b));
    for (sample, value) in &pts {
        let id = new_id("aut");
        s.automation.insert(
            id.clone(),
            AutomationPoint { id, owner_id: owner_id.into(), param, sample: *sample, value: clamp_automation(param, *value) },
        );
    }
    Ok(pts.len())
}

/// Remove all breakpoints of a track or bus (one lane, or both when `param` is None).
pub fn clear_automation(s: &mut ProjectState, owner_id: &str, param: Option<AutomationParam>) -> Result<usize> {
    check_owner(s, owner_id)?;
    let before = s.automation.len();
    s.automation.retain(|_, p| !(p.owner_id == owner_id && param.is_none_or(|x| p.param == x)));
    Ok(before - s.automation.len())
}

/// Value a lane has at `at` (linear between breakpoints, held at the ends),
/// or the neutral value when the lane is empty.
pub fn automation_value_at(s: &ProjectState, owner_id: &str, param: AutomationParam, at: i64) -> f32 {
    let mut pts: Vec<&AutomationPoint> = s.automation.values().filter(|p| p.owner_id == owner_id && p.param == param).collect();
    pts.sort_by_key(|p| p.sample);
    let neutral = match param {
        AutomationParam::Pan => s.tracks.get(owner_id).map(|t| t.pan).unwrap_or(0.0),
        AutomationParam::VolumeDb => 0.0,
        // the value set in the app, if any (otherwise the plugin's own state decides)
        AutomationParam::Plugin(id) => plugin_slot(s, owner_id).and_then(|(p, _)| p.params.get(&id)).map(|v| *v as f32).unwrap_or(0.5),
    };
    match pts.as_slice() {
        [] => neutral,
        [first, ..] if at <= first.sample => first.value,
        [.., last] if at >= last.sample => last.value,
        _ => {
            let i = pts.windows(2).position(|w| at >= w[0].sample && at < w[1].sample).unwrap_or(0);
            let (a, b) = (pts[i], pts[i + 1]);
            a.value + (b.value - a.value) * ((at - a.sample) as f32 / (b.sample - a.sample).max(1) as f32)
        }
    }
}

/// Douglas–Peucker thinning in value space: drop points that a straight line
/// between their neighbours reproduces within `tol`.
fn thin(points: &[(i64, f32)], tol: f32) -> Vec<(i64, f32)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0usize, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (sa, va) = points[a];
        let (sb, vb) = points[b];
        let mut worst = (0.0f32, 0usize);
        for (k, &(sk, vk)) in points.iter().enumerate().take(b).skip(a + 1) {
            let t = (sk - sa) as f32 / (sb - sa).max(1) as f32;
            let err = (vk - (va + (vb - va) * t)).abs();
            if err > worst.0 {
                worst = (err, k);
            }
        }
        if worst.0 > tol {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Commit one recorded automation pass (fader/pan moves captured during
/// playback in Touch/Latch/Write). `samples` are `(timeline sample, value)` in
/// capture order. The lane is replaced over `[start, end]`; the value before
/// the pass is kept up to `start - ramp`, the recording is thinned, the last
/// value holds to `end`, and the lane ramps back to its previous automation by
/// `end + ramp`. Returns the number of breakpoints written.
pub fn write_automation_pass(
    s: &mut ProjectState,
    owner_id: &str,
    param: AutomationParam,
    samples: &[(i64, f32)],
    start: i64,
    end: i64,
    ramp: i64,
) -> Result<usize> {
    check_owner(s, owner_id)?;
    if samples.is_empty() || end < start {
        return Ok(0);
    }
    let ramp = ramp.max(0);
    let before = automation_value_at(s, owner_id, param, start - ramp);
    let after = automation_value_at(s, owner_id, param, end + ramp);
    let had_points = s.automation.values().any(|p| p.owner_id == owner_id && p.param == param);
    s.automation.retain(|_, p| !(p.owner_id == owner_id && p.param == param && p.sample >= start - ramp && p.sample <= end + ramp));
    // capture → monotonic, deduplicated, clamped, within the pass
    let mut rec: Vec<(i64, f32)> = vec![];
    for &(t, v) in samples {
        let t = t.clamp(start, end);
        let v = clamp_automation(param, v);
        match rec.last_mut() {
            Some(l) if l.0 >= t => l.1 = v,
            _ => rec.push((t, v)),
        }
    }
    if rec[0].0 > start {
        rec.insert(0, (start, rec[0].1));
    }
    let last = rec[rec.len() - 1].1;
    if rec[rec.len() - 1].0 < end {
        rec.push((end, last));
    }
    let tol = match param {
        AutomationParam::Pan => 0.01,
        AutomationParam::Plugin(_) => 0.002,
        AutomationParam::VolumeDb => 0.25,
    };
    let mut out = thin(&rec, tol);
    // a plugin lane's first pass has nothing to return to: the lane simply holds
    // its first and last written values (the plugin's static value is unknown here)
    let new_plugin_lane = matches!(param, AutomationParam::Plugin(_)) && !had_points;
    if ramp > 0 && !new_plugin_lane {
        if had_points || (before - out[0].1).abs() > tol {
            out.insert(0, (start - ramp, before));
        }
        out.push((end + ramp, after));
    }
    let n = out.len();
    for (t, v) in out {
        let id = new_id("aut");
        s.automation.insert(id.clone(), AutomationPoint { id, owner_id: owner_id.into(), param, sample: t, value: v });
    }
    Ok(n)
}

/// Remove the automation of plugin slots that no longer exist (after a slot is removed).
pub fn drop_orphan_plugin_automation(s: &mut ProjectState) -> usize {
    let live: std::collections::HashSet<String> = s
        .tracks
        .values()
        .flat_map(|t| &t.inserts)
        .chain(s.buses.values().flat_map(|b| &b.inserts))
        .filter_map(Insert::as_plugin)
        .map(|p| p.id.clone())
        .collect();
    let before = s.automation.len();
    s.automation.retain(|_, p| !matches!(p.param, AutomationParam::Plugin(_)) || live.contains(&p.owner_id));
    before - s.automation.len()
}

/// Set the automation mode of a track or bus.
pub fn set_automation_mode(s: &mut ProjectState, owner_id: &str, mode: AutoMode) -> Result<()> {
    if let Some(t) = s.tracks.get_mut(owner_id) {
        t.automation_mode = mode;
        return Ok(());
    }
    if let Some(b) = s.buses.get_mut(owner_id) {
        b.automation_mode = mode;
        return Ok(());
    }
    Err(not_found("track or bus", owner_id))
}

// ---------------------------------------------------------------- track folders
//
// Folders are `/`-separated paths on `Track::folder` (e.g. "Scene 12/Foley"),
// so they travel with the track into manifests and handoff packages.

/// Normalise a folder path: trim segments, drop empties, join with "/".
pub fn normalize_folder(path: &str) -> String {
    path.split('/').map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/")
}

/// True when `folder` is `path` or inside it.
pub fn in_folder(folder: &str, path: &str) -> bool {
    !path.is_empty() && (folder == path || folder.starts_with(&format!("{path}/")))
}

/// Move tracks into `folder` ("" = top level). Returns how many changed.
pub fn move_tracks_to_folder(s: &mut ProjectState, track_ids: &[String], folder: &str) -> Result<usize> {
    let f = normalize_folder(folder);
    for id in track_ids {
        s.track(id)?;
    }
    let mut n = 0;
    for id in track_ids {
        let t = s.tracks.get_mut(id).unwrap();
        if t.folder != f {
            t.folder = f.clone();
            n += 1;
        }
    }
    Ok(n)
}

/// Rename/move a folder and everything under it ("Scene 12/Foley" → "Reel 1/Scene 12/Foley").
pub fn rename_folder(s: &mut ProjectState, from: &str, to: &str) -> Result<usize> {
    let (from, to) = (normalize_folder(from), normalize_folder(to));
    if from.is_empty() {
        return Err(invalid("cannot rename the top level"));
    }
    if in_folder(&to, &from) && to != from {
        return Err(invalid("cannot move a folder inside itself"));
    }
    let mut n = 0;
    for t in s.tracks.values_mut().filter(|t| in_folder(&t.folder, &from)) {
        let rest = &t.folder[from.len()..];
        t.folder = normalize_folder(&format!("{to}{rest}"));
        n += 1;
    }
    if n == 0 {
        return Err(not_found("folder", &from));
    }
    Ok(n)
}

/// Remove a folder level: its tracks (and subfolders) move up to the parent.
pub fn ungroup_folder(s: &mut ProjectState, path: &str) -> Result<usize> {
    let path = normalize_folder(path);
    let parent = path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
    rename_folder(s, &path, &parent)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderFlag {
    Mute,
    Solo,
}

/// Set mute or solo on every track in a folder (and its subfolders).
pub fn set_folder_flag(s: &mut ProjectState, path: &str, flag: FolderFlag, on: bool) -> Result<usize> {
    let path = normalize_folder(path);
    let mut n = 0;
    for t in s.tracks.values_mut().filter(|t| in_folder(&t.folder, &path)) {
        match flag {
            FolderFlag::Mute => t.mute = on,
            FolderFlag::Solo => t.solo = on,
        }
        n += 1;
    }
    if n == 0 {
        return Err(not_found("folder", &path));
    }
    Ok(n)
}

// ---------------------------------------------------------------- takes

pub struct TakeImport<'a> {
    pub wav: &'a Path,
    pub cue_id: Option<String>,
    pub track_id: Option<String>,
    pub slate: String,
    /// Explicit timeline position; when `None` the BWF time reference is used,
    /// falling back to the cue start.
    pub timeline_start: Option<i64>,
    /// Measured round-trip latency / sync offset in samples (subtracted).
    pub sync_offset: i64,
}

/// Register a recorded take (non-destructive: the WAV is referenced, never modified).
pub fn add_take(s: &mut ProjectState, t: TakeImport) -> Result<String> {
    let aid = import_wav(
        s,
        t.wav,
        AssetMeta {
            kind: AssetKind::Recorded,
            license: Some(License {
                name: "Original recording".into(),
                redistributable: true,
                source: "production".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let asset = s.asset(&aid)?.clone();
    if asset.sample_rate != s.project.sample_rate {
        return Err(invalid(format!("take is {} Hz but project is {} Hz", asset.sample_rate, s.project.sample_rate)));
    }
    let cue = t.cue_id.as_ref().map(|c| s.cue(c).cloned()).transpose()?;
    let track_id = match (&t.track_id, &cue) {
        (Some(tr), _) => s.track(tr)?.id.clone(),
        (None, Some(c)) => match s.clips_for_cue(&c.id).first() {
            Some(cl) => cl.track_id.clone(),
            None => crate::builder::ensure_track(s, c, crate::builder::TrackPolicy::IdentityPerScene)?.0,
        },
        (None, None) => return Err(invalid("a take needs a cue or a track")),
    };
    let origin = s.tb().origin_since_midnight();
    let base = match (t.timeline_start, asset.time_reference, &cue) {
        (Some(p), _, _) => p,
        (None, Some(tr), _) => tr as i64 - origin,
        (None, None, Some(c)) => c.start_sample,
        (None, None, None) => 0,
    };
    let number = s.takes.values().filter(|k| k.cue_id == t.cue_id && k.track_id == track_id).map(|k| k.number).max().unwrap_or(0) + 1;
    let id = new_id("tak");
    s.takes.insert(
        id.clone(),
        Take {
            id: id.clone(),
            cue_id: t.cue_id,
            track_id,
            asset_id: aid,
            number,
            slate: t.slate,
            timeline_start: base - t.sync_offset,
            sync_offset: t.sync_offset,
            notes: String::new(),
            recorded_at: now_unix(),
        },
    );
    Ok(id)
}

/// Put a take on one existing instance (clip). The previous asset stays in history.
pub fn use_take_for_clip(s: &mut ProjectState, clip_id: &str, take_id: &str) -> Result<()> {
    let take = s.takes.get(take_id).ok_or_else(|| not_found("take", take_id))?.clone();
    set_clip_asset(s, clip_id, &take.asset_id, &format!("take:{}", take.number), false)?;
    let c = s.clip_mut(clip_id)?;
    c.take_id = Some(take.id.clone());
    if let Some(cid) = c.cue_id.clone() {
        let cue = s.cue_mut(&cid)?;
        cue.status = CueStatus::Recorded;
        cue.timing_origin = TimingOrigin::Recorded;
    }
    Ok(())
}

/// Use a take for a whole cue: existing instances are muted (kept for
/// comparison/revert) and the take is placed at its recorded position.
pub fn use_take_for_cue(s: &mut ProjectState, cue_id: &str, take_id: &str) -> Result<String> {
    let take = s.takes.get(take_id).ok_or_else(|| not_found("take", take_id))?.clone();
    let asset = s.asset(&take.asset_id)?.clone();
    for c in s.clips.values_mut().filter(|c| c.cue_id.as_deref() == Some(cue_id)) {
        c.muted = true;
    }
    let id = new_id("clp");
    s.clips.insert(
        id.clone(),
        ClipInstance {
            id: id.clone(),
            track_id: take.track_id.clone(),
            cue_id: Some(cue_id.into()),
            asset_id: asset.id.clone(),
            timeline_start: take.timeline_start,
            source_offset: 0,
            length: asset.length_samples,
            gain_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            fill: FillMode::OneShot,
            muted: false,
            take_id: Some(take.id.clone()),
            performance_event_id: None,
            sub_index: None,
            is_placeholder: false,
            history: vec![AssetHistoryEntry { asset_id: asset.id.clone(), origin: format!("take:{}", take.number), at: now_unix() }],
        },
    );
    let cue = s.cue_mut(cue_id)?;
    cue.status = CueStatus::Recorded;
    Ok(id)
}

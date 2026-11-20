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
    Ok(())
}

pub fn trim_end(s: &mut ProjectState, clip_id: &str, new_length: i64) -> Result<()> {
    if new_length <= 0 {
        return Err(invalid("clip length must be positive"));
    }
    s.clip_mut(clip_id)?.length = new_length;
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

pub fn set_automation(s: &mut ProjectState, track_id: &str, param: AutomationParam, sample: i64, value: f32) -> Result<String> {
    s.track(track_id)?;
    if let Some(p) = s.automation.values_mut().find(|p| p.track_id == track_id && p.param == param && p.sample == sample) {
        p.value = value;
        return Ok(p.id.clone());
    }
    let id = new_id("aut");
    s.automation.insert(id.clone(), AutomationPoint { id: id.clone(), track_id: track_id.into(), param, sample, value });
    Ok(id)
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

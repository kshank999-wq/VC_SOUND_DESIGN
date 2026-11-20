//! Picture reference import, timecode probing and picture-change reconform (spec §13).

use crate::error::{invalid, Result};
use crate::ids::{new_id, now_unix};
use crate::model::*;
use crate::state::ProjectState;
use crate::timecode::FrameRate;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct ProbeInfo {
    pub frame_rate: Option<FrameRate>,
    pub start_tc: Option<String>,
    pub duration_secs: Option<f64>,
    pub has_audio: bool,
}

/// Probe a media file with `ffprobe` when it is installed. Absence of ffprobe
/// is not an error: the user supplies the start TC instead.
pub fn probe(path: &Path) -> Option<ProbeInfo> {
    let out =
        Command::new("ffprobe").args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]).arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut info = ProbeInfo::default();
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    for st in &streams {
        match st["codec_type"].as_str() {
            Some("video") if info.frame_rate.is_none() => {
                if let Some(r) = st["r_frame_rate"].as_str() {
                    if let Some((n, d)) = r.split_once('/') {
                        if let (Ok(n), Ok(d)) = (n.parse::<u32>(), d.parse::<u32>()) {
                            if d > 0 {
                                // normalise e.g. 24/1, 24000/1001
                                info.frame_rate = FrameRate::new(n, d, false).ok();
                            }
                        }
                    }
                }
                if let Some(tc) = st["tags"]["timecode"].as_str() {
                    info.start_tc = Some(tc.to_string());
                }
            }
            Some("audio") => info.has_audio = true,
            Some("data") if info.start_tc.is_none() => {
                if let Some(tc) = st["tags"]["timecode"].as_str() {
                    info.start_tc = Some(tc.to_string());
                }
            }
            _ => {}
        }
    }
    if info.start_tc.is_none() {
        info.start_tc = v["format"]["tags"]["timecode"].as_str().map(|s| s.to_string());
    }
    info.duration_secs = v["format"]["duration"].as_str().and_then(|d| d.parse().ok());
    Some(info)
}

pub struct PictureImport<'a> {
    pub path: &'a Path,
    pub label: String,
    /// User-supplied start TC; overrides probed TC when given.
    pub start_tc: Option<String>,
    /// Known duration in frames when probing is unavailable.
    pub duration_frames: Option<i64>,
    pub change_notes: String,
}

/// Import a picture version. The original media is never modified. A new
/// version links to the previous one so history is preserved.
pub fn import_picture(s: &mut ProjectState, imp: PictureImport) -> Result<String> {
    if !imp.path.exists() {
        return Err(crate::error::not_found("picture file", &imp.path.display().to_string()));
    }
    let rate = s.project.frame_rate;
    let probed = probe(imp.path).unwrap_or_default();
    if let Some(pr) = probed.frame_rate {
        if (pr.as_f64() - rate.as_f64()).abs() > 0.001 {
            return Err(invalid(format!(
                "picture frame rate {} differs from project {}; transcode or create the project at the picture rate",
                pr.label(),
                rate.label()
            )));
        }
    }
    let start_tc = imp.start_tc.clone().or(probed.start_tc.clone()).unwrap_or_else(|| rate.frames_to_tc(s.project.start_frame));
    // probed DF/NDF follows separator in TC
    let source_start_frame = rate.tc_to_frames(&start_tc)?;
    let duration_frames = imp.duration_frames.or(probed.duration_secs.map(|d| (d * rate.as_f64()).round() as i64)).unwrap_or(0);
    let id = new_id("pic");
    let previous_id = s.project.current_picture_id.clone();
    s.pictures.insert(
        id.clone(),
        PictureVersion {
            id: id.clone(),
            label: imp.label,
            media_path: std::fs::canonicalize(imp.path)?.to_string_lossy().to_string(),
            fingerprint: crate::hash::fingerprint_file(imp.path)?,
            source_start_frame,
            duration_frames,
            media_frame_rate: probed.frame_rate,
            has_reference_audio: probed.has_audio,
            previous_id,
            change_notes: imp.change_notes,
            imported_at: now_unix(),
        },
    );
    s.project.current_picture_id = Some(id.clone());
    Ok(id)
}

/// Timeline sample range covered by a picture version.
pub fn picture_range(s: &ProjectState, pic: &PictureVersion) -> (i64, i64) {
    let tb = s.tb();
    let a = tb.sample_at_frame(pic.source_start_frame);
    let b = tb.sample_at_frame(pic.source_start_frame + pic.duration_frames);
    (a, b)
}

/// One editorial change: picture frames `[old_start, old_end)` of the old cut
/// now begin at `new_start` (absolute frames). Frames not covered by any
/// change are treated as removed.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct EditChange {
    pub old_start: i64,
    pub old_end: i64,
    pub new_start: i64,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ReconformReport {
    pub shifted: Vec<(String, i64)>,
    pub removed: Vec<String>,
    pub unchanged: usize,
}

/// Compare cues against an editorial change list. Cues are NEVER moved here:
/// each affected cue gets a `pending_shift` and `NeedsReview` status for the
/// user to approve with [`apply_reconform`].
pub fn reconform(s: &mut ProjectState, new_picture_id: &str, changes: &[EditChange]) -> Result<ReconformReport> {
    if !s.pictures.contains_key(new_picture_id) {
        return Err(crate::error::not_found("picture", new_picture_id));
    }
    let tb = s.tb();
    let mut rep = ReconformReport::default();
    for cue in s.cues.values_mut() {
        if cue.status == CueStatus::Conceptual {
            continue;
        }
        let f = tb.frame_at(cue.start_sample);
        match changes.iter().find(|c| f >= c.old_start && f < c.old_end) {
            Some(c) => {
                let shift_frames = c.new_start - c.old_start;
                let shift = tb.sample_at_frame(f + shift_frames) - tb.sample_at_frame(f);
                if shift == 0 {
                    rep.unchanged += 1;
                    cue.picture_id = Some(new_picture_id.to_string());
                } else {
                    cue.pending_shift = Some(shift);
                    cue.review_status = ReviewStatus::NeedsReview;
                    rep.shifted.push((cue.id.clone(), shift));
                }
            }
            None => {
                cue.review_status = ReviewStatus::NeedsReview;
                cue.notes = format!("{} [reconform: picture section removed]", cue.notes).trim().to_string();
                rep.removed.push(cue.id.clone());
            }
        }
    }
    Ok(rep)
}

/// Apply reviewed reconform shifts to the given cues (and their clips/markers).
pub fn apply_reconform(s: &mut ProjectState, cue_ids: &[String]) -> Result<usize> {
    let pic = s.project.current_picture_id.clone();
    let mut n = 0;
    for id in cue_ids {
        let cue = s.cue_mut(id)?;
        let Some(shift) = cue.pending_shift.take() else { continue };
        cue.start_sample += shift;
        cue.end_sample += shift;
        for e in cue.sub_events.iter_mut() {
            *e += shift;
        }
        cue.review_status = ReviewStatus::Accepted;
        cue.picture_id = pic.clone();
        let marker = cue.marker_id.clone();
        for c in s.clips.values_mut().filter(|c| c.cue_id.as_deref() == Some(id.as_str())) {
            c.timeline_start += shift;
        }
        if let Some(m) = marker.and_then(|m| s.markers.get_mut(&m)) {
            m.start_sample += shift;
            m.end_sample += shift;
            for e in m.sub_events.iter_mut() {
                *e += shift;
            }
        }
        n += 1;
    }
    Ok(n)
}

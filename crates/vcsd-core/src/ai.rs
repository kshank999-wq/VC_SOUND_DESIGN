//! AI-assisted timing (spec §10). User-directed: the user chooses which cues
//! exist; assistants only *propose* timing. Proposals never change the
//! project until accepted. Disabled/offline mode leaves every other workflow
//! intact. Cloud assistants require explicit opt-in.

use crate::error::{invalid, not_found, Error, Result};
use crate::ids::{new_id, now_unix};
use crate::model::*;
use crate::state::ProjectState;
use std::path::Path;
use std::process::Command;

pub struct AssistContext<'a> {
    pub state: &'a ProjectState,
    pub cue: &'a Cue,
    pub picture: Option<&'a PictureVersion>,
}

#[derive(Debug, Clone)]
pub struct ProposalDraft {
    pub start: i64,
    pub end: i64,
    pub sub_events: Vec<i64>,
    pub confidence: f32,
    pub reason: String,
}

pub trait TimingAssistant {
    fn name(&self) -> &str;
    /// Local assistants never send footage off the machine.
    fn is_local(&self) -> bool {
        true
    }
    fn propose(&self, ctx: &AssistContext) -> Result<Vec<ProposalDraft>>;
}

/// Baseline local assistant: proposes evenly spaced events at a typical
/// cadence. It has no visual information, so it always reports low confidence.
pub struct CadenceHeuristic {
    pub step_secs: f64,
}

impl TimingAssistant for CadenceHeuristic {
    fn name(&self) -> &str {
        "local:cadence-heuristic"
    }
    fn propose(&self, ctx: &AssistContext) -> Result<Vec<ProposalDraft>> {
        let c = ctx.cue;
        if c.event_type == EventType::Point {
            return Ok(vec![]);
        }
        let step = (self.step_secs * ctx.state.project.sample_rate as f64) as i64;
        let tb = ctx.state.tb();
        let mut subs = vec![];
        let mut t = c.start_sample;
        while t < c.end_sample {
            subs.push(tb.snap(t));
            t += step;
        }
        Ok(vec![ProposalDraft {
            start: c.start_sample,
            end: c.end_sample,
            sub_events: subs,
            confidence: 0.25,
            reason: format!("even {:.2}s cadence; no visual analysis — verify against picture", self.step_secs),
        }])
    }
}

/// Local visual-onset assistant using FFmpeg's per-frame scene-change score
/// as a motion-energy signal. Requires `ffmpeg` on PATH.
pub struct VisualOnsetAssistant {
    /// Search window around point cues, in frames.
    pub window_frames: i64,
}

impl Default for VisualOnsetAssistant {
    fn default() -> Self {
        Self { window_frames: 12 }
    }
}

/// Per-frame motion scores (absolute frame, score) for a picture range.
pub fn motion_scores(media: &Path, rate_fps: f64, source_start_frame: i64, from_frame: i64, to_frame: i64) -> Result<Vec<(i64, f32)>> {
    let from = (from_frame - source_start_frame).max(0);
    let dur = (to_frame - source_start_frame - from).max(1);
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-ss"])
        .arg(format!("{:.6}", from as f64 / rate_fps))
        .arg("-i")
        .arg(media)
        .arg("-t")
        .arg(format!("{:.6}", dur as f64 / rate_fps))
        .args(["-an", "-vf", "select='gte(scene,0)',metadata=print:file=-", "-f", "null", "-"])
        .output()
        .map_err(|e| Error::Disabled(format!("ffmpeg unavailable: {e}")))?;
    if !out.status.success() {
        return Err(invalid(format!("ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""))));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut res = vec![];
    let mut pts: Option<f64> = None;
    for line in text.lines() {
        if let Some(i) = line.find("pts_time:") {
            pts = line[i + 9..].split_whitespace().next().and_then(|v| v.parse().ok());
        } else if let Some(v) = line.strip_prefix("lavfi.scene_score=") {
            if let (Some(p), Ok(sc)) = (pts, v.trim().parse::<f32>()) {
                res.push((source_start_frame + from + (p * rate_fps).round() as i64, sc));
            }
        }
    }
    Ok(res)
}

impl TimingAssistant for VisualOnsetAssistant {
    fn name(&self) -> &str {
        "local:visual-onset"
    }
    fn propose(&self, ctx: &AssistContext) -> Result<Vec<ProposalDraft>> {
        let pic = ctx.picture.ok_or_else(|| invalid("no picture imported"))?;
        let tb = ctx.state.tb();
        let c = ctx.cue;
        let fps = ctx.state.project.frame_rate.as_f64();
        let (f0, f1) = match c.event_type {
            EventType::Point => (tb.frame_at(c.start_sample) - self.window_frames, tb.frame_at(c.start_sample) + self.window_frames + 1),
            _ => (tb.frame_at(c.start_sample), tb.frame_at(c.end_sample) + 1),
        };
        let scores = motion_scores(Path::new(&pic.media_path), fps, pic.source_start_frame, f0, f1)?;
        if scores.is_empty() {
            return Ok(vec![ProposalDraft {
                start: c.start_sample,
                end: c.end_sample,
                sub_events: vec![],
                confidence: 0.0,
                reason: "no decodable frames in range (off-picture?); spot manually".into(),
            }]);
        }
        let peak = scores.iter().map(|s| s.1).fold(0.0f32, f32::max);
        match c.event_type {
            EventType::Point => {
                let (f, sc) = scores.iter().cloned().fold((0, -1.0f32), |b, x| if x.1 > b.1 { x } else { b });
                let conf = (sc / 0.3).clamp(0.0, 1.0);
                let reason = if sc < 0.02 {
                    "no clear visual onset near marker (occluded/off-screen?); manual spotting recommended".to_string()
                } else {
                    format!("strongest visual change at {} (score {sc:.3})", ctx.state.project.frame_rate.frames_to_tc(f))
                };
                let at = tb.sample_at_frame(f);
                Ok(vec![ProposalDraft { start: at, end: at, sub_events: vec![], confidence: if sc < 0.02 { 0.05 } else { conf }, reason }])
            }
            _ => {
                let n = scores.len() as f32;
                let mean = scores.iter().map(|s| s.1).sum::<f32>() / n;
                let sd = (scores.iter().map(|s| (s.1 - mean).powi(2)).sum::<f32>() / n).sqrt();
                let thr = mean + sd * 0.75;
                let min_gap = (fps * 0.25).round() as i64;
                let mut subs: Vec<i64> = vec![];
                for w in scores.windows(3) {
                    let (f, v) = w[1];
                    if v > thr && v >= w[0].1 && v >= w[2].1 && subs.last().is_none_or(|&l| f - l >= min_gap) {
                        subs.push(f);
                    }
                }
                let conf = if peak < 0.02 || subs.is_empty() { 0.05 } else { ((peak - mean) / (peak + 1e-6)).clamp(0.1, 0.9) };
                Ok(vec![ProposalDraft {
                    start: c.start_sample,
                    end: c.end_sample,
                    sub_events: subs.iter().map(|&f| tb.sample_at_frame(f)).collect(),
                    confidence: conf,
                    reason: if subs.is_empty() {
                        "no repeating visual onsets found; manual tapping recommended".into()
                    } else {
                        format!("{} motion onsets above mean+0.75σ", subs.len())
                    },
                }])
            }
        }
    }
}

/// Run an assistant on cues. Results are cached per picture fingerprint, so
/// re-running on the same picture version returns existing proposals.
pub fn run(s: &mut ProjectState, a: &dyn TimingAssistant, cue_ids: &[String]) -> Result<Vec<String>> {
    if !s.project.ai.enabled {
        return Err(Error::Disabled("AI assistance is turned off for this project".into()));
    }
    if !a.is_local() && !s.project.ai.allow_cloud {
        return Err(Error::Disabled("cloud AI requires explicit opt-in; footage is never uploaded implicitly".into()));
    }
    let fp = s.current_picture().map(|p| p.fingerprint.clone()).unwrap_or_else(|| "no-picture".into());
    let mut out = vec![];
    for cid in cue_ids {
        let cue = s.cue(cid)?.clone();
        let cached: Vec<String> = s
            .proposals
            .values()
            .filter(|p| p.cue_id == *cid && p.assistant == a.name() && p.picture_fingerprint == fp && p.status == ProposalStatus::Pending)
            .map(|p| p.id.clone())
            .collect();
        if !cached.is_empty() {
            out.extend(cached);
            continue;
        }
        let drafts = a.propose(&AssistContext { state: s, cue: &cue, picture: s.current_picture() })?;
        for d in drafts {
            let id = new_id("aip");
            s.proposals.insert(
                id.clone(),
                AiProposal {
                    id: id.clone(),
                    cue_id: cid.clone(),
                    assistant: a.name().into(),
                    picture_fingerprint: fp.clone(),
                    start_sample: d.start,
                    end_sample: d.end,
                    sub_events: d.sub_events,
                    confidence: d.confidence,
                    reason: d.reason,
                    status: ProposalStatus::Pending,
                    created_at: now_unix(),
                    before_start: cue.start_sample,
                    before_end: cue.end_sample,
                    before_sub_events: cue.sub_events.clone(),
                },
            );
            out.push(id);
        }
    }
    Ok(out)
}

/// Reject a proposal. Nothing else in the project changes.
pub fn reject(s: &mut ProjectState, id: &str) -> Result<()> {
    let p = s.proposals.get_mut(id).ok_or_else(|| not_found("proposal", id))?;
    p.status = ProposalStatus::Rejected;
    Ok(())
}

/// Accept a proposal, optionally with user drag-adjusted timing.
pub fn accept(s: &mut ProjectState, id: &str, adjusted: Option<(i64, i64, Vec<i64>)>) -> Result<()> {
    let p = s.proposals.get(id).ok_or_else(|| not_found("proposal", id))?.clone();
    if p.status != ProposalStatus::Pending {
        return Err(invalid("proposal already resolved"));
    }
    let (start, end, subs) = adjusted.unwrap_or((p.start_sample, p.end_sample, p.sub_events.clone()));
    let cue = s.cue(&p.cue_id)?.clone();
    let delta = start - cue.start_sample;
    let sub_clips: Vec<String> = s.clips_for_cue(&cue.id).iter().filter(|c| c.sub_index.is_some()).map(|c| c.id.clone()).collect();
    let whole_clips: Vec<String> = s.clips_for_cue(&cue.id).iter().filter(|c| c.sub_index.is_none()).map(|c| c.id.clone()).collect();
    {
        let c = s.cue_mut(&cue.id)?;
        c.start_sample = start;
        c.end_sample = end.max(start);
        if !subs.is_empty() {
            c.sub_events = subs.clone();
        }
        c.timing_origin = TimingOrigin::AiSuggested;
        c.review_status = ReviewStatus::Accepted;
    }
    for cid in whole_clips {
        s.clip_mut(&cid)?.timeline_start += delta;
    }
    if !subs.is_empty() && !sub_clips.is_empty() {
        let all_placeholder = sub_clips.iter().all(|c| s.clips[c].is_placeholder);
        if sub_clips.len() == subs.len() {
            let mut ordered = sub_clips.clone();
            ordered.sort_by_key(|c| s.clips[c].sub_index);
            for (cid, &t) in ordered.iter().zip(subs.iter()) {
                s.clip_mut(cid)?.timeline_start = t;
            }
        } else if all_placeholder {
            for cid in &sub_clips {
                s.clips.remove(cid);
            }
            crate::builder::populate(s, &crate::builder::PopulateOptions { cue_ids: Some(vec![cue.id.clone()]), ..Default::default() })?;
        }
    }
    if let Some(m) = cue.marker_id.and_then(|m| s.markers.get_mut(&m)) {
        m.start_sample = start;
        m.end_sample = end.max(start);
        if !subs.is_empty() {
            m.sub_events = subs;
        }
    }
    s.proposals.get_mut(id).unwrap().status = ProposalStatus::Accepted;
    Ok(())
}

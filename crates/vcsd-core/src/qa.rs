//! Delivery QA (spec §15.2): media linked, hash verification, overlaps,
//! out-of-range, channels, sample-rate conversion, rights metadata and sync.

use crate::model::*;
use crate::state::ProjectState;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct QaIssue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
}

impl QaIssue {
    pub fn new(severity: Severity, code: &str, message: impl Into<String>, entity: Option<&str>) -> Self {
        Self { severity, code: code.into(), message: message.into(), entity: entity.map(String::from) }
    }
}

#[derive(Debug, Clone, Default)]
pub struct QaOptions {
    /// Verify SHA-256 of every referenced media file (slower).
    pub verify_hashes: bool,
    /// Temp references will be excluded from the render.
    pub exclude_temp_refs: bool,
}

pub fn run(s: &ProjectState, o: &QaOptions) -> Vec<QaIssue> {
    let mut v = vec![];
    let tb = s.tb();
    let used: BTreeSet<&str> = s.clips.values().filter(|c| !c.muted).map(|c| c.asset_id.as_str()).collect();
    let mut hash_cache: BTreeMap<&str, bool> = BTreeMap::new();
    for aid in &used {
        let Some(a) = s.assets.get(*aid) else {
            v.push(QaIssue::new(Severity::Error, "asset_unknown", format!("clip references unknown asset {aid}"), Some(aid)));
            continue;
        };
        if !Path::new(&a.path).exists() {
            v.push(QaIssue::new(
                Severity::Error,
                "media_missing",
                format!("'{}' not found at {} — relink before export", a.name, a.path),
                Some(&a.id),
            ));
            continue;
        }
        if o.verify_hashes {
            let ok = *hash_cache
                .entry(aid)
                .or_insert_with(|| crate::hash::sha256_file(Path::new(&a.path)).map(|h| h == a.sha256).unwrap_or(false));
            if !ok {
                v.push(QaIssue::new(
                    Severity::Error,
                    "hash_mismatch",
                    format!("'{}' content changed since import (checksum mismatch)", a.name),
                    Some(&a.id),
                ));
            }
        }
        if a.sample_rate != s.project.sample_rate {
            v.push(QaIssue::new(
                Severity::Warning,
                "sample_rate_conversion",
                format!("'{}' is {} Hz; will be converted to {} Hz (not bit-accurate)", a.name, a.sample_rate, s.project.sample_rate),
                Some(&a.id),
            ));
        }
        if a.channels > 2 {
            v.push(QaIssue::new(
                Severity::Warning,
                "unsupported_channels",
                format!("'{}' has {} channels; stems are stereo and will fold down", a.name, a.channels),
                Some(&a.id),
            ));
        }
        match &a.license {
            None => v.push(QaIssue::new(
                Severity::Warning,
                "rights_missing",
                format!("'{}' has no license/provenance metadata", a.name),
                Some(&a.id),
            )),
            Some(l) if l.temp_reference && !o.exclude_temp_refs => v.push(QaIssue::new(
                Severity::Warning,
                "temp_reference",
                format!("temp reference '{}' would be baked into the deliverable", a.name),
                Some(&a.id),
            )),
            Some(l) if !l.redistributable && !l.temp_reference => v.push(QaIssue::new(
                Severity::Warning,
                "rights_not_redistributable",
                format!("'{}' ({}) is not marked redistributable; confirm rights for delivery", a.name, l.name),
                Some(&a.id),
            )),
            _ => {}
        }
    }
    // overlaps between different cues on one track
    let mut by_track: BTreeMap<&str, Vec<&ClipInstance>> = BTreeMap::new();
    for c in s.clips.values().filter(|c| !c.muted) {
        by_track.entry(c.track_id.as_str()).or_default().push(c);
    }
    for (tid, mut clips) in by_track {
        clips.sort_by_key(|c| (c.timeline_start, c.id.clone()));
        for (i, a) in clips.iter().enumerate() {
            for b in clips[i + 1..].iter().take_while(|b| b.timeline_start < a.end()) {
                if a.cue_id != b.cue_id && a.end() - b.timeline_start > a.fade_out.max(b.fade_in) {
                    let tn = s.tracks.get(tid).map(|t| t.name.as_str()).unwrap_or(tid);
                    v.push(QaIssue::new(
                        Severity::Warning,
                        "clip_overlap",
                        format!("overlapping clips on '{tn}' at {} without crossfade", tb.tc_at(b.timeline_start)),
                        Some(&b.id),
                    ));
                }
            }
        }
    }
    // range checks
    if let Some(pic) = s.current_picture().filter(|p| p.duration_frames > 0) {
        let (a, z) = crate::picture::picture_range(s, pic);
        for c in s.clips.values().filter(|c| !c.muted) {
            if c.timeline_start < a || c.timeline_start >= z {
                v.push(QaIssue::new(
                    Severity::Warning,
                    "out_of_range",
                    format!("clip at {} starts outside picture {}", tb.tc_at(c.timeline_start), pic.label),
                    Some(&c.id),
                ));
            }
        }
    }
    for c in s.cues.values() {
        if c.status == CueStatus::Conceptual {
            v.push(QaIssue::new(
                Severity::Info,
                "cue_unspotted",
                format!("{} '{}' is script-only (not yet spotted)", c.code, c.description),
                Some(&c.id),
            ));
            continue;
        }
        if c.status == CueStatus::Omitted {
            continue;
        }
        if let Some(sc) = s.scenes.get(&c.scene_id) {
            if c.start_sample < sc.start_sample - sc.handle_samples || c.start_sample >= sc.end_sample + sc.handle_samples {
                v.push(QaIssue::new(
                    Severity::Warning,
                    "cue_outside_scene",
                    format!("{} starts at {} outside scene {}", c.code, tb.tc_at(c.start_sample), sc.number),
                    Some(&c.id),
                ));
            }
        }
        if !s.clips.values().any(|cl| cl.cue_id.as_deref() == Some(c.id.as_str()) && !cl.muted) {
            v.push(QaIssue::new(
                Severity::Warning,
                "cue_without_media",
                format!("{} '{}' has no audible media", c.code, c.description),
                Some(&c.id),
            ));
        }
        if c.status == CueStatus::NeedsRecording {
            v.push(QaIssue::new(Severity::Warning, "needs_recording", format!("{} still needs recording", c.code), Some(&c.id)));
        }
        if c.pending_shift.is_some() || c.review_status == ReviewStatus::NeedsReview {
            v.push(QaIssue::new(Severity::Warning, "reconform_review", format!("{} needs picture-change review", c.code), Some(&c.id)));
        }
    }
    for sc in s.scores.values() {
        for w in &sc.warnings {
            v.push(QaIssue::new(Severity::Error, "score_sync", format!("score '{}': {w}", sc.title), Some(&sc.id)));
        }
    }
    let pending = s.proposals.values().filter(|p| p.status == ProposalStatus::Pending).count();
    if pending > 0 {
        v.push(QaIssue::new(Severity::Info, "ai_pending", format!("{pending} AI timing proposals awaiting review (not applied)"), None));
    }
    for t in s.tracks.values().filter(|t| t.mute) {
        if s.clips.values().any(|c| c.track_id == t.id) {
            v.push(QaIssue::new(
                Severity::Info,
                "muted_track",
                format!("track '{}' is muted and will not be heard in mixes/stems", t.name),
                Some(&t.id),
            ));
        }
    }
    v.sort_by(|a, b| (a.severity, &a.code, &a.message).cmp(&(b.severity, &b.code, &b.message)));
    v
}

pub fn count(v: &[QaIssue], sev: Severity) -> usize {
    v.iter().filter(|i| i.severity == sev).count()
}

pub fn to_markdown(v: &[QaIssue]) -> String {
    let mut out = format!(
        "# Delivery QA\n\n- Errors: {}\n- Warnings: {}\n- Info: {}\n\n",
        count(v, Severity::Error),
        count(v, Severity::Warning),
        count(v, Severity::Info)
    );
    if v.is_empty() {
        out.push_str("No issues found.\n");
    }
    for i in v {
        out.push_str(&format!("- **{:?}** `{}` — {}\n", i.severity, i.code, i.message));
    }
    out
}

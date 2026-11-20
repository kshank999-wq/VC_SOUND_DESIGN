//! Cue chart, manifests and playhead queries (spec §14). Output is
//! reproducible from saved project state: deterministic sorting, stable IDs,
//! and an explicit `date` parameter instead of wall-clock time.

use crate::model::*;
use crate::state::ProjectState;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct CueRow {
    pub cue_id: String,
    pub code: String,
    pub scene: String,
    pub category: String,
    pub identity: String,
    pub description: String,
    pub event_type: EventType,
    pub start_tc: String,
    pub end_tc: String,
    pub duration_tc: String,
    pub start_sample: i64,
    pub end_sample: i64,
    pub sub_events: usize,
    pub track: String,
    pub media: String,
    pub source_status: String,
    pub timing_origin: TimingOrigin,
    pub review: ReviewStatus,
    pub status: CueStatus,
    pub priority: Priority,
    pub owner: String,
    pub picture_version: String,
    pub notes: String,
    pub media_missing: bool,
}

fn source_status(s: &ProjectState, c: &Cue, clips: &[&ClipInstance]) -> String {
    if c.status == CueStatus::Conceptual {
        return "conceptual (unspotted)".into();
    }
    let audible: Vec<&&ClipInstance> = clips.iter().filter(|c| !c.muted).collect();
    if audible.is_empty() {
        return "no media".into();
    }
    let mut kinds = BTreeSet::new();
    for cl in audible {
        let k = if cl.performance_event_id.is_some() {
            "performed"
        } else if cl.take_id.is_some() {
            "recorded take"
        } else {
            match s.assets.get(&cl.asset_id).map(|a| a.kind) {
                Some(AssetKind::Placeholder) => "placeholder",
                Some(AssetKind::Recorded) => "recorded",
                Some(AssetKind::Library) | Some(AssetKind::Imported) => "library",
                Some(AssetKind::Score) => "score",
                Some(AssetKind::Rendered) => "rendered",
                None => "missing",
            }
        };
        kinds.insert(k);
    }
    kinds.into_iter().collect::<Vec<_>>().join("+")
}

pub fn cue_rows(s: &ProjectState, scene_id: Option<&str>) -> Vec<CueRow> {
    let tb = s.tb();
    s.sorted_cues(scene_id)
        .into_iter()
        .map(|c| {
            let clips = s.clips_for_cue(&c.id);
            let mut tracks = BTreeSet::new();
            let mut media = BTreeSet::new();
            let mut missing = false;
            for cl in clips.iter().filter(|c| !c.muted) {
                if let Some(t) = s.tracks.get(&cl.track_id) {
                    tracks.insert(t.name.clone());
                }
                match s.assets.get(&cl.asset_id) {
                    Some(a) => {
                        media.insert(Path::new(&a.path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
                        if !Path::new(&a.path).exists() {
                            missing = true;
                        }
                    }
                    None => missing = true,
                }
            }
            let conceptual = c.status == CueStatus::Conceptual;
            CueRow {
                cue_id: c.id.clone(),
                code: c.code.clone(),
                scene: s.scenes.get(&c.scene_id).map(|x| x.number.clone()).unwrap_or_default(),
                category: c.category.display(),
                identity: c.identity_id.as_ref().and_then(|i| s.identities.get(i)).map(|i| i.name.clone()).unwrap_or_default(),
                description: c.description.clone(),
                event_type: c.event_type,
                start_tc: if conceptual { String::new() } else { tb.tc_at(c.start_sample) },
                end_tc: if conceptual { String::new() } else { tb.tc_at(c.end_sample) },
                duration_tc: if conceptual || c.event_type == EventType::Point { String::new() } else { tb.duration_tc(c.duration()) },
                start_sample: c.start_sample,
                end_sample: c.end_sample,
                sub_events: c.sub_events.len(),
                track: tracks.into_iter().collect::<Vec<_>>().join(" | "),
                media: media.into_iter().collect::<Vec<_>>().join(" | "),
                source_status: source_status(s, c, &clips),
                timing_origin: c.timing_origin,
                review: c.review_status,
                status: c.status,
                priority: c.priority,
                owner: c.owner.clone(),
                picture_version: c.picture_id.as_ref().and_then(|p| s.pictures.get(p)).map(|p| p.label.clone()).unwrap_or_default(),
                notes: c.notes.clone(),
                media_missing: missing,
            }
        })
        .collect()
}

pub fn csv_escape(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|x| x.as_str().map(String::from)).unwrap_or_default()
}

pub fn rows_to_csv(s: &ProjectState, rows: &[CueRow]) -> String {
    let fr = s.project.frame_rate;
    let mut out = String::new();
    out.push_str(&format!("# frame_rate={},drop_frame={},sample_rate={}\n", fr.label(), fr.drop_frame, s.project.sample_rate));
    out.push_str("cue_id,code,scene,category,identity,description,event_type,start_tc,end_tc,duration_tc,start_sample,end_sample,sub_events,track,media,source_status,timing_origin,review,status,priority,owner,picture_version,notes,media_missing\n");
    for r in rows {
        let cols = [
            r.cue_id.clone(),
            r.code.clone(),
            r.scene.clone(),
            r.category.clone(),
            r.identity.clone(),
            r.description.clone(),
            enum_str(&r.event_type),
            r.start_tc.clone(),
            r.end_tc.clone(),
            r.duration_tc.clone(),
            r.start_sample.to_string(),
            r.end_sample.to_string(),
            r.sub_events.to_string(),
            r.track.clone(),
            r.media.clone(),
            r.source_status.clone(),
            enum_str(&r.timing_origin),
            enum_str(&r.review),
            enum_str(&r.status),
            enum_str(&r.priority),
            r.owner.clone(),
            r.picture_version.clone(),
            r.notes.clone(),
            r.media_missing.to_string(),
        ];
        out.push_str(&cols.iter().map(|c| csv_escape(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportHeader {
    pub film: String,
    pub reel: String,
    pub scene: String,
    pub scene_title: String,
    pub picture_version: String,
    pub sample_rate: u32,
    pub frame_rate: String,
    pub drop_frame: bool,
    pub start_tc: String,
    pub end_tc: String,
    pub date: String,
}

pub fn header(s: &ProjectState, scene_id: Option<&str>, date: &str) -> ReportHeader {
    let tb = s.tb();
    let sc = scene_id.and_then(|id| s.scenes.get(id));
    let (a, z) = match sc {
        Some(sc) => (sc.start_sample, sc.end_sample),
        None => (s.scenes.values().map(|x| x.start_sample).min().unwrap_or(0), s.scenes.values().map(|x| x.end_sample).max().unwrap_or(0)),
    };
    ReportHeader {
        film: s.project.name.clone(),
        reel: s.project.reel.clone(),
        scene: sc.map(|x| x.number.clone()).unwrap_or_else(|| "ALL".into()),
        scene_title: sc.map(|x| x.title.clone()).unwrap_or_else(|| "Master cue inventory".into()),
        picture_version: s.current_picture().map(|p| p.label.clone()).unwrap_or_else(|| "(no picture)".into()),
        sample_rate: s.project.sample_rate,
        frame_rate: s.project.frame_rate.label(),
        drop_frame: s.project.frame_rate.drop_frame,
        start_tc: tb.tc_at(a),
        end_tc: tb.tc_at(z),
        date: date.into(),
    }
}

pub fn manifest_json(s: &ProjectState, scene_id: Option<&str>, date: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": "vc.sounddesign.cue-report",
        "version": 1,
        "header": header(s, scene_id, date),
        "cues": cue_rows(s, scene_id),
    })
}

// ---------------------------------------------------------------- playhead query

#[derive(Debug, Clone, Serialize)]
pub struct ActiveSound {
    pub cue_id: Option<String>,
    pub code: String,
    pub identity: String,
    pub category: String,
    pub description: String,
    pub start_tc: String,
    pub end_tc: String,
    pub tracks: Vec<String>,
    pub buses: Vec<String>,
    pub media: Vec<String>,
}

/// Every sound active at timeline sample `at`: cue intervals (point cues cover
/// one frame) plus any audible clip under the playhead, including layered
/// clips, score and buses.
pub fn active_at(s: &ProjectState, at: i64) -> Vec<ActiveSound> {
    let tb = s.tb();
    let one = s.one_frame();
    let mut out: Vec<ActiveSound> = vec![];
    let mut cue_ids: BTreeSet<String> =
        s.cues.values().filter(|c| c.status != CueStatus::Conceptual && c.covers(at, one)).map(|c| c.id.clone()).collect();
    let live_clips: Vec<&ClipInstance> = s.clips.values().filter(|c| !c.muted && at >= c.timeline_start && at < c.end()).collect();
    for c in &live_clips {
        if let Some(id) = &c.cue_id {
            cue_ids.insert(id.clone());
        }
    }
    let describe = |clips: &[&ClipInstance]| {
        let mut tracks = BTreeSet::new();
        let mut buses = BTreeSet::new();
        let mut media = BTreeSet::new();
        for cl in clips {
            if let Some(t) = s.tracks.get(&cl.track_id) {
                tracks.insert(t.name.clone());
                if let Some(b) = s.buses.get(&t.bus_id) {
                    buses.insert(b.name.clone());
                }
            }
            if let Some(a) = s.assets.get(&cl.asset_id) {
                media.insert(a.name.clone());
            }
        }
        (tracks.into_iter().collect(), buses.into_iter().collect(), media.into_iter().collect())
    };
    let mut cues: Vec<&Cue> = cue_ids.iter().filter_map(|id| s.cues.get(id)).collect();
    cues.sort_by(|a, b| (a.start_sample, &a.code).cmp(&(b.start_sample, &b.code)));
    for c in cues {
        let clips: Vec<&ClipInstance> = live_clips.iter().copied().filter(|cl| cl.cue_id.as_deref() == Some(c.id.as_str())).collect();
        let (tracks, buses, media) = describe(&clips);
        out.push(ActiveSound {
            cue_id: Some(c.id.clone()),
            code: c.code.clone(),
            identity: c.identity_id.as_ref().and_then(|i| s.identities.get(i)).map(|i| i.name.clone()).unwrap_or_default(),
            category: c.category.display(),
            description: c.description.clone(),
            start_tc: tb.tc_at(c.start_sample),
            end_tc: tb.tc_at(c.end_sample),
            tracks,
            buses,
            media,
        });
    }
    for cl in live_clips.iter().filter(|c| c.cue_id.is_none()) {
        let (tracks, buses, media) = describe(&[cl]);
        let t = s.tracks.get(&cl.track_id);
        out.push(ActiveSound {
            cue_id: None,
            code: if t.is_some_and(|t| t.category == Category::Music) { "SCORE".into() } else { "CLIP".into() },
            identity: String::new(),
            category: t.map(|t| t.category.display()).unwrap_or_default(),
            description: t.map(|t| t.name.clone()).unwrap_or_default(),
            start_tc: tb.tc_at(cl.timeline_start),
            end_tc: tb.tc_at(cl.end()),
            tracks,
            buses,
            media,
        });
    }
    out
}

// ---------------------------------------------------------------- overview layout

#[derive(Debug, Clone)]
pub struct LaneLayout {
    pub name: String,
    pub category: Category,
    pub color: String,
    /// rows of (cue code, start, end, label)
    pub rows: Vec<Vec<(String, i64, i64, String)>>,
}

/// Lay cues out in marker-lane bands with greedy stacking for overlaps.
pub fn overview_layout(s: &ProjectState, scene_id: Option<&str>) -> Vec<LaneLayout> {
    let one = s.one_frame();
    let mut lanes: Vec<&MarkerLane> = s.lanes.values().filter(|l| !l.hidden).collect();
    lanes.sort_by_key(|l| (l.order, l.id.clone()));
    let mut out = vec![];
    let mut used: BTreeSet<String> = BTreeSet::new();
    let cues = s.sorted_cues(scene_id);
    let mut push_lane = |name: String, cat: Category, color: String, items: Vec<&Cue>| {
        let mut rows: Vec<Vec<(String, i64, i64, String)>> = vec![];
        for c in items {
            let end = c.end_sample.max(c.start_sample + one);
            let label = match c.identity_id.as_ref().and_then(|i| s.identities.get(i)) {
                Some(i) => i.name.clone(),
                None => c.description.clone(),
            };
            match rows.iter_mut().find(|r| r.last().is_none_or(|l| l.2 <= c.start_sample)) {
                Some(r) => r.push((c.code.clone(), c.start_sample, end, label)),
                None => rows.push(vec![(c.code.clone(), c.start_sample, end, label)]),
            }
        }
        if !rows.is_empty() {
            out.push(LaneLayout { name, category: cat, color, rows });
        }
    };
    for l in lanes {
        let items: Vec<&Cue> = cues
            .iter()
            .copied()
            .filter(|c| c.status != CueStatus::Conceptual)
            .filter(|c| {
                c.lane_id.as_deref() == Some(l.id.as_str()) || (c.lane_id.is_none() && c.category == l.category && !used.contains(&c.id))
            })
            .collect();
        for c in &items {
            used.insert(c.id.clone());
        }
        push_lane(l.name.clone(), l.category.clone(), l.color.clone(), items);
    }
    let rest: Vec<&Cue> = cues.iter().copied().filter(|c| c.status != CueStatus::Conceptual && !used.contains(&c.id)).collect();
    push_lane("Other".into(), Category::Custom("other".into()), "#555753".into(), rest);
    out
}

// ---------------------------------------------------------------- scene summary

#[derive(Debug, Clone, Serialize)]
pub struct SceneSummary {
    pub scene: String,
    pub cues: usize,
    pub by_status: std::collections::BTreeMap<String, usize>,
    pub missing_media: Vec<String>,
    pub overlaps: Vec<String>,
    pub ambience_gaps: Vec<String>,
    pub muted_tracks: Vec<String>,
    pub sync_warnings: Vec<String>,
    pub pending_ai: usize,
}

pub fn scene_summary(s: &ProjectState, scene_id: &str) -> crate::Result<SceneSummary> {
    let sc = s.scene(scene_id)?;
    let tb = s.tb();
    let rows = cue_rows(s, Some(scene_id));
    let mut by_status = std::collections::BTreeMap::new();
    for r in &rows {
        *by_status.entry(enum_str(&r.status)).or_insert(0) += 1;
    }
    let missing_media = rows.iter().filter(|r| r.media_missing || r.source_status == "no media").map(|r| r.code.clone()).collect();
    let cues = s.sorted_cues(Some(scene_id));
    let mut overlaps = vec![];
    for (i, a) in cues.iter().enumerate() {
        for b in &cues[i + 1..] {
            if a.identity_id.is_some() && a.identity_id == b.identity_id && b.start_sample < a.end_sample && a.end_sample > a.start_sample {
                overlaps.push(format!("{} overlaps {} (same identity)", a.code, b.code));
            }
        }
    }
    // ambience coverage gaps
    let mut bg: Vec<(i64, i64)> =
        cues.iter().filter(|c| c.category == Category::Background).map(|c| (c.start_sample, c.end_sample)).collect();
    bg.sort();
    let mut gaps = vec![];
    let mut cur = sc.start_sample;
    for (a, z) in bg {
        if a > cur {
            gaps.push(format!("{} – {}", tb.tc_at(cur), tb.tc_at(a)));
        }
        cur = cur.max(z);
    }
    if cur < sc.end_sample {
        gaps.push(format!("{} – {}", tb.tc_at(cur), tb.tc_at(sc.end_sample)));
    }
    let muted_tracks = s
        .tracks
        .values()
        .filter(|t| t.mute && t.scene_id.as_deref().is_none_or(|x| x == scene_id))
        .filter(|t| s.clips.values().any(|c| c.track_id == t.id))
        .map(|t| t.name.clone())
        .collect();
    let sync_warnings = cues
        .iter()
        .filter(|c| c.review_status == ReviewStatus::NeedsReview || c.pending_shift.is_some())
        .map(|c| format!("{} needs reconform review", c.code))
        .chain(s.scores.values().flat_map(|sc| sc.warnings.iter().map(move |w| format!("score '{}': {w}", sc.title))))
        .collect();
    let pending_ai = s.proposals.values().filter(|p| p.status == ProposalStatus::Pending && cues.iter().any(|c| c.id == p.cue_id)).count();
    Ok(SceneSummary {
        scene: sc.number.clone(),
        cues: rows.len(),
        by_status,
        missing_media,
        overlaps,
        ambience_gaps: gaps,
        muted_tracks,
        sync_warnings,
        pending_ai,
    })
}

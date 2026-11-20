//! VC Film Studio (script breakdown) and VC Composer (score) contracts (spec §7, §12).
//! Versioned JSON boundaries that work locally before any cloud API exists.

use crate::error::{invalid, Result};
use crate::ids::{new_id, now_unix};
use crate::library::{import_wav, AssetMeta};
use crate::model::*;
use crate::state::ProjectState;
use crate::timecode::FrameRate;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILM_STUDIO_SCHEMA: &str = "vc.filmstudio.breakdown";
pub const COMPOSER_SCHEMA: &str = "vc.composer.score";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsBreakdown {
    pub schema: String,
    pub version: u32,
    #[serde(default)]
    pub production: String,
    #[serde(default)]
    pub revision: String,
    pub scenes: Vec<FsScene>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsScene {
    pub scene_id: String,
    pub number: String,
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub characters: Vec<String>,
    #[serde(default)]
    pub lines: Vec<FsLine>,
    #[serde(default)]
    pub sound: Vec<FsSoundItem>,
    /// Optional picture timing when the cut is known.
    #[serde(default)]
    pub start_tc: Option<String>,
    #[serde(default)]
    pub end_tc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsLine {
    pub line_id: String,
    #[serde(default)]
    pub character: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsSoundItem {
    pub item_id: String,
    pub category: String,
    pub description: String,
    #[serde(default)]
    pub character: Option<String>,
    #[serde(default)]
    pub prop: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub line_id: Option<String>,
    /// Recurring identity name, e.g. "Greg / Boots".
    #[serde(default)]
    pub identity: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct FsImportReport {
    pub scenes_created: usize,
    pub scenes_updated: usize,
    pub identities_created: usize,
    pub cues_created: usize,
    pub cues_updated: usize,
    /// Fields changed both locally and upstream — left for user review.
    pub conflicts: Vec<String>,
}

/// Import or re-import a Film Studio breakdown. Matching uses stable external
/// IDs; local timing and editorial work are never overwritten.
pub fn import_film_studio(s: &mut ProjectState, json: &str) -> Result<FsImportReport> {
    let b: FsBreakdown = serde_json::from_str(json)?;
    if b.schema != FILM_STUDIO_SCHEMA || b.version != 1 {
        return Err(invalid(format!("unsupported breakdown schema {} v{}", b.schema, b.version)));
    }
    let mut r = FsImportReport::default();
    let tb = s.tb();
    let mut cursor = s.scenes.values().map(|x| x.end_sample).max().unwrap_or(0);
    for fs in &b.scenes {
        let existing = s.scenes.values().find(|x| x.external_id.as_deref() == Some(&fs.scene_id)).map(|x| x.id.clone());
        let scene_id = match existing {
            Some(id) => {
                let sc = s.scenes.get_mut(&id).unwrap();
                if !fs.heading.is_empty() {
                    sc.title = fs.heading.clone();
                }
                r.scenes_updated += 1;
                id
            }
            None => {
                let (a, z) = match (&fs.start_tc, &fs.end_tc) {
                    (Some(a), Some(z)) => (tb.sample_at_tc(a)?, tb.sample_at_tc(z)?),
                    // provisional placeholder range: refine after picture spotting
                    _ => (cursor, cursor + 60 * s.project.sample_rate as i64),
                };
                cursor = cursor.max(z);
                let id = crate::spotting::add_scene(s, &fs.number, &fs.heading, a, z)?;
                s.scenes.get_mut(&id).unwrap().external_id = Some(fs.scene_id.clone());
                r.scenes_created += 1;
                id
            }
        };
        for name in &fs.characters {
            if !s.characters.values().any(|c| c.name.eq_ignore_ascii_case(name)) {
                let id = new_id("chr");
                s.characters.insert(id.clone(), Character { id, name: name.clone(), external_id: None });
            }
        }
        for (i, l) in fs.lines.iter().enumerate() {
            let ex = s.script_refs.values().find(|x| x.external_id.as_deref() == Some(&l.line_id)).map(|x| x.id.clone());
            let id = ex.unwrap_or_else(|| new_id("scr"));
            s.script_refs.insert(
                id.clone(),
                ScriptReference {
                    id,
                    scene_id: scene_id.clone(),
                    external_id: Some(l.line_id.clone()),
                    order: i as u32,
                    character: l.character.clone(),
                    text: l.text.clone(),
                },
            );
        }
        let anchor = s.scenes[&scene_id].start_sample;
        for item in &fs.sound {
            let cat = Category::parse(&item.category);
            let identity_id = match &item.identity {
                Some(name) => Some(match s.find_identity(name) {
                    Ok(i) => i.id.clone(),
                    Err(_) => {
                        r.identities_created += 1;
                        let id = crate::spotting::add_identity(
                            s,
                            crate::spotting::NewIdentity {
                                name: name.clone(),
                                category: cat.clone(),
                                character: item.character.clone(),
                                surface: item.surface.clone(),
                                prop: item.prop.clone(),
                            },
                        )?;
                        s.identities.get_mut(&id).unwrap().external_id = Some(item.item_id.clone());
                        id
                    }
                }),
                None => None,
            };
            let script_ref = item
                .line_id
                .as_ref()
                .and_then(|l| s.script_refs.values().find(|x| x.external_id.as_deref() == Some(l)).map(|x| x.id.clone()));
            if let Some(c) = s.cues.values_mut().find(|c| c.external_id.as_deref() == Some(&item.item_id)) {
                if c.description != item.description {
                    if c.status == CueStatus::Conceptual {
                        c.description = item.description.clone();
                        r.cues_updated += 1;
                    } else {
                        r.conflicts.push(format!("{}: upstream description '{}' vs local '{}'", c.code, item.description, c.description));
                    }
                }
                continue;
            }
            let code = crate::spotting::next_cue_code(s, &scene_id, &cat)?;
            let id = new_id("cue");
            s.cues.insert(
                id.clone(),
                Cue {
                    id,
                    scene_id: scene_id.clone(),
                    code,
                    identity_id,
                    category: cat,
                    event_type: EventType::Point,
                    lane_id: None,
                    start_sample: anchor,
                    end_sample: anchor,
                    sub_events: vec![],
                    description: item.description.clone(),
                    intent: String::new(),
                    script_ref_id: script_ref,
                    character: item.character.clone(),
                    surface: item.surface.clone(),
                    prop: item.prop.clone(),
                    placeholder_asset_id: None,
                    asset_override_id: None,
                    timing_origin: TimingOrigin::Imported,
                    review_status: ReviewStatus::Unreviewed,
                    status: CueStatus::Conceptual,
                    priority: Priority::Normal,
                    owner: String::new(),
                    notes: format!("from script breakdown rev {}", b.revision),
                    marker_id: None,
                    picture_id: None,
                    external_id: Some(item.item_id.clone()),
                    pending_shift: None,
                },
            );
            r.cues_created += 1;
        }
    }
    Ok(r)
}

/// Round-trip only sound-design fields back to Film Studio, keyed by stable IDs.
pub fn export_film_studio_sound(s: &ProjectState) -> serde_json::Value {
    let tb = s.tb();
    let items: Vec<serde_json::Value> = s
        .sorted_cues(None)
        .into_iter()
        .filter_map(|c| {
            let ext = c.external_id.clone()?;
            Some(serde_json::json!({
                "item_id": ext,
                "cue_code": c.code,
                "status": c.status,
                "start_tc": if c.status == CueStatus::Conceptual { serde_json::Value::Null } else { tb.tc_at(c.start_sample).into() },
                "end_tc": if c.status == CueStatus::Conceptual { serde_json::Value::Null } else { tb.tc_at(c.end_sample).into() },
                "identity": c.identity_id.as_ref().and_then(|i| s.identities.get(i)).map(|i| i.name.clone()),
            }))
        })
        .collect();
    serde_json::json!({
        "schema": "vc.sounddesign.breakdown-update",
        "version": 1,
        "frame_rate": s.project.frame_rate.label(),
        "items": items,
    })
}

// ---------------------------------------------------------------- Composer

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposerScore {
    pub schema: String,
    pub version: u32,
    pub title: String,
    /// WAV path, relative to the JSON file.
    pub file: String,
    pub start_tc: String,
    pub frame_rate: String,
    pub sample_rate: u32,
    #[serde(default)]
    pub tempo_map: Vec<ComposerTempo>,
    /// Optional scene number the cue is written against.
    #[serde(default)]
    pub sync_scene: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposerTempo {
    pub bar: u32,
    #[serde(default = "one")]
    pub beat: u32,
    pub tc: String,
    pub bpm: f32,
}

fn one() -> u32 {
    1
}

/// Import a timecoded score export. Never assumes music starts at 00:00:00:00;
/// every mismatch is recorded as a warning that QA turns into a blocking error.
pub fn import_score(s: &mut ProjectState, json_path: &Path) -> Result<String> {
    let sc: ComposerScore = serde_json::from_slice(&std::fs::read(json_path)?)?;
    if sc.schema != COMPOSER_SCHEMA || sc.version != 1 {
        return Err(invalid(format!("unsupported score schema {} v{}", sc.schema, sc.version)));
    }
    let rate = FrameRate::parse(&sc.frame_rate)?;
    let wav = json_path.parent().unwrap_or(Path::new(".")).join(&sc.file);
    let aid = import_wav(
        s,
        &wav,
        AssetMeta {
            kind: AssetKind::Score,
            category: Some(Category::Music),
            license: Some(License {
                name: "VC Composer score".into(),
                redistributable: true,
                source: "composer".into(),
                ..Default::default()
            }),
            loopable: Some(false),
            ..Default::default()
        },
    )?;
    let asset = s.asset(&aid)?.clone();
    let mut warnings = vec![];
    let proj = s.project.frame_rate;
    if rate != proj {
        warnings.push(format!("score frame rate {} differs from project {}", rate.label(), proj.label()));
    }
    if sc.sample_rate != s.project.sample_rate || asset.sample_rate != sc.sample_rate {
        warnings.push(format!(
            "score sample rate (declared {} Hz, file {} Hz) differs from project {} Hz",
            sc.sample_rate, asset.sample_rate, s.project.sample_rate
        ));
    }
    // Position by declared TC interpreted at the score's own rate, as absolute seconds.
    let declared_frames = rate.tc_to_frames(&sc.start_tc)?;
    let abs_samples = rate.sample_at_frame(declared_frames, s.project.sample_rate);
    let origin = s.tb().origin_since_midnight();
    let timeline_start = abs_samples - origin;
    if let Some(tr) = asset.time_reference {
        let file_pos = tr as i64 - origin;
        if (file_pos - timeline_start).abs() > 1 {
            let tb = s.tb();
            warnings.push(format!(
                "declared start {} disagrees with embedded BWF time reference {} ({} samples)",
                sc.start_tc,
                tb.tc_at(file_pos),
                file_pos - timeline_start
            ));
        }
    }
    if timeline_start < 0 {
        warnings.push(format!("score starts at {} before project start {}", sc.start_tc, proj.frames_to_tc(s.project.start_frame)));
    }
    if let Some(num) = &sc.sync_scene {
        match s.find_scene(num) {
            Ok(scene) if scene.start_sample != timeline_start => {
                let tb = s.tb();
                warnings.push(format!(
                    "score is written against scene {num} at {} but scene starts at {}",
                    tb.tc_at(timeline_start),
                    tb.tc_at(scene.start_sample)
                ));
            }
            Err(_) => warnings.push(format!("sync scene {num} not found")),
            _ => {}
        }
    }
    let tb = s.tb();
    let tempo_map = sc
        .tempo_map
        .iter()
        .map(|t| {
            Ok(TempoMark {
                bar: t.bar,
                beat: t.beat,
                sample: rate.sample_at_frame(rate.tc_to_frames(&t.tc)?, s.project.sample_rate) - origin,
                bpm: t.bpm,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    // Score lane
    let bus = crate::builder::bus_for_category(s, &Category::Music);
    let track_id = new_id("trk");
    let order = s.tracks.values().map(|t| t.order + 1).max().unwrap_or(0);
    s.tracks.insert(
        track_id.clone(),
        Track {
            id: track_id.clone(),
            name: format!("MUS • Score • {}", sc.title),
            scene_id: None,
            identity_id: None,
            category: Category::Music,
            bus_id: bus,
            channels: asset.channels.clamp(1, 2),
            order,
            folder: "Music/Score".into(),
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
    let clip_id = new_id("clp");
    s.clips.insert(
        clip_id.clone(),
        ClipInstance {
            id: clip_id,
            track_id: track_id.clone(),
            cue_id: None,
            asset_id: aid.clone(),
            timeline_start,
            source_offset: 0,
            length: asset.length_samples,
            gain_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            fill: FillMode::OneShot,
            muted: false,
            take_id: None,
            performance_event_id: None,
            sub_index: None,
            is_placeholder: false,
            history: vec![AssetHistoryEntry { asset_id: aid.clone(), origin: "composer".into(), at: now_unix() }],
        },
    );
    let _ = tb;
    let id = new_id("scr");
    s.scores.insert(
        id.clone(),
        ScoreImport {
            id: id.clone(),
            title: sc.title,
            asset_id: aid,
            declared_start_tc: sc.start_tc,
            declared_frame_rate: rate,
            declared_sample_rate: sc.sample_rate,
            timeline_start,
            track_id,
            tempo_map,
            warnings,
            imported_at: now_unix(),
        },
    );
    Ok(id)
}

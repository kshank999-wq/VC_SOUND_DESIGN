//! Receiving side of a handoff package: rebuild a timeline from the WAVs and
//! cue manifest alone, with no interchange adapter (spec §20 scenario 31).

use crate::error::{invalid, Result};
use crate::ids::{new_id, now_unix};
use crate::library::{import_wav, AssetMeta};
use crate::model::*;
use crate::state::{NewProject, ProjectState};
use crate::timecode::FrameRate;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Default, Serialize)]
pub struct ReconstructReport {
    pub tracks: usize,
    pub cues: usize,
    pub checksum_failures: Vec<String>,
}

/// Build a new project state from an export package directory.
pub fn reconstruct(pkg: &Path) -> Result<(ProjectState, ReconstructReport)> {
    let mut rep = ReconstructReport::default();
    if pkg.join("checksums.sha256").exists() {
        rep.checksum_failures = crate::export::verify_checksums(pkg)?;
    }
    let m: serde_json::Value = serde_json::from_slice(&std::fs::read(pkg.join("Manifests/cue_manifest.json"))?)?;
    if m["schema"] != "vc.sounddesign.handoff-manifest" {
        return Err(invalid("not a VC Sound Design handoff manifest"));
    }
    let p = &m["project"];
    let rate = FrameRate::new(
        p["frame_rate_num"].as_u64().unwrap_or(24) as u32,
        p["frame_rate_den"].as_u64().unwrap_or(1) as u32,
        p["drop_frame"].as_bool().unwrap_or(false),
    )?;
    let mut s = ProjectState::new(NewProject {
        name: format!("{} (reconstructed)", p["name"].as_str().unwrap_or("Handoff")),
        frame_rate: rate,
        sample_rate: p["sample_rate"].as_u64().unwrap_or(48_000) as u32,
        start_tc: p["start_tc"].as_str().unwrap_or("00:00:00:00").to_string(),
    })?;
    let bus_by_name = |s: &ProjectState, name: &str| s.buses.values().find(|b| b.name == name).map(|b| b.id.clone());
    let master = crate::mixer::master_bus(&s)?.id.clone();
    for t in m["tracks"].as_array().cloned().unwrap_or_default() {
        let file = pkg.join(t["file"].as_str().unwrap_or_default());
        let aid = import_wav(
            &mut s,
            &file,
            AssetMeta {
                kind: AssetKind::Imported,
                license: Some(License { name: "handoff package".into(), redistributable: true, ..Default::default() }),
                ..Default::default()
            },
        )?;
        let a = s.asset(&aid)?.clone();
        let bus = t["bus"].as_str().and_then(|b| bus_by_name(&s, b)).unwrap_or(master.clone());
        let tid = new_id("trk");
        s.tracks.insert(
            tid.clone(),
            Track {
                id: tid.clone(),
                name: t["name"].as_str().unwrap_or("Track").to_string(),
                scene_id: None,
                identity_id: None,
                category: Category::parse(t["category"].as_str().unwrap_or("fx").split('/').next_back().unwrap_or("fx")),
                bus_id: bus,
                channels: a.channels,
                order: t["order"].as_u64().unwrap_or(0) as u32,
                folder: t["folder"].as_str().unwrap_or_default().to_string(),
                gain_db: 0.0,
                pan: t["pan"].as_f64().unwrap_or(0.0) as f32,
                mute: t["muted"].as_bool().unwrap_or(false),
                solo: false,
                armed: false,
                frozen_asset_id: None,
                inserts: vec![],
                automation_mode: AutoMode::Read,
            },
        );
        let cid = new_id("clp");
        s.clips.insert(
            cid.clone(),
            ClipInstance {
                id: cid,
                track_id: tid,
                cue_id: None,
                asset_id: aid.clone(),
                timeline_start: t["start_sample"].as_i64().unwrap_or(0),
                source_offset: 0,
                length: a.length_samples,
                gain_db: 0.0,
                fade_in: 0,
                fade_out: 0,
                fill: FillMode::OneShot,
                muted: false,
                take_id: None,
                performance_event_id: None,
                sub_index: None,
                is_placeholder: false,
                history: vec![AssetHistoryEntry { asset_id: aid, origin: "handoff".into(), at: now_unix() }],
            },
        );
        rep.tracks += 1;
    }
    // scenes & cues (as markers of record; audio already lives in the track prints)
    for c in m["cues"].as_array().cloned().unwrap_or_default() {
        let num = c["scene"].as_str().unwrap_or("?").to_string();
        let start = c["start_sample"].as_i64().unwrap_or(0);
        let end = c["end_sample"].as_i64().unwrap_or(start);
        let scene_id = match s.scenes.values().find(|x| x.number == num) {
            Some(x) => x.id.clone(),
            None => crate::spotting::add_scene(&mut s, &num, "", start, end.max(start + 1))?,
        };
        let sc = s.scenes.get_mut(&scene_id).unwrap();
        sc.start_sample = sc.start_sample.min(start);
        sc.end_sample = sc.end_sample.max(end.max(start + 1));
        let identity_id = match c["identity"].as_str() {
            Some(n) => Some(match s.find_identity(n) {
                Ok(i) => i.id.clone(),
                Err(_) => crate::spotting::add_identity(
                    &mut s,
                    crate::spotting::NewIdentity {
                        name: n.into(),
                        category: Category::parse(c["category"].as_str().unwrap_or("fx").split('/').next_back().unwrap_or("fx")),
                        character: None,
                        surface: None,
                        prop: None,
                    },
                )?,
            }),
            None => None,
        };
        let id = c["cue_id"].as_str().map(String::from).unwrap_or_else(|| new_id("cue"));
        s.cues.insert(
            id.clone(),
            Cue {
                id,
                scene_id,
                code: c["code"].as_str().unwrap_or_default().into(),
                identity_id,
                category: Category::parse(c["category"].as_str().unwrap_or("fx").split('/').next_back().unwrap_or("fx")),
                event_type: serde_json::from_value(c["event_type"].clone()).unwrap_or(EventType::Point),
                lane_id: None,
                start_sample: start,
                end_sample: end,
                sub_events: serde_json::from_value(c["sub_events"].clone()).unwrap_or_default(),
                description: c["description"].as_str().unwrap_or_default().into(),
                intent: String::new(),
                script_ref_id: None,
                character: None,
                surface: None,
                prop: None,
                placeholder_asset_id: None,
                asset_override_id: None,
                timing_origin: TimingOrigin::Imported,
                review_status: ReviewStatus::Unreviewed,
                status: serde_json::from_value(c["status"].clone()).unwrap_or_default(),
                priority: Priority::Normal,
                owner: c["owner"].as_str().unwrap_or_default().into(),
                notes: c["notes"].as_str().unwrap_or_default().into(),
                marker_id: None,
                picture_id: None,
                external_id: None,
                pending_shift: None,
            },
        );
        rep.cues += 1;
    }
    Ok((s, rep))
}

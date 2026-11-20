//! `vcsd demo`: the spec §5 reference workflow, end to end, on generated media.

use std::path::Path;
use std::process::Command;
use vcsd_core::builder::{populate, PopulateOptions};
use vcsd_core::editing::{self, ReplaceScope, TakeImport};
use vcsd_core::export::{ExportOptions, ExportProfile};
use vcsd_core::library::{self, Query};
use vcsd_core::perform::{Pad, PadMap, PerformanceRecorder};
use vcsd_core::picture::{import_picture, PictureImport};
use vcsd_core::spotting::*;
use vcsd_core::*;

fn find(s: &ProjectState, q: &str) -> Vec<String> {
    library::search(s, &Query { text: q.into(), ..Default::default() }).iter().map(|a| a.id.clone()).collect()
}

/// Build the demo project; returns the progress log.
pub fn run(dir: &Path) -> Result<Vec<String>> {
    let mut log: Vec<String> = vec![];
    macro_rules! println {
        ($($t:tt)*) => { log.push(format!($($t)*)) };
    }
    if dir.join(layout::PROJECT_FILE).exists() {
        return Err(Error::Conflict(format!("{} already contains a project", dir.display())));
    }
    macro_rules! step {
        ($n:expr, $t:expr) => {
            log.push(format!("[{}] {}", $n, $t))
        };
    }
    step!(1, "Create project — 24 fps, 48 kHz, start 01:00:00:00");
    let mut s = Session::create(
        dir,
        NewProject { name: "Gravel Road".into(), frame_rate: FrameRate::FPS_24, sample_rate: 48_000, start_tc: "01:00:00:00".into() },
    )?;
    s.state.project.reel = "R1".into();
    s.state.project.contacts.push(Contact { role: "Sound supervisor".into(), name: "(enter name)".into(), email: String::new() });

    step!(2, "Import picture reference");
    let vid = dir.join("Picture/GravelRoad_v1.mp4");
    let made = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=640x360:r=24:d=60",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-timecode",
            "01:00:00:00",
        ])
        .arg(&vid)
        .status()
        .map(|st| st.success())
        .unwrap_or(false);
    if !made {
        std::fs::write(&vid, b"placeholder picture (ffmpeg not available)")?;
    }
    s.edit("picture", |st| {
        import_picture(
            st,
            PictureImport {
                path: &vid,
                label: "v1 (locked cut)".into(),
                start_tc: Some("01:00:00:00".into()),
                duration_frames: Some(24 * 60),
                change_notes: String::new(),
            },
        )
    })?;
    println!(
        "    {}",
        if made { "generated 60 s test picture with embedded TC" } else { "ffmpeg unavailable — using placeholder picture file" }
    );

    step!(3, "Install starter pack (procedural, owned, redistributable) and scenes");
    let pack = dir.join("Audio/LibraryRefs/VC_Starter_Pack");
    synth::write_starter_pack(&pack, 48_000)?;
    let (_, rep) = s.edit("pack", |st| library::import_pack(st, &pack, true))?;
    println!("    indexed {} placeholder sounds", rep.imported.len());
    for (n, t, a, z) in [
        ("12", "EXT. GRAVEL DRIVE - MORNING", "01:00:00:00", "01:00:20:00"),
        ("13", "INT. KITCHEN - CONTINUOUS", "01:00:20:00", "01:00:40:00"),
        ("14", "EXT. ROAD - DAY", "01:00:40:00", "01:01:00:00"),
    ] {
        let tb = s.state.tb();
        let (a, z) = (tb.sample_at_tc(a)?, tb.sample_at_tc(z)?);
        s.edit("scene", |st| add_scene(st, n, t, a, z))?;
    }

    step!(4, "Sound identities: Greg / Boots (gravel, alternating feet), Birds, Front Door, Kitchen Glass");
    let gravel = find(&s.state, "boot gravel");
    let greg = s.edit("id", |st| {
        let id = add_identity(
            st,
            NewIdentity {
                name: "Greg / Boots".into(),
                category: Category::Footsteps,
                character: Some("Greg".into()),
                surface: Some("Gravel".into()),
                prop: None,
            },
        )?;
        assign_identity_sound(st, &id, &gravel[0], gravel.clone(), VariationMode::AlternateFeet)?;
        Ok(id)
    })?;
    let birds = s.edit("id", |st| {
        add_identity(
            st,
            NewIdentity { name: "Birds / Morning".into(), category: Category::Background, character: None, surface: None, prop: None },
        )
    })?;
    let door = s.edit("id", |st| {
        add_identity(
            st,
            NewIdentity {
                name: "Front Door / Handle".into(),
                category: Category::Props,
                character: None,
                surface: None,
                prop: Some("Door handle".into()),
            },
        )
    })?;
    let glass = s.edit("id", |st| {
        add_identity(
            st,
            NewIdentity {
                name: "Kitchen / Glass".into(),
                category: Category::HardFx,
                character: None,
                surface: None,
                prop: Some("Glass".into()),
            },
        )
    })?;

    step!(5, "Live spotting with hotkeys / MIDI pads (simulated taps against picture)");
    let mut spot = LiveSpotter::new();
    let lane = |s: &Session, c: &str| s.state.find_lane(c).unwrap().id.clone();
    spot.bind(
        "key:f",
        SpotBinding {
            lane_id: lane(&s, "footsteps"),
            kind: EventType::Repeated,
            label: "Greg walks up the drive".into(),
            identity_id: Some(greg.clone()),
        },
    );
    spot.bind(
        "key:b",
        SpotBinding { lane_id: lane(&s, "bg"), kind: EventType::Range, label: "morning birds".into(), identity_id: Some(birds.clone()) },
    );
    spot.bind(
        "midi:10:38",
        SpotBinding {
            lane_id: lane(&s, "props"),
            kind: EventType::Repeated,
            label: "door handle jiggle".into(),
            identity_id: Some(door.clone()),
        },
    );
    spot.bind(
        "key:g",
        SpotBinding {
            lane_id: lane(&s, "fx"),
            kind: EventType::Point,
            label: "glass smashes on floor".into(),
            identity_id: Some(glass.clone()),
        },
    );
    let tb = s.state.tb();
    let t = |tc: &str| tb.sample_at_tc(tc).unwrap();
    spot.tap(&mut s.state, "key:b", t("01:00:00:00"))?;
    spot.tap(&mut s.state, "key:b", t("01:00:20:00"))?;
    for i in 0..10 {
        spot.tap(&mut s.state, "key:f", t("01:00:03:00") + i * 26_000 + (i % 3) * 300)?;
    }
    spot.finish(&mut s.state, "key:f", t("01:00:09:00"))?;
    for i in 0..5 {
        spot.midi(&mut s.state, &[0x99, 38, 110], t("01:00:12:00") + i * 9_000)?;
    }
    spot.finish(&mut s.state, "midi:10:38", t("01:00:13:00"))?;
    spot.tap(&mut s.state, "key:g", t("01:00:27:14"))?;
    let l_bg = lane(&s, "bg");
    let l_fx = lane(&s, "fx");
    let l_ft = lane(&s, "footsteps");
    s.edit("markers", |st| {
        add_marker(
            st,
            NewMarker {
                lane_id: l_bg.clone(),
                scene_id: None,
                kind: EventType::Range,
                start: t("01:00:20:00"),
                end: t("01:00:40:00"),
                label: "kitchen room tone".into(),
                identity_id: None,
                sub_events: vec![],
                origin: TimingOrigin::Manual,
            },
        )?;
        add_marker(
            st,
            NewMarker {
                lane_id: l_ft.clone(),
                scene_id: None,
                kind: EventType::Range,
                start: t("01:00:22:00"),
                end: t("01:00:25:00"),
                label: "Greg crosses kitchen".into(),
                identity_id: Some(greg.clone()),
                sub_events: vec![],
                origin: TimingOrigin::Manual,
            },
        )?;
        add_marker(
            st,
            NewMarker {
                lane_id: l_bg.clone(),
                scene_id: None,
                kind: EventType::Range,
                start: t("01:00:40:00"),
                end: t("01:01:00:00"),
                label: "wind exterior".into(),
                identity_id: None,
                sub_events: vec![],
                origin: TimingOrigin::Manual,
            },
        )?;
        add_marker(
            st,
            NewMarker {
                lane_id: l_fx.clone(),
                scene_id: None,
                kind: EventType::Range,
                start: t("01:00:46:00"),
                end: t("01:00:50:00"),
                label: "car pass by".into(),
                identity_id: None,
                sub_events: vec![],
                origin: TimingOrigin::Manual,
            },
        )?;
        add_marker(
            st,
            NewMarker {
                lane_id: l_ft.clone(),
                scene_id: None,
                kind: EventType::Range,
                start: t("01:00:51:00"),
                end: t("01:00:55:00"),
                label: "Greg walks to road".into(),
                identity_id: Some(greg.clone()),
                sub_events: vec![],
                origin: TimingOrigin::Manual,
            },
        )
    })?;
    println!("    {} markers across {} lanes", s.state.markers.len(), s.state.lanes.len());

    step!(6, "Build cue chart and populate placeholders");
    let new = s.edit("cues", |st| markers_to_cues(st, None))?;
    let rep = s.edit("populate", |st| populate(st, &PopulateOptions::default()))?;
    println!("    {} cues · {} clips on {} tracks", new.len(), rep.clips_created, rep.tracks_created.len());
    for w in &rep.warnings {
        println!("    ⚠ {}: {}", w.cue_code, w.message);
    }

    step!(7, "Scene 13 is a wooden kitchen floor: scene-local override for Greg's boots");
    let kitchen = s.state.cues.values().find(|c| c.description == "Greg crosses kitchen").unwrap().id.clone();
    let wood = find(&s.state, "shoe wood")[0].clone();
    s.edit("override", |st| editing::replace_cue_asset(st, &kitchen, &wood, ReplaceScope::IdentityInScene, false))?;

    step!(8, "Foley artist performs scene 14 steps on a pad (velocity, round-robin) — take 1");
    let road = s.state.cues.values().find(|c| c.description == "Greg walks to road").unwrap().id.clone();
    let track = s.state.clips_for_cue(&road)[0].track_id.clone();
    let mut pads = PadMap::default();
    pads.set("pad:1", Pad { name: "Greg gravel".into(), assets: gravel.clone(), mode: VariationMode::RoundRobin, ..Default::default() });
    let mut rec = PerformanceRecorder::new(&track, Some(&road), pads);
    for (i, v) in [96u8, 80, 110, 70, 100, 88, 120, 76].iter().enumerate() {
        rec.trigger("pad:1", t("01:00:51:06") + i as i64 * 23_500, *v)?;
    }
    s.edit("perform", |st| rec.commit(st))?;

    step!(9, "Record a new door-handle take and drop it on one jiggle instance");
    let take_wav = dir.join("Audio/RecordedFoley/door_handle_take01.wav");
    wav::write(&take_wav, &synth::handle_rattle(48_000, 7), wav::SampleFormat::Int24, None)?;
    let door_cue = s.state.cues.values().find(|c| c.description == "door handle jiggle").unwrap().id.clone();
    let inst = s.state.clips_for_cue(&door_cue)[2].id.clone();
    let pos = s.state.clips[&inst].timeline_start;
    s.edit("take", |st| {
        let take = editing::add_take(
            st,
            TakeImport {
                wav: &take_wav,
                cue_id: Some(door_cue.clone()),
                track_id: None,
                slate: "12-3".into(),
                timeline_start: Some(pos),
                sync_offset: 0,
            },
        )?;
        editing::use_take_for_clip(st, &inst, &take)
    })?;

    step!(10, "Composer score at 01:00:20:00 and AI timing assist (suggestions only)");
    let score_dir = dir.join("Audio/Imported/Score");
    std::fs::create_dir_all(&score_dir)?;
    let mut music = synth::room_tone(48_000, 10.0);
    for (i, v) in music.data.iter_mut().enumerate() {
        let tt = (i / 2) as f32 / 48_000.0;
        *v += 0.08 * (2.0 * std::f32::consts::PI * 220.0 * tt).sin() * (0.5 + 0.5 * (tt * 1.5).sin());
    }
    wav::write(
        &score_dir.join("1M1.wav"),
        &music,
        wav::SampleFormat::Int24,
        Some(&wav::Bext { time_reference: (s.state.tb().origin_since_midnight() + t("01:00:20:00")) as u64, ..Default::default() }),
    )?;
    std::fs::write(score_dir.join("1M1.json"), serde_json::json!({"schema": "vc.composer.score", "version": 1, "title": "1M1 Kitchen", "file": "1M1.wav", "start_tc": "01:00:20:00", "frame_rate": "24", "sample_rate": 48000, "sync_scene": "13", "tempo_map": [{"bar": 1, "tc": "01:00:20:00", "bpm": 90.0}]}).to_string())?;
    s.edit("score", |st| integrations::import_score(st, &score_dir.join("1M1.json")))?;
    s.state.project.ai.enabled = true;
    let glass_cue = s.state.cues.values().find(|c| c.identity_id.as_deref() == Some(glass.as_str())).unwrap().id.clone();
    let assist: Box<dyn ai::TimingAssistant> =
        if made { Box::new(ai::VisualOnsetAssistant::default()) } else { Box::new(ai::CadenceHeuristic { step_secs: 0.5 }) };
    match s.edit("ai", |st| ai::run(st, assist.as_ref(), std::slice::from_ref(&glass_cue))) {
        Ok(ids) => {
            for id in ids {
                let p = &s.state.proposals[&id];
                println!(
                    "    proposal for {}: conf {:.2} — {} (left pending for review)",
                    s.state.cues[&p.cue_id].code, p.confidence, p.reason
                );
            }
        }
        Err(e) => println!("    AI assist skipped: {e}"),
    }

    step!(11, "Rough mix: Foley -2 dB with high-pass insert, scene summary");
    for tr in s.state.tracks.values_mut() {
        if tr.category == Category::Footsteps {
            tr.gain_db = -2.0;
            tr.inserts = vec![Insert::HighPass { hz: 60.0, q: 0.7 }];
        }
    }
    s.save()?;
    let sc12 = s.state.find_scene("12")?.id.clone();
    let sum = report::scene_summary(&s.state, &sc12)?;
    println!("    scene 12: {} cues, ambience gaps {:?}", sum.cues, sum.ambience_gaps);
    let act = report::active_at(&s.state, t("01:00:12:10"));
    println!("    active at 01:00:12:10: {}", act.iter().map(|a| a.code.clone()).collect::<Vec<_>>().join(", "));

    step!(12, "Export: Resolve/Fairlight package and Pro studio handoff (with 500 ms handles)");
    let date = ids::date_from_unix(ids::now_unix());
    let root = dir.to_path_buf();
    let r1 = s.edit("export", |st| export::export(st, Some(&root), &ExportOptions::new(ExportProfile::Resolve, &date)))?;
    let mut pro = ExportOptions::new(ExportProfile::ProStudio, &date);
    pro.handle_samples = 24_000;
    let r2 = s.edit("export", |st| export::export(st, Some(&root), &pro))?;
    s.save()?;
    for r in [&r1, &r2] {
        println!(
            "    {} files → {} (QA: {} errors, {} warnings)",
            r.files.len(),
            r.dir.display(),
            qa::count(&r.qa, qa::Severity::Error),
            qa::count(&r.qa, qa::Severity::Warning)
        );
    }
    println!("Done: {}", dir.display());
    Ok(log)
}

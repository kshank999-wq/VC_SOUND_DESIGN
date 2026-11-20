#![allow(clippy::doc_lazy_continuation)]
//! Spec §20 acceptance scenarios (22–31), automated.

mod common;
use common::*;
use vcsd_core::builder::{populate, PopulateOptions, TrackPolicy};
use vcsd_core::editing::*;
use vcsd_core::export::{export, ExportOptions, ExportProfile};
use vcsd_core::mixer::{Mixer, RenderOptions};
use vcsd_core::perform::{Pad, PadMap, PerformanceRecorder};
use vcsd_core::picture::{import_picture, PictureImport};
use vcsd_core::spotting::*;
use vcsd_core::*;

/// 22. Import 24 fps reference at non-zero start TC, add a one-frame glass
/// event, save/reopen; the event remains on the exact frame.
#[test]
fn s22_one_frame_event_survives_save_reopen() {
    let (mut s, root) = session("s22", "24", "01:00:00:00");
    let vid = fake_video(&root);
    s.edit("pic", |st| {
        import_picture(
            st,
            PictureImport {
                path: &vid,
                label: "v1".into(),
                start_tc: Some("01:00:00:00".into()),
                duration_frames: Some(24 * 60),
                change_notes: String::new(),
            },
        )
    })
    .unwrap();
    scene(&mut s, "1", "01:00:00:00", "01:00:30:00");
    let at = tc(&s, "01:00:05:12");
    let mid = marker(&mut s, "fx", EventType::Point, at + 777, at + 777, "glass breaks", None);
    // snap: live spotting would snap; here we snap explicitly
    let snapped = s.state.tb().snap(at + 777);
    s.edit("snap", |st| update_marker(st, &mid, snapped, snapped, None)).unwrap();
    s.edit("cues", |st| markers_to_cues(st, None)).unwrap();
    s.save().unwrap();
    let s2 = Session::open(&root).unwrap();
    let cue = s2.state.cues.values().next().unwrap();
    assert_eq!(cue.start_sample, at);
    assert_eq!(cue.end_sample, at, "point event");
    assert_eq!(s2.state.tb().tc_at(cue.start_sample), "01:00:05:12");
    assert_eq!(s2.state.tb().frame_at(cue.start_sample), FrameRate::FPS_24.tc_to_frames("01:00:05:12").unwrap());
    assert_eq!(s2.state.project.frame_rate, FrameRate::FPS_24);
    assert_eq!(s2.state.current_picture().unwrap().source_start_frame, 86_400);
}

/// 23. Greg's boots across three scenes; generic gravel assigned; one scene
/// edited to custom Foley; identity discoverable everywhere; override isolated.
#[test]
fn s23_identity_across_scenes_with_isolated_override() {
    let (mut s, _root) = session("s23", "24", "01:00:00:00");
    with_pack(&mut s);
    let gravel = assets(&s, "boot gravel");
    assert_eq!(gravel.len(), 4);
    let custom = asset(&s, "heel concrete 01");
    let greg = s
        .edit("id", |st| {
            add_identity(
                st,
                NewIdentity {
                    name: "Greg / Boots".into(),
                    category: Category::Footsteps,
                    character: Some("Greg".into()),
                    surface: Some("Gravel".into()),
                    prop: None,
                },
            )
        })
        .unwrap();
    let g = gravel.clone();
    s.edit("assign", |st| assign_identity_sound(st, &greg, &g[0], g.clone(), VariationMode::RoundRobin)).unwrap();
    let scenes = [("1", "01:00:00:00", "01:00:20:00"), ("2", "01:00:20:00", "01:00:40:00"), ("3", "01:00:40:00", "01:01:00:00")];
    let mut cue_by_scene = vec![];
    for (n, a, z) in scenes {
        scene(&mut s, n, a, z);
        let st = tc(&s, a) + 48_000 * 2;
        marker(&mut s, "footsteps", EventType::Range, st, st + 48_000 * 3, "Greg walks", Some(&greg));
        let ids = s.edit("cues", |x| markers_to_cues(x, None)).unwrap();
        cue_by_scene.push(ids[0].clone());
    }
    let rep = s.edit("populate", |st| populate(st, &PopulateOptions::default())).unwrap();
    assert_eq!(rep.tracks_created.len(), 3, "one scene-local track per scene");
    assert!(s.state.tracks.values().all(|t| t.name == "FOLEY • Greg • Boots"));
    // custom Foley in scene 2 only
    s.edit("replace", |st| replace_cue_asset(st, &cue_by_scene[1], &custom, ReplaceScope::IdentityInScene, false)).unwrap();
    // discoverable across all scenes
    let all = cues_for_identity(&s.state, &greg);
    assert_eq!(all.len(), 3);
    let scene_nums: Vec<String> = all.iter().map(|c| s.state.scenes[&c.scene_id].number.clone()).collect();
    assert_eq!(scene_nums, vec!["1", "2", "3"]);
    // override isolated
    for (i, cid) in cue_by_scene.iter().enumerate() {
        let clips = s.state.clips_for_cue(cid);
        assert!(!clips.is_empty());
        for c in clips {
            if i == 1 {
                assert_eq!(c.asset_id, custom);
            } else {
                assert!(gravel.contains(&c.asset_id), "scene {} must keep gravel", i + 1);
            }
        }
    }
    assert_eq!(s.state.identities[&greg].default_asset_id.as_deref(), Some(gravel[0].as_str()));
    // ripple needs explicit confirmation
    assert!(matches!(
        s.edit("ripple", |st| replace_cue_asset(st, &cue_by_scene[0], &custom, ReplaceScope::IdentityEverywhere, false)),
        Err(Error::Conflict(_))
    ));
}

/// 24. Footsteps range + eight MIDI taps → eight editable events within tolerance.
#[test]
fn s24_eight_midi_taps_become_editable_events() {
    let (mut s, _root) = session("s24", "23.976", "00:59:58:00");
    with_pack(&mut s);
    scene(&mut s, "4", "01:00:00:00", "01:00:30:00");
    let greg = s
        .edit("id", |st| {
            add_identity(
                st,
                NewIdentity { name: "Greg / Boots".into(), category: Category::Footsteps, character: None, surface: None, prop: None },
            )
        })
        .unwrap();
    let g = assets(&s, "boot gravel");
    s.edit("assign", |st| assign_identity_sound(st, &greg, &g[0], g.clone(), VariationMode::AlternateFeet)).unwrap();
    let mut spot = LiveSpotter::new();
    spot.bind(
        "midi:10:36",
        SpotBinding {
            lane_id: lane(&s, "footsteps"),
            kind: EventType::Repeated,
            label: "Greg walks".into(),
            identity_id: Some(greg.clone()),
        },
    );
    let base = tc(&s, "01:00:03:00");
    // human-ish taps: ~0.52 s apart with jitter
    let taps: Vec<i64> = (0..8).map(|i| base + i * 25_000 + [0, 310, -220, 140, 0, -90, 260, 30][i as usize]).collect();
    for &t in &taps {
        let out = spot.midi(&mut s.state, &[0x99, 36, 100], t).unwrap();
        assert!(matches!(out, Some(SpotOutcome::SubEvent(_))));
    }
    // note-off and unmapped notes are ignored
    assert_eq!(spot.midi(&mut s.state, &[0x89, 36, 0], base).unwrap(), None);
    assert_eq!(spot.midi(&mut s.state, &[0x99, 40, 100], base).unwrap(), None);
    let mid = spot.finish(&mut s.state, "midi:10:36", taps[7] + 20_000).unwrap();
    let m = &s.state.markers[&mid];
    assert_eq!(m.sub_events.len(), 8);
    let one_frame = s.state.one_frame();
    for (e, t) in m.sub_events.iter().zip(&taps) {
        assert!((t - e).abs() < one_frame, "within one frame (snap-to-frame)");
    }
    s.edit("cues", |st| markers_to_cues(st, None)).unwrap();
    let rep = s.edit("populate", |st| populate(st, &PopulateOptions::default())).unwrap();
    assert_eq!(rep.clips_created, 8);
    let cue_id = s.state.markers[&mid].cue_id.clone().unwrap();
    let clips: Vec<String> = s.state.clips_for_cue(&cue_id).iter().map(|c| c.id.clone()).collect();
    assert_eq!(clips.len(), 8);
    // editable: move the 3rd step 2 frames later
    let c3 = clips[2].clone();
    let new_pos = s.state.clips[&c3].timeline_start + 2 * one_frame;
    s.edit("move", |st| move_clip(st, &c3, new_pos)).unwrap();
    assert_eq!(s.state.clips[&c3].timeline_start, new_pos);
    // alternate feet: consecutive steps use different variants
    let a0 = &s.state.clips_for_cue(&cue_id)[0].asset_id;
    let a1 = &s.state.clips_for_cue(&cue_id)[1].asset_id;
    assert_ne!(a0, a1);

    // Same scenario through the perform-to-picture recorder (no quantize, sample exact)
    let track = s.state.clips[&c3].track_id.clone();
    let mut pads = PadMap::default();
    pads.set("midi:10:36", Pad { name: "boots".into(), assets: g.clone(), ..Default::default() });
    let mut rec = PerformanceRecorder::new(&track, None, pads);
    for &t in &taps {
        assert!(rec.midi(&[0x99, 36, 90], t).unwrap());
    }
    let pid = s.edit("perform", |st| rec.commit(st)).unwrap();
    let perf_clips: Vec<i64> = {
        let mut v: Vec<i64> = s
            .state
            .clips
            .values()
            .filter(|c| c.performance_event_id.as_ref().is_some_and(|e| s.state.performance_events[e].performance_id == pid))
            .map(|c| c.timeline_start)
            .collect();
        v.sort();
        v
    };
    assert_eq!(perf_clips, taps, "performed triggers are sample-exact with quantize off");
}

/// 25. Repeated door-jiggle → placeholders → replace one instance with a
/// recorded take; original source persists in history.
#[test]
fn s25_replace_instance_with_recorded_take_keeps_history() {
    let (mut s, root) = session("s25", "25", "10:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "7", "10:00:00:00", "10:00:20:00");
    let a = tc(&s, "10:00:02:00");
    let l = lane(&s, "props");
    let subs = vec![a, a + 12_000, a + 24_000, a + 36_000];
    s.edit("mrk", |st| {
        add_marker(
            st,
            NewMarker {
                lane_id: l,
                scene_id: None,
                kind: EventType::Repeated,
                start: a,
                end: a + 48_000,
                label: "door handle jiggle".into(),
                identity_id: None,
                sub_events: subs.clone(),
                origin: TimingOrigin::Manual,
            },
        )
    })
    .unwrap();
    let cue = s.edit("cues", |st| markers_to_cues(st, None)).unwrap()[0].clone();
    let rep = s.edit("populate", |st| populate(st, &PopulateOptions::default())).unwrap();
    assert_eq!(rep.clips_created, 4);
    let placeholder = s.state.clips_for_cue(&cue)[0].asset_id.clone();
    assert!(s.state.assets[&placeholder].name.contains("door_handle"), "picked by description: {}", s.state.assets[&placeholder].name);
    // record a new take (simulated capture written to RecordedFoley)
    let take_wav = root.join("Audio/RecordedFoley/door_jiggle_take1.wav");
    wav::write(&take_wav, &synth::handle_rattle(48_000, 2), wav::SampleFormat::Int24, None).unwrap();
    let target = s.state.clips_for_cue(&cue)[2].id.clone();
    let pos = s.state.clips[&target].timeline_start;
    let take = s
        .edit("take", |st| {
            add_take(
                st,
                TakeImport {
                    wav: &take_wav,
                    cue_id: Some(cue.clone()),
                    track_id: None,
                    slate: "7-1".into(),
                    timeline_start: Some(pos),
                    sync_offset: 0,
                },
            )
        })
        .unwrap();
    s.edit("use take", |st| use_take_for_clip(st, &target, &take)).unwrap();
    let c = &s.state.clips[&target];
    assert_eq!(c.timeline_start, pos);
    assert_eq!(c.history.first().unwrap().asset_id, placeholder);
    assert_eq!(c.history.first().unwrap().origin, "placeholder");
    assert_eq!(c.history.last().unwrap().asset_id, s.state.takes[&take].asset_id);
    assert_eq!(s.state.cues[&cue].status, CueStatus::Recorded);
    // other instances untouched
    for (i, cl) in s.state.clips_for_cue(&cue).iter().enumerate() {
        if i != 2 {
            assert_eq!(cl.asset_id, placeholder);
        }
    }
    // revert keeps the take available
    s.edit("revert", |st| revert_to_placeholder(st, &cue)).unwrap();
    assert_eq!(s.state.clips[&target].asset_id, placeholder);
    assert!(s.state.takes.contains_key(&take));
    assert!(s.state.clips[&target].history.len() >= 3);
    // undo restores the take on the instance
    s.undo().unwrap();
    assert_eq!(s.state.clips[&target].asset_id, s.state.takes[&take].asset_id);
}

/// 26. Run AI alignment on marked footsteps, reject proposals; no clip moves.
#[test]
fn s26_rejected_ai_proposals_change_nothing() {
    let (mut s, _root) = session("s26", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "2", "01:00:00:00", "01:00:20:00");
    let a = tc(&s, "01:00:01:00");
    marker(&mut s, "footsteps", EventType::Range, a, a + 48_000 * 4, "walks to door", None);
    let cue = s.edit("cues", |st| markers_to_cues(st, None)).unwrap()[0].clone();
    s.edit("populate", |st| populate(st, &PopulateOptions::default())).unwrap();
    let before = clip_times(&s.state);
    let cue_before = serde_json::to_string(&s.state.cues[&cue]).unwrap();
    let assistant = ai::CadenceHeuristic { step_secs: 0.45 };
    // disabled by default: offline/manual workflow intact
    assert!(matches!(s.edit("ai", |st| ai::run(st, &assistant, std::slice::from_ref(&cue))), Err(Error::Disabled(_))));
    s.state.project.ai.enabled = true;
    let props = s.edit("ai", |st| ai::run(st, &assistant, std::slice::from_ref(&cue))).unwrap();
    assert!(!props.is_empty());
    assert!(s.state.proposals[&props[0]].confidence < 0.5, "no visual info → low confidence");
    assert_eq!(clip_times(&s.state), before, "proposals alone never edit");
    for p in &props {
        s.edit("reject", |st| ai::reject(st, p)).unwrap();
    }
    assert_eq!(clip_times(&s.state), before);
    assert_eq!(serde_json::to_string(&s.state.cues[&cue]).unwrap(), cue_before);
    // accepting a fresh proposal does move things (sanity)
    let p2 = s.edit("ai", |st| ai::run(st, &assistant, std::slice::from_ref(&cue))).unwrap();
    s.edit("accept", |st| ai::accept(st, &p2[0], None)).unwrap();
    assert_eq!(s.state.cues[&cue].timing_origin, TimingOrigin::AiSuggested);
    assert_ne!(clip_times(&s.state), before);
}

fn score_fixture(dir: &std::path::Path, declared: &str, bwf_tc_frames_24: i64, rate: &str) -> std::path::PathBuf {
    let wav = dir.join("cue_1m1.wav");
    let tr = (bwf_tc_frames_24 * 2000) as u64; // 24fps @ 48k
    wav::write(
        &wav,
        &synth::room_tone(48_000, 3.0),
        wav::SampleFormat::Int24,
        Some(&wav::Bext { time_reference: tr, ..Default::default() }),
    )
    .unwrap();
    let json = dir.join("cue_1m1.json");
    std::fs::write(&json, serde_json::json!({"schema": "vc.composer.score", "version": 1, "title": "1M1", "file": "cue_1m1.wav", "start_tc": declared, "frame_rate": rate, "sample_rate": 48000, "tempo_map": [{"bar": 1, "tc": declared, "bpm": 92.0}]}).to_string()).unwrap();
    json
}

/// 27. Score with non-zero start TC; a deliberate TC mismatch is detected before final export.
#[test]
fn s27_score_timecode_mismatch_blocks_export() {
    let (mut s, root) = session("s27", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:20:00");
    let ok = score_fixture(&root.join("Audio/Imported"), "01:00:04:00", 86_400 + 96, "24");
    let sid = s.edit("score", |st| integrations::import_score(st, &ok)).unwrap();
    assert!(s.state.scores[&sid].warnings.is_empty(), "{:?}", s.state.scores[&sid].warnings);
    assert_eq!(s.state.scores[&sid].timeline_start, 4 * 48_000, "placed at declared non-zero TC");
    // deliberate mismatch: declared 01:00:06:00 but file stamped 01:00:06:12
    let dir2 = root.join("Audio/Imported/bad");
    std::fs::create_dir_all(&dir2).unwrap();
    let bad = score_fixture(&dir2, "01:00:06:00", 86_400 + 144 + 12, "24");
    // make content differ so it is a distinct asset
    let mut b = synth::wind_loop(48_000, 2.0);
    b.data[0] = 0.01;
    wav::write(
        &dir2.join("cue_1m1.wav"),
        &b,
        wav::SampleFormat::Int24,
        Some(&wav::Bext { time_reference: ((86_400 + 156) * 2000) as u64, ..Default::default() }),
    )
    .unwrap();
    let sid2 = s.edit("score", |st| integrations::import_score(st, &bad)).unwrap();
    assert!(s.state.scores[&sid2].warnings.iter().any(|w| w.contains("disagrees")), "{:?}", s.state.scores[&sid2].warnings);
    let qa = qa::run(&s.state, &Default::default());
    assert!(qa.iter().any(|i| i.code == "score_sync" && i.severity == qa::Severity::Error));
    let mut o = ExportOptions::new(ExportProfile::Resolve, "2026-11-20");
    let r = export(&mut s.state, Some(&root), &o);
    assert!(matches!(r, Err(Error::QaBlocked(_))), "final export blocked");
    o.force = true;
    let r = export(&mut s.state, Some(&root), &o).unwrap();
    assert!(r.qa.iter().any(|i| i.code == "score_sync"), "forced export still documents the issue");
}

/// 28. Landscape report for overlapping footsteps/birds/car; at a queried
/// frame report all active identities with exact in/out TC.
#[test]
fn s28_overlap_report_and_playhead_query() {
    let (mut s, root) = session("s28", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "9", "01:00:00:00", "01:00:30:00");
    let mut ids = vec![];
    for (name, cat) in
        [("Greg / Boots", Category::Footsteps), ("Birds / Morning", Category::Background), ("Car / Pass-by", Category::HardFx)]
    {
        ids.push(
            s.edit("id", |st| {
                add_identity(st, NewIdentity { name: name.into(), category: cat, character: None, surface: None, prop: None })
            })
            .unwrap(),
        );
    }
    marker_tc(&mut s, "footsteps", EventType::Range, "01:00:02:00", "01:00:08:00", "Greg walks", Some(&ids[0]));
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:20:00", "birds", Some(&ids[1]));
    marker_tc(&mut s, "fx", EventType::Range, "01:00:05:00", "01:00:09:00", "car passes", Some(&ids[2]));
    s.edit("cues", |st| markers_to_cues(st, None)).unwrap();
    s.edit("populate", |st| populate(st, &PopulateOptions::default())).unwrap();
    let q = tc(&s, "01:00:06:10");
    let active = report::active_at(&s.state, q);
    let idents: Vec<&str> = active.iter().map(|a| a.identity.as_str()).collect();
    for want in ["Greg / Boots", "Birds / Morning", "Car / Pass-by"] {
        assert!(idents.contains(&want), "{want} active at 01:00:06:10: {idents:?}");
    }
    let car = active.iter().find(|a| a.identity == "Car / Pass-by").unwrap();
    assert_eq!((car.start_tc.as_str(), car.end_tc.as_str()), ("01:00:05:00", "01:00:09:00"));
    let birds = active.iter().find(|a| a.identity.starts_with("Birds")).unwrap();
    assert_eq!(birds.buses, vec!["Backgrounds/Atmos".to_string()]);
    // nothing but birds after the car leaves
    let later = report::active_at(&s.state, tc(&s, "01:00:12:00"));
    assert_eq!(later.iter().filter(|a| a.cue_id.is_some()).count(), 1);
    // landscape PDF + deterministic CSV
    let scene_id = s.state.find_scene("9").unwrap().id.clone();
    let pdf = pdf::cue_report(&s.state, Some(&scene_id), "2026-11-20");
    assert!(pdf.starts_with(b"%PDF-1.4"));
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/MediaBox [0 0 792 612]"), "landscape");
    assert!(text.contains("Page 1 of"));
    std::fs::write(root.join("Reports/scene9.pdf"), &pdf).unwrap();
    let rows = report::cue_rows(&s.state, Some(&scene_id));
    assert_eq!(report::rows_to_csv(&s.state, &rows), report::rows_to_csv(&s.state, &report::cue_rows(&s.state, Some(&scene_id))));
    assert!(report::rows_to_csv(&s.state, &rows).starts_with("# frame_rate=24,drop_frame=false,sample_rate=48000"));
}

fn track_print(st: &ProjectState, tid: &str, a: i64, n: usize) -> wav::AudioBuffer {
    Mixer::new(st, RenderOptions { honor_solo: false, ..Default::default() }).render_track_print(tid, a, n).unwrap()
}

/// 29 & 31. Export aligned mono Foley + stereo ambience; rebuild a timeline
/// from WAVs + manifest only; waveforms align within one sample.
#[test]
fn s29_s31_export_reimport_alignment_and_reconstruction() {
    let (mut s, root) = session("s29", "23.976", "00:59:59:00");
    with_pack(&mut s);
    scene(&mut s, "3", "01:00:00:00", "01:00:12:00");
    let greg = s
        .edit("id", |st| {
            add_identity(
                st,
                NewIdentity { name: "Greg / Boots".into(), category: Category::Footsteps, character: None, surface: None, prop: None },
            )
        })
        .unwrap();
    let g = assets(&s, "boot gravel");
    s.edit("assign", |st| assign_identity_sound(st, &greg, &g[0], g.clone(), VariationMode::RoundRobin)).unwrap();
    marker_tc(&mut s, "footsteps", EventType::Range, "01:00:01:00", "01:00:05:07", "Greg walks", Some(&greg));
    marker_tc(&mut s, "props", EventType::Point, "01:00:06:03", "01:00:06:03", "door handle rattle", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:12:00", "birds morning", None);
    s.edit("cues", |st| markers_to_cues(st, None)).unwrap();
    s.edit("populate", |st| populate(st, &PopulateOptions { policy: TrackPolicy::IdentityPerScene, ..Default::default() })).unwrap();
    // mix moves that must survive in the track prints
    let foley_track = s.state.tracks.values().find(|t| t.category == Category::Footsteps).unwrap().id.clone();
    s.state.tracks.get_mut(&foley_track).unwrap().gain_db = -3.0;
    s.edit("auto", |st| set_automation(st, &foley_track, AutomationParam::VolumeDb, 48_000 * 2, 0.0)).unwrap();
    s.edit("auto", |st| set_automation(st, &foley_track, AutomationParam::VolumeDb, 48_000 * 4, -12.0)).unwrap();
    let out = root.join("Exports/test_resolve");
    let mut o = ExportOptions::new(ExportProfile::Resolve, "2026-11-20");
    o.out_dir = Some(out.clone());
    o.format = wav::SampleFormat::Int24;
    let res = export(&mut s.state, Some(&root), &o).unwrap();
    assert_eq!(qa::count(&res.qa, qa::Severity::Error), 0, "{:?}", res.qa);
    assert!(export::verify_checksums(&out).unwrap().is_empty());
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("Manifests/cue_manifest.json")).unwrap()).unwrap();
    let tracks = manifest["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 3);
    // mono foley, stereo ambience
    for t in tracks {
        let info = wav::read_info(&out.join(t["file"].as_str().unwrap())).unwrap();
        let expect = if t["category"].as_str().unwrap().starts_with("Backgrounds") { 2 } else { 1 };
        assert_eq!(info.channels, expect, "{}", t["name"]);
        assert_eq!(info.bext.unwrap().time_reference as i64, s.state.tb().origin_since_midnight() + t["start_sample"].as_i64().unwrap());
    }
    // 31: reconstruct from WAVs + manifest only
    let (rs, rep) = handoff::reconstruct(&out).unwrap();
    assert!(rep.checksum_failures.is_empty());
    assert_eq!(rep.tracks, 3);
    assert_eq!(rep.cues, s.state.cues.len());
    let mut names_a: Vec<String> = s.state.tracks.values().map(|t| t.name.clone()).collect();
    let mut names_b: Vec<String> = rs.tracks.values().map(|t| t.name.clone()).collect();
    names_a.sort();
    names_b.sort();
    assert_eq!(names_a, names_b, "track layout reconstructed");
    for t in rs.tracks.values() {
        let orig = s.state.tracks.values().find(|x| x.name == t.name).unwrap();
        assert_eq!(rs.buses[&t.bus_id].name, s.state.buses[&orig.bus_id].name, "bus mapping");
    }
    for c in s.state.cues.values() {
        let rc = &rs.cues[&c.id];
        assert_eq!((rc.code.as_str(), rc.start_sample, rc.end_sample), (c.code.as_str(), c.start_sample, c.end_sample));
    }
    // 29: waveform alignment within one sample against source
    let (a, z) = export::resolve_range(&s.state, &export::ExportRange::All).unwrap();
    let n = (z - a) as usize;
    for t in rs.tracks.values() {
        let orig = s.state.tracks.values().find(|x| x.name == t.name).unwrap();
        let src = track_print(&s.state, &orig.id, a, n);
        let back = track_print(&rs, &t.id, a, n);
        assert_eq!(src.channels, back.channels);
        let tol = 1.0 / 8_388_607.0 * 1.01;
        let max_err = src.data.iter().zip(&back.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(max_err <= tol + 1e-7, "{}: max err {max_err}", t.name);
        // lag check: best correlation at 0 among ±3 samples
        let ch = src.channels as usize;
        let corr = |lag: i64| -> f64 {
            (0..n as i64)
                .filter_map(|i| {
                    let j = i + lag;
                    (j >= 0 && (j as usize) < n).then(|| src.data[i as usize * ch] as f64 * back.data[j as usize * ch] as f64)
                })
                .sum()
        };
        let best = (-3..=3).max_by(|x, y| corr(*x).partial_cmp(&corr(*y)).unwrap()).unwrap();
        assert_eq!(best, 0, "{} aligned", t.name);
    }
}

/// 30. Move imported library; project shows missing media and relinks by hash without changing cues.
#[test]
fn s30_relink_moved_library_by_hash() {
    let (mut s, root) = session("s30", "24", "01:00:00:00");
    let lib = root.join("Audio/LibraryRefs/mylib");
    synth::write_starter_pack(&lib, 48_000).unwrap();
    s.edit("lib", |st| {
        library::import_directory(
            st,
            &lib,
            library::AssetMeta {
                license: Some(License { name: "own".into(), redistributable: true, ..Default::default() }),
                ..Default::default()
            },
        )
    })
    .unwrap();
    scene(&mut s, "5", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:03:00", "01:00:03:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:10:00", "wind exterior", None);
    s.edit("cues", |st| markers_to_cues(st, None)).unwrap();
    s.edit("populate", |st| populate(st, &PopulateOptions::default())).unwrap();
    s.save().unwrap();
    let cues_before = serde_json::to_string(&s.state.cues).unwrap();
    let clips_before = serde_json::to_string(&s.state.clips).unwrap();
    let moved = root.join("moved_elsewhere/renamed_lib");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&lib, &moved).unwrap();
    let mut s = Session::open(&root).unwrap();
    assert!(!relink::missing_assets(&s.state).is_empty());
    let qa = qa::run(&s.state, &Default::default());
    assert!(qa.iter().any(|i| i.code == "media_missing"));
    let rows = report::cue_rows(&s.state, None);
    assert!(rows.iter().any(|r| r.media_missing));
    let rep = s.edit("relink", |st| relink::relink_by_hash(st, &[root.as_path()])).unwrap();
    assert!(rep.still_missing.is_empty(), "{:?}", rep.still_missing);
    assert!(relink::missing_assets(&s.state).is_empty());
    assert_eq!(serde_json::to_string(&s.state.cues).unwrap(), cues_before);
    assert_eq!(serde_json::to_string(&s.state.clips).unwrap(), clips_before);
    assert!(s.state.assets.values().all(|a| a.path.contains("renamed_lib")));
}

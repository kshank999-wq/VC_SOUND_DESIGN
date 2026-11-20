//! Additional workflow guarantees beyond the §20 scenarios.

mod common;
use common::*;
use vcsd_core::builder::{populate, preview, PopulateOptions};
use vcsd_core::mixer::{Mixer, RenderOptions};
use vcsd_core::picture::{apply_reconform, import_picture, reconform, EditChange, PictureImport};
use vcsd_core::spotting::*;
use vcsd_core::transport::{CaptureDevice, Transport};
use vcsd_core::*;

#[test]
fn failed_edit_is_atomic_and_undo_redo_work() {
    let (mut s, _r) = session("undo", "24", "01:00:00:00");
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    let before = serde_json::to_string(&s.state.scenes).unwrap();
    // mutates, then fails → must roll back
    let r: Result<()> = s.edit("bad", |st| {
        add_scene(st, "2", "x", 0, 10)?;
        add_scene(st, "1", "dup", 0, 10).map(|_| ())
    });
    assert!(r.is_err());
    assert_eq!(serde_json::to_string(&s.state.scenes).unwrap(), before);
    scene(&mut s, "2", "01:00:10:00", "01:00:20:00");
    assert_eq!(s.state.scenes.len(), 2);
    assert_eq!(s.undo().unwrap().as_deref(), Some("scene"));
    assert_eq!(s.state.scenes.len(), 1);
    s.redo().unwrap();
    assert_eq!(s.state.scenes.len(), 2);
    assert!(s.state.audit.iter().any(|a| a.action == "redo: scene"));
}

#[test]
fn marker_edit_updates_cue_and_ghost_preview_is_pure() {
    let (mut s, _r) = session("mrk", "25", "00:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "00:00:00:00", "00:00:10:00");
    let m = marker_tc(&mut s, "fx", EventType::Point, "00:00:02:00", "00:00:02:00", "glass", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    let snap = serde_json::to_string(&s.state).unwrap();
    let plan = preview(&s.state, &PopulateOptions::default()).unwrap();
    assert_eq!(plan.clips_created, 1);
    assert_eq!(serde_json::to_string(&s.state).unwrap(), snap, "preview does not mutate");
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let to = tc(&s, "00:00:03:05");
    s.edit("mv", |st| update_marker(st, &m, to, to, None)).unwrap();
    let cue = s.state.cues.values().next().unwrap();
    assert_eq!(cue.start_sample, to);
    assert_eq!(s.state.clips_for_cue(&cue.id)[0].timeline_start, to);
    // repopulate never destroys custom work
    let cid = cue.id.clone();
    let other = asset(&s, "glass smash 02");
    s.edit("r", |st| editing::replace_cue_asset(st, &cid, &other, editing::ReplaceScope::Cue, false)).unwrap();
    let rep = s.edit("p", |st| populate(st, &PopulateOptions { repopulate: true, ..Default::default() })).unwrap();
    assert_eq!(rep.clips_created, 0);
    assert_eq!(s.state.clips_for_cue(&cid)[0].asset_id, other);
}

#[test]
fn reconform_flags_without_moving_until_approved() {
    let (mut s, root) = session("rcf", "24", "01:00:00:00");
    with_pack(&mut s);
    let v = fake_video(&root);
    s.edit("p1", |st| {
        import_picture(
            st,
            PictureImport { path: &v, label: "v1".into(), start_tc: None, duration_frames: Some(2400), change_notes: String::new() },
        )
    })
    .unwrap();
    scene(&mut s, "1", "01:00:00:00", "01:01:00:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:10:00", "01:00:10:00", "glass", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:40:00", "01:00:40:00", "car door", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let before = clip_times(&s.state);
    let v2 = root.join("Picture/reference_v2.mov");
    std::fs::write(&v2, b"v2 bytes").unwrap();
    let p2 = s
        .edit("p2", |st| {
            import_picture(
                st,
                PictureImport {
                    path: &v2,
                    label: "v2".into(),
                    start_tc: None,
                    duration_frames: Some(2448),
                    change_notes: "2s insert at 01:00:30".into(),
                },
            )
        })
        .unwrap();
    let f = |t: &str| FrameRate::FPS_24.tc_to_frames(t).unwrap();
    let changes = [
        EditChange { old_start: f("01:00:00:00"), old_end: f("01:00:30:00"), new_start: f("01:00:00:00") },
        EditChange { old_start: f("01:00:30:00"), old_end: f("01:01:00:00"), new_start: f("01:00:32:00") },
    ];
    let rep = s.edit("rc", |st| reconform(st, &p2, &changes)).unwrap();
    assert_eq!(rep.shifted.len(), 1);
    assert_eq!(rep.unchanged, 1);
    assert_eq!(clip_times(&s.state), before, "nothing moves until reviewed");
    assert!(qa::run(&s.state, &Default::default()).iter().any(|i| i.code == "reconform_review"));
    let ids: Vec<String> = rep.shifted.iter().map(|x| x.0.clone()).collect();
    s.edit("apply", |st| apply_reconform(st, &ids)).unwrap();
    let moved = &s.state.cues[&ids[0]];
    assert_eq!(s.state.tb().tc_at(moved.start_sample), "01:00:42:00");
    assert_eq!(s.state.clips_for_cue(&ids[0])[0].timeline_start, moved.start_sample);
    assert_eq!(s.state.pictures[&p2].previous_id.as_ref().map(|p| s.state.pictures[p].label.as_str()), Some("v1"));
}

#[test]
fn film_studio_import_reimport_and_roundtrip() {
    let (mut s, _r) = session("fs", "24", "01:00:00:00");
    let doc = |desc: &str| {
        serde_json::json!({
        "schema": "vc.filmstudio.breakdown", "version": 1, "production": "Gravel Road", "revision": "B",
        "scenes": [{"scene_id": "FS-SC-012", "number": "12", "heading": "EXT. ROAD - DAY", "characters": ["Greg"],
            "start_tc": "01:00:00:00", "end_tc": "01:00:45:00",
            "lines": [{"line_id": "L1", "character": "Greg", "text": "Greg crunches up the gravel drive."}],
            "sound": [
                {"item_id": "SND-1", "category": "footsteps", "description": desc, "character": "Greg", "identity": "Greg / Boots", "surface": "gravel", "line_id": "L1"},
                {"item_id": "SND-2", "category": "ambience", "description": "Morning birds"}
            ]}]
    }).to_string()
    };
    let r = s.edit("fs", |st| integrations::import_film_studio(st, &doc("Greg walks up drive"))).unwrap();
    assert_eq!((r.scenes_created, r.cues_created, r.identities_created), (1, 2, 1));
    let cue = s.state.cues.values().find(|c| c.external_id.as_deref() == Some("SND-1")).unwrap().clone();
    assert_eq!(cue.status, CueStatus::Conceptual);
    assert!(cue.script_ref_id.is_some(), "linked to script line");
    // spot it against picture, then re-import with an upstream change
    let a = tc(&s, "01:00:05:00");
    s.edit("spot", |st| spot_conceptual_cue(st, &cue.id, a, a + 96_000, EventType::Range)).unwrap();
    let r2 = s.edit("fs2", |st| integrations::import_film_studio(st, &doc("Greg strides up the drive"))).unwrap();
    assert_eq!((r2.scenes_created, r2.cues_created), (0, 0));
    assert_eq!(r2.conflicts.len(), 1, "spotted cue not overwritten; conflict surfaced");
    assert_eq!(s.state.cues[&cue.id].start_sample, a);
    let out = integrations::export_film_studio_sound(&s.state);
    let item = out["items"].as_array().unwrap().iter().find(|i| i["item_id"] == "SND-1").unwrap();
    assert_eq!(item["start_tc"], "01:00:05:00");
    assert_eq!(item["identity"], "Greg / Boots");
}

#[test]
fn transport_blocks_match_offline_render_and_loop() {
    let (mut s, _r) = session("tr", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:06:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:01:00", "01:00:01:00", "glass smash", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    // inserts with state (filters + compressor) must be continuous across blocks
    for t in s.state.tracks.values_mut() {
        t.inserts = vec![
            Insert::HighPass { hz: 80.0, q: 0.7 },
            Insert::Compressor { threshold_db: -18.0, ratio: 3.0, attack_ms: 5.0, release_ms: 80.0, makeup_db: 2.0 },
        ];
    }
    let n = 48_000 * 3;
    let offline = Mixer::new(&s.state, RenderOptions::default()).render_master(0, n).unwrap();
    let mut dev = CaptureDevice::new(48_000);
    transport::play_range(&s.state, &mut dev, 0, n as i64, 512).unwrap();
    assert_eq!(dev.captured.frames(), n);
    let max = offline.data.iter().zip(&dev.captured.data).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-6, "block render == offline render (max diff {max})");
    // sample-accurate loop wrap
    let mixer = Mixer::new(&s.state, RenderOptions::default());
    let mut t = Transport::new(1000);
    t.loop_range = Some((48_000, 48_700));
    t.play_from(48_000);
    t.next_block(&mixer).unwrap();
    assert_eq!(t.position, 48_300);
    t.step_frames(&s.state, 1);
    assert_eq!(t.position, 50_000);
}

#[test]
fn autosave_recovery_and_schema_guard() {
    let (mut s, root) = session("as", "24", "01:00:00:00");
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    std::thread::sleep(std::time::Duration::from_millis(20));
    s.autosave().unwrap();
    let rec = Session::recover(&root).unwrap().expect("newer autosave offered for recovery");
    assert_eq!(rec.scenes.len(), 1);
    s.save().unwrap();
    assert!(Session::recover(&root).unwrap().is_none(), "save clears autosave");
    let back = Session::open(&root).unwrap();
    assert_eq!(back.state.audit.len(), s.state.audit.len());
}

#[test]
fn performance_rerender_keeps_triggers() {
    let (mut s, _r) = session("pf", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "footsteps", EventType::Range, "01:00:01:00", "01:00:04:00", "walk", None);
    let cue = s.edit("c", |st| markers_to_cues(st, None)).unwrap()[0].clone();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let track = s.state.clips_for_cue(&cue)[0].track_id.clone();
    let mut pads = perform::PadMap::default();
    pads.set("key:f", perform::Pad { name: "gravel".into(), assets: assets(&s, "boot gravel"), ..Default::default() });
    let mut rec = perform::PerformanceRecorder::new(&track, Some(&cue), pads);
    rec.latency_compensation = 240;
    for (i, v) in [100u8, 64, 127, 90].iter().enumerate() {
        rec.trigger("key:f", 48_000 + i as i64 * 24_000 + 240, *v).unwrap();
    }
    let pid = s.edit("take1", |st| rec.commit(st)).unwrap();
    let audible: Vec<&ClipInstance> = s.state.clips_for_cue(&cue).into_iter().filter(|c| !c.muted).collect();
    assert_eq!(audible.len(), 4, "placeholders muted, performance audible");
    assert_eq!(audible[0].timeline_start, 48_000, "latency compensated");
    assert!((audible[1].gain_db - perform::velocity_db(64)).abs() < 1e-6);
    // new shoes: re-render same triggers with wood
    let mut wood = perform::PadMap::default();
    wood.set("key:f", perform::Pad { name: "wood".into(), assets: assets(&s, "shoe wood"), ..Default::default() });
    s.edit("rr", |st| perform::rerender_with(st, &pid, wood)).unwrap();
    let audible: Vec<&ClipInstance> = s.state.clips_for_cue(&cue).into_iter().filter(|c| !c.muted).collect();
    assert_eq!(audible.len(), 4);
    assert!(audible.iter().all(|c| s.state.assets[&c.asset_id].name.contains("wood")));
    assert_eq!(audible.iter().map(|c| c.timeline_start).collect::<Vec<_>>(), vec![48_000, 72_000, 96_000, 120_000]);
}

#[test]
fn visual_onset_assistant_finds_flash_frame() {
    if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let (mut s, root) = session("vis", "24", "01:00:00:00");
    // 4 s grey clip with a white flash at frame 50 (00:00:02:02)
    let vid = root.join("Picture/flash.mp4");
    let st = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:s=160x90:r=24:d=4",
            "-vf",
            "drawbox=enable='between(n,50,53)':color=white:t=fill",
            "-c:v",
            "libx264",
            "-g",
            "1",
            "-pix_fmt",
            "yuv420p",
            "-timecode",
            "01:00:00:00",
        ])
        .arg(&vid)
        .status()
        .unwrap();
    assert!(st.success());
    s.edit("pic", |x| {
        import_picture(
            x,
            PictureImport { path: &vid, label: "flash".into(), start_tc: None, duration_frames: None, change_notes: String::new() },
        )
    })
    .unwrap();
    assert_eq!(s.state.current_picture().unwrap().source_start_frame, 86_400, "TC probed from media");
    scene(&mut s, "1", "01:00:00:00", "01:00:04:00");
    // user marks roughly where the glass breaks (6 frames late)
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:08", "01:00:02:08", "glass breaks", None);
    let cue = s.edit("c", |x| markers_to_cues(x, None)).unwrap();
    s.state.project.ai.enabled = true;
    let p = s.edit("ai", |x| ai::run(x, &ai::VisualOnsetAssistant::default(), &cue)).unwrap();
    let prop = &s.state.proposals[&p[0]];
    assert_eq!(s.state.tb().tc_at(prop.start_sample), "01:00:02:02", "{}", prop.reason);
    assert!(prop.confidence > 0.5);
    // cached per picture fingerprint
    let again = s.edit("ai", |x| ai::run(x, &ai::VisualOnsetAssistant::default(), &cue)).unwrap();
    assert_eq!(again, p);
}

#[test]
fn trims_keep_fades_valid_and_respect_source_length() {
    let (mut s, _r) = session("trim", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    let cue = s.edit("c", |st| markers_to_cues(st, None)).unwrap()[0].clone();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let clip = s.state.clips_for_cue(&cue)[0].clone();
    let full = editing::source_len(&s.state, &clip).unwrap();
    assert_eq!(clip.length, full, "point cue places the whole one-shot");
    s.edit("fades", |st| editing::set_fades(st, &clip.id, 4800, 9600)).unwrap();
    // head trim keeps audio in place and shrinks fades that no longer fit
    let start = clip.timeline_start;
    s.edit("trim head", |st| editing::trim_start(st, &clip.id, start + full - 6000)).unwrap();
    let c = &s.state.clips[&clip.id];
    assert_eq!((c.length, c.source_offset), (6000, full - 6000));
    assert!(c.fade_in + c.fade_out <= c.length);
    // one-shot tail cannot run past the source; trimming shorter is fine
    assert!(s.edit("too long", |st| editing::trim_end(st, &clip.id, 6001)).is_err());
    s.edit("tail", |st| editing::trim_end(st, &clip.id, 1000)).unwrap();
    let c = &s.state.clips[&clip.id];
    assert_eq!(c.length, 1000);
    assert!(c.fade_in + c.fade_out <= 1000, "{} + {}", c.fade_in, c.fade_out);
    // pulling the head back restores audio (non-destructive)
    s.edit("restore head", |st| editing::trim_start(st, &clip.id, start)).unwrap();
    let c = &s.state.clips[&clip.id];
    assert_eq!((c.timeline_start, c.source_offset), (start, 0));
    assert!(s.edit("before source", |st| editing::trim_start(st, &clip.id, start - 1)).is_err());
}

#[test]
fn automation_lanes_shape_volume_and_pan() {
    let (mut s, root) = session("auto", "24", "01:00:00:00");
    // constant 0.5 mono source so rendered values are easy to check
    let src = root.join("Audio/Imported/const.wav");
    wav::write(&src, &wav::AudioBuffer { sample_rate: 48_000, channels: 1, data: vec![0.5; 48_000 * 3] }, wav::SampleFormat::Float32, None)
        .unwrap();
    s.edit("imp", |st| {
        library::import_wav(
            st,
            &src,
            library::AssetMeta {
                kind: AssetKind::Placeholder,
                category: Some(Category::HardFx),
                loopable: Some(false),
                ..Default::default()
            },
        )
    })
    .unwrap();
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Range, "01:00:00:00", "01:00:02:00", "const", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let t = s.state.tracks.values().next().unwrap().id.clone();
    let sr = 48_000i64;
    let v0 = s.edit("a", |st| editing::set_automation(st, &t, AutomationParam::VolumeDb, 0, 0.0)).unwrap();
    s.edit("a", |st| editing::set_automation(st, &t, AutomationParam::VolumeDb, sr, -20.0)).unwrap();
    s.edit("a", |st| editing::set_automation(st, &t, AutomationParam::Pan, 0, -1.0)).unwrap();
    s.edit("a", |st| editing::set_automation(st, &t, AutomationParam::Pan, sr, 1.0)).unwrap();
    let m = Mixer::new(&s.state, RenderOptions::default());
    let print = m.render_track_print(&t, 0, (2 * sr) as usize).unwrap();
    let db = |v: f32| 20.0 * (v / 0.5).log10();
    assert!(db(print.data[0]).abs() < 0.01);
    assert!((db(print.data[(sr / 2) as usize]) + 10.0).abs() < 0.05, "interpolates in dB: {}", db(print.data[(sr / 2) as usize]));
    assert!((db(print.data[(sr * 3 / 2) as usize]) + 20.0).abs() < 0.05, "holds the last value");
    let st = m.render_track_stereo(&t, 0, (2 * sr) as usize).unwrap();
    assert!(st.data[0] > 0.49 && st.data[1].abs() < 1e-6, "hard left at start");
    let k = (sr * 3 / 2) as usize * 2;
    assert!(st.data[k].abs() < 1e-6 && st.data[k + 1] > 0.04, "hard right after the sweep");
    // values are clamped; -96 dB is silence
    let low = s.edit("a", |st| editing::set_automation(st, &t, AutomationParam::VolumeDb, sr / 4, -500.0)).unwrap();
    assert_eq!(s.state.automation[&low].value, -96.0);
    let m = Mixer::new(&s.state, RenderOptions::default());
    assert_eq!(m.render_track_print(&t, sr / 4, 1).unwrap().data[0], 0.0);
    // moving onto another breakpoint replaces it; delete and clear
    s.edit("mv", |st| editing::move_automation(st, &low, sr, -6.0)).unwrap();
    let vol: Vec<&AutomationPoint> = s.state.automation.values().filter(|p| p.param == AutomationParam::VolumeDb).collect();
    assert_eq!(vol.len(), 2, "no duplicate breakpoints at one sample");
    s.edit("del", |st| editing::delete_automation(st, &v0)).unwrap();
    assert_eq!(s.edit("clr", |st| editing::clear_automation(st, &t, Some(AutomationParam::Pan))).unwrap(), 2);
    assert_eq!(s.state.automation.len(), 1);
    s.undo().unwrap();
    assert_eq!(s.state.automation.len(), 3, "undo restores the cleared lane");
}

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

#[test]
fn bus_and_master_automation() {
    let (mut s, root) = session("busauto", "24", "01:00:00:00");
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
                license: Some(License { name: "own".into(), redistributable: true, ..Default::default() }),
                ..Default::default()
            },
        )
    })
    .unwrap();
    scene(&mut s, "1", "01:00:00:00", "01:00:02:00");
    marker_tc(&mut s, "fx", EventType::Range, "01:00:00:00", "01:00:02:00", "const", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let fx = s.state.buses.values().find(|b| b.name == "Hard FX").unwrap().id.clone();
    let master = mixer::master_bus(&s.state).unwrap().id.clone();
    let sr = 48_000i64;
    // stem bus: 0 dB → -20 dB over the first second
    s.edit("a", |st| editing::set_automation(st, &fx, AutomationParam::VolumeDb, 0, 0.0)).unwrap();
    s.edit("a", |st| editing::set_automation(st, &fx, AutomationParam::VolumeDb, sr, -20.0)).unwrap();
    let m = Mixer::new(&s.state, RenderOptions::default());
    let stem = m.render_bus(&fx, 0, (2 * sr) as usize).unwrap();
    let centre = stem.data[0]; // mono track panned centre into the stereo bus
    let db = |v: f32| 20.0 * (v / centre).log10();
    assert!(
        (db(stem.data[(sr / 2) as usize * 2]) + 10.0).abs() < 0.05,
        "bus volume interpolates: {}",
        db(stem.data[(sr / 2) as usize * 2])
    );
    assert!((db(stem.data[(sr * 3 / 2) as usize * 2]) + 20.0).abs() < 0.05);
    // master: balance hard right → left channel silent, right untouched
    s.edit("m", |st| editing::set_automation(st, &master, AutomationParam::Pan, 0, 1.0)).unwrap();
    let m = Mixer::new(&s.state, RenderOptions::default());
    let mix = m.render_master(0, sr as usize).unwrap();
    assert_eq!(mix.data[0], 0.0, "left muted by master balance");
    assert!((mix.data[1] - centre).abs() < 1e-6, "right at full level");
    // master volume to silence
    s.edit("m", |st| editing::set_automation(st, &master, AutomationParam::VolumeDb, 0, -96.0)).unwrap();
    assert_eq!(Mixer::new(&s.state, RenderOptions::default()).render_master(0, 100).unwrap().peak(), 0.0);
    // unknown owners are rejected; clearing works per bus
    assert!(s.edit("bad", |st| editing::set_automation(st, "bus_nope", AutomationParam::Pan, 0, 0.0)).is_err());
    assert_eq!(s.edit("clr", |st| editing::clear_automation(st, &master, None)).unwrap(), 2);
    // export: the Hard FX stem carries its automation
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-20");
    o.out_dir = Some(root.join("Exports/busauto"));
    o.format = wav::SampleFormat::Float32;
    let r = export::export(&mut s.state, Some(&root), &o).unwrap();
    let stem_file = r.files.iter().find(|f| f.contains("STEM_Hard_FX")).expect("hard fx stem");
    let (_, b) = wav::read(&root.join("Exports/busauto").join(stem_file)).unwrap();
    assert!((20.0 * (b.data[(sr * 3 / 2) as usize * 2] / b.data[0]).log10() + 20.0).abs() < 0.05, "stem file is automated");
    let rm: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("Exports/busauto/Manifests/routing_map.json")).unwrap()).unwrap();
    let fx_entry = rm["buses"].as_array().unwrap().iter().find(|b| b["name"] == "Hard FX").unwrap();
    assert_eq!(fx_entry["automation"].as_array().unwrap().len(), 2, "bus moves documented in the routing map");
    assert_eq!(fx_entry["automation"][1]["tc"], "01:00:01:00");
    // projects saved before bus automation used `track_id`
    let old: AutomationPoint = serde_json::from_str(r#"{"id":"aut_1","track_id":"trk_1","param":"pan","sample":0,"value":0.5}"#).unwrap();
    assert_eq!(old.owner_id, "trk_1");
}

#[test]
fn automation_write_pass_commit() {
    let (mut s, _r) = session("wpass", "24", "01:00:00:00");
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    let bus = s.state.buses.values().find(|b| b.name == "Foley").unwrap().id.clone();
    let sr = 48_000i64;
    let ramp = 2400;
    // existing lane: flat -3 dB with points at 0 and 8 s
    s.edit("a", |st| editing::set_automation(st, &bus, AutomationParam::VolumeDb, 0, -3.0)).unwrap();
    s.edit("a", |st| editing::set_automation(st, &bus, AutomationParam::VolumeDb, 8 * sr, -3.0)).unwrap();
    // a pass from 2 s to 4 s: linear fade -3 → -15 captured every 10 ms, then holds
    let mut cap = vec![];
    for k in 0..=100 {
        let t = 2 * sr + k * 480;
        let v = if t < 3 * sr { -3.0 - 12.0 * ((t - 2 * sr) as f32 / sr as f32) } else { -15.0 };
        cap.push((t, v));
    }
    let n = s.edit("pass", |st| editing::write_automation_pass(st, &bus, AutomationParam::VolumeDb, &cap, 2 * sr, 4 * sr, ramp)).unwrap();
    assert!(n <= 7, "101 captured values thinned to a few breakpoints, got {n}");
    let at = |st: &ProjectState, t: i64| editing::automation_value_at(st, &bus, AutomationParam::VolumeDb, t);
    assert!((at(&s.state, sr) + 3.0).abs() < 0.01, "untouched before the pass");
    assert!((at(&s.state, 2 * sr + sr / 2) + 9.0).abs() < 0.3, "ramp captured: {}", at(&s.state, 2 * sr + sr / 2));
    assert!((at(&s.state, 7 * sr / 2) + 15.0).abs() < 0.01, "held to the end of the pass");
    assert!((at(&s.state, 4 * sr + ramp) + 3.0).abs() < 0.01, "returns to the prior automation after the ramp");
    assert!((at(&s.state, 6 * sr) + 3.0).abs() < 0.01);
    // one undo removes the whole pass
    s.undo().unwrap();
    assert!((at(&s.state, 7 * sr / 2) + 3.0).abs() < 0.01);
    // pan pass on an empty lane of a track-less bus starts from neutral
    let n = s.edit("pan", |st| editing::write_automation_pass(st, &bus, AutomationParam::Pan, &[(sr, 0.5)], sr, 2 * sr, ramp)).unwrap();
    assert_eq!(n, 4, "ramp in from centre, hold, ramp back");
    assert!((editing::automation_value_at(&s.state, &bus, AutomationParam::Pan, 3 * sr)).abs() < 1e-6);
    s.edit("mode", |st| editing::set_automation_mode(st, &bus, AutoMode::Latch)).unwrap();
    assert_eq!(s.state.buses[&bus].automation_mode, AutoMode::Latch);
}

#[test]
fn mixer_live_override_replaces_lane() {
    let (mut s, root) = session("ovr", "24", "01:00:00:00");
    let src = root.join("Audio/Imported/const.wav");
    wav::write(&src, &wav::AudioBuffer { sample_rate: 48_000, channels: 1, data: vec![0.5; 48_000] }, wav::SampleFormat::Float32, None)
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
    scene(&mut s, "1", "01:00:00:00", "01:00:01:00");
    marker_tc(&mut s, "fx", EventType::Range, "01:00:00:00", "01:00:00:12", "const", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let t = s.state.tracks.values().next().unwrap().id.clone();
    s.edit("a", |st| editing::set_automation(st, &t, AutomationParam::VolumeDb, 0, -20.0)).unwrap();
    let m = Mixer::new(&s.state, RenderOptions::default());
    m.overrides.borrow_mut().insert((t.clone(), AutomationParam::VolumeDb), -6.0);
    let v = m.render_track_print(&t, 100, 1).unwrap().data[0];
    assert!((20.0 * (v / 0.5).log10() + 6.0).abs() < 0.01, "live value wins over the stored lane");
}

#[test]
fn track_folders_move_rename_ungroup_and_flags() {
    let (mut s, _r) = session("folders", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "12", "01:00:00:00", "01:00:20:00");
    marker_tc(&mut s, "footsteps", EventType::Range, "01:00:01:00", "01:00:03:00", "walk", None);
    marker_tc(&mut s, "props", EventType::Point, "01:00:04:00", "01:00:04:00", "door handle", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:20:00", "birds", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let folders: std::collections::BTreeSet<String> = s.state.tracks.values().map(|t| t.folder.clone()).collect();
    assert!(folders.contains("Scene 12/Foley") && folders.contains("Scene 12/Backgrounds & Atmos"), "{folders:?}");
    // rename a parent: subfolders follow
    let n = s.edit("ren", |st| editing::rename_folder(st, " Scene 12 ", "Reel 1 / Scene 12")).unwrap();
    assert_eq!(n, 3);
    assert!(s.state.tracks.values().all(|t| t.folder.starts_with("Reel 1/Scene 12/")));
    assert!(s.edit("loop", |st| editing::rename_folder(st, "Reel 1", "Reel 1/inner")).is_err(), "no folder inside itself");
    // folder mute/solo hits every track below
    s.edit("m", |st| editing::set_folder_flag(st, "Reel 1/Scene 12/Foley", editing::FolderFlag::Mute, true)).unwrap();
    let muted: Vec<bool> = s.state.tracks.values().map(|t| t.mute).collect();
    assert_eq!(muted.iter().filter(|m| **m).count(), 2, "only the Foley tracks");
    // move one track to a new folder, then ungroup a level
    let bg = s.state.tracks.values().find(|t| t.category == Category::Background).unwrap().id.clone();
    s.edit("mv", |st| editing::move_tracks_to_folder(st, std::slice::from_ref(&bg), "Ambience//Stems/")).unwrap();
    assert_eq!(s.state.tracks[&bg].folder, "Ambience/Stems");
    s.edit("ug", |st| editing::ungroup_folder(st, "Reel 1")).unwrap();
    assert!(s.state.tracks.values().filter(|t| t.id != bg).all(|t| t.folder.starts_with("Scene 12/")));
    s.edit("top", |st| editing::move_tracks_to_folder(st, std::slice::from_ref(&bg), "")).unwrap();
    assert_eq!(s.state.tracks[&bg].folder, "");
    // folders travel with the handoff manifest's routing map (unchanged contract)
    s.undo().unwrap();
    assert_eq!(s.state.tracks[&bg].folder, "Ambience/Stems");
}

/// A stand-in plugin: delays by `LAT` samples (and reports it) and halves the level.
/// Per slot: delay line, write index, next expected position.
type Line = (Vec<f32>, usize, i64);

struct DelayHost {
    lines: std::sync::Mutex<std::collections::HashMap<String, Line>>,
}
const LAT: i64 = 1000;

impl mixer::PluginHost for DelayHost {
    fn process(
        &self,
        slot: &PluginInsert,
        buf: &mut wav::AudioBuffer,
        pos: i64,
        _auto: &[mixer::ParamCurve],
    ) -> std::result::Result<(), String> {
        let ch = buf.channels as usize;
        let mut lines = self.lines.lock().unwrap();
        let (line, w, next) = lines.entry(slot.id.clone()).or_insert_with(|| (vec![0.0; LAT as usize * ch], 0, pos));
        if *next != pos {
            // discontinuity: flush, like a real plugin reset
            line.iter_mut().for_each(|v| *v = 0.0);
        }
        for f in 0..buf.frames() {
            for c in 0..ch {
                let i = (*w * ch) + c;
                let out = line[i];
                line[i] = buf.data[f * ch + c];
                buf.data[f * ch + c] = out * 0.5;
            }
            *w = (*w + 1) % LAT as usize;
        }
        *next = pos + buf.frames() as i64;
        Ok(())
    }
    fn latency(&self, _slot: &PluginInsert) -> i64 {
        LAT
    }
    fn continues_at(&self, slot: &PluginInsert) -> Option<i64> {
        self.lines.lock().unwrap().get(&slot.id).map(|l| l.2)
    }
}

fn plugin_slot(id: &str) -> Insert {
    Insert::Plugin(PluginInsert {
        id: id.into(),
        plugin_id: "vst3:test".into(),
        format: "vst3".into(),
        name: "Test Delay".into(),
        vendor: String::new(),
        path: String::new(),
        bypass: false,
        state: None,
        params: Default::default(),
    })
}

#[test]
fn plugin_latency_is_compensated_and_blocks_match_offline() {
    let (mut s, _r) = session("plug", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:01:00", "01:00:01:00", "glass smash", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let n = 48_000 * 3;
    let dry = Mixer::new(&s.state, RenderOptions::default()).render_master(0, n).unwrap();
    assert!(dry.peak() > 0.01);

    // one plugin on every track, one on the master: 2 × LAT of latency in the path
    for (i, t) in s.state.tracks.values_mut().enumerate() {
        t.inserts = vec![plugin_slot(&format!("t{i}"))];
    }
    let master = mixer::master_bus(&s.state).unwrap().id.clone();
    s.state.buses.get_mut(&master).unwrap().inserts = vec![plugin_slot("m")];
    let host = || -> std::sync::Arc<dyn mixer::PluginHost> { std::sync::Arc::new(DelayHost { lines: Default::default() }) };

    // offline, in one go: the delayed audio still lands on the original samples
    let wet = Mixer::new(&s.state, RenderOptions::default()).with_plugins(Some(host())).render_master(0, n).unwrap();
    let max = dry.data.iter().zip(&wet.data).map(|(d, w)| (d * 0.25 - w).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-6, "compensated: wet == dry × ¼, sample-aligned (max diff {max})");

    // starting right at a sound (a locate): the chains are pre-rolled, so the
    // onset is there instead of `LAT` samples of silence
    let onset = (0..dry.frames()).find(|&f| dry.data[f * 2].abs() > 1e-4).unwrap() as i64;
    let from = onset - 10;
    let a = Mixer::new(&s.state, RenderOptions::default()).render_master(from, 4000).unwrap();
    let b = Mixer::new(&s.state, RenderOptions::default()).with_plugins(Some(host())).render_master(from, 4000).unwrap();
    let max = a.data.iter().zip(&b.data).map(|(d, w)| (d * 0.25 - w).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-6, "pre-rolled after a jump (max diff {max})");

    // in transport-sized blocks the plugins see one continuous stream
    let m = Mixer::new(&s.state, RenderOptions::default()).with_plugins(Some(host()));
    let mut blocks = vec![];
    for b in (0..n).step_by(512) {
        blocks.extend(m.render_master(b as i64, 512.min(n - b)).unwrap().data);
    }
    let max = wet.data.iter().zip(&blocks).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-6, "block render == offline render (max diff {max})");
    assert!(m.warnings.borrow().is_empty());

    // bypass removes both the processing and the compensation
    for t in s.state.tracks.values_mut() {
        if let Insert::Plugin(p) = &mut t.inserts[0] {
            p.bypass = true;
        }
    }
    let half = Mixer::new(&s.state, RenderOptions::default()).with_plugins(Some(host())).render_master(0, n).unwrap();
    let max = dry.data.iter().zip(&half.data).map(|(d, w)| (d * 0.5 - w).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-6, "only the master plugin left (max diff {max})");

    // without a host (e.g. CLI export) the slots pass audio through and say so
    let m = Mixer::new(&s.state, RenderOptions::default());
    let thru = m.render_master(0, n).unwrap();
    assert_eq!(thru.data, dry.data);
    assert!(m.warnings.borrow().iter().any(|w| w.contains("Test Delay") && w.contains("passed through")));
}

#[test]
fn plugin_slots_round_trip_through_the_project_file() {
    let mut ins = plugin_slot("x");
    if let Insert::Plugin(p) = &mut ins {
        p.params.insert(7, 0.25);
    }
    let v = serde_json::to_value(&ins).unwrap();
    assert_eq!(v["type"], "plugin");
    assert_eq!(v["plugin_id"], "vst3:test");
    assert_eq!(v["params"]["7"], 0.25);
    let back: Insert = serde_json::from_value(v).unwrap();
    assert_eq!(back.as_plugin().unwrap(), ins.as_plugin().unwrap());
    // older fields are optional
    let min: Insert = serde_json::from_value(
        serde_json::json!({"type": "plugin", "id": "a", "plugin_id": "au:aufx:dely:appl", "format": "au", "name": "AUDelay"}),
    )
    .unwrap();
    assert!(min.as_plugin().unwrap().state.is_none());
}

#[test]
fn plugin_parameter_lanes_become_per_block_curves() {
    let (mut s, _r) = session("pauto", "24", "01:00:00:00");
    with_pack(&mut s);
    let track = s.state.tracks.values().next().map(|t| t.id.clone());
    let track = match track {
        Some(t) => t,
        None => {
            scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
            marker_tc(&mut s, "fx", EventType::Point, "01:00:01:00", "01:00:01:00", "glass smash", None);
            s.edit("c", |st| markers_to_cues(st, None)).unwrap();
            s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
            s.state.tracks.values().next().unwrap().id.clone()
        }
    };
    s.state.tracks.get_mut(&track).unwrap().inserts = vec![plugin_slot("slot1")];
    let p = AutomationParam::Plugin(7);
    assert_eq!(serde_json::to_value(p).unwrap(), "plugin:7");
    assert_eq!(serde_json::from_value::<AutomationParam>(serde_json::json!("plugin:7")).unwrap(), p);
    s.edit("a", |st| editing::set_automation(st, "slot1", p, 1000, 0.0)).unwrap();
    s.edit("a", |st| editing::set_automation(st, "slot1", p, 2000, 1.0)).unwrap();
    s.edit("a", |st| editing::set_automation(st, "slot1", p, 2500, 1.7)).unwrap(); // clamped to 1
    assert!(editing::set_automation(&mut s.state, "nope", p, 0, 0.5).is_err(), "owner must be a track, bus or plugin slot");

    let m = Mixer::new(&s.state, RenderOptions::default());
    // a block across the first ramp: start value, the breakpoint, end value
    let c = m.plugin_curves("slot1", 1500, 1000);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].id, 7);
    let pts = &c[0].points;
    assert_eq!(pts[0], (0, 0.5));
    assert_eq!(pts[1], (500, 1.0));
    assert_eq!(*pts.last().unwrap(), (999, 1.0));
    // before the lane: held at the first value; other slots: nothing
    assert_eq!(m.plugin_curves("slot1", 0, 100)[0].points, vec![(0, 0.0), (99, 0.0)]);
    assert!(m.plugin_curves("other", 0, 100).is_empty());
    // a value being written live overrides the lane
    m.overrides.borrow_mut().insert(("slot1".into(), p), 0.25);
    assert_eq!(m.plugin_curves("slot1", 1500, 1000)[0].points, vec![(0, 0.25)]);

    // removing the slot removes its lanes
    s.state.tracks.get_mut(&track).unwrap().inserts.clear();
    assert_eq!(editing::drop_orphan_plugin_automation(&mut s.state), 3);
}

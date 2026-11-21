//! OTIO import: our own export read back (the mix must survive), and a timeline
//! written by the OpenTimelineIO library in an editor's layout
//! (`scripts/otio/make_fixture.py`). The library test is skipped (loudly) when
//! it isn't installed: `python3 -m pip install opentimelineio==0.18.1`, or set
//! VCSD_OTIO_PYTHON.

mod common;
use common::*;
use std::path::Path;
use vcsd_core::builder::{populate, PopulateOptions};
use vcsd_core::otio_import::{import, OtioImportOptions};
use vcsd_core::spotting::*;
use vcsd_core::*;

fn python() -> Option<String> {
    let py = std::env::var("VCSD_OTIO_PYTHON").unwrap_or_else(|_| "python3".into());
    let ok = std::process::Command::new(&py).args(["-c", "import opentimelineio"]).output().map(|o| o.status.success()).unwrap_or(false);
    ok.then_some(py)
}

#[test]
fn an_exported_otio_imports_back_to_the_same_mix() {
    let (mut s, root) = session("otiort", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:06:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:05:00", "01:00:05:00", "door slam", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let lens: std::collections::HashMap<String, i64> = st.assets.values().map(|a| (a.id.clone(), a.length_samples)).collect();
    for c in st.clips.values_mut() {
        c.fill = FillMode::OneShot;
        c.length = c.length.min(lens[&c.asset_id] - c.source_offset);
    }
    let cue = |st: &ProjectState, what: &str| {
        let id = st.cues.values().find(|c| c.description.contains(what)).unwrap().id.clone();
        st.clips.values().find(|c| c.cue_id.as_deref() == Some(id.as_str()) && !c.muted).unwrap().id.clone()
    };
    let (glass, wind, door) = (cue(st, "glass"), cue(st, "wind"), cue(st, "door"));
    let g = st.clips.get_mut(&glass).unwrap();
    g.gain_db = -6.0;
    g.fade_in = 480;
    g.fade_out = 960;
    let (gt, wt, dt) = (st.clips[&glass].track_id.clone(), st.clips[&wind].track_id.clone(), st.clips[&door].track_id.clone());
    st.tracks.get_mut(&gt).unwrap().gain_db = -3.0;
    st.tracks.get_mut(&dt).unwrap().pan = 0.5;
    let t0 = st.clips[&glass].timeline_start;
    for (dx, db) in [(0, 0.0), (sr / 4, -18.0), (sr / 2, 0.0)] {
        editing::set_automation(st, &gt, AutomationParam::VolumeDb, t0 + dx, db).unwrap();
    }
    let w0 = st.clips[&wind].timeline_start;
    editing::set_automation(st, &wt, AutomationParam::Pan, w0, -0.6).unwrap();
    editing::set_automation(st, &wt, AutomationParam::Pan, w0 + 2 * sr, 0.6).unwrap();

    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());

    let (mut s2, root2) = session("otiort2", "24", "01:00:00:00");
    scene(&mut s2, "1", "01:00:00:00", "01:00:10:00");
    let rep = s2.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root2, &file))).unwrap();
    assert!(rep.missing_media.is_empty(), "{:?}", rep.missing_media);
    assert_eq!(rep.start_tc, "01:00:00:00");
    assert_eq!(rep.markers, s.state.cues.len());
    assert!(rep.notes.is_empty(), "our own metadata carries the mix: {:?}", rep.notes);
    let st2 = &s2.state;
    assert_eq!(st2.tracks.len(), s.state.tracks.len());
    let gc = st2.clips.values().find(|c| c.timeline_start == t0 && c.fade_in == 480).expect("glass clip");
    assert_eq!((gc.fade_out, gc.gain_db), (960, -6.0));
    let g2 = &st2.tracks[&gc.track_id];
    assert_eq!((g2.gain_db, g2.channels), (-3.0, 1));
    let lane: Vec<(i64, f32)> = {
        let mut v: Vec<(i64, f32)> = st2
            .automation
            .values()
            .filter(|p| p.owner_id == g2.id && p.param == AutomationParam::VolumeDb)
            .map(|p| (p.sample, p.value))
            .collect();
        v.sort_by_key(|p| p.0);
        v
    };
    assert_eq!(lane, vec![(t0, 0.0), (t0 + sr / 4, -18.0), (t0 + sr / 2, 0.0)], "the lane exactly");
    assert!(st2.tracks.values().any(|t| t.channels == 2), "the stereo bed");

    // the mix: the same, sample for sample (everything is carried exactly)
    let (start, end) = mixer::timeline_extent(&s.state);
    let n = (end - start) as usize;
    let a = mixer::Mixer::new(&s.state, mixer::RenderOptions::default()).render_master(start, n).unwrap();
    let b = mixer::Mixer::new(st2, mixer::RenderOptions::default()).render_master(start, n).unwrap();
    assert!(a.peak() > 0.05);
    let max = a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-5, "mix differs by {max}");
}

#[test]
fn an_editors_otio_imports_clips_transitions_nesting_media_and_markers() {
    let Some(py) = python() else {
        eprintln!("SKIPPED: OpenTimelineIO not installed (python3 -m pip install opentimelineio==0.18.1)");
        return;
    };
    let dir = tmp("otiofixture");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/otio/make_fixture.py");
    let out = std::process::Command::new(&py).arg(script).arg(&dir).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let file = dir.join("fixture.otio");

    let (mut s, root) = session("otioimp", "25", "10:00:00:00");
    scene(&mut s, "1", "10:00:00:00", "10:00:10:00");
    let rep = s.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root, &file))).unwrap();
    assert_eq!(rep.timeline, "Reel 2");
    assert_eq!(rep.start_tc, "10:00:00:00");
    assert_eq!(rep.missing_media.len(), 1, "{:?}", rep.missing_media);
    assert!(rep.missing_media[0].contains("missing reference"), "{:?}", rep.missing_media);
    assert!(rep.notes.iter().any(|n| n.starts_with("levels, pan and automation aren't in OTIO")), "{:?}", rep.notes);
    assert_eq!(rep.markers, 2, "the timeline marker and the clip marker");
    let st = &s.state;
    let f = 1920; // samples per frame at 25 fps
    let names: Vec<&str> = st.tracks.values().map(|t| t.name.as_str()).collect();
    assert_eq!(rep.tracks.len(), 3, "DIA 1, MX and MX / inner (video and the offline-only track ignored): {names:?}");
    let track = |name: &str| st.tracks.values().find(|t| t.name == name).unwrap_or_else(|| panic!("track {name} in {names:?}"));
    let clips = |tid: &str| {
        let mut v: Vec<&ClipInstance> = st.clips.values().filter(|c| c.track_id == tid).collect();
        v.sort_by_key(|c| c.timeline_start);
        v
    };

    // DIA 1: a fade in from the gap, a dissolve with handles, a disabled clip
    let dia = track("DIA 1");
    assert_eq!((dia.channels, dia.category.clone(), dia.folder.as_str()), (2, Category::Dialogue, "OTIO/Reel 2"));
    let d = clips(&dia.id);
    assert_eq!(d.len(), 3);
    // A: 10 frames into the media (its range starts at 10:00:00:00), on past the cut by the dissolve's 5
    assert_eq!((d[0].timeline_start, d[0].source_offset, d[0].length), (25 * f, 10 * f, 55 * f));
    assert_eq!((d[0].fade_in, d[0].fade_out), (5 * f, 10 * f));
    // B: starts 5 frames before the cut, into its handle
    assert_eq!((d[1].timeline_start, d[1].source_offset, d[1].length, d[1].fade_in), (70 * f, 65 * f, 30 * f, 10 * f));
    assert!(d[2].muted && d[2].timeline_start == 110 * f, "the disabled clip, muted");
    let a = &st.assets[&d[0].asset_id];
    assert!(Path::new(&a.path).starts_with(&dir), "the WAV used where it is (relative URL)");

    // MX: the moved AIFF found by name and converted; the nested stack trimmed to its range
    let mx = track("MX");
    let m = clips(&mx.id);
    assert_eq!((m.len(), m[0].timeline_start, m[0].length), (1, 0, 25 * f));
    let (info, raw) = wav::read_raw(Path::new(&st.assets[&m[0].asset_id].path)).unwrap();
    assert_eq!((info.channels, info.sample_rate, info.frames), (1, 48000, 96000));
    let want = (0.4f64 * (2.0 * std::f64::consts::PI * 220.0 * 100.0 / 48000.0).sin() * 32767.0).round() as i16;
    assert_eq!(i16::from_le_bytes([raw[200], raw[201]]), want, "AIFF samples, byte-swapped");
    let inner = track("MX / inner");
    let n = clips(&inner.id);
    assert_eq!((n.len(), n[0].timeline_start, n[0].source_offset, n[0].length), (1, 25 * f, 5 * f, 20 * f));

    // markers: the timeline's, and clip A's (media frame 20 → timeline frame 35)
    let mut mk: Vec<(i64, String)> = st.markers.values().map(|m| (m.start_sample, m.label.clone())).collect();
    mk.sort();
    assert_eq!(mk, vec![(35 * f, "line — pickup".to_string()), (50 * f, "door knock".to_string())]);
    assert!(st.markers.values().all(|m| st.lanes[&m.lane_id].name == "OTIO markers"));

    // placed elsewhere on request
    let (mut s2, root2) = session("otioimp2", "25", "10:00:00:00");
    let o = OtioImportOptions { at: Some(1000 * f), ..OtioImportOptions::for_project(&root2, &file) };
    let rep = s2.edit("i", |st| import(st, &file, &o)).unwrap();
    assert!(rep.notes.iter().any(|n| n.starts_with("markers outside every scene")), "{:?}", rep.notes);
    assert!(s2.state.clips.values().any(|c| c.timeline_start == 1025 * f));
}

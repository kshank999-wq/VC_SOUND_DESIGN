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
    assert!(rep.notes.iter().any(|n| n.starts_with("no levels or pan in the OTIO")), "{:?}", rep.notes);
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

/// A project with one automated glass clip: clip −6 dB, fader −3 dB, a dip to −18 dB.
fn levels_project(name: &str) -> (state::Session, std::path::PathBuf, i64) {
    let (mut s, root) = session(name, "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let id = st.clips.values().find(|c| !c.muted).unwrap().id.clone();
    let c = st.clips.get_mut(&id).unwrap();
    c.fill = FillMode::OneShot;
    c.gain_db = -6.0;
    let (t0, tid) = (c.timeline_start, c.track_id.clone());
    st.tracks.get_mut(&tid).unwrap().gain_db = -3.0;
    for (dx, db) in [(0, 0.0), (sr / 4, -18.0), (sr / 2, 0.0)] {
        editing::set_automation(st, &tid, AutomationParam::VolumeDb, t0 + dx, db).unwrap();
    }
    // a pan sweep from hard left to right of centre
    editing::set_automation(st, &tid, AutomationParam::Pan, t0, -1.0).unwrap();
    editing::set_automation(st, &tid, AutomationParam::Pan, t0 + sr / 2, 0.4).unwrap();
    (s, root, t0)
}

/// What a track plays at `x`, in dB: clip gain + fader + volume lane (held at its ends).
fn level_at(st: &ProjectState, clip: &ClipInstance, x: i64) -> f64 {
    let t = &st.tracks[&clip.track_id];
    let mut lane: Vec<(i64, f32)> = st
        .automation
        .values()
        .filter(|p| p.owner_id == t.id && p.param == AutomationParam::VolumeDb)
        .map(|p| (p.sample, p.value))
        .collect();
    lane.sort_by_key(|p| p.0);
    let v = if lane.is_empty() {
        0.0
    } else {
        let i = lane.partition_point(|p| p.0 <= x);
        let (a, b) = (lane[i.saturating_sub(1)], lane[i.min(lane.len() - 1)]);
        if b.0 > a.0 {
            a.1 as f64 + (b.1 - a.1) as f64 * (x - a.0) as f64 / (b.0 - a.0) as f64
        } else {
            a.1 as f64
        }
    };
    clip.gain_db as f64 + t.gain_db as f64 + v
}

/// The track's pan at `x`: its lane (held at the ends), else its static pan.
fn pan_at(st: &ProjectState, clip: &ClipInstance, x: i64) -> f64 {
    let t = &st.tracks[&clip.track_id];
    let mut lane: Vec<(i64, f32)> =
        st.automation.values().filter(|p| p.owner_id == t.id && p.param == AutomationParam::Pan).map(|p| (p.sample, p.value)).collect();
    lane.sort_by_key(|p| p.0);
    if lane.is_empty() {
        return t.pan as f64;
    }
    let i = lane.partition_point(|p| p.0 <= x);
    let (a, b) = (lane[i.saturating_sub(1)], lane[i.min(lane.len() - 1)]);
    if b.0 > a.0 {
        a.1 as f64 + (b.1 - a.1) as f64 * (x - a.0) as f64 / (b.0 - a.0) as f64
    } else {
        a.1 as f64
    }
}

fn same_levels(a: &ProjectState, b: &ProjectState, t0: i64, tol: f64) {
    let ca = a.clips.values().find(|c| c.timeline_start == t0 && !c.muted).unwrap();
    let cb = b.clips.values().find(|c| c.timeline_start == t0 && c.length == ca.length).expect("the clip, in place");
    for k in 0..=200 {
        let x = t0 + ca.length * k / 200;
        let (wa, wb) = (level_at(a, ca, x), level_at(b, cb, x));
        assert!((wa - wb).abs() < tol, "level at {x}: {wb:.3} dB imported vs {wa:.3} dB");
        let (pa, pb) = (pan_at(a, ca, x), pan_at(b, cb, x));
        assert!((pa - pb).abs() < 0.003, "pan at {x}: {pb:.4} imported vs {pa:.4}");
    }
}

/// Remove every `vcsd` key (our own metadata), as a tool that drops unknown metadata would.
fn strip_vcsd(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.remove("vcsd");
            m.values_mut().for_each(strip_vcsd);
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(strip_vcsd),
        _ => {}
    }
}

#[test]
fn levels_come_back_from_gain_effects_alone() {
    let (mut s, root, t0) = levels_project("otiolevels");
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    strip_vcsd(&mut v);
    let bare = file.with_file_name("bare.otio");
    std::fs::write(&bare, serde_json::to_string(&v).unwrap()).unwrap();

    let (mut s2, root2) = session("otiolevels2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &bare, &OtioImportOptions::for_project(&root2, &bare))).unwrap();
    assert!(!rep.notes.iter().any(|n| n.starts_with("no levels")), "{:?}", rep.notes);
    assert_eq!(rep.automated_tracks, 1);
    let c = s2.state.clips.values().find(|c| c.timeline_start == t0).unwrap();
    assert!((c.gain_db + 6.0).abs() < 0.001, "clip gain from its effect: {}", c.gain_db);
    // fader × lane, through linear-amplitude keyframes and back to a dB lane
    same_levels(&s.state, &s2.state, t0, 0.25);
}

#[test]
fn levels_come_back_from_an_aaf_converted_by_the_opentimelineio_aaf_adapter() {
    let Some(py) = python() else {
        eprintln!("SKIPPED: OpenTimelineIO not installed");
        return;
    };
    let has_adapter =
        std::process::Command::new(&py).args(["-c", "import otio_aaf_adapter"]).output().map(|o| o.status.success()).unwrap_or(false);
    if !has_adapter {
        eprintln!("SKIPPED: the OpenTimelineIO AAF adapter isn't installed (python3 -m pip install otio-aaf-adapter==2.0.0)");
        return;
    }
    let (mut s, root, t0) = levels_project("otioadapter");
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.aaf = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let aaf = r.dir.join(r.files.iter().find(|f| f.ends_with(".aaf")).unwrap());
    let otio = r.dir.join("from_aaf.otio");
    let conv = "import sys, opentimelineio as otio\n\
                tl = otio.adapters.read_from_file(sys.argv[1], adapter_name='AAF', simplify=True)\n\
                otio.adapters.write_to_file(tl, sys.argv[2])";
    let out = std::process::Command::new(&py).args(["-c", conv]).arg(&aaf).arg(&otio).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let (mut s2, root2) = session("otioadapter2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &otio, &OtioImportOptions::for_project(&root2, &otio))).unwrap();
    assert!(rep.missing_media.is_empty(), "{:?}", rep.missing_media);
    assert!(!rep.notes.iter().any(|n| n.starts_with("no levels")), "{:?}", rep.notes);
    assert_eq!(rep.start_tc, "01:00:00:00");
    // the AAF folds the fader into clip gain (−9 dB); the lane is the track's gain effect
    let c = s2.state.clips.values().find(|c| c.timeline_start == t0).expect("the clip, in place");
    assert!((c.gain_db + 9.0).abs() < 0.01, "{}", c.gain_db);
    // two 0.1 dB fits (AAF out, OTIO in) between the lanes
    same_levels(&s.state, &s2.state, t0, 0.25);
}

#[test]
fn bus_levels_folded_into_an_otio_come_back_as_the_tracks_levels() {
    let (mut s, root, t0) = levels_project("otiobusrt");
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let tid = st.clips.values().find(|c| c.timeline_start == t0).unwrap().track_id.clone();
    let bus = st.tracks[&tid].bus_id.clone();
    let master = st.buses.values().find(|b| b.parent_id.is_none()).unwrap().id.clone();
    st.buses.get_mut(&bus).unwrap().gain_db = -2.0;
    editing::set_automation(st, &bus, AutomationParam::VolumeDb, t0, 0.0).unwrap();
    editing::set_automation(st, &bus, AutomationParam::VolumeDb, t0 + sr / 2, -8.0).unwrap();
    editing::set_automation(st, &master, AutomationParam::Pan, t0, 0.3).unwrap();
    editing::set_automation(st, &master, AutomationParam::Pan, t0 + sr, -0.3).unwrap();
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());

    // a fresh project: its buses are flat, so the track carries what the buses did
    let (mut s2, root2) = session("otiobusrt2", "24", "01:00:00:00");
    s2.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root2, &file))).unwrap();
    let st2 = &s2.state;
    let c2 = st2.clips.values().find(|c| c.timeline_start == t0).unwrap();
    assert!((c2.gain_db + 6.0).abs() < 1e-3, "clip gain as it was");
    assert!(
        st2.automation.values().any(|p| p.owner_id == c2.track_id && p.param == AutomationParam::Pan),
        "the master's balance, as the track's pan"
    );
    let c1 = s.state.clips.values().find(|c| c.timeline_start == t0).unwrap();
    let n = c1.length as usize;
    let a = mixer::Mixer::new(&s.state, mixer::RenderOptions::default()).render_master(t0, n).unwrap();
    let b = mixer::Mixer::new(st2, mixer::RenderOptions::default()).render_master(t0, n).unwrap();
    assert!(a.peak() > 0.01);
    let max = a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(max < 0.02 * a.peak(), "mix differs by {max} (peak {})", a.peak());
}

/// A test plugin: gain = 2 × parameter 0 (from its automation, else 0.5).
struct GainPlugin;

impl mixer::PluginHost for GainPlugin {
    fn process(
        &self,
        _slot: &PluginInsert,
        buf: &mut wav::AudioBuffer,
        _pos: i64,
        auto: &[mixer::ParamCurve],
    ) -> std::result::Result<(), String> {
        let ch = buf.channels as usize;
        for f in 0..buf.frames() {
            let v = auto.iter().find(|c| c.id == 0).map(|c| {
                let i = c.points.partition_point(|p| p.0 as usize <= f).max(1) - 1;
                let (a, b) = (c.points[i], c.points[(i + 1).min(c.points.len() - 1)]);
                if b.0 > a.0 {
                    a.1 + (b.1 - a.1) * (f as f64 - a.0 as f64) / (b.0 - a.0) as f64
                } else {
                    a.1
                }
            });
            let g = 2.0 * v.unwrap_or(0.5) as f32;
            for c in 0..ch {
                buf.data[f * ch + c] *= g;
            }
        }
        Ok(())
    }
    fn latency(&self, _slot: &PluginInsert) -> i64 {
        0
    }
}

#[test]
fn plugins_and_their_automation_come_back_from_an_otio() {
    let (mut s, root, t0) = levels_project("otioplugrt");
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let tid = st.clips.values().find(|c| c.timeline_start == t0).unwrap().track_id.clone();
    let plugin = PluginInsert {
        id: "ins_gain".into(),
        plugin_id: "vst3:test".into(),
        format: "vst3".into(),
        name: "Gain".into(),
        vendor: "Test".into(),
        path: String::new(),
        bypass: false,
        state: Some("c3RhdGU=".into()),
        params: [(0u32, 0.3f64)].into_iter().collect(),
    };
    st.tracks.get_mut(&tid).unwrap().inserts = vec![Insert::HighPass { hz: 120.0, q: 0.7 }, Insert::Plugin(plugin.clone())];
    for (dx, v) in [(0, 0.1f32), (sr / 4, 0.9), (sr / 2, 0.4)] {
        editing::set_automation(st, "ins_gain", AutomationParam::Plugin(0), t0 + dx, v).unwrap();
    }
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.otio = true;
    o.force = true;
    o.plugins = Some(mixer::PluginHostRef(std::sync::Arc::new(GainPlugin)));
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());

    let (mut s2, root2) = session("otioplugrt2", "24", "01:00:00:00");
    s2.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root2, &file))).unwrap();
    let st2 = &s2.state;
    let c2 = st2.clips.values().find(|c| c.timeline_start == t0).unwrap();
    let t2 = &st2.tracks[&c2.track_id];
    assert_eq!(t2.inserts.len(), 2);
    assert!(matches!(t2.inserts[0], Insert::HighPass { hz, q } if hz == 120.0 && q == 0.7));
    let Insert::Plugin(p2) = &t2.inserts[1] else { panic!("{:?}", t2.inserts[1]) };
    assert_ne!(p2.id, plugin.id, "a new slot id in the new project");
    assert_eq!(PluginInsert { id: plugin.id.clone(), ..p2.clone() }, plugin, "id, format, state and values as they were");
    let mut pts: Vec<(i64, f32)> = st2.automation.values().filter(|a| a.owner_id == p2.id).map(|a| (a.sample, a.value)).collect();
    pts.sort_by_key(|p| p.0);
    assert_eq!(pts, vec![(t0, 0.1), (t0 + sr / 4, 0.9), (t0 + sr / 2, 0.4)], "the automation, exactly");

    // through the plugin, the same sound
    let n = s.state.clips.values().find(|c| c.timeline_start == t0).unwrap().length as usize;
    let host: std::sync::Arc<dyn mixer::PluginHost> = std::sync::Arc::new(GainPlugin);
    let a = mixer::Mixer::new(&s.state, mixer::RenderOptions::default()).with_plugins(Some(host.clone())).render_master(t0, n).unwrap();
    let b = mixer::Mixer::new(st2, mixer::RenderOptions::default()).with_plugins(Some(host)).render_master(t0, n).unwrap();
    assert!(a.peak() > 0.01);
    let max = a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-5, "mix differs by {max}");
}

/// The levels project with an automated test plugin on its track, exported as OTIO.
fn plugin_otio(name: &str) -> (state::Session, std::path::PathBuf, i64, std::path::PathBuf) {
    let (mut s, root, t0) = levels_project(name);
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let tid = st.clips.values().find(|c| c.timeline_start == t0).unwrap().track_id.clone();
    st.tracks.get_mut(&tid).unwrap().inserts = vec![Insert::Plugin(PluginInsert {
        id: "ins_gain".into(),
        plugin_id: "vst3:test".into(),
        format: "vst3".into(),
        name: "Gain".into(),
        vendor: "Test".into(),
        path: String::new(),
        bypass: false,
        state: None,
        params: Default::default(),
    })];
    for (dx, v) in [(0, 0.1f32), (sr / 4, 0.9), (sr / 2, 0.4)] {
        editing::set_automation(st, "ins_gain", AutomationParam::Plugin(0), t0 + dx, v).unwrap();
    }
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());
    (s, root, t0, file)
}

/// The imported plugin (the only one) and its parameter-0 lane, sorted.
fn plugin_and_lane(st: &ProjectState) -> (PluginInsert, Vec<(i64, f32)>) {
    let p = st.tracks.values().flat_map(|t| t.inserts.iter()).find_map(|i| i.as_plugin().cloned()).expect("a plugin");
    let mut pts: Vec<(i64, f32)> = st
        .automation
        .values()
        .filter(|a| a.owner_id == p.id && a.param == AutomationParam::Plugin(0))
        .map(|a| (a.sample, a.value))
        .collect();
    pts.sort_by_key(|x| x.0);
    (p, pts)
}

fn lane_value(pts: &[(i64, f32)], x: i64) -> f64 {
    let i = pts.partition_point(|p| p.0 <= x);
    let (a, b) = (pts[i.saturating_sub(1)], pts[i.min(pts.len() - 1)]);
    if b.0 > a.0 && x >= a.0 {
        a.1 as f64 + (b.1 - a.1) as f64 * (x - a.0) as f64 / (b.0 - a.0) as f64
    } else {
        a.1 as f64
    }
}

/// Rewrite the plugin effect's description in an exported OTIO.
fn edit_plugin_effect(file: &Path, out: &str, f: impl Fn(&mut serde_json::Value)) -> std::path::PathBuf {
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    for t in v["tracks"]["children"].as_array_mut().unwrap() {
        for e in t["effects"].as_array_mut().unwrap() {
            if e["metadata"]["vcsd"]["insert"]["type"] == "plugin" {
                f(e);
            }
        }
    }
    let p = file.with_file_name(out);
    std::fs::write(&p, serde_json::to_string(&v).unwrap()).unwrap();
    p
}

#[test]
fn plugin_automation_comes_back_from_keyframes_alone_and_a_disabled_effect_is_bypassed() {
    let (s, _, t0, file) = plugin_otio("otiopkf");
    let sr = s.state.project.sample_rate as i64;
    let (_, want) = plugin_and_lane(&s.state);
    // as a tool that keeps keyframes but not our raw points, and switches the effect off
    let bare = edit_plugin_effect(&file, "keyframes.otio", |e| {
        e["metadata"]["vcsd"]["automation"]["0"].as_object_mut().unwrap().remove("points");
        e["enabled"] = serde_json::json!(false);
    });
    let (mut s2, root2) = session("otiopkf2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &bare, &OtioImportOptions::for_project(&root2, &bare))).unwrap();
    assert_eq!((rep.inserts, rep.plugins, rep.automated_parameters), (1, 1, 1));
    let (p, got) = plugin_and_lane(&s2.state);
    assert!(p.bypass, "a disabled effect comes in bypassed");
    for k in 0..=200 {
        let x = t0 + k * (sr / 2) / 200;
        assert!(
            (lane_value(&got, x) - lane_value(&want, x)).abs() < 1e-4,
            "parameter at {x}: {} vs {}",
            lane_value(&got, x),
            lane_value(&want, x)
        );
    }
    for &(t, _) in &want {
        assert!(got.iter().any(|g| (g.0 - t).abs() <= 1), "breakpoint {t} within a sample: {got:?}");
    }

    // step keyframes: each value held until the next
    let steps = edit_plugin_effect(&file, "steps.otio", |e| {
        let a = &mut e["metadata"]["vcsd"]["automation"]["0"];
        a.as_object_mut().unwrap().remove("points");
        a["keyframe_interpolation"] = serde_json::json!("Constant");
    });
    let (mut s3, root3) = session("otiopkf3", "24", "01:00:00:00");
    s3.edit("i", |st| import(st, &steps, &OtioImportOptions::for_project(&root3, &steps))).unwrap();
    let (_, held) = plugin_and_lane(&s3.state);
    assert!((lane_value(&held, t0 + sr / 8) - 0.1).abs() < 1e-6, "held at 0.1 until 0.25 s: {held:?}");
    assert!((lane_value(&held, t0 + sr / 4 + sr / 8) - 0.9).abs() < 1e-6, "then 0.9: {held:?}");
}

#[test]
fn plugin_automation_keeps_its_timing_at_another_sample_rate() {
    let (_, _, t0, file) = plugin_otio("otiopsr");
    let root = tmp("otiopsr2");
    let mut s2 = state::Session::create(
        &root,
        NewProject {
            name: "44.1".into(),
            frame_rate: FrameRate::parse("24").unwrap(),
            sample_rate: 44_100,
            start_tc: "01:00:00:00".into(),
        },
    )
    .unwrap();
    s2.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root, &file))).unwrap();
    let (_, got) = plugin_and_lane(&s2.state);
    let at = |x: i64| (x as f64 * 44_100.0 / 48_000.0).round() as i64;
    let want: Vec<(i64, f32)> = vec![(at(t0), 0.1), (at(t0 + 12_000), 0.9), (at(t0 + 24_000), 0.4)];
    assert_eq!(got, want, "the same moments at 44.1 kHz");
}

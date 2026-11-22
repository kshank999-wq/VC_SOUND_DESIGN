//! OTIO import: our own export read back (the mix must survive), and a timeline
//! written by the OpenTimelineIO library in an editor's layout
//! (`scripts/otio/make_fixture.py`). The library test is skipped (loudly) when
//! it isn't installed: `python3 -m pip install opentimelineio==0.18.1`, or set
//! VCSD_OTIO_PYTHON.

mod common;
use common::*;
use std::path::Path;
use vcsd_core::builder::{populate, PopulateOptions};
use vcsd_core::otio_import::{import, BusLevels, OtioImportOptions};
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
    let fold = OtioImportOptions { buses: BusLevels::Fold, ..OtioImportOptions::for_project(&root2, &file) };
    let rep = s2.edit("i", |st| import(st, &file, &fold)).unwrap();
    assert_eq!(rep.bus_levels, "fold");
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

/// The levels project with bus automation: its track on a new sub-bus "FX Sub"
/// (with an automated plugin) under the track's bus (−2 dB, a fade), and the
/// master's balance moving. Exported as OTIO.
fn bus_otio(name: &str) -> (state::Session, i64, std::path::PathBuf, String) {
    let (mut s, root, t0) = levels_project(name);
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let tid = st.clips.values().find(|c| c.timeline_start == t0).unwrap().track_id.clone();
    let parent = st.tracks[&tid].bus_id.clone();
    let parent_name = st.buses[&parent].name.clone();
    let sub = mixer::add_bus(st, "FX Sub", Some(&parent)).unwrap();
    st.tracks.get_mut(&tid).unwrap().bus_id = sub.clone();
    let b = st.buses.get_mut(&sub).unwrap();
    b.gain_db = 1.5;
    b.inserts = vec![Insert::Plugin(PluginInsert {
        id: "ins_bus".into(),
        plugin_id: "vst3:test".into(),
        format: "vst3".into(),
        name: "Gain".into(),
        vendor: "Test".into(),
        path: String::new(),
        bypass: false,
        state: None,
        params: Default::default(),
    })];
    editing::set_automation(st, "ins_bus", AutomationParam::Plugin(0), t0, 0.2).unwrap();
    editing::set_automation(st, "ins_bus", AutomationParam::Plugin(0), t0 + sr / 3, 0.8).unwrap();
    st.buses.get_mut(&parent).unwrap().gain_db = -2.0;
    editing::set_automation(st, &parent, AutomationParam::VolumeDb, t0, 0.0).unwrap();
    editing::set_automation(st, &parent, AutomationParam::VolumeDb, t0 + sr / 2, -8.0).unwrap();
    let master = st.buses.values().find(|b| b.parent_id.is_none()).unwrap().id.clone();
    editing::set_automation(st, &master, AutomationParam::Pan, t0, 0.3).unwrap();
    editing::set_automation(st, &master, AutomationParam::Pan, t0 + sr, -0.3).unwrap();
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.otio = true;
    o.force = true;
    o.plugins = Some(mixer::PluginHostRef(std::sync::Arc::new(GainPlugin)));
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());
    (s, t0, file, parent_name)
}

/// A bus's lane as (sample, value), sorted.
fn bus_lane(st: &ProjectState, bus: &str, p: AutomationParam) -> Vec<(i64, f32)> {
    let mut v: Vec<(i64, f32)> =
        st.automation.values().filter(|a| a.owner_id == bus && a.param == p).map(|a| (a.sample, a.value)).collect();
    v.sort_by_key(|x| x.0);
    v
}

fn bus_named<'a>(st: &'a ProjectState, name: &str) -> &'a Bus {
    st.buses.values().find(|b| b.name == name).unwrap_or_else(|| panic!("bus {name}"))
}

#[test]
fn buses_and_their_automation_come_back_from_our_otio() {
    let (s, t0, file, parent_name) = bus_otio("otiobuses");
    let (mut s2, root2) = session("otiobuses2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root2, &file))).unwrap();
    assert_eq!((rep.bus_levels.as_str(), rep.buses_restored, rep.buses_created), ("buses", 3, 1), "{rep:?}");
    let (a, b) = (&s.state, &s2.state);
    // the tree: FX Sub created under the project's own bus of the same name
    let sub = bus_named(b, "FX Sub");
    let parent = bus_named(b, &parent_name);
    assert_eq!(sub.parent_id.as_deref(), Some(parent.id.as_str()));
    let c2 = b.clips.values().find(|c| c.timeline_start == t0).unwrap();
    assert_eq!(b.tracks[&c2.track_id].bus_id, sub.id, "the track routed to it");
    assert_eq!(b.tracks[&c2.track_id].gain_db, -3.0, "the track's own fader, not the folded level");
    // levels, inserts and automation as they were
    assert_eq!((sub.gain_db, parent.gain_db), (1.5, -2.0));
    for (x, y) in [(bus_named(a, &parent_name), parent), (bus_named(a, "Review Mix"), bus_named(b, "Review Mix"))] {
        for p in [AutomationParam::VolumeDb, AutomationParam::Pan] {
            assert_eq!(bus_lane(a, &x.id, p), bus_lane(b, &y.id, p), "{} {p:?}", x.name);
        }
    }
    let slot = sub.inserts[0].as_plugin().unwrap();
    assert_ne!(slot.id, "ins_bus");
    assert_eq!(bus_lane(b, &slot.id, AutomationParam::Plugin(0)), bus_lane(a, "ins_bus", AutomationParam::Plugin(0)));
    // the same mix, through the bus plugin
    let n = a.clips.values().find(|c| c.timeline_start == t0).unwrap().length as usize;
    let host: std::sync::Arc<dyn mixer::PluginHost> = std::sync::Arc::new(GainPlugin);
    let ma = mixer::Mixer::new(a, mixer::RenderOptions::default()).with_plugins(Some(host.clone())).render_master(t0, n).unwrap();
    let mb = mixer::Mixer::new(b, mixer::RenderOptions::default()).with_plugins(Some(host)).render_master(t0, n).unwrap();
    assert!(ma.peak() > 0.01);
    let max = ma.data.iter().zip(&mb.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-5, "mix differs by {max}");
}

#[test]
fn a_project_whose_buses_have_their_own_levels_keeps_them_unless_asked() {
    let (_, t0, file, parent_name) = bus_otio("otiobusy");
    let sr = 48_000i64;
    // the receiving project already mixed its bus: auto folds instead
    let (mut s2, root2) = session("otiobusy2", "24", "01:00:00:00");
    let pid = bus_named(&s2.state, &parent_name).id.clone();
    s2.edit("g", |st| {
        st.buses.get_mut(&pid).unwrap().gain_db = -4.0;
        editing::set_automation(st, &pid, AutomationParam::VolumeDb, 100 * sr, -1.0).map(|_| ())
    })
    .unwrap();
    let rep = s2.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root2, &file))).unwrap();
    assert_eq!(rep.bus_levels, "fold");
    assert!(rep.notes.iter().any(|n| n.starts_with("bus levels folded into the tracks") && n.contains(&parent_name)), "{:?}", rep.notes);
    let st2 = &s2.state;
    assert_eq!(st2.buses[&pid].gain_db, -4.0, "its level kept");
    assert_eq!(bus_lane(st2, &pid, AutomationParam::VolumeDb), vec![(100 * sr, -1.0)], "its automation kept");
    assert!(!st2.buses.values().any(|b| b.name == "FX Sub"), "no buses made");
    let c2 = st2.clips.values().find(|c| c.timeline_start == t0).unwrap();
    assert!(
        st2.automation.values().any(|a| a.owner_id == c2.track_id && a.param == AutomationParam::VolumeDb),
        "the track carries the bus fade"
    );

    // asked to restore: the file's level and lanes within its range, the project's beyond it
    let (mut s3, root3) = session("otiobusy3", "24", "01:00:00:00");
    let pid3 = bus_named(&s3.state, &parent_name).id.clone();
    s3.edit("g", |st| {
        st.buses.get_mut(&pid3).unwrap().gain_db = -4.0;
        editing::set_automation(st, &pid3, AutomationParam::VolumeDb, t0 + sr / 4, -30.0)?;
        editing::set_automation(st, &pid3, AutomationParam::VolumeDb, 100 * sr, -1.0).map(|_| ())
    })
    .unwrap();
    let o = OtioImportOptions { buses: BusLevels::Buses, ..OtioImportOptions::for_project(&root3, &file) };
    let rep = s3.edit("i", |st| import(st, &file, &o)).unwrap();
    assert_eq!(rep.bus_levels, "buses");
    let st3 = &s3.state;
    assert_eq!(st3.buses[&pid3].gain_db, -2.0);
    assert_eq!(bus_lane(st3, &pid3, AutomationParam::VolumeDb), vec![(t0, 0.0), (t0 + sr / 2, -8.0), (100 * sr, -1.0)]);
}

fn lane_of(st: &ProjectState, owner: &str, p: AutomationParam) -> Vec<(i64, f32)> {
    let mut v: Vec<(i64, f32)> =
        st.automation.values().filter(|a| a.owner_id == owner && a.param == p).map(|a| (a.sample, a.value)).collect();
    v.sort_by_key(|x| x.0);
    v
}

#[test]
fn a_stereo_track_split_left_right_by_the_aaf_adapter_comes_back_as_one() {
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
    let (mut s, root) = session("otiostereo", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:06:00", "wind exterior", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let lens: std::collections::HashMap<String, i64> = st.assets.values().map(|a| (a.id.clone(), a.length_samples)).collect();
    for c in st.clips.values_mut() {
        c.fill = FillMode::OneShot;
        c.length = c.length.min(lens[&c.asset_id] - c.source_offset);
    }
    let wind = st.clips.values().find(|c| !c.muted).unwrap().clone();
    assert_eq!(st.tracks[&wind.track_id].channels, 2);
    let t0 = wind.timeline_start;
    // balance swept across the centre, and a dip
    editing::set_automation(st, &wind.track_id, AutomationParam::Pan, t0, -0.6).unwrap();
    editing::set_automation(st, &wind.track_id, AutomationParam::Pan, t0 + 2 * sr, 0.6).unwrap();
    editing::set_automation(st, &wind.track_id, AutomationParam::VolumeDb, t0 + sr, 0.0).unwrap();
    editing::set_automation(st, &wind.track_id, AutomationParam::VolumeDb, t0 + 3 * sr, -12.0).unwrap();
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
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

    let (mut s2, root2) = session("otiostereo2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &otio, &OtioImportOptions::for_project(&root2, &otio))).unwrap();
    assert_eq!((rep.stereo_pairs, rep.tracks.len(), rep.clips), (1, 1, 1), "the L and R tracks as one: {rep:?}");
    let st2 = &s2.state;
    let c2 = st2.clips.values().next().unwrap();
    let t2 = &st2.tracks[&c2.track_id];
    assert_eq!(t2.channels, 2);
    let (a, b) = (&s.state, st2);
    let val = |l: &[(i64, f32)], x: i64, base: f64| {
        if l.is_empty() {
            return base;
        }
        let i = l.partition_point(|p| p.0 <= x);
        let (p, q) = (l[i.saturating_sub(1)], l[i.min(l.len() - 1)]);
        if q.0 > p.0 && x >= p.0 {
            p.1 as f64 + (q.1 - p.1) as f64 * (x - p.0) as f64 / (q.0 - p.0) as f64
        } else {
            p.1 as f64
        }
    };
    let (pa, pb) = (lane_of(a, &wind.track_id, AutomationParam::Pan), lane_of(b, &t2.id, AutomationParam::Pan));
    let (va, vb) = (lane_of(a, &wind.track_id, AutomationParam::VolumeDb), lane_of(b, &t2.id, AutomationParam::VolumeDb));
    for k in 0..=200 {
        let x = t0 + wind.length * k / 200;
        // what each side plays: the level, then the balance (centre = unity, the far side dips)
        let sides = |level_db: f64, bal: f64| {
            let g = 10f64.powf(level_db / 20.0);
            let b = bal.clamp(-1.0, 1.0);
            if b <= 0.0 {
                (g, g * (1.0 + b))
            } else {
                (g * (1.0 - b), g)
            }
        };
        let (la, ra) = sides(val(&va, x, 0.0), val(&pa, x, 0.0));
        let (lb, rb) = sides(val(&vb, x, 0.0) + t2.gain_db as f64, val(&pb, x, t2.pan as f64));
        for (side, want, got) in [("left", la, lb), ("right", ra, rb)] {
            assert!((20.0 * (got / want).log10()).abs() < 0.25, "{side} at {x}: {got:.5} vs {want:.5}");
        }
    }
}

#[test]
fn stepped_pan_keyframes_come_back_held() {
    let (mut s, root, t0) = levels_project("otiopanstep");
    let sr = s.state.project.sample_rate as i64;
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    strip_vcsd(&mut v);
    for t in v["tracks"]["children"].as_array_mut().unwrap() {
        for e in t["effects"].as_array_mut().unwrap() {
            if e["name"] == "Mono Audio Pan" {
                e["metadata"]["AAF"]["Parameters"]["Pan"]["keyframe_interpolation"] = serde_json::json!("Constant");
            }
        }
    }
    let stepped = file.with_file_name("stepped.otio");
    std::fs::write(&stepped, serde_json::to_string(&v).unwrap()).unwrap();
    let (mut s2, root2) = session("otiopanstep2", "24", "01:00:00:00");
    s2.edit("i", |st| import(st, &stepped, &OtioImportOptions::for_project(&root2, &stepped))).unwrap();
    let c2 = s2.state.clips.values().find(|c| c.timeline_start == t0).unwrap();
    let pan = lane_of(&s2.state, &c2.track_id, AutomationParam::Pan);
    // the sweep −1 → +0.4 over 0.5 s, held at its keyframes: hard left until the next one
    let at = |x: i64| {
        let i = pan.partition_point(|p| p.0 <= x);
        let (p, q) = (pan[i.saturating_sub(1)], pan[i.min(pan.len() - 1)]);
        if q.0 > p.0 && x >= p.0 {
            p.1 as f64 + (q.1 - p.1) as f64 * (x - p.0) as f64 / (q.0 - p.0) as f64
        } else {
            p.1 as f64
        }
    };
    assert!((at(t0 + sr / 8) + 1.0).abs() < 1e-3, "held at −1 a quarter-second in: {pan:?}");
    assert!((at(t0 + sr / 2 + sr / 8) - 0.4).abs() < 1e-3, "then +0.4: {pan:?}");
}

#[test]
fn volume_automation_comes_back_from_an_editors_aaf_converted_by_the_aaf_adapter() {
    let Some(py) = python() else {
        eprintln!("SKIPPED: OpenTimelineIO not installed");
        return;
    };
    let has = |m: &str| {
        std::process::Command::new(&py).args(["-c", &format!("import {m}")]).output().map(|o| o.status.success()).unwrap_or(false)
    };
    if !has("otio_aaf_adapter") || !has("aaf2") {
        eprintln!("SKIPPED: the OpenTimelineIO AAF adapter or pyaaf2 isn't installed");
        return;
    }
    // the AAF import's editor-style fixture (pyaaf2), converted to OTIO by the AAF adapter
    let dir = tmp("otioaaffixture");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/aaf/make_fixture.py");
    let out = std::process::Command::new(&py).arg(script).arg(&dir).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let otio = dir.join("fixture.otio");
    let conv = "import sys, opentimelineio as otio\n\
                tl = otio.adapters.read_from_file(sys.argv[1], adapter_name='AAF', simplify=True)\n\
                otio.adapters.write_to_file(tl, sys.argv[2])";
    let out = std::process::Command::new(&py).args(["-c", conv]).arg(dir.join("fixture.aaf")).arg(&otio).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let (mut s, root) = session("otioaaffix", "25", "10:00:00:00");
    let rep = s.edit("i", |st| import(st, &otio, &OtioImportOptions::for_project(&root, &otio))).unwrap();
    assert!(rep.notes.iter().any(|n| n.starts_with("gain above +12 dB clamped")), "{:?}", rep.notes);
    let st = &s.state;
    let f = 1920; // samples per frame at 25 fps
    let vol = |tid: &str| lane_of(st, tid, AutomationParam::VolumeDb);
    let at = |l: &[(i64, f32)], x: i64| {
        let i = l.partition_point(|p| p.0 <= x);
        let (p, q) = (l[i.saturating_sub(1)], l[i.min(l.len() - 1)]);
        if q.0 > p.0 && x >= p.0 {
            p.1 as f64 + (q.1 - p.1) as f64 * (x - p.0) as f64 / (q.0 - p.0) as f64
        } else {
            p.1 as f64
        }
    };
    // CLIPGAIN (the adapter drops track names; its clip is the one 10 frames in): the clip's
    // gain envelope 1 → 0.25 → 1 as the track's volume lane
    let cg = st.clips.values().find(|c| c.timeline_start == 10 * f && c.length == 40 * f).expect("the CLIPGAIN clip");
    let env = vol(&cg.track_id);
    for k in 0..40 * f {
        let u = k as f64 / (40 * f) as f64;
        let amp = if u <= 0.5 { 1.0 - 1.5 * u } else { 0.25 + 1.5 * (u - 0.5) };
        let (want, got) = (20.0 * amp.log10(), at(&env, 10 * f + k));
        assert!((got - want).abs() < 0.11, "clip envelope at {k}: {got:.3} vs {want:.3} dB");
    }
    // LEVEL: Stereo Audio Gain, its Level stepping from −6 dB to +18 dB (clamped to +12)
    let lv = st
        .tracks
        .keys()
        .map(|t| vol(t))
        .find(|l| l.iter().any(|p| (p.1 - 12.0).abs() < 1e-6))
        .expect("the LEVEL track's lane, reaching +12 dB");
    assert!((at(&lv, 10 * f) + 6.0206).abs() < 0.01 && (at(&lv, 25 * f - 1) + 6.0206).abs() < 0.01, "held: {lv:?}");
    assert!((at(&lv, 25 * f) - 12.0).abs() < 1e-6 && (at(&lv, 45 * f) - 12.0).abs() < 1e-6, "then +12: {lv:?}");
    // MX: the track gain curve 1 → 0.25 → 1 over 100 frames, under a constant clip gain of 0.5
    let mx = st
        .clips
        .values()
        .find(|c| c.timeline_start == 0 && c.length == 75 * f && (c.gain_db + 6.0206).abs() < 0.01)
        .expect("the MX clip, its gain −6 dB");
    let ml = vol(&mx.track_id);
    for k in 0..=15 {
        let x = (k * 100 * f / 20).min(75 * f - 1);
        let u = x as f64 / (100 * f) as f64;
        let amp = if u <= 0.5 { 1.0 - 1.5 * u } else { 0.25 + 1.5 * (u - 0.5) };
        assert!((at(&ml, x) - 20.0 * amp.log10()).abs() < 0.11, "MX volume at {u:.2}: {}", at(&ml, x));
    }
}

#[test]
fn importing_back_into_the_project_it_came_from_keeps_its_buses_and_doesnt_double_their_levels() {
    let (mut s, t0, file, parent_name) = bus_otio("otioback");
    let before_auto = s.state.automation.len();
    let sub = bus_named(&s.state, "FX Sub").id.clone();
    let parent = bus_named(&s.state, &parent_name).id.clone();
    let orig_track = s.state.clips.values().find(|c| c.timeline_start == t0).unwrap().track_id.clone();
    let orig_lanes = [AutomationParam::VolumeDb, AutomationParam::Pan].map(|p| bus_lane(&s.state, &orig_track, p));

    // the editor sends it back: the project's buses already hold these levels
    let o = OtioImportOptions { folder: Some("Back".into()), ..OtioImportOptions::for_project(s.root().unwrap(), &file) };
    let rep = s.edit("i", |st| import(st, &file, &o)).unwrap();
    assert_eq!((rep.bus_levels.as_str(), rep.buses_matched, rep.buses_restored, rep.buses_created), ("buses", 3, 0, 0), "{rep:?}");
    assert!(rep.notes.iter().all(|n| !n.starts_with("bus levels folded")), "{:?}", rep.notes);
    let st = &s.state;
    // the buses untouched: same plugin slot, same lanes
    assert_eq!(bus_named(st, "FX Sub").inserts[0].as_plugin().unwrap().id, "ins_bus");
    assert_eq!(st.buses.values().filter(|b| b.name == "FX Sub").count(), 1, "no second FX Sub");
    // the new track: on the same sub-bus, with its own levels (not the buses' folded in)
    let new_track = st.tracks.values().find(|t| vcsd_core::editing::in_folder(&t.folder, "Back")).unwrap();
    assert_eq!(new_track.bus_id, sub);
    assert_eq!(new_track.gain_db, -3.0);
    for (p, want) in [AutomationParam::VolumeDb, AutomationParam::Pan].into_iter().zip(&orig_lanes) {
        assert_eq!(&bus_lane(st, &new_track.id, p), want, "{p:?}");
    }
    let added = st.automation.len() - before_auto;
    let own: usize = orig_lanes.iter().map(|l| l.len()).sum();
    assert_eq!(added, own, "only the new track's own lanes added: nothing on the buses");

    // after the bus has changed, auto folds (and says which bus differs)
    let (mut s2, _, file2, parent_name2) = bus_otio("otioback2");
    let p2 = bus_named(&s2.state, &parent_name2).id.clone();
    s2.edit("g", |st| {
        st.buses.get_mut(&p2).unwrap().gain_db = -5.0;
        Ok(())
    })
    .unwrap();
    let o2 = OtioImportOptions::for_project(s2.root().unwrap(), &file2);
    let rep = s2.edit("i", |st| import(st, &file2, &o2)).unwrap();
    assert_eq!(rep.bus_levels, "fold");
    assert!(
        rep.notes.iter().any(|n| n.starts_with("bus levels folded") && n.contains(&parent_name2) && !n.contains("FX Sub")),
        "{:?}",
        rep.notes
    );
    let _ = parent;
}

#[test]
fn muted_tracks_kept_disabled_in_our_otio_come_back_muted() {
    let (mut s, root) = session("otiomuted", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:06:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:05:00", "01:00:05:00", "door slam", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    let lens: std::collections::HashMap<String, i64> = st.assets.values().map(|a| (a.id.clone(), a.length_samples)).collect();
    for c in st.clips.values_mut() {
        c.fill = FillMode::OneShot;
        c.length = c.length.min(lens[&c.asset_id] - c.source_offset);
    }
    let clip_of = |st: &ProjectState, what: &str| {
        let cue = st.cues.values().find(|c| c.description.contains(what)).unwrap().id.clone();
        st.clips.values().find(|c| c.cue_id.as_deref() == Some(cue.as_str()) && !c.muted).unwrap().id.clone()
    };
    let (glass, wind, door) = (clip_of(st, "glass"), clip_of(st, "wind"), clip_of(st, "door"));
    let gt = st.clips[&glass].track_id.clone();
    st.tracks.get_mut(&gt).unwrap().mute = true;
    st.tracks.get_mut(&gt).unwrap().gain_db = -3.0;
    let wbus = st.tracks[&st.clips[&wind].track_id].bus_id.clone();
    st.buses.get_mut(&wbus).unwrap().mute = true;
    st.clips.get_mut(&door).unwrap().muted = true;
    let [g0, w0, d0] = [&glass, &wind, &door].map(|c| st.clips[c].timeline_start);
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.otio = true;
    o.force = true;
    o.include_muted = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());

    // other apps: the tracks and the clip disabled (OTIO's own mute)
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let tracks = raw["tracks"]["children"].as_array().unwrap();
    let gname = &s.state.tracks[&gt].name;
    let by_name = |n: &str| tracks.iter().find(|t| t["name"].as_str().unwrap().contains(n)).unwrap_or_else(|| panic!("{n}"));
    assert_eq!(by_name(gname)["enabled"], false);
    assert_eq!(by_name("wind")["enabled"], false, "on a muted bus");
    let disabled_clips = tracks
        .iter()
        .flat_map(|t| t["children"].as_array().unwrap())
        .filter(|c| c["OTIO_SCHEMA"] == "Clip.2" && c["enabled"] == false)
        .count();
    assert_eq!(disabled_clips, 1, "the muted clip, disabled (the others play, inside their disabled tracks)");
    if let Some(py) = python() {
        let out = std::process::Command::new(&py)
            .args(["-c", "import sys, opentimelineio as o; t = o.adapters.read_from_file(sys.argv[1]); print(sum(1 for x in t.tracks if not x.enabled))"])
            .arg(&file)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "2", "{}", String::from_utf8_lossy(&out.stderr));
    }

    // our import, restoring the buses: muted as it was
    let (mut s2, root2) = session("otiomuted2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root2, &file))).unwrap();
    assert_eq!(rep.bus_levels, "buses");
    let st2 = &s2.state;
    let at = |t: i64| st2.clips.values().find(|c| c.timeline_start == t).unwrap_or_else(|| panic!("clip at {t}"));
    let g = &st2.tracks[&at(g0).track_id];
    assert!(g.mute && g.gain_db == -3.0, "the muted track, its fader intact");
    let w = &st2.tracks[&at(w0).track_id];
    assert!(!w.mute && st2.buses[&w.bus_id].mute, "the wind's track unmuted, on its muted bus");
    assert!(at(d0).muted, "the muted clip");

    // folding the buses: both silent tracks muted
    let (mut s3, root3) = session("otiomuted3", "24", "01:00:00:00");
    let fold = OtioImportOptions { buses: BusLevels::Fold, ..OtioImportOptions::for_project(&root3, &file) };
    s3.edit("i", |st| import(st, &file, &fold)).unwrap();
    let st3 = &s3.state;
    let at3 = |t: i64| st3.clips.values().find(|c| c.timeline_start == t).unwrap();
    assert!(st3.tracks[&at3(g0).track_id].mute && st3.tracks[&at3(w0).track_id].mute);
    assert!(at3(d0).muted);
}

#[test]
fn mutes_carried_as_zero_gains_by_an_aaf_through_the_adapter_come_back_as_mutes() {
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
    // a muted glass track (−3 dB fader, −6 dB clip), the wind on a muted bus, a muted door clip (−4 dB)
    let (mut s, root) = session("otiozero", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:06:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:05:00", "01:00:05:00", "door slam", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    let lens: std::collections::HashMap<String, i64> = st.assets.values().map(|a| (a.id.clone(), a.length_samples)).collect();
    for c in st.clips.values_mut() {
        c.fill = FillMode::OneShot;
        c.length = c.length.min(lens[&c.asset_id] - c.source_offset);
    }
    let clip_of = |st: &ProjectState, what: &str| {
        let cue = st.cues.values().find(|c| c.description.contains(what)).unwrap().id.clone();
        st.clips.values().find(|c| c.cue_id.as_deref() == Some(cue.as_str()) && !c.muted).unwrap().id.clone()
    };
    let (glass, wind, door) = (clip_of(st, "glass"), clip_of(st, "wind"), clip_of(st, "door"));
    st.clips.get_mut(&glass).unwrap().gain_db = -6.0;
    let gt = st.clips[&glass].track_id.clone();
    st.tracks.get_mut(&gt).unwrap().gain_db = -3.0;
    st.tracks.get_mut(&gt).unwrap().mute = true;
    let wbus = st.tracks[&st.clips[&wind].track_id].bus_id.clone();
    st.buses.get_mut(&wbus).unwrap().mute = true;
    st.clips.get_mut(&door).unwrap().muted = true;
    st.clips.get_mut(&door).unwrap().gain_db = -4.0;
    let [g0, w0, d0] = [&glass, &wind, &door].map(|c| st.clips[c].timeline_start);
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.aaf = true;
    o.force = true;
    o.include_muted = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let aaf = r.dir.join(r.files.iter().find(|f| f.ends_with(".aaf")).unwrap());
    let otio = r.dir.join("from_aaf.otio");
    let conv = "import sys, opentimelineio as otio\n\
                tl = otio.adapters.read_from_file(sys.argv[1], adapter_name='AAF', simplify=True)\n\
                otio.adapters.write_to_file(tl, sys.argv[2])";
    let out = std::process::Command::new(&py).args(["-c", conv]).arg(&aaf).arg(&otio).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let (mut s2, root2) = session("otiozero2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &otio, &OtioImportOptions::for_project(&root2, &otio))).unwrap();
    assert!(rep.notes.iter().any(|n| n.starts_with("zero gains read as mutes")), "{:?}", rep.notes);
    let st2 = &s2.state;
    let at = |t: i64| st2.clips.values().find(|c| c.timeline_start == t).unwrap_or_else(|| panic!("clip at {t}"));
    let g = at(g0);
    assert!(st2.tracks[&g.track_id].mute, "the muted track, muted");
    assert!((g.gain_db + 9.0).abs() < 0.01, "its levels kept (clip −6 with the −3 fader the AAF folds in): {}", g.gain_db);
    let w = &st2.tracks[&at(w0).track_id];
    assert!(w.mute && w.channels == 2, "the muted bus's stereo track: one track, muted");
    let d = at(d0);
    assert!(d.muted && (d.gain_db + 4.0).abs() < 0.01, "the muted clip, with its own gain: {} {}", d.muted, d.gain_db);
    assert!(!st2.tracks[&d.track_id].mute);
}

#[test]
fn a_disabled_compound_clip_comes_back_muted() {
    let Some(py) = python() else {
        eprintln!("SKIPPED: OpenTimelineIO not installed (python3 -m pip install opentimelineio==0.18.1)");
        return;
    };
    let dir = tmp("otiodisabledstack");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/otio/make_fixture.py");
    let out = std::process::Command::new(&py).arg(script).arg(&dir).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // switch the nested stack (MX's compound clip) off
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("fixture.otio")).unwrap()).unwrap();
    let mx = v["tracks"]["children"].as_array_mut().unwrap().iter_mut().find(|t| t["name"] == "MX").unwrap();
    let stack =
        mx["children"].as_array_mut().unwrap().iter_mut().find(|c| c["OTIO_SCHEMA"].as_str().unwrap().starts_with("Stack")).unwrap();
    stack["enabled"] = serde_json::json!(false);
    let file = dir.join("disabled.otio");
    std::fs::write(&file, serde_json::to_string(&v).unwrap()).unwrap();
    let (mut s, root) = session("otiodisabledstack2", "25", "10:00:00:00");
    s.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root, &file))).unwrap();
    let st = &s.state;
    let inner = st.tracks.values().find(|t| t.name == "MX / inner").expect("the compound's track");
    assert!(inner.mute, "muted with its compound clip");
    assert!(!st.tracks.values().find(|t| t.name == "MX").unwrap().mute, "the track around it plays");
}

#[test]
fn solo_comes_back_from_our_otio_and_says_when_it_silences_the_project() {
    let (mut s, root, t0) = levels_project("otiosolo");
    let tid = s.state.clips.values().find(|c| c.timeline_start == t0).unwrap().track_id.clone();
    s.state.tracks.get_mut(&tid).unwrap().solo = true;
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let track = &raw["tracks"]["children"][0];
    assert_eq!(
        (track["metadata"]["vcsd"]["solo"].as_bool(), track["enabled"].as_bool()),
        (Some(true), Some(true)),
        "flagged, still audible"
    );
    let rep: serde_json::Value = serde_json::from_slice(&std::fs::read(r.dir.join("OTIO/otio_report.json")).unwrap()).unwrap();
    assert!(rep["losses"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().starts_with("solo is monitoring state")));

    // into a fresh project: soloed, nothing else to silence
    let (mut s2, root2) = session("otiosolo2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &file, &OtioImportOptions::for_project(&root2, &file))).unwrap();
    assert_eq!(rep.soloed_tracks, 1);
    assert!(s2.state.tracks.values().all(|t| t.solo));
    assert!(!rep.notes.iter().any(|n| n.starts_with("solo restored")), "{:?}", rep.notes);

    // into a project with other tracks, none soloed: restored, and the report says what it does
    let mut v = raw.clone();
    v["tracks"]["children"][0]["metadata"]["vcsd"]["solo"] = serde_json::json!(false);
    let plain = file.with_file_name("plain.otio");
    std::fs::write(&plain, serde_json::to_string(&v).unwrap()).unwrap();
    let (mut s3, root3) = session("otiosolo3", "24", "01:00:00:00");
    s3.edit("a", |st| {
        import(st, &plain, &OtioImportOptions { folder: Some("Other".into()), ..OtioImportOptions::for_project(&root3, &plain) })
    })
    .unwrap();
    let rep = s3.edit("b", |st| import(st, &file, &OtioImportOptions::for_project(&root3, &file))).unwrap();
    assert_eq!(rep.soloed_tracks, 1);
    assert!(rep.notes.iter().any(|n| n.starts_with("solo restored on imported tracks")), "{:?}", rep.notes);
    assert!(s3.state.tracks.values().filter(|t| vcsd_core::editing::in_folder(&t.folder, "Other")).all(|t| !t.solo));
}

#[test]
fn track_folders_and_order_come_back_from_our_otio() {
    let (mut s, root) = session("otioorder", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:06:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:05:00", "01:00:05:00", "door slam", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    // the project's layout: tracks in more than one folder, a custom order
    let st = &mut s.state;
    let mut ids: Vec<String> = st.tracks.keys().cloned().collect();
    ids.sort();
    st.tracks.get_mut(&ids[0]).unwrap().folder = "Reel A/Specials".into();
    for (k, id) in ids.iter().rev().enumerate() {
        st.tracks.get_mut(id).unwrap().order = k as u32;
    }
    let mut want: Vec<(String, String)> = st.tracks.values().map(|t| (t.folder.clone(), t.name.clone())).collect();
    let mut sorted: Vec<&Track> = st.tracks.values().collect();
    sorted.sort_by(|a, b| (&a.folder, a.order, &a.name).cmp(&(&b.folder, b.order, &b.name)));
    let want_order: Vec<String> = sorted.iter().map(|t| t.name.clone()).collect();
    want.sort();
    assert!(want.iter().map(|w| &w.0).collect::<std::collections::BTreeSet<_>>().len() >= 2, "more than one folder: {want:?}");
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());

    // into a project that already has tracks
    let (mut s2, root2) = session("otioorder2", "24", "01:00:00:00");
    s2.edit("a", |st| {
        import(st, &file, &OtioImportOptions { folder: Some("Earlier".into()), ..OtioImportOptions::for_project(&root2, &file) })
    })
    .unwrap();
    let before = s2.state.tracks.values().map(|t| t.order).max().unwrap();
    let rep = s2.edit("b", |st| import(st, &file, &OtioImportOptions::for_project(&root2, &file))).unwrap();
    let st2 = &s2.state;
    let base = format!("OTIO/{}", rep.timeline.replace('/', " & "));
    let mut got: Vec<&Track> = rep.tracks.iter().map(|t| &st2.tracks[t]).collect();
    // the folder tree rebuilt under the import's folder
    let mut layout: Vec<(String, String)> =
        got.iter().map(|t| (t.folder.strip_prefix(&format!("{base}/")).unwrap_or(&t.folder).to_string(), t.name.clone())).collect();
    layout.sort();
    assert_eq!(layout, want, "each track in its own folder, under {base}");
    // the project's order kept, after the tracks already there
    got.sort_by_key(|t| t.order);
    assert_eq!(got.iter().map(|t| t.name.clone()).collect::<Vec<_>>(), want_order);
    assert!(got.iter().all(|t| t.order > before), "after the existing tracks");
}

#[test]
fn track_colors_come_back_from_otio() {
    assert_eq!(parse_hex_color("#FA0"), Some("#ffaa00".into()));
    assert_eq!(parse_hex_color("#12345678"), Some("#123456".into()), "alpha dropped");
    assert_eq!(parse_hex_color("red"), None);

    let (mut s, root, t0) = levels_project("otiocolor");
    let tid = s.state.clips.values().find(|c| c.timeline_start == t0).unwrap().track_id.clone();
    s.state.tracks.get_mut(&tid).unwrap().color = Some("#3a7bd5".into());
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let file = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());
    let color_after = |file: &Path, name: &str| {
        let (mut s2, root2) = session(name, "24", "01:00:00:00");
        s2.edit("i", |st| import(st, file, &OtioImportOptions::for_project(&root2, file))).unwrap();
        s2.state.tracks.values().next().unwrap().color.clone()
    };
    // our own: round trip
    assert_eq!(color_after(&file, "otiocolor2"), Some("#3a7bd5".into()));

    // other apps': OTIO 0.18's own Color, or a named color, without our metadata
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    strip_vcsd(&mut v);
    v["tracks"]["children"][0]["color"] = serde_json::json!({"OTIO_SCHEMA": "Color.1", "r": 1.0, "g": 0.5, "b": 0.0, "a": 1.0, "name": ""});
    let native = file.with_file_name("native.otio");
    std::fs::write(&native, serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(color_after(&native, "otiocolor3"), Some("#ff7f00".into()), "as OpenTimelineIO converts it (below)");
    v["tracks"]["children"][0]["color"] = serde_json::json!("ORANGE");
    let named = file.with_file_name("named.otio");
    std::fs::write(&named, serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(color_after(&named, "otiocolor4"), Some("#ff7f00".into()));
    v["tracks"]["children"][0].as_object_mut().unwrap().remove("color");
    let none = file.with_file_name("none.otio");
    std::fs::write(&none, serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(color_after(&none, "otiocolor5"), None, "none: the category's");

    // OpenTimelineIO itself reads a file with a track Color (0.18)
    if let Some(py) = python() {
        let out = std::process::Command::new(&py)
            .args([
                "-c",
                "import sys, opentimelineio as o; t = o.adapters.read_from_file(sys.argv[1]); print(t.tracks[0].color.to_hex()[:7])",
            ])
            .arg(&native)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "#ff7f00", "{}", String::from_utf8_lossy(&out.stderr));
    }
}

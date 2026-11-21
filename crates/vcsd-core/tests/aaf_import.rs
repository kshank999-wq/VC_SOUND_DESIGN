//! AAF import: our own export read back (the mix must survive), and an AAF
//! written by pyaaf2 in an editor's layout (`scripts/aaf/make_fixture.py`).
//! The pyaaf2 test is skipped (loudly) when pyaaf2 isn't installed:
//! `python3 -m pip install pyaaf2==1.7.1`, or set VCSD_AAF_PYTHON.

mod common;
use common::*;
use std::path::Path;
use vcsd_core::aaf_import::{import, AafImportOptions};
use vcsd_core::builder::{populate, PopulateOptions};
use vcsd_core::spotting::*;
use vcsd_core::*;

fn python() -> Option<String> {
    let py = std::env::var("VCSD_AAF_PYTHON").unwrap_or_else(|_| "python3".into());
    let ok = std::process::Command::new(&py).args(["-c", "import aaf2"]).output().map(|o| o.status.success()).unwrap_or(false);
    ok.then_some(py)
}

fn opts(root: &Path) -> AafImportOptions {
    AafImportOptions { media_dir: root.join("Audio/Imported/AAF"), ..Default::default() }
}

#[test]
fn an_exported_aaf_imports_back_to_the_same_mix() {
    let (mut s, root) = session("aafrt", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:06:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:05:00", "01:00:05:00", "door slam", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    // no loops: AAF butt-splices loop repeats where the mixer crossfades them
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
    let g = st.clips.get_mut(&glass).unwrap();
    g.gain_db = -6.0;
    g.fade_in = 480;
    g.fade_out = 960;
    // the door's sound is stereo on a mono track (AAF keeps its left channel only): use a mono one
    let mono = st.clips[&glass].asset_id.clone();
    let dc = st.clips.get_mut(&door).unwrap();
    dc.asset_id = mono;
    dc.length = dc.length.min(lens[&dc.asset_id]);
    let (gt, wt, dt) = (st.clips[&glass].track_id.clone(), st.clips[&wind].track_id.clone(), st.clips[&door].track_id.clone());
    st.tracks.get_mut(&gt).unwrap().gain_db = -3.0;
    st.tracks.get_mut(&dt).unwrap().pan = 0.5;
    let t0 = st.clips[&glass].timeline_start;
    for (dx, db) in [(0, 0.0), (sr / 4, -18.0), (sr / 2, 0.0)] {
        editing::set_automation(st, &gt, AutomationParam::VolumeDb, t0 + dx, db).unwrap();
    }
    // the stereo background: balance swept left → right, and a fade down
    let w0 = st.clips[&wind].timeline_start;
    editing::set_automation(st, &wt, AutomationParam::Pan, w0, -0.6).unwrap();
    editing::set_automation(st, &wt, AutomationParam::Pan, w0 + 2 * sr, 0.6).unwrap();
    editing::set_automation(st, &wt, AutomationParam::VolumeDb, w0 + sr, 0.0).unwrap();
    editing::set_automation(st, &wt, AutomationParam::VolumeDb, w0 + 3 * sr, -12.0).unwrap();
    assert_eq!(st.tracks[&wt].channels, 2, "the background is a stereo track");

    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.aaf = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let aaf = r.dir.join(r.files.iter().find(|f| f.ends_with(".aaf")).unwrap());

    // into a fresh project at the same rate and start
    let (mut s2, root2) = session("aafrt2", "24", "01:00:00:00");
    scene(&mut s2, "1", "01:00:00:00", "01:00:10:00");
    let rep = s2.edit("i", |st| import(st, &aaf, &opts(&root2))).unwrap();
    assert!(rep.missing_media.is_empty(), "{:?}", rep.missing_media);
    assert_eq!(rep.start_tc, "01:00:00:00");
    assert!(rep.stereo_tracks >= 1, "the L/R pair came back as one stereo track: {rep:?}");
    assert_eq!(rep.markers, s.state.cues.len(), "cue markers → spotting markers");
    assert!(rep.automated_tracks >= 2, "{rep:?}");
    let st2 = &s2.state;
    assert!(st2.tracks.values().all(|t| t.folder.starts_with("AAF/")));
    // the glass clip: same place, fades and source range
    let glass_lane: Vec<(i64, f32)> = {
        let gt2 = &st2.clips.values().find(|c| c.timeline_start == t0).unwrap().track_id;
        let mut v: Vec<(i64, f32)> = st2
            .automation
            .values()
            .filter(|p| &p.owner_id == gt2 && p.param == AutomationParam::VolumeDb)
            .map(|p| (p.sample, p.value))
            .collect();
        v.sort_by_key(|p| p.0);
        v
    };
    assert!(glass_lane.len() <= 4, "a dip in dB comes back as its breakpoints: {glass_lane:?}");
    assert!(glass_lane.iter().any(|p| p.0 == t0 + sr / 4 && (p.1 + 18.0).abs() < 0.05), "{glass_lane:?}");
    let gc = st2.clips.values().find(|c| c.timeline_start == t0 && c.fade_in == 480).expect("glass clip");
    assert_eq!(gc.fade_out, 960);
    assert_eq!((gc.length, gc.source_offset), (s.state.clips[&glass].length, s.state.clips[&glass].source_offset));
    assert!((gc.gain_db + 9.0).abs() < 0.01, "clip gain −6 with the −3 fader folded in: {}", gc.gain_db);
    assert!(st2.tracks.values().any(|t| t.channels == 1 && (t.pan - 0.5).abs() < 0.01), "the door's pan");

    // the mix survives: master renders agree within the 0.1 dB curve fits (both ways)
    let (start, end) = mixer::timeline_extent(&s.state);
    let n = (end - start) as usize;
    let a = mixer::Mixer::new(&s.state, mixer::RenderOptions::default()).render_master(start, n).unwrap();
    let b = mixer::Mixer::new(st2, mixer::RenderOptions::default()).render_master(start, n).unwrap();
    assert!(a.peak() > 0.05);
    let max = a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(max < 0.03 * a.peak(), "mix differs by {max} (peak {})", a.peak());

    // importing again reuses the same media (no new assets)
    let assets = s2.state.assets.len();
    s2.edit("i2", |st| import(st, &aaf, &opts(&root2))).unwrap();
    assert_eq!(s2.state.assets.len(), assets);
}

#[test]
fn an_editors_aaf_imports_clips_fades_gain_media_and_markers() {
    let Some(py) = python() else {
        eprintln!("SKIPPED: pyaaf2 not installed (python3 -m pip install pyaaf2==1.7.1)");
        return;
    };
    let dir = tmp("aaffixture");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/aaf/make_fixture.py");
    let out = std::process::Command::new(&py).arg(script).arg(&dir).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let aaf = dir.join("fixture.aaf");

    let (mut s, root) = session("aafimp", "25", "10:00:00:00");
    scene(&mut s, "1", "10:00:00:00", "10:00:10:00");
    let rep = s.edit("i", |st| import(st, &aaf, &opts(&root))).unwrap();
    assert_eq!(rep.composition, "Reel 1 v3");
    assert_eq!(rep.start_tc, "10:00:00:00");
    assert!(rep.missing_media.is_empty(), "{:?}", rep.missing_media);
    assert_eq!(rep.tracks.len(), 3, "three sound tracks (the picture track is ignored): {rep:?}");
    assert_eq!(rep.clips, 4);
    assert_eq!(rep.markers, 1);
    let st = &s.state;
    let f = 1920; // samples per frame at 25 fps
    let track = |name: &str| st.tracks.values().find(|t| t.name == name).unwrap_or_else(|| panic!("track {name}"));
    let clips = |tid: &str| {
        let mut v: Vec<&ClipInstance> = st.clips.values().filter(|c| c.track_id == tid).collect();
        v.sort_by_key(|c| c.timeline_start);
        v
    };

    // DIA 1: two clips crossfaded by the transition; channel 1 of the embedded stereo file
    let dia = track("DIA 1");
    assert_eq!((dia.channels, dia.category.clone()), (1, Category::Dialogue));
    let d = clips(&dia.id);
    assert_eq!(d.len(), 2);
    assert_eq!((d[0].timeline_start, d[0].length, d[0].source_offset), (25 * f, 50 * f, 0));
    assert_eq!((d[1].timeline_start, d[1].length, d[1].source_offset), (65 * f, 40 * f, 60 * f));
    assert_eq!((d[0].fade_out, d[1].fade_in), (10 * f, 10 * f), "the transition as a crossfade");
    let a = &st.assets[&d[0].asset_id];
    assert_eq!(a.channels, 1, "a mono extract of channel 1");
    let (_, extract) = wav::read_raw(Path::new(&a.path)).unwrap();
    let (_, orig) = wav::read_raw(&dir.join("dialog.wav")).unwrap();
    let left: Vec<u8> = orig.chunks_exact(4).flat_map(|fr| fr[0..2].to_vec()).collect();
    assert!(extract == left, "bit-exact left channel of the embedded essence");

    // MX: the linked music found by name next to the AAF; clip gain, volume curve and pan
    let mx = track("MX");
    let m = clips(&mx.id);
    assert_eq!(m.len(), 1);
    assert_eq!((m[0].timeline_start, m[0].length, m[0].source_offset), (0, 75 * f, 25 * f));
    assert!((m[0].gain_db + 6.0206).abs() < 0.01, "{}", m[0].gain_db);
    assert_eq!(std::fs::canonicalize(&st.assets[&m[0].asset_id].path).unwrap(), std::fs::canonicalize(dir.join("music.wav")).unwrap());
    assert!((mx.pan - 0.5).abs() < 0.001, "pan 0.75 → +0.5: {}", mx.pan);
    let lane: Vec<(i64, f32)> = {
        let mut v: Vec<(i64, f32)> = st
            .automation
            .values()
            .filter(|p| p.owner_id == mx.id && p.param == AutomationParam::VolumeDb)
            .map(|p| (p.sample, p.value))
            .collect();
        v.sort_by_key(|p| p.0);
        v
    };
    assert!(lane.len() >= 3, "{lane:?}");
    let at = |s: i64| {
        let i = lane.partition_point(|p| p.0 <= s);
        let (p, q) = (lane[i.saturating_sub(1)], lane[i.min(lane.len() - 1)]);
        if q.0 == p.0 {
            p.1 as f64
        } else {
            p.1 as f64 + (q.1 - p.1) as f64 * (s - p.0) as f64 / (q.0 - p.0) as f64
        }
    };
    // AAF: amplitude 1 → 0.25 → 1, linear, over the track's 100 frames
    // (the clip plays the first 75 frames)
    for k in 0..=15 {
        let x = (k * 100 * f / 20).min(75 * f - 1);
        let u = x as f64 / (100 * f) as f64;
        let amp = if u <= 0.5 { 1.0 - 1.5 * u } else { 0.25 + 1.5 * (u - 0.5) };
        let want = 20.0 * amp.log10();
        assert!((at(x) - want).abs() < 0.11, "volume at {u:.2}: {} vs {want}", at(x));
    }

    // FX 3: channel 3 of the two-track MXF (its second track's right channel), bit-exact
    let fx = track("FX 3");
    let x = clips(&fx.id);
    assert_eq!((x.len(), x[0].timeline_start, x[0].length), (1, 0, 50 * f));
    let xa = &st.assets[&x[0].asset_id];
    assert_eq!(xa.channels, 1);
    let (_, got) = wav::read_raw(Path::new(&xa.path)).unwrap();
    let (_, want) = wav::read_raw(&dir.join("fx_ch3.wav")).unwrap();
    assert!(got == want, "channel 3 of the MXF");

    // the marker, in its own lane
    let mk = st.markers.values().next().unwrap();
    assert_eq!((mk.start_sample, mk.label.as_str(), mk.origin), (50 * f, "door knock", TimingOrigin::Imported));
    assert_eq!(st.lanes[&mk.lane_id].name, "AAF markers");

    // the music moved away: its clip is reported, the dialog still comes in
    std::fs::rename(dir.join("music.wav"), dir.join("music.moved")).unwrap();
    let (mut s2, root2) = session("aafimp2", "25", "10:00:00:00");
    let rep = s2.edit("i", |st| import(st, &aaf, &opts(&root2))).unwrap();
    assert_eq!(rep.clips, 3);
    assert_eq!(rep.missing_media.len(), 1, "{:?}", rep.missing_media);
    assert!(rep.missing_media[0].contains("music.wav not found"), "{:?}", rep.missing_media);
    assert!(rep.notes.iter().any(|n| n.starts_with("markers outside every scene")), "{:?}", rep.notes);
    // …and a folder to search finds it
    std::fs::create_dir_all(dir.join("elsewhere/deep")).unwrap();
    std::fs::rename(dir.join("music.moved"), dir.join("elsewhere/deep/music.wav")).unwrap();
    let (mut s3, root3) = session("aafimp3", "25", "10:00:00:00");
    let o3 = AafImportOptions { search_dirs: vec![dir.join("elsewhere")], at: Some(0), ..opts(&root3) };
    let rep = s3.edit("i", |st| import(st, &aaf, &o3)).unwrap();
    assert!(rep.missing_media.is_empty(), "{:?}", rep.missing_media);
}

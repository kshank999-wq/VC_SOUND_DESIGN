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
    // as AAF carries it (other apps' view): buses folded into the tracks
    let fold = AafImportOptions { buses: vcsd_core::otio_import::BusLevels::Fold, ..opts(&root2) };
    let rep = s2.edit("i", |st| import(st, &aaf, &fold)).unwrap();
    assert_eq!(rep.bus_levels, "fold");
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
    assert_eq!(rep.tracks.len(), 7, "seven sound tracks (the picture track is ignored): {rep:?}");
    assert_eq!(rep.clips, 8);
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

    // PAN: a linear sweep −1 → +1 at frame 25 → 0 at the end; STEP: −0.5 held, then +0.5
    let pan_lane = |name: &str| {
        let t = track(name);
        let mut v: Vec<(i64, f32)> =
            st.automation.values().filter(|p| p.owner_id == t.id && p.param == AutomationParam::Pan).map(|p| (p.sample, p.value)).collect();
        v.sort_by_key(|p| p.0);
        v
    };
    let pan_at = |lane: &[(i64, f32)], x: i64| {
        let i = lane.partition_point(|p| p.0 <= x);
        let (p, q) = (lane[i.saturating_sub(1)], lane[i.min(lane.len() - 1)]);
        if q.0 > p.0 && x >= p.0 {
            p.1 as f64 + (q.1 - p.1) as f64 * (x - p.0) as f64 / (q.0 - p.0) as f64
        } else {
            p.1 as f64
        }
    };
    let sweep = pan_lane("PAN");
    assert!(sweep.len() <= 4, "breakpoints, not a fit: {sweep:?}");
    for k in 0..50 * f {
        let u = k as f64 / (50 * f) as f64;
        let want = if u <= 0.5 { -1.0 + 4.0 * u } else { 1.0 - 2.0 * (u - 0.5) };
        assert!((pan_at(&sweep, k) - want).abs() < 0.003, "pan at {k}: {} vs {want}", pan_at(&sweep, k));
    }
    let step = pan_lane("STEP");
    assert!((pan_at(&step, 12 * f) + 0.5).abs() < 1e-6 && (pan_at(&step, 25 * f - 1) + 0.5).abs() < 1e-6, "held: {step:?}");
    assert!((pan_at(&step, 25 * f) - 0.5).abs() < 1e-6 && (pan_at(&step, 40 * f) - 0.5).abs() < 1e-6, "then the jump: {step:?}");
    assert!(!rep.notes.iter().any(|n| n.contains("interpolation")), "{:?}", rep.notes);

    // CLIPGAIN: the clip's gain envelope as the track's volume lane, where the clip plays
    let vol_lane = |name: &str| {
        let t = track(name);
        let mut v: Vec<(i64, f32)> = st
            .automation
            .values()
            .filter(|p| p.owner_id == t.id && p.param == AutomationParam::VolumeDb)
            .map(|p| (p.sample, p.value))
            .collect();
        v.sort_by_key(|p| p.0);
        v
    };
    let env = vol_lane("CLIPGAIN");
    assert_eq!(clips(&track("CLIPGAIN").id)[0].timeline_start, 10 * f);
    for k in 0..40 * f {
        let u = k as f64 / (40 * f) as f64;
        let amp = if u <= 0.5 { 1.0 - 1.5 * u } else { 0.25 + 1.5 * (u - 0.5) };
        let want = 20.0 * amp.log10();
        let got = pan_at(&env, 10 * f + k);
        assert!((got - want).abs() < 0.11, "clip envelope at {k}: {got:.3} vs {want:.3} dB");
    }
    // LEVEL: Stereo Audio Gain's Level, stepped: −6 dB held, then +18 dB clamped to +12
    let lv = vol_lane("LEVEL");
    assert!((pan_at(&lv, 10 * f) + 6.0206).abs() < 0.01 && (pan_at(&lv, 25 * f - 1) + 6.0206).abs() < 0.01, "{lv:?}");
    assert!((pan_at(&lv, 25 * f) - 12.0).abs() < 1e-6 && (pan_at(&lv, 45 * f) - 12.0).abs() < 1e-6, "{lv:?}");
    assert!(rep.notes.iter().any(|n| n.starts_with("gain above +12 dB clamped")), "{:?}", rep.notes);
    assert!(
        !rep.notes.iter().any(|n| n.starts_with("audio effects other than gain and pan")),
        "Stereo Audio Gain is read: {:?}",
        rep.notes
    );

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

#[test]
fn buses_and_their_automation_come_back_from_our_aaf() {
    let (mut s, root) = session("aafbus", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
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
    let clip_of = |st: &ProjectState, what: &str| {
        let cue = st.cues.values().find(|c| c.description.contains(what)).unwrap().id.clone();
        st.clips.values().find(|c| c.cue_id.as_deref() == Some(cue.as_str()) && !c.muted).unwrap().id.clone()
    };
    let (glass, wind) = (clip_of(st, "glass"), clip_of(st, "wind"));
    st.clips.get_mut(&glass).unwrap().gain_db = -6.0;
    let (gt, wt) = (st.clips[&glass].track_id.clone(), st.clips[&wind].track_id.clone());
    let t0 = st.clips[&glass].timeline_start;
    st.tracks.get_mut(&gt).unwrap().gain_db = -3.0;
    editing::set_automation(st, &gt, AutomationParam::Pan, t0, -0.5).unwrap();
    editing::set_automation(st, &gt, AutomationParam::Pan, t0 + sr, 0.5).unwrap();
    // the glass on a new sub-bus (+1.5 dB, a built-in insert) under its bus (−2 dB, a fade);
    // the master's balance moving; the wind's bus dipping
    let parent = st.tracks[&gt].bus_id.clone();
    let parent_name = st.buses[&parent].name.clone();
    let sub = mixer::add_bus(st, "FX Sub", Some(&parent)).unwrap();
    st.tracks.get_mut(&gt).unwrap().bus_id = sub.clone();
    st.buses.get_mut(&sub).unwrap().gain_db = 1.5;
    st.buses.get_mut(&sub).unwrap().inserts = vec![Insert::Gain { db: -1.0 }];
    st.buses.get_mut(&parent).unwrap().gain_db = -2.0;
    editing::set_automation(st, &parent, AutomationParam::VolumeDb, t0, 0.0).unwrap();
    editing::set_automation(st, &parent, AutomationParam::VolumeDb, t0 + sr / 2, -8.0).unwrap();
    let wbus = st.tracks[&wt].bus_id.clone();
    editing::set_automation(st, &wbus, AutomationParam::VolumeDb, sr, 0.0).unwrap();
    editing::set_automation(st, &wbus, AutomationParam::VolumeDb, 3 * sr, -6.0).unwrap();
    let master = st.buses.values().find(|b| b.parent_id.is_none()).unwrap().id.clone();
    editing::set_automation(st, &master, AutomationParam::Pan, 0, 0.3).unwrap();
    editing::set_automation(st, &master, AutomationParam::Pan, 5 * sr, -0.3).unwrap();

    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.aaf = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let aaf = r.dir.join(r.files.iter().find(|f| f.ends_with(".aaf")).unwrap());

    // an independent reader sees our description as an ordinary mob comment
    if let Some(py) = python() {
        let code = "import sys, json, aaf2\n\
                    f = aaf2.open(sys.argv[1], 'r')\n\
                    m = next(f.content.compositionmobs())\n\
                    print(json.dumps(json.loads(dict(m.comments.items())['VCSD'])['buses']))";
        let out = std::process::Command::new(&py).args(["-c", code]).arg(&aaf).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let buses: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(buses.as_array().unwrap().iter().any(|b| b["name"] == "FX Sub" && b["gain_db"] == 1.5), "{buses}");
    }

    let (mut s2, root2) = session("aafbus2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &aaf, &opts(&root2))).unwrap();
    assert_eq!(rep.bus_levels, "buses", "{rep:?}");
    assert_eq!(rep.buses_created, 1, "FX Sub");
    assert!(rep.buses_restored >= 4, "FX Sub, its parent, the wind's bus and the master: {rep:?}");
    let (a, b) = (&s.state, &s2.state);
    let named = |st: &ProjectState, n: &str| st.buses.values().find(|x| x.name == n).unwrap_or_else(|| panic!("bus {n}")).clone();
    let (sub2, parent2) = (named(b, "FX Sub"), named(b, &parent_name));
    assert_eq!(sub2.parent_id.as_deref(), Some(parent2.id.as_str()));
    assert_eq!((sub2.gain_db, parent2.gain_db), (1.5, -2.0));
    assert!(matches!(sub2.inserts.as_slice(), [Insert::Gain { db }] if *db == -1.0));
    let lane = |st: &ProjectState, owner: &str, p: AutomationParam| {
        let mut v: Vec<(i64, f32)> =
            st.automation.values().filter(|x| x.owner_id == owner && x.param == p).map(|x| (x.sample, x.value)).collect();
        v.sort_by_key(|x| x.0);
        v
    };
    for name in [parent_name.as_str(), "Review Mix", a.buses[&wbus].name.as_str()] {
        let (x, y) = (named(a, name), named(b, name));
        for p in [AutomationParam::VolumeDb, AutomationParam::Pan] {
            assert_eq!(lane(a, &x.id, p), lane(b, &y.id, p), "{name} {p:?}");
        }
    }
    // the glass: routed to FX Sub with its own fader, pan lane and clip gain (not folded)
    let gc = b.clips.values().find(|c| c.timeline_start == t0).unwrap();
    let gtrack = &b.tracks[&gc.track_id];
    assert_eq!(gtrack.bus_id, sub2.id);
    assert_eq!(gtrack.gain_db, -3.0);
    assert!((gc.gain_db + 6.0).abs() < 1e-3, "{}", gc.gain_db);
    assert_eq!(lane(b, &gtrack.id, AutomationParam::Pan), lane(a, &gt, AutomationParam::Pan));
    let wtrack = b.tracks.values().find(|t| t.channels == 2).expect("the stereo bed, paired");
    assert_eq!(wtrack.bus_id, named(b, &a.buses[&wbus].name).id);

    // the same mix
    let (start, end) = mixer::timeline_extent(a);
    let n = (end - start) as usize;
    let ma = mixer::Mixer::new(a, mixer::RenderOptions::default()).render_master(start, n).unwrap();
    let mb = mixer::Mixer::new(b, mixer::RenderOptions::default()).render_master(start, n).unwrap();
    assert!(ma.peak() > 0.05);
    let max = ma.data.iter().zip(&mb.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-4 * ma.peak().max(1.0), "mix differs by {max}");

    // folding instead: the project's buses untouched, the tracks carrying the levels
    let (mut s3, root3) = session("aafbus3", "24", "01:00:00:00");
    let fold = AafImportOptions { buses: vcsd_core::otio_import::BusLevels::Fold, ..opts(&root3) };
    let rep = s3.edit("i", |st| import(st, &aaf, &fold)).unwrap();
    assert_eq!((rep.bus_levels.as_str(), rep.buses_restored), ("fold", 0));
    assert!(!s3.state.buses.values().any(|b| b.name == "FX Sub"));
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

/// A glass hit through a high-pass and an automated test plugin, exported as AAF.
fn plugin_aaf(name: &str, print: bool) -> (state::Session, std::path::PathBuf, i64) {
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
    let (t0, tid) = (c.timeline_start, c.track_id.clone());
    st.tracks.get_mut(&tid).unwrap().inserts = vec![
        Insert::HighPass { hz: 150.0, q: 0.7 },
        Insert::Plugin(PluginInsert {
            id: "ins_gain".into(),
            plugin_id: "vst3:test".into(),
            format: "vst3".into(),
            name: "Gain".into(),
            vendor: "Test".into(),
            path: String::new(),
            bypass: false,
            state: Some("c3RhdGU=".into()),
            params: [(0u32, 0.3f64)].into_iter().collect(),
        }),
    ];
    for (dx, v) in [(0, 0.1f32), (sr / 4, 0.9), (sr / 2, 0.4)] {
        editing::set_automation(st, "ins_gain", AutomationParam::Plugin(0), t0 + dx, v).unwrap();
    }
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.aaf = true;
    o.aaf_print_plugins = print;
    o.force = true;
    o.plugins = Some(mixer::PluginHostRef(std::sync::Arc::new(GainPlugin)));
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let aaf = r.dir.join(r.files.iter().find(|f| f.ends_with(".aaf")).unwrap());
    (s, aaf, t0)
}

#[test]
fn track_plugins_and_their_automation_come_back_from_our_aaf() {
    let (s, aaf, t0) = plugin_aaf("aafplug", false);
    let sr = s.state.project.sample_rate as i64;
    for mode in [vcsd_core::otio_import::BusLevels::Auto, vcsd_core::otio_import::BusLevels::Fold] {
        let (mut s2, root2) = session(&format!("aafplug2{mode:?}"), "24", "01:00:00:00");
        let o = AafImportOptions { buses: mode, ..opts(&root2) };
        let rep = s2.edit("i", |st| import(st, &aaf, &o)).unwrap();
        assert_eq!((rep.inserts, rep.plugins, rep.automated_parameters), (2, 1, 1), "{mode:?}: {rep:?}");
        let st2 = &s2.state;
        let c2 = st2.clips.values().find(|c| c.timeline_start == t0).unwrap();
        let t2 = &st2.tracks[&c2.track_id];
        assert!(matches!(t2.inserts[0], Insert::HighPass { hz, .. } if hz == 150.0), "{:?}", t2.inserts);
        let Insert::Plugin(p) = &t2.inserts[1] else { panic!("{:?}", t2.inserts) };
        assert_ne!(p.id, "ins_gain");
        assert_eq!((p.plugin_id.as_str(), p.state.as_deref(), p.params.get(&0)), ("vst3:test", Some("c3RhdGU="), Some(&0.3)));
        let mut pts: Vec<(i64, f32)> = st2.automation.values().filter(|a| a.owner_id == p.id).map(|a| (a.sample, a.value)).collect();
        pts.sort_by_key(|x| x.0);
        assert_eq!(pts, vec![(t0, 0.1), (t0 + sr / 4, 0.9), (t0 + sr / 2, 0.4)], "{mode:?}");

        // through the high-pass and the plugin, the same sound
        let n = s.state.clips.values().find(|c| c.timeline_start == t0).unwrap().length as usize;
        let host: std::sync::Arc<dyn mixer::PluginHost> = std::sync::Arc::new(GainPlugin);
        let a = mixer::Mixer::new(&s.state, mixer::RenderOptions::default()).with_plugins(Some(host.clone())).render_master(t0, n).unwrap();
        let b = mixer::Mixer::new(st2, mixer::RenderOptions::default()).with_plugins(Some(host)).render_master(t0, n).unwrap();
        assert!(a.peak() > 0.01);
        let max = a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(max < 1e-4 * a.peak().max(1.0), "{mode:?}: mix differs by {max}");
    }
}

#[test]
fn a_printed_track_comes_back_as_its_render_without_its_plugins_again() {
    let (_, aaf, t0) = plugin_aaf("aafplugprint", true);
    let (mut s2, root2) = session("aafplugprint2", "24", "01:00:00:00");
    let rep = s2.edit("i", |st| import(st, &aaf, &opts(&root2))).unwrap();
    assert_eq!(rep.inserts, 0);
    assert!(rep.notes.iter().any(|n| n.starts_with("inserts of printed tracks not rebuilt") && n.ends_with("(2)")), "{:?}", rep.notes);
    let st2 = &s2.state;
    assert!(st2.tracks.values().all(|t| t.inserts.is_empty()));
    assert!(st2.automation.values().all(|a| !matches!(a.param, AutomationParam::Plugin(_))));
    let _ = t0;
}

#[test]
fn mono_pan_automation_comes_back_from_our_aaf() {
    let (mut s, root) = session("aafpan", "24", "01:00:00:00");
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
    let (t0, len, tid) = (c.timeline_start, c.length, c.track_id.clone());
    assert_eq!(st.tracks[&tid].channels, 1);
    // hard left → right of centre → hard right, with a hold in the middle
    for (dx, p) in [(0, -1.0f32), (sr / 4, 0.3), (sr / 2, 0.3), (sr, 1.0)] {
        editing::set_automation(st, &tid, AutomationParam::Pan, t0 + dx, p).unwrap();
    }
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-22");
    o.aaf = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let aaf = r.dir.join(r.files.iter().find(|f| f.ends_with(".aaf")).unwrap());
    let lane = |st: &ProjectState, t: &str| {
        let mut v: Vec<(i64, f32)> =
            st.automation.values().filter(|p| p.owner_id == t && p.param == AutomationParam::Pan).map(|p| (p.sample, p.value)).collect();
        v.sort_by_key(|p| p.0);
        v
    };
    let at = |l: &[(i64, f32)], x: i64| {
        let i = l.partition_point(|p| p.0 <= x);
        let (p, q) = (l[i.saturating_sub(1)], l[i.min(l.len() - 1)]);
        if q.0 > p.0 && x >= p.0 {
            p.1 as f64 + (q.1 - p.1) as f64 * (x - p.0) as f64 / (q.0 - p.0) as f64
        } else {
            p.1 as f64
        }
    };
    let want = lane(&s.state, &tid);
    for (mode, name) in [(vcsd_core::otio_import::BusLevels::Fold, "aafpanf"), (vcsd_core::otio_import::BusLevels::Auto, "aafpana")] {
        let (mut s2, root2) = session(name, "24", "01:00:00:00");
        let o2 = AafImportOptions { buses: mode, ..opts(&root2) };
        let rep = s2.edit("i", |st| import(st, &aaf, &o2)).unwrap();
        assert_eq!(rep.panned_tracks, 1, "{mode:?}");
        let c2 = s2.state.clips.values().find(|c| c.timeline_start == t0).unwrap();
        let got = lane(&s2.state, &c2.track_id);
        if mode == vcsd_core::otio_import::BusLevels::Auto {
            assert_eq!(got, want, "restored: the lane itself");
        } else {
            // from the AAF's Mono Audio Pan: the lane's breakpoints are its keyframes
            assert!(got.len() <= want.len() + 2, "{got:?}");
            for k in 0..=200 {
                let x = t0 + len * k / 200;
                assert!((at(&got, x) - at(&want, x)).abs() < 0.003, "pan at {x}: {} vs {}", at(&got, x), at(&want, x));
            }
        }
    }
}

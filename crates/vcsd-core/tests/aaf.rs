//! AAF export, read back with pyaaf2 (an independent AAF implementation) via
//! `scripts/aaf/inspect.py`. Skipped (loudly) when pyaaf2 isn't installed:
//! `python3 -m pip install pyaaf2==1.7.1`, or set VCSD_AAF_PYTHON.

mod common;
use common::*;
use std::path::Path;
use vcsd_core::aaf::{self, AafOptions};
use vcsd_core::builder::{populate, PopulateOptions};
use vcsd_core::spotting::*;
use vcsd_core::*;

fn python() -> Option<String> {
    let py = std::env::var("VCSD_AAF_PYTHON").unwrap_or_else(|_| "python3".into());
    let ok = std::process::Command::new(&py).args(["-c", "import aaf2"]).output().map(|o| o.status.success()).unwrap_or(false);
    ok.then_some(py)
}

fn inspect(py: &str, file: &Path) -> serde_json::Value {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/aaf/inspect.py");
    let out = std::process::Command::new(py).arg(script).arg(file).output().unwrap();
    assert!(out.status.success(), "pyaaf2 could not read the file:\n{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn aaf_rebuilds_the_timeline_in_an_independent_reader() {
    let Some(py) = python() else {
        eprintln!("SKIPPED: pyaaf2 not installed (python3 -m pip install pyaaf2==1.7.1)");
        return;
    };
    let (mut s, root) = session("aaf", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:08:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:05:00", "01:00:05:00", "door slam", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();

    let st = &mut s.state;
    let glass_cue = st.cues.values().find(|c| c.description.contains("glass")).unwrap().id.clone();
    let glass = st.clips.values_mut().find(|c| c.cue_id.as_deref() == Some(glass_cue.as_str()) && !c.muted).unwrap();
    glass.gain_db = -6.0;
    glass.fade_in = 480;
    glass.fade_out = 960;
    let glass_id = glass.id.clone();
    let glass_track = glass.track_id.clone();
    // an overlapping copy on the same track: must land on a second layer
    let mut dup = glass.clone();
    dup.id = "clip_dup".into();
    dup.timeline_start += 100;
    st.clips.insert(dup.id.clone(), dup);
    // the door's track muted: left out
    let door_cue = st.cues.values().find(|c| c.description.contains("door")).unwrap().id.clone();
    let door_track = st.clips.values().find(|c| c.cue_id.as_deref() == Some(door_cue.as_str())).unwrap().track_id.clone();
    st.tracks.get_mut(&door_track).unwrap().mute = true;
    st.tracks.get_mut(&glass_track).unwrap().gain_db = -6.0; // folded into clip gain
                                                             // volume automation on the glass track: a dip from 0 to −20 dB and back
    let sr0 = st.project.sample_rate as i64;
    for (t, v) in [(2 * sr0, 0.0f32), (3 * sr0, -20.0), (4 * sr0, 0.0)] {
        editing::set_automation(st, &glass_track, AutomationParam::VolumeDb, t, v).unwrap();
    }
    // pan: a sweep on the (mono) glass track, a balance move across the centre on the stereo wind
    for (t, v) in [(2 * sr0, -1.0f32), (6 * sr0, 1.0)] {
        editing::set_automation(st, &glass_track, AutomationParam::Pan, t, v).unwrap();
    }
    let wind_cue = st.cues.values().find(|c| c.description.contains("wind")).unwrap().id.clone();
    let wind_track = st.clips.values().find(|c| c.cue_id.as_deref() == Some(wind_cue.as_str())).unwrap().track_id.clone();
    assert_eq!(st.tracks[&wind_track].channels, 2, "the ambience is a stereo track");
    for (t, v) in [(sr0, -0.5f32), (5 * sr0, 0.5)] {
        editing::set_automation(st, &wind_track, AutomationParam::Pan, t, v).unwrap();
    }
    assert_eq!(st.tracks[&glass_track].channels, 1);

    let sr = st.project.sample_rate as i64;
    let start = sr; // from 01:00:01:00: clips before it are trimmed
    let end = 9 * sr;
    let used: Vec<String> = st.clips.values().map(|c| c.asset_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let (media, _notes) = aaf::prepare_media(st, &used, &root.join("Exports/aaf/Media")).unwrap();
    let file = root.join("Exports/aaf/test.aaf");
    let rep = aaf::write(
        st,
        &file,
        &media,
        &AafOptions {
            name: "Gravel Road — scene 1".into(),
            start,
            end,
            exclude_temp_refs: false,
            prints: Default::default(),
            include_muted: false,
        },
    )
    .unwrap();
    assert!(rep.losses.iter().any(|l| l.starts_with("muted tracks left out (1)")), "{:?}", rep.losses);
    assert!(rep.losses.iter().any(|l| l.starts_with("overlapping clips split")), "{:?}", rep.losses);
    assert!(rep.losses.iter().any(|l| l.starts_with("track fader gain folded")), "{:?}", rep.losses);
    assert!(!rep.losses.iter().any(|l| l.starts_with("track pan")), "pan is carried: {:?}", rep.losses);
    assert!(rep.losses.iter().any(|l| l.contains("pan law") && l.ends_with("(1)")), "{:?}", rep.losses);
    assert_eq!(rep.panned_tracks, 2);
    assert!(
        !rep.losses.iter().any(|l| l.contains("volume") && l.contains("track ")),
        "track volume automation is carried: {:?}",
        rep.losses
    );
    assert_eq!(rep.automated_tracks, 1);

    let v = inspect(&py, &file);
    assert_eq!(v["product"], "VC Sound Design");
    assert_eq!(v["composition"], "Gravel Road — scene 1");
    // timecode: 01:00:01:00 at 24 fps
    assert_eq!(v["timecode"]["start"], 86_400 + 24);
    assert_eq!(v["timecode"]["fps"], 24);
    assert_eq!(v["timecode"]["drop"], false);
    assert_eq!(v["timecode"]["edit_rate"], serde_json::json!([24, 1]));

    let tracks = v["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), rep.tracks);
    for t in tracks {
        assert_eq!(t["media_kind"], "Sound");
        assert_eq!(t["edit_rate"], serde_json::json!([48000, 1]));
        assert_eq!(t["length"], end - start, "every track spans the range");
    }
    // the glass clip: placed relative to the range, with fades and folded gain
    let gc = &s.state.clips[&glass_id];
    let gtrack = &s.state.tracks[&glass_track];
    let clip = tracks
        .iter()
        .filter(|t| t["name"].as_str().unwrap().starts_with(&gtrack.name))
        .flat_map(|t| t["components"].as_array().unwrap())
        .find(|c| c["kind"] == "SourceClip" && c["fade_in"] == 480)
        .expect("glass clip");
    assert_eq!(clip["length"], gc.length);
    assert_eq!(clip["start"], gc.source_offset);
    assert_eq!(clip["fade_out"], 960);
    let g = clip["gain"].as_f64().unwrap();
    assert!((g - 10f64.powf(-12.0 / 20.0)).abs() < 1e-5, "clip −6 dB + fader −6 dB: {g}");
    assert_eq!(clip["wrapped_in"], "Mono Audio Gain");
    // its media is linked through master → file → import mobs to a real WAV
    let m = &clip["media"];
    assert_eq!(m["descriptor"], "WAVEDescriptor");
    assert_eq!(m["tape_descriptor"], "ImportDescriptor");
    assert_eq!(m["summary_riff"], "RIFF");
    let url = m["urls"][0].as_str().unwrap();
    let mut path = url.strip_prefix("file://").unwrap().replace("%20", " ");
    if cfg!(windows) {
        path = path.trim_start_matches('/').to_string(); // file:///C:/…
    }
    assert!(Path::new(&path).exists(), "linked media exists: {url}");
    // the volume lane: on every layer/channel of that track, linear control points over the
    // range that reproduce the mixer's dB ramp within 0.1 dB; other tracks have none
    let lane: Vec<(i64, f32)> = {
        let mut v: Vec<(i64, f32)> = s
            .state
            .automation
            .values()
            .filter(|p| p.owner_id == glass_track && p.param == AutomationParam::VolumeDb)
            .map(|p| (p.sample, p.value))
            .collect();
        v.sort_by_key(|p| p.0);
        v
    };
    let db_at = |t: i64| {
        let i = lane.partition_point(|p| p.0 <= t);
        let v = match (i.checked_sub(1).map(|k| lane[k]), lane.get(i)) {
            (Some(a), Some(b)) => a.1 + (b.1 - a.1) * ((t - a.0) as f32 / (b.0 - a.0) as f32),
            (Some(a), None) => a.1,
            (None, Some(b)) => b.1,
            _ => 0.0,
        };
        v as f64
    };
    for t in tracks {
        let mine = t["name"].as_str().unwrap().starts_with(&gtrack.name);
        let wind = t["name"].as_str().unwrap().starts_with(&s.state.tracks[&wind_track].name);
        if !mine {
            assert!(wind || t["volume"].is_null(), "only automated tracks get a track gain: {}", t["name"]);
            continue;
        }
        assert_eq!(t["volume"]["operation"], "Mono Audio Gain");
        assert_eq!(t["volume"]["interpolation"], "LinearInterp");
        let pts: Vec<(f64, f64)> =
            t["volume"]["points"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
        assert_eq!(pts.first().unwrap().0, 0.0);
        let span = (end - start) as f64;
        for k in 0..400 {
            let rel = k as f64 / 400.0 * (span - 2.0) / span;
            let i = pts.partition_point(|p| p.0 <= rel) - 1;
            let (a, b) = (pts[i], pts[(i + 1).min(pts.len() - 1)]);
            let g = if b.0 > a.0 { a.1 + (b.1 - a.1) * (rel - a.0) / (b.0 - a.0) } else { a.1 };
            let want = db_at(start + (rel * span).round() as i64);
            assert!((20.0 * g.log10() - want).abs() < 0.12, "at {rel:.3}: {:.2} dB vs {want:.2} dB", 20.0 * g.log10());
        }
    }

    // pan on the mono glass track: Mono Audio Pan following (pan + 1) / 2 exactly
    let pan_lane: Vec<(i64, f32)> = {
        let mut v: Vec<(i64, f32)> = s
            .state
            .automation
            .values()
            .filter(|p| p.owner_id == glass_track && p.param == AutomationParam::Pan)
            .map(|p| (p.sample, p.value))
            .collect();
        v.sort_by_key(|p| p.0);
        v
    };
    let lerp = |pts: &[(f64, f64)], x: f64| {
        let i = pts.partition_point(|p| p.0 <= x).max(1) - 1;
        let (a, b) = (pts[i], pts[(i + 1).min(pts.len() - 1)]);
        if b.0 > a.0 {
            a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0)
        } else {
            a.1
        }
    };
    let lane_at = |lane: &[(i64, f32)], t: i64| {
        let i = lane.partition_point(|p| p.0 <= t);
        (match (i.checked_sub(1).map(|k| lane[k]), lane.get(i)) {
            (Some(a), Some(b)) => a.1 + (b.1 - a.1) * ((t - a.0) as f32 / (b.0 - a.0) as f32),
            (Some(a), None) => a.1,
            (None, Some(b)) => b.1,
            _ => 0.0,
        }) as f64
    };
    let span = (end - start) as f64;
    for t in tracks.iter().filter(|t| t["name"].as_str().unwrap().starts_with(&gtrack.name)) {
        assert_eq!(t["pan"]["operation"], "Mono Audio Pan");
        let pts: Vec<(f64, f64)> =
            t["pan"]["points"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
        for k in 0..200 {
            let rel = k as f64 / 200.0 * (span - 2.0) / span;
            let want = (lane_at(&pan_lane, start + (rel * span).round() as i64) + 1.0) / 2.0;
            assert!((lerp(&pts, rel) - want).abs() < 1e-4, "pan at {rel:.3}: {} vs {want}", lerp(&pts, rel));
        }
    }
    // the stereo wind: hard-panned L/R, the balance in each side's gain (within 0.12 dB)
    let wname = s.state.tracks[&wind_track].name.clone();
    let wlane: Vec<(i64, f32)> = {
        let mut v: Vec<(i64, f32)> = s
            .state
            .automation
            .values()
            .filter(|p| p.owner_id == wind_track && p.param == AutomationParam::Pan)
            .map(|p| (p.sample, p.value))
            .collect();
        v.sort_by_key(|p| p.0);
        v
    };
    for (suffix, left) in [(" L", true), (" R", false)] {
        let t = tracks.iter().find(|t| t["name"] == format!("{wname}{suffix}").as_str()).expect("wind side");
        assert_eq!(t["pan"]["constant"], if left { 0.0 } else { 1.0 }, "hard {suffix}");
        let pts: Vec<(f64, f64)> =
            t["volume"]["points"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
        for k in 0..400 {
            let rel = k as f64 / 400.0 * (span - 2.0) / span;
            let p = lane_at(&wlane, start + (rel * span).round() as i64);
            let want = if left {
                if p <= 0.0 {
                    1.0
                } else {
                    1.0 - p
                }
            } else if p <= 0.0 {
                1.0 + p
            } else {
                1.0
            };
            assert!((20.0 * (lerp(&pts, rel) / want).log10()).abs() < 0.12, "{suffix} at {rel:.3}: {} vs {want}", lerp(&pts, rel));
        }
    }

    // the overlap went to a second layer of the same track
    assert!(tracks.iter().any(|t| t["name"].as_str().unwrap().starts_with(&gtrack.name) && t["name"].as_str().unwrap().ends_with("(2)")));
    // the muted door track isn't there
    let door_name = &s.state.tracks[&door_track].name;
    assert!(!tracks.iter().any(|t| t["name"].as_str().unwrap().starts_with(door_name.as_str())));
    // the wind ambience (starting before the range) is trimmed at the range start
    let wind_name = &s
        .state
        .tracks
        .values()
        .find(|t| {
            s.state.clips.values().any(|c| {
                c.track_id == t.id && s.state.cues.get(c.cue_id.as_deref().unwrap_or("")).is_some_and(|q| q.description.contains("wind"))
            })
        })
        .unwrap()
        .name;
    let wind = tracks.iter().find(|t| t["name"].as_str().unwrap().starts_with(wind_name.as_str())).unwrap();
    assert_eq!(wind["components"][0]["kind"], "SourceClip", "no leading filler: the clip starts before the range");

    // markers: the cues inside the range, at their positions relative to it
    let mk = v["markers"].as_array().unwrap();
    let mut want: Vec<(i64, String)> = s
        .state
        .cues
        .values()
        .filter(|c| c.start_sample >= start && c.start_sample < end)
        .map(|c| (c.start_sample - start, c.code.clone()))
        .collect();
    want.sort();
    let mut got: Vec<(i64, String)> = mk
        .iter()
        .map(|m| (m["position"].as_i64().unwrap(), m["comment"].as_str().unwrap().split(' ').next().unwrap().to_string()))
        .collect();
    got.sort();
    assert_eq!(got, want);
    assert!(mk.iter().all(|m| m["kind"] == "DescriptiveMarker"), "Media Composer's kind of marker");
}

#[test]
fn export_package_includes_the_aaf_its_media_and_what_it_loses() {
    let (mut s, root) = session("aafx", "25", "10:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "10:00:00:00", "10:00:06:00");
    marker_tc(&mut s, "fx", EventType::Point, "10:00:01:00", "10:00:01:00", "glass smash", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.aaf = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let aaf_file = r.files.iter().find(|f| f.starts_with("AAF/") && f.ends_with(".aaf")).expect("AAF in the package").clone();
    assert!(r.files.iter().any(|f| f.starts_with("AAF/Media/") && f.ends_with(".wav")), "linked media copied in");
    assert!(r.files.iter().any(|f| f == "AAF/aaf_report.json"));
    assert!(export::verify_checksums(&r.dir).unwrap().is_empty(), "AAF files are in the checksums");
    let readme = std::fs::read_to_string(r.dir.join("README.md")).unwrap();
    assert!(readme.contains(&format!("`{aaf_file}` holds the editable timeline")));
    assert!(readme.contains("bus routing not carried: tracks are flat, with bus levels and automation folded into each track"));
    assert!(r.qa.iter().any(|q| q.code == "aaf_loss"));
    // without the option, nothing AAF
    let o2 = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    let r2 = s.edit("x", |st| export::export(st, Some(&root), &export::ExportOptions { force: true, ..o2 })).unwrap();
    assert!(!r2.files.iter().any(|f| f.starts_with("AAF/")));
    if let Some(py) = python() {
        let v = inspect(&py, &r.dir.join(&aaf_file));
        assert_eq!(v["timecode"]["start"], 10 * 3600 * 25);
        assert_eq!(v["timecode"]["fps"], 25);
        assert_eq!(v["markers"].as_array().unwrap().len(), 1);
    }
}

/// What a listener hears from the AAF in a host with a −3 dB constant-power pan law,
/// vs what VC Sound Design's mixer plays, for tracks whose buses have levels,
/// volume automation and balance automation (AAF has no buses: they are folded in).
#[test]
fn bus_levels_and_automation_are_folded_into_each_track() {
    let Some(py) = python() else {
        eprintln!("SKIPPED: pyaaf2 not installed (python3 -m pip install pyaaf2==1.7.1)");
        return;
    };
    let (mut s, root) = session("aafbus", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:08:00", "wind exterior", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let track_of = |st: &ProjectState, what: &str| {
        let cue = st.cues.values().find(|c| c.description.contains(what)).unwrap().id.clone();
        st.clips.values().find(|c| c.cue_id.as_deref() == Some(cue.as_str())).unwrap().track_id.clone()
    };
    let (glass, wind) = (track_of(st, "glass"), track_of(st, "wind"));
    assert_eq!((st.tracks[&glass].channels, st.tracks[&wind].channels), (1, 2));
    let (gbus, wbus) = (st.tracks[&glass].bus_id.clone(), st.tracks[&wind].bus_id.clone());
    assert_ne!(gbus, wbus);
    let master = mixer::master_bus(st).unwrap().id.clone();
    // the glass: panned left → right; its bus −3 dB with a balance move across the centre
    for (t, v) in [(2 * sr, -0.5f32), (7 * sr, 0.5)] {
        editing::set_automation(st, &glass, AutomationParam::Pan, t, v).unwrap();
    }
    st.buses.get_mut(&gbus).unwrap().gain_db = -3.0;
    for (t, v) in [(sr, -0.4f32), (6 * sr, 0.6)] {
        editing::set_automation(st, &gbus, AutomationParam::Pan, t, v).unwrap();
    }
    // the wind's bus −6 dB; the master dips 12 dB in the middle
    st.buses.get_mut(&wbus).unwrap().gain_db = -6.0;
    for (t, v) in [(3 * sr, 0.0f32), (4 * sr, -12.0), (5 * sr, 0.0)] {
        editing::set_automation(st, &master, AutomationParam::VolumeDb, t, v).unwrap();
    }

    let (start, end) = (sr, 9 * sr);
    let used: Vec<String> = st.clips.values().map(|c| c.asset_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let (media, _) = aaf::prepare_media(st, &used, &root.join("Exports/aafbus/Media")).unwrap();
    let file = root.join("Exports/aafbus/bus.aaf");
    let rep = aaf::write(
        st,
        &file,
        &media,
        &AafOptions { name: "bus".into(), start, end, exclude_temp_refs: false, prints: Default::default(), include_muted: false },
    )
    .unwrap();
    assert!(!rep.losses.iter().any(|l| l.contains("automation not carried")), "{:?}", rep.losses);
    assert!(rep.losses.iter().any(|l| l.starts_with("bus routing not carried: tracks are flat, with bus levels")));
    assert!(rep.losses.iter().any(|l| l.starts_with("bus balance on mono tracks folded") && l.ends_with("(1)")), "{:?}", rep.losses);
    let v = inspect(&py, &file);
    let tracks = v["tracks"].as_array().unwrap();

    // the mixer's side of it
    let st = &s.state;
    let lane = |owner: &str, p: AutomationParam| {
        let mut l: Vec<(i64, f32)> =
            st.automation.values().filter(|x| x.owner_id == owner && x.param == p).map(|x| (x.sample, x.value)).collect();
        l.sort_by_key(|x| x.0);
        l
    };
    let at = |l: &[(i64, f32)], t: i64| {
        let i = l.partition_point(|p| p.0 <= t);
        (match (i.checked_sub(1).map(|k| l[k]), l.get(i)) {
            (Some(a), Some(b)) => a.1 + (b.1 - a.1) * ((t - a.0) as f32 / (b.0 - a.0) as f32),
            (Some(a), None) => a.1,
            (None, Some(b)) => b.1,
            _ => 0.0,
        }) as f64
    };
    let bal = |p: f64| if p <= 0.0 { (1.0, 1.0 + p) } else { (1.0 - p, 1.0) };
    let db = |x: f64| 10f64.powf(x / 20.0);
    let (gpan, gbal, mvol) =
        (lane(&glass, AutomationParam::Pan), lane(&gbus, AutomationParam::Pan), lane(&master, AutomationParam::VolumeDb));
    // the AAF side: a curve's value at a fraction of the range (control points, linear)
    let curve = |t: &serde_json::Value, key: &str, rel: f64, default: f64| -> f64 {
        let fx = &t[key];
        if fx.is_null() {
            return default;
        }
        if let Some(c) = fx["constant"].as_f64() {
            return c;
        }
        let pts: Vec<(f64, f64)> =
            fx["points"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
        let i = pts.partition_point(|p| p.0 <= rel).max(1) - 1;
        let (a, b) = (pts[i], pts[(i + 1).min(pts.len() - 1)]);
        if b.0 > a.0 {
            a.1 + (b.1 - a.1) * (rel - a.0) / (b.0 - a.0)
        } else {
            a.1
        }
    };
    let close = |got: f64, want: f64, what: &str| {
        if want > 0.002 {
            assert!((20.0 * (got / want).log10()).abs() < 0.15, "{what}: {got} vs {want}");
        } else {
            assert!((got - want).abs() < 0.002, "{what}: {got} vs {want}");
        }
    };
    let span = (end - start) as f64;
    let gname = st.tracks[&glass].name.clone();
    let wname = st.tracks[&wind].name.clone();
    for k in 0..400 {
        let rel = k as f64 / 400.0 * (span - 2.0) / span;
        let x = start + (rel * span).round() as i64;
        let master_lvl = db(at(&mvol, x));
        // mono glass: constant-power pan, then the bus's balance and levels
        let (gl, gr) = mixer::pan_gains(at(&gpan, x) as f32);
        let (bl, br) = bal(at(&gbal, x));
        let lvl = db(-3.0) * master_lvl;
        let (want_l, want_r) = (lvl * gl as f64 * bl, lvl * gr as f64 * br);
        let t = tracks.iter().find(|t| t["name"] == gname.as_str()).expect("glass track");
        let g = curve(t, "volume", rel, 1.0);
        let q = curve(t, "pan", rel, 0.5) * std::f64::consts::FRAC_PI_2;
        close(g * q.cos(), want_l, &format!("glass L at {rel:.3}"));
        close(g * q.sin(), want_r, &format!("glass R at {rel:.3}"));
        // stereo wind: each side's gain carries its bus and the master
        for (suffix, left) in [(" L", true), (" R", false)] {
            let t = tracks.iter().find(|t| t["name"] == format!("{wname}{suffix}").as_str()).expect("wind side");
            assert_eq!(t["pan"]["constant"], if left { 0.0 } else { 1.0 });
            close(curve(t, "volume", rel, 1.0), db(-6.0) * master_lvl, &format!("wind{suffix} at {rel:.3}"));
        }
    }
}

/// A stand-in plugin: gain = 2 × its parameter 0, sample by sample from the automation it's given.
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
fn printing_bakes_plugins_and_their_automation_into_the_aaf() {
    let (mut s, root) = session("aafprint", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:06:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:01:00", "01:00:01:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:05:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:04:00", "01:00:04:00", "door slam", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    let sr = st.project.sample_rate as i64;
    let glass = {
        let cue = st.cues.values().find(|c| c.description.contains("glass")).unwrap().id.clone();
        st.clips.values().find(|c| c.cue_id.as_deref() == Some(cue.as_str())).unwrap().track_id.clone()
    };
    // a plugin on the glass track whose gain is automated from ×0 up to ×2, and a −6 dB fader
    let t = st.tracks.get_mut(&glass).unwrap();
    t.gain_db = -6.0;
    t.inserts.push(Insert::Plugin(PluginInsert {
        id: "slot_g".into(),
        plugin_id: "vst3:test".into(),
        format: "vst3".into(),
        name: "Gain".into(),
        vendor: String::new(),
        path: String::new(),
        bypass: false,
        state: None,
        params: Default::default(),
    }));
    editing::set_automation(st, "slot_g", AutomationParam::Plugin(0), sr / 2, 0.0).unwrap();
    editing::set_automation(st, "slot_g", AutomationParam::Plugin(0), 2 * sr, 1.0).unwrap();
    // another track with the same name (names repeat across scenes) and a built-in effect: printed too, to its own file
    let wind = {
        let cue = st.cues.values().find(|c| c.description.contains("wind")).unwrap().id.clone();
        st.clips.values().find(|c| c.cue_id.as_deref() == Some(cue.as_str())).unwrap().track_id.clone()
    };
    let gname0 = st.tracks[&glass].name.clone();
    let w = st.tracks.get_mut(&wind).unwrap();
    w.name = gname0;
    w.inserts = vec![Insert::HighPass { hz: 200.0, q: 0.7 }];

    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.aaf_print_plugins = true;
    o.aaf = true;
    o.force = true;
    o.plugins = Some(mixer::PluginHostRef(std::sync::Arc::new(GainPlugin)));
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let st = &s.state;
    let (start, end) = export::resolve_range(st, &export::ExportRange::All).unwrap();
    let gname = st.tracks[&glass].name.clone();
    let prints: Vec<&String> = r.files.iter().filter(|f| f.contains("_print")).collect();
    assert_eq!(prints.len(), 2, "two prints, two files: {prints:?}");
    for f in &prints {
        assert!(f.starts_with(&format!("AAF/Media/{}_print", vcsd_core::layout::safe_name(&gname))), "{f}");
    }

    // the glass print = the dry clips × the plugin's automated gain (pre-fader: no −6 dB)
    let dry = mixer::Mixer::new(st, mixer::RenderOptions::default()).render_track_prefader(&glass, start, (end - start) as usize).unwrap();
    let lane = |x: i64| ((x - sr / 2) as f64 / (1.5 * sr as f64)).clamp(0.0, 1.0);
    assert!(dry.peak() > 0.01);
    let ch = dry.channels as usize;
    let diff = |rel: &str| {
        let (_, print) = wav::read(&r.dir.join(rel)).unwrap();
        (0..dry.frames())
            .map(|f| (dry.data[f * ch] as f64 * 2.0 * lane(start + f as i64) - print.data[f * ch] as f64).abs())
            .fold(0.0f64, f64::max)
    };
    let (print_rel, max) = prints.iter().map(|f| ((*f).clone(), diff(f))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    assert!(max < 1e-4, "plugin and its automation baked in (max diff {max})");

    let rep: serde_json::Value = serde_json::from_slice(&std::fs::read(r.dir.join("AAF/aaf_report.json")).unwrap()).unwrap();
    assert_eq!(rep["printed_tracks"], 2);
    let losses: Vec<&str> = rep["losses"].as_array().unwrap().iter().filter_map(|l| l.as_str()).collect();
    assert!(losses.iter().any(|l| l.starts_with("tracks printed with their plugins")), "{losses:?}");
    assert!(!losses.iter().any(|l| l.starts_with("plugin parameter automation not carried")), "{losses:?}");
    assert!(std::fs::read_to_string(r.dir.join("README.md")).unwrap().contains("tracks with plugins or insert effects are printed"));

    if let Some(py) = python() {
        let v = inspect(&py, &r.dir.join(r.files.iter().find(|f| f.ends_with(".aaf")).unwrap()));
        let tracks = v["tracks"].as_array().unwrap();
        let glass_file = print_rel.rsplit('/').next().unwrap().to_string();
        let gt = tracks
            .iter()
            .find(|t| {
                t["components"].as_array().unwrap().iter().any(|c| c["media"]["urls"][0].as_str().is_some_and(|u| u.ends_with(&glass_file)))
            })
            .expect("printed glass track");
        assert_eq!(gt["name"], gname.as_str());
        let clips: Vec<&serde_json::Value> = gt["components"].as_array().unwrap().iter().filter(|c| c["kind"] == "SourceClip").collect();
        assert_eq!(clips.len(), 1, "one clip: the print");
        assert_eq!(clips[0]["length"], end - start);
        // the two same-named tracks link to different files
        let urls: std::collections::BTreeSet<String> = tracks
            .iter()
            .flat_map(|t| t["components"].as_array().unwrap())
            .filter_map(|c| c["media"]["urls"][0].as_str())
            .filter(|u| u.contains("_print"))
            .map(String::from)
            .collect();
        assert_eq!(urls.len(), 2, "{urls:?}");
        // its fader stays editable as track gain (it isn't in the print)
        let g = gt["volume"]["constant"].as_f64().expect("fader as constant track gain");
        assert!((20.0 * g.log10() + 6.0).abs() < 0.01, "{g}");
        // other tracks keep their editable clips
        assert!(tracks.iter().any(|t| t["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "SourceClip" && !c["media"]["urls"][0].as_str().unwrap().contains("_print"))));
    }
}

#[test]
fn cue_markers_carry_their_own_or_category_colors() {
    let Some(py) = python() else {
        eprintln!("SKIPPED: pyaaf2 not installed (python3 -m pip install pyaaf2==1.7.1)");
        return;
    };
    let (mut s, root) = session("aafmarkercolors", "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    // one cue per palette color: FX red, BG green, design magenta, dialogue blue, music purple, foley yellow
    let cues = [("fx", "glass"), ("bg", "wind"), ("design", "whoosh"), ("dialogue", "line"), ("music", "sting"), ("footsteps", "steps")];
    for (k, (lane_cat, what)) in cues.iter().enumerate() {
        let tc = format!("01:00:0{}:00", k + 1);
        marker_tc(&mut s, lane_cat, EventType::Point, &tc, &tc, what, None);
    }
    // and markers an editor colored themselves (their cues carry those colors instead)
    let custom = [("teal", "#12a4c8"), ("amber", "#e07010"), ("rosy", "#ff8080"), ("violet", "#5a2080")];
    for (k, (what, _)) in custom.iter().enumerate() {
        let tc = format!("01:00:0{}:12", k + 6);
        marker_tc(&mut s, "fx", EventType::Point, &tc, &tc, what, None);
    }
    for (what, hex) in custom {
        s.state.markers.values_mut().find(|m| m.label == what).unwrap().color = Some(hex.into());
    }
    // and a range: its duration travels as the marker's Length
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:00:20", "rain", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-23");
    o.aaf = true;
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let aaf = r.dir.join(r.files.iter().find(|f| f.ends_with(".aaf")).unwrap());
    let otio = r.dir.join(r.files.iter().find(|f| f.ends_with(".otio")).unwrap());

    // pyaaf2: each marker's CommentMarkerColor, 16-bit channels
    let code = r#"
import sys, json, aaf2
f = aaf2.open(sys.argv[1], 'r')
m = next(f.content.compositionmobs())
out = {}
for sl in m.slots:
    if sl.segment.classdef.class_name != 'Sequence' or sl.segment.media_kind != 'DescriptiveMetadata':
        continue
    for c in sl.segment.components:
        v = c['CommentMarkerColor'].value
        out[c['Comment'].value] = [v['red'], v['green'], v['blue']]
print(json.dumps(out))
"#;
    let out = std::process::Command::new(&py).args(["-c", code]).arg(&aaf).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let colors: std::collections::BTreeMap<String, Vec<u32>> = serde_json::from_slice(&out.stdout).unwrap();
    let of =
        |what: &str| colors.iter().find(|(k, _)| k.contains(what)).map(|(_, v)| v.clone()).unwrap_or_else(|| panic!("{what}: {colors:?}"));
    assert_eq!(of("glass"), [65535, 0, 0]);
    assert_eq!(of("wind"), [0, 65535, 0]);
    assert_eq!(of("whoosh"), [65535, 0, 65535]);
    assert_eq!(of("line"), [0, 0, 65535]);
    assert_eq!(of("sting"), [32639, 0, 32639]);
    assert_eq!(of("steps"), [65535, 65535, 0]);
    assert_eq!(of("teal"), [0x12 * 257, 0xa4 * 257, 0xc8 * 257], "a marker's own color, exactly");

    // through the OpenTimelineIO AAF adapter: the same marker colors as our OTIO export
    let adapter =
        std::process::Command::new(&py).args(["-c", "import otio_aaf_adapter"]).output().map(|o| o.status.success()).unwrap_or(false);
    if !adapter {
        eprintln!("SKIPPED (adapter part): the OpenTimelineIO AAF adapter isn't installed");
        return;
    }
    let names = r#"
import sys, json, opentimelineio as otio
kw = {'adapter_name': 'AAF', 'simplify': True} if sys.argv[1].endswith('.aaf') else {}
t = otio.adapters.read_from_file(sys.argv[1], **kw)
# wherever a reader hangs them (the adapter puts them on clips, sometimes twice)
ms = [m for item in [t.tracks] + list(t.find_children()) for m in getattr(item, 'markers', [])]
print(json.dumps(sorted({((m.name or '') + ' ' + (m.comment or ''), m.color + ' ' + str(round(m.marked_range.duration.to_seconds(), 6))) for m in ms})))
"#;
    let read = |f: &Path| {
        let out = std::process::Command::new(&py).args(["-c", names]).arg(f).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let v: Vec<(String, String)> = serde_json::from_slice(&out.stdout).unwrap();
        v
    };
    let (via_aaf, ours) = (read(&aaf), read(&otio));
    let color_of = |list: &[(String, String)], what: &str| {
        list.iter().find(|(t, _)| t.contains(what)).map(|x| x.1.clone()).unwrap_or_else(|| panic!("{what}: {list:?}"))
    };
    // colors and durations alike
    for what in cues.iter().map(|c| c.1).chain(custom.iter().map(|c| c.0)).chain(["rain"]) {
        assert_eq!(color_of(&via_aaf, what), color_of(&ours, what), "{what}: AAF → adapter vs our OTIO");
    }
    assert_eq!(color_of(&ours, "rain"), "GREEN 0.833333", "the range: 20 frames at 24 fps");
    assert!(color_of(&ours, "glass").ends_with(" 0.0"), "a point");
    let named: Vec<String> = custom.iter().map(|c| color_of(&ours, c.0).split(' ').next().unwrap().to_string()).collect();
    assert_eq!(named, ["BLUE", "ORANGE", "PINK", "PURPLE"], "own colors named as the adapter names them");
}

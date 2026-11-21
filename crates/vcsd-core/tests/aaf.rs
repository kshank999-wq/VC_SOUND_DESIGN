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

    let sr = st.project.sample_rate as i64;
    let start = sr; // from 01:00:01:00: clips before it are trimmed
    let end = 9 * sr;
    let used: Vec<String> = st.clips.values().map(|c| c.asset_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let (media, _notes) = aaf::prepare_media(st, &used, &root.join("Exports/aaf/Media")).unwrap();
    let file = root.join("Exports/aaf/test.aaf");
    let rep =
        aaf::write(st, &file, &media, &AafOptions { name: "Gravel Road — scene 1".into(), start, end, exclude_temp_refs: false }).unwrap();
    assert!(rep.losses.iter().any(|l| l.starts_with("muted tracks left out (1)")), "{:?}", rep.losses);
    assert!(rep.losses.iter().any(|l| l.starts_with("overlapping clips split")), "{:?}", rep.losses);
    assert!(rep.losses.iter().any(|l| l.starts_with("track fader gain folded")), "{:?}", rep.losses);

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
    assert!(mk.iter().all(|m| m["kind"] == "CommentMarker"));
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
    assert!(readme.contains("bus routing and bus gain not carried"));
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

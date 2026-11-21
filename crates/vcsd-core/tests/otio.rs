//! OTIO export, read back with the OpenTimelineIO library via
//! `scripts/otio/otio_summary.py`. The library checks are skipped (loudly) when it
//! isn't installed: `python3 -m pip install opentimelineio==0.18.1`, or set
//! VCSD_OTIO_PYTHON. The JSON itself is checked either way.

mod common;
use common::*;
use std::path::Path;
use vcsd_core::builder::{populate, PopulateOptions};
use vcsd_core::spotting::*;
use vcsd_core::*;

fn python() -> Option<String> {
    let py = std::env::var("VCSD_OTIO_PYTHON").unwrap_or_else(|_| "python3".into());
    let ok = std::process::Command::new(&py).args(["-c", "import opentimelineio"]).output().map(|o| o.status.success()).unwrap_or(false);
    ok.then_some(py)
}

fn inspect(py: &str, file: &Path) -> serde_json::Value {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/otio/otio_summary.py");
    let out = std::process::Command::new(py).arg(script).arg(file).output().unwrap();
    assert!(out.status.success(), "OpenTimelineIO could not read the file:\n{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A scene with glass (fades, gain, an overlapping copy, automation), a stereo wind bed
/// and a muted door track.
fn project(name: &str) -> (state::Session, std::path::PathBuf, String, String) {
    let (mut s, root) = session(name, "24", "01:00:00:00");
    with_pack(&mut s);
    scene(&mut s, "1", "01:00:00:00", "01:00:10:00");
    marker_tc(&mut s, "fx", EventType::Point, "01:00:02:00", "01:00:02:00", "glass smash", None);
    marker_tc(&mut s, "bg", EventType::Range, "01:00:00:00", "01:00:08:00", "wind exterior", None);
    marker_tc(&mut s, "fx", EventType::Point, "01:00:05:00", "01:00:05:00", "door slam", None);
    s.edit("c", |st| markers_to_cues(st, None)).unwrap();
    s.edit("p", |st| populate(st, &PopulateOptions::default())).unwrap();
    let st = &mut s.state;
    // one-shot clips, so each is one OTIO clip (loops are written as butt-spliced repeats)
    let lens: std::collections::HashMap<String, i64> = st.assets.values().map(|a| (a.id.clone(), a.length_samples)).collect();
    for c in st.clips.values_mut() {
        c.fill = FillMode::OneShot;
        c.length = c.length.min(lens[&c.asset_id] - c.source_offset);
    }
    let cue_clip = |st: &ProjectState, what: &str| {
        let cue = st.cues.values().find(|c| c.description.contains(what)).unwrap().id.clone();
        st.clips.values().find(|c| c.cue_id.as_deref() == Some(cue.as_str()) && !c.muted).unwrap().id.clone()
    };
    let glass = cue_clip(st, "glass");
    let g = st.clips.get_mut(&glass).unwrap();
    g.gain_db = -6.0;
    g.fade_in = 480;
    g.fade_out = 960;
    let mut dup = g.clone();
    dup.id = "clip_dup".into();
    dup.timeline_start += 100;
    st.clips.insert(dup.id.clone(), dup);
    let gt = st.clips[&glass].track_id.clone();
    st.tracks.get_mut(&gt).unwrap().gain_db = -3.0;
    let sr = st.project.sample_rate as i64;
    for (t, v) in [(2 * sr, 0.0f32), (3 * sr, -20.0)] {
        editing::set_automation(st, &gt, AutomationParam::VolumeDb, t, v).unwrap();
    }
    // pan: a sweep on the glass track, and the stereo wind bed's balance held right of centre
    for (t, v) in [(2 * sr, -1.0f32), (6 * sr, 1.0)] {
        editing::set_automation(st, &gt, AutomationParam::Pan, t, v).unwrap();
    }
    let wind = cue_clip(st, "wind");
    let wt = st.clips[&wind].track_id.clone();
    st.tracks.get_mut(&wt).unwrap().pan = 0.5;
    let door = cue_clip(st, "door");
    let dt = st.clips[&door].track_id.clone();
    st.tracks.get_mut(&dt).unwrap().mute = true;
    (s, root, glass, gt)
}

#[test]
fn otio_rebuilds_the_timeline_in_opentimelineio() {
    let (mut s, root, glass, gt) = project("otio");
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.otio = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    let st = &s.state;
    let rel = r.files.iter().find(|f| f.ends_with(".otio")).expect("an .otio in the package").clone();
    assert!(rel.starts_with("OTIO/"));
    assert!(!r.files.iter().any(|f| f.ends_with(".aaf")), "no AAF asked for");
    assert!(r.files.iter().any(|f| f.starts_with("OTIO/Media/") && f.ends_with(".wav")), "media with the OTIO when there's no AAF");
    let rep: serde_json::Value = serde_json::from_slice(&std::fs::read(r.dir.join("OTIO/otio_report.json")).unwrap()).unwrap();
    let losses: Vec<&str> = rep["losses"].as_array().unwrap().iter().filter_map(|l| l.as_str()).collect();
    assert!(losses.iter().any(|l| l.starts_with("muted tracks left out (1)")), "{losses:?}");
    assert!(losses.iter().any(|l| l.starts_with("overlapping clips split onto extra tracks (1)")), "{losses:?}");
    assert!(losses.iter().any(|l| l.starts_with("pan uses the receiving app's pan law") && l.ends_with("(2)")), "{losses:?}");
    assert!(r.qa.iter().any(|q| q.code == "otio_loss"));
    let readme = std::fs::read_to_string(r.dir.join("README.md")).unwrap();
    assert!(readme.contains("same timeline as OpenTimelineIO") && readme.contains("`OTIO/Media/`"), "{readme}");

    // the JSON as written
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(r.dir.join(&rel)).unwrap()).unwrap();
    assert_eq!(raw["OTIO_SCHEMA"], "Timeline.1");
    assert_eq!(raw["global_start_time"]["value"], 86_400.0, "01:00:00:00 at 24 fps");
    assert_eq!(raw["metadata"]["vcsd"]["start_tc"], "01:00:00:00");

    let Some(py) = python() else {
        eprintln!("SKIPPED: OpenTimelineIO not installed (python3 -m pip install opentimelineio==0.18.1)");
        return;
    };
    let v = inspect(&py, &r.dir.join(&rel));
    let (start, end) = export::resolve_range(st, &export::ExportRange::All).unwrap();
    assert_eq!(v["global_start"], serde_json::json!([86_400.0, 24.0]));
    assert_eq!(v["duration"], end - start);
    let tracks = v["tracks"].as_array().unwrap();
    assert!(tracks.iter().all(|t| t["kind"] == "Audio" && t["duration"] == end - start), "every track spans the range");
    let gtrack = &st.tracks[&gt];
    assert!(tracks.iter().any(|t| t["name"] == format!("{} (2)", gtrack.name)), "the overlapping copy on a second layer");
    let door_named: Vec<&str> = tracks.iter().filter_map(|t| t["name"].as_str()).filter(|n| n.contains("door")).collect();
    assert!(door_named.is_empty(), "the muted track is left out: {door_named:?}");

    // every audible clip, placed exactly, linking real media
    let mut seen = 0;
    for c in st.clips.values().filter(|c| !c.muted && !st.tracks[&c.track_id].mute) {
        let item = tracks
            .iter()
            .flat_map(|t| t["items"].as_array().unwrap())
            .find(|i| i["kind"] == "Clip" && i["start"] == c.timeline_start - start && i["duration"] == c.length)
            .unwrap_or_else(|| panic!("clip {} at {}", c.id, c.timeline_start));
        assert_eq!(item["source"], c.source_offset);
        assert!(Path::new(item["file"].as_str().unwrap()).is_file(), "{}", item["file"]);
        assert_eq!(item["available"], st.assets[&c.asset_id].length_samples);
        seen += 1;
    }
    assert_eq!(seen, rep["clips"].as_u64().unwrap() as usize);

    // the glass clip: gain and fades in metadata; its fade in from silence as a dissolve
    let gc = &st.clips[&glass];
    let layer = tracks.iter().find(|t| t["name"] == gtrack.name.as_str()).unwrap();
    let items = layer["items"].as_array().unwrap();
    let k = items.iter().position(|i| i["kind"] == "Clip" && i["start"] == gc.timeline_start - start).unwrap();
    assert_eq!(items[k]["metadata"]["vcsd"]["gain_db"], -6.0, "clip gain without the fader");
    assert_eq!(items[k]["metadata"]["vcsd"]["fade_in"], 480);
    let before = &items[k - 1];
    assert_eq!(
        (before["kind"].as_str(), before["type"].as_str(), &before["in"], &before["out"]),
        (Some("Transition"), Some("SMPTE_Dissolve"), &serde_json::json!(0), &serde_json::json!(480))
    );
    assert_eq!(items[k - 2]["kind"], "Gap", "faded in from silence");
    let after = &items[k + 1];
    assert_eq!((after["kind"].as_str(), &after["in"], &after["out"]), (Some("Transition"), &serde_json::json!(960), &serde_json::json!(0)));
    // the track's fader and automation in its metadata (offsets from the range start)
    let meta = &layer["metadata"]["vcsd"];
    assert_eq!(meta["gain_db"], -3.0);
    assert_eq!(meta["channels"], 1);
    let sr = st.project.sample_rate as i64;
    assert_eq!(meta["volume_db"], serde_json::json!([[2 * sr - start, 0.0], [3 * sr - start, -20.0]]));
    let wind = tracks.iter().find(|t| t["name"].as_str().unwrap().contains("wind")).unwrap();
    assert_eq!(wind["metadata"]["vcsd"]["channels"], 2, "a stereo track is one OTIO track");
    // the stereo bed: no level effect at unity; its balance as a constant pan
    let we = wind["effects"].as_array().unwrap();
    assert_eq!(we.len(), 1, "{we:?}");
    assert_eq!((we[0]["name"].as_str(), we[0]["parameters"]["Pan"].as_str()), (Some("Mono Audio Pan"), Some("750000/1000000")));

    // levels as AAF-style gain effects (the OpenTimelineIO AAF adapter's encoding)
    let ce = &items[k]["effects"][0];
    assert_eq!(
        (ce["name"].as_str(), ce["parameters"]["Amplitude"].as_str()),
        (Some("Mono Audio Gain"), Some("501187/1000000")),
        "clip −6 dB"
    );
    let te = &layer["effects"];
    assert_eq!(te.as_array().unwrap().len(), 2, "gain, then pan");
    assert_eq!(te[0]["name"], "Mono Audio Gain");
    let amp = &te[0]["parameters"]["Amplitude"];
    assert_eq!((amp["_aaf_keyframed_property"].as_bool(), amp["keyframe_interpolation"].as_str()), (Some(true), Some("Linear")));
    let kf: Vec<(f64, f64)> =
        amp["keyframe_values"].as_array().unwrap().iter().map(|k| (k[0].as_f64().unwrap(), k[1].as_f64().unwrap())).collect();
    let at = |u: f64| {
        let i = kf.partition_point(|p| p.0 <= u);
        let (a, b) = (kf[i.saturating_sub(1)], kf[i.min(kf.len() - 1)]);
        if b.0 > a.0 {
            a.1 + (b.1 - a.1) * (u - a.0) / (b.0 - a.0)
        } else {
            a.1
        }
    };
    // against the mixer: the −3 dB fader on top of the lane (0 → −20 dB from 2 s to 3 s, in dB)
    let len = (end - start) as f64;
    for k in 0..=400 {
        let x = start + (k as f64 / 400.0 * (len - 1.0)) as i64;
        let lane = if x <= 2 * sr {
            0.0
        } else if x >= 3 * sr {
            -20.0
        } else {
            -20.0 * (x - 2 * sr) as f64 / sr as f64
        };
        let want = -3.0 + lane;
        let got = 20.0 * at((x - start) as f64 / len).log10();
        assert!((got - want).abs() < 0.12, "volume at {x}: {got:.3} dB vs {want:.3} dB");
    }
    // pan: the lane's breakpoints as keyframes (0 left … 1 right), so exact everywhere
    let pan = &te[1];
    assert_eq!(pan["name"], "Mono Audio Pan");
    let pk: Vec<(f64, f64)> = pan["parameters"]["Pan"]["keyframe_values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (k[0].as_f64().unwrap(), k[1].as_f64().unwrap()))
        .collect();
    assert!(pk.len() <= 4, "breakpoints, not a fit: {pk:?}");
    let pat = |u: f64| {
        let i = pk.partition_point(|p| p.0 <= u);
        let (a, b) = (pk[i.saturating_sub(1)], pk[i.min(pk.len() - 1)]);
        if b.0 > a.0 {
            a.1 + (b.1 - a.1) * (u - a.0) / (b.0 - a.0)
        } else {
            a.1
        }
    };
    for k in 0..=400 {
        let x = start + (k as f64 / 400.0 * (len - 1.0)) as i64;
        let lane = ((x - 2 * sr) as f64 / (4 * sr) as f64 * 2.0 - 1.0).clamp(-1.0, 1.0);
        let want = (lane + 1.0) / 2.0;
        assert!((pat((x - start) as f64 / len) - want).abs() < 1e-5, "pan at {x}");
    }

    // cues → timeline markers
    let markers = v["markers"].as_array().unwrap();
    assert_eq!(markers.len(), st.cues.len());
    let glass_cue = st.cues.values().find(|c| c.description.contains("glass")).unwrap();
    let m = markers.iter().find(|m| m["name"] == glass_cue.code.as_str()).unwrap();
    assert_eq!(
        (m["start"].as_i64(), m["color"].as_str(), m["comment"].as_str()),
        (Some(glass_cue.start_sample - start), Some("RED"), Some("glass smash"))
    );
}

#[test]
fn otio_alongside_aaf_shares_media_and_prints() {
    let (mut s, root, _, gt) = project("otioaaf");
    s.state.tracks.get_mut(&gt).unwrap().inserts = vec![Insert::HighPass { hz: 200.0, q: 0.7 }];
    let mut o = export::ExportOptions::new(export::ExportProfile::Resolve, "2026-11-21");
    o.otio = true;
    o.aaf = true;
    o.aaf_print_plugins = true;
    o.force = true;
    let r = s.edit("x", |st| export::export(st, Some(&root), &o)).unwrap();
    assert!(r.files.iter().any(|f| f.ends_with(".aaf")) && r.files.iter().any(|f| f.ends_with(".otio")));
    assert!(!r.files.iter().any(|f| f.starts_with("OTIO/Media/")), "one copy of the media, with the AAF");
    let rel = r.files.iter().find(|f| f.ends_with(".otio")).unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(r.dir.join(rel)).unwrap()).unwrap();
    let urls: Vec<String> = raw["tracks"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["children"].as_array().unwrap().clone())
        .filter_map(|c| c["media_references"]["DEFAULT_MEDIA"]["target_url"].as_str().map(String::from))
        .collect();
    assert!(!urls.is_empty() && urls.iter().all(|u| u.contains("/AAF/Media/")), "{urls:?}");
    // the printed track: one clip of its render, the same file the AAF links
    let gname = &s.state.tracks[&gt].name;
    let printed: Vec<&serde_json::Value> =
        raw["tracks"]["children"].as_array().unwrap().iter().filter(|t| t["metadata"]["vcsd"]["printed"] == true).collect();
    assert_eq!(printed.len(), 1);
    assert_eq!(printed[0]["name"], gname.as_str());
    let clips: Vec<&serde_json::Value> =
        printed[0]["children"].as_array().unwrap().iter().filter(|c| c["OTIO_SCHEMA"] == "Clip.2").collect();
    assert_eq!(clips.len(), 1);
    assert!(clips[0]["media_references"]["DEFAULT_MEDIA"]["target_url"].as_str().unwrap().contains("_print"));
    let rep: serde_json::Value = serde_json::from_slice(&std::fs::read(r.dir.join("OTIO/otio_report.json")).unwrap()).unwrap();
    assert_eq!(rep["printed_tracks"], 1);
    let readme = std::fs::read_to_string(r.dir.join("README.md")).unwrap();
    assert!(readme.contains("holds the editable timeline") && readme.contains("same timeline as OpenTimelineIO"));
    if let Some(py) = python() {
        let v = inspect(&py, &r.dir.join(rel));
        assert_eq!(v["tracks"].as_array().unwrap().len(), rep["tracks"].as_u64().unwrap() as usize);
    }
}

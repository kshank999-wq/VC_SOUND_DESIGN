#![allow(dead_code)]
use std::path::{Path, PathBuf};
use vcsd_core::library::{import_pack, Query};
use vcsd_core::spotting::*;
use vcsd_core::*;

pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vcsd_test_{name}_{}_{}", std::process::id(), ids::new_id("t")));
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn pack_dir() -> PathBuf {
    // one shared pack per test binary run (generated once, race-free)
    static PACK: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PACK.get_or_init(|| {
        let d = tmp("pack");
        synth::write_starter_pack(&d, 48_000).unwrap();
        d
    })
    .clone()
}

pub fn session(name: &str, fps: &str, start_tc: &str) -> (Session, PathBuf) {
    let root = tmp(name);
    let s = Session::create(
        &root,
        NewProject {
            name: format!("Test {name}"),
            frame_rate: FrameRate::parse(fps).unwrap(),
            sample_rate: 48_000,
            start_tc: start_tc.into(),
        },
    )
    .unwrap();
    (s, root)
}

pub fn with_pack(s: &mut Session) -> String {
    let p = pack_dir();
    s.edit("import pack", |st| Ok(import_pack(st, &p, true)?.0)).unwrap()
}

pub fn asset(s: &Session, words: &str) -> String {
    let v = library::search(&s.state, &Query { text: words.into(), ..Default::default() });
    v.first().unwrap_or_else(|| panic!("no asset for '{words}'")).id.clone()
}

pub fn assets(s: &Session, words: &str) -> Vec<String> {
    library::search(&s.state, &Query { text: words.into(), ..Default::default() }).iter().map(|a| a.id.clone()).collect()
}

pub fn tc(s: &Session, tc: &str) -> i64 {
    s.state.tb().sample_at_tc(tc).unwrap()
}

pub fn scene(s: &mut Session, num: &str, a: &str, z: &str) -> String {
    let (a, z) = (tc(s, a), tc(s, z));
    s.edit("scene", |st| add_scene(st, num, &format!("Scene {num}"), a, z)).unwrap()
}

pub fn lane(s: &Session, cat: &str) -> String {
    s.state.find_lane(cat).unwrap().id.clone()
}

pub fn marker(s: &mut Session, lane_cat: &str, kind: EventType, a: i64, z: i64, label: &str, ident: Option<&str>) -> String {
    let l = lane(s, lane_cat);
    let ident = ident.map(String::from);
    s.edit("marker", |st| {
        add_marker(
            st,
            NewMarker {
                lane_id: l,
                scene_id: None,
                kind,
                start: a,
                end: z,
                label: label.into(),
                identity_id: ident,
                sub_events: vec![],
                origin: TimingOrigin::Manual,
            },
        )
    })
    .unwrap()
}

pub fn fake_video(dir: &Path) -> PathBuf {
    let p = dir.join("Picture/reference_v1.mov");
    std::fs::write(&p, b"not really a movie, just bytes for fingerprinting").unwrap();
    p
}

pub fn clip_times(s: &ProjectState) -> Vec<(String, i64, i64)> {
    let mut v: Vec<_> = s.clips.values().map(|c| (c.id.clone(), c.timeline_start, c.length)).collect();
    v.sort();
    v
}

pub fn marker_tc(s: &mut Session, lane_cat: &str, kind: EventType, a: &str, z: &str, label: &str, ident: Option<&str>) -> String {
    let (a, z) = (tc(s, a), tc(s, z));
    marker(s, lane_cat, kind, a, z, label, ident)
}

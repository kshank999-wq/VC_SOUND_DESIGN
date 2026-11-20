//! Writing automation from controls during real-time playback (null device clock).
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use vcsd_engine::Engine;

fn call(e: &Arc<Engine>, cmd: &str, a: Value) -> Value {
    e.call(cmd, a).unwrap_or_else(|err| panic!("{cmd}: {err}"))
}
fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Lane value at a sample (same rules as the engine).
fn lane_at(st: &Value, owner: &str, param: &str, t: i64) -> f64 {
    let mut p: Vec<(i64, f64)> = st["automation"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["owner_id"] == owner && x["param"] == param)
        .map(|x| (x["sample"].as_i64().unwrap(), x["value"].as_f64().unwrap()))
        .collect();
    p.sort_by_key(|x| x.0);
    if p.is_empty() {
        return 0.0;
    }
    if t <= p[0].0 {
        return p[0].1;
    }
    for w in p.windows(2) {
        if t >= w[0].0 && t < w[1].0 {
            return w[0].1 + (w[1].1 - w[0].1) * (t - w[0].0) as f64 / (w[1].0 - w[0].0) as f64;
        }
    }
    p.last().unwrap().1
}

fn setup() -> (Arc<Engine>, String, String) {
    let dir = std::env::temp_dir().join(format!("vcsd_aw_{}_{}", std::process::id(), vcsd_core::ids::new_id("t")));
    let e = Engine::with_prefs(None);
    call(&e, "project.new", json!({"dir": dir, "name": "aw", "fps": "24", "start_tc": "01:00:00:00"}));
    let lib = dir.join("Audio/LibraryRefs/fx");
    std::fs::create_dir_all(&lib).unwrap();
    let wavp = lib.join("const.wav");
    vcsd_core::wav::write(
        &wavp,
        &vcsd_core::wav::AudioBuffer { sample_rate: 48_000, channels: 1, data: vec![0.5; 48_000 * 12] },
        vcsd_core::wav::SampleFormat::Float32,
        None,
    )
    .unwrap();
    call(&e, "library.import", json!({"path": lib.parent().unwrap(), "placeholder": true}));
    call(&e, "scene.add", json!({"number": "1", "title": "t", "start": 0, "end": 48000 * 12}));
    call(&e, "marker.add", json!({"lane": "fx", "kind": "range", "start": 0, "end": 48000 * 10, "label": "const"}));
    call(&e, "cues.build", json!({}));
    call(&e, "populate", json!({}));
    let st = call(&e, "state.get", json!({}));
    assert_eq!(st["clips"].as_array().unwrap().len(), 1, "constant tone placed");
    let t = st["tracks"][0]["id"].as_str().unwrap().to_string();
    let master = st["buses"].as_array().unwrap().iter().find(|b| b["parent_id"].is_null()).unwrap()["id"].as_str().unwrap().to_string();
    (e, t, master)
}

fn pos(e: &Arc<Engine>) -> i64 {
    call(e, "transport.state", json!({}))["position"].as_i64().unwrap()
}

#[test]
fn latch_touch_write_and_read() {
    let (e, t, master) = setup();
    let sr = 48_000i64;
    // track meters are post-pan: a centred mono 0.5 source reads 0.5 × −3 dB
    let full = 0.5 / 2f64.sqrt();
    let meter = |e: &Arc<Engine>| {
        let mut m = 0.0f64;
        for _ in 0..4 {
            sleep(40);
            m = m.max(call(e, "transport.state", json!({}))["meters"][t.as_str()].as_f64().unwrap_or(0.0));
        }
        m
    };

    // Read: touches are not captured
    call(&e, "transport.play", json!({"from": 0}));
    sleep(150);
    let r = call(&e, "automation.touch", json!({"owner_id": t, "param": "volume_db", "value": -10, "phase": "begin"}));
    assert_eq!(r["capturing"], false);
    call(&e, "transport.stop", json!({}));
    assert!(call(&e, "state.get", json!({}))["automation"].as_array().unwrap().is_empty());

    // Latch: touch at -6, move to -12, release; value latches until stop
    call(&e, "automation.mode", json!({"owner_id": t, "mode": "latch"}));
    call(&e, "transport.play", json!({"from": 0}));
    sleep(300);
    call(&e, "automation.touch", json!({"owner_id": t, "param": "volume_db", "value": -6, "phase": "begin"}));
    let t_begin = pos(&e);
    sleep(250);
    call(&e, "automation.touch", json!({"owner_id": t, "param": "volume_db", "value": -12, "phase": "move"}));
    let t_move = pos(&e);
    sleep(150);
    call(&e, "automation.touch", json!({"owner_id": t, "param": "volume_db", "value": -12, "phase": "end"}));
    let live = meter(&e);
    assert!((20.0 * (live / full).log10() + 12.0).abs() < 1.0, "monitoring the latched value live: {live}");
    let writing = call(&e, "transport.state", json!({}))["writing"].clone();
    assert_eq!(writing.as_array().unwrap().len(), 1, "latch stays armed after release");
    sleep(300);
    let stop = call(&e, "transport.stop", json!({}));
    assert!(stop["automation_points_written"].as_u64().unwrap() >= 3);
    let st = call(&e, "state.get", json!({}));
    assert!(lane_at(&st, &t, "volume_db", t_begin - 4000).abs() < 0.01, "untouched before the first touch");
    assert!((lane_at(&st, &t, "volume_db", t_begin) + 6.0).abs() < 0.6, "touch value recorded at the touch");
    assert!((lane_at(&st, &t, "volume_db", t_move) + 12.0).abs() < 0.6, "move recorded where it happened");
    let end_s = pos(&e);
    assert!((lane_at(&st, &t, "volume_db", end_s - 4000) + 12.0).abs() < 0.01, "latched to the stop point");
    assert!(lane_at(&st, &t, "volume_db", end_s + 5 * sr).abs() < 0.01, "returns to the previous level after the pass");
    call(&e, "undo", json!({}));
    assert!(call(&e, "state.get", json!({}))["automation"].as_array().unwrap().is_empty(), "one undo removes the pass");

    // Touch: releases return to the existing automation immediately, while still playing
    call(&e, "automation.mode", json!({"owner_id": t, "mode": "touch"}));
    call(&e, "transport.play", json!({"from": 0}));
    sleep(200);
    call(&e, "automation.touch", json!({"owner_id": t, "param": "volume_db", "value": -20, "phase": "begin"}));
    let tb = pos(&e);
    sleep(250);
    call(&e, "automation.touch", json!({"owner_id": t, "param": "volume_db", "value": -20, "phase": "end"}));
    let tr = pos(&e);
    let st = call(&e, "state.get", json!({}));
    assert!(call(&e, "transport.state", json!({}))["writing"].as_array().unwrap().is_empty(), "touch pass committed on release");
    assert!((lane_at(&st, &t, "volume_db", (tb + tr) / 2) + 20.0).abs() < 0.01);
    let back = meter(&e);
    assert!((20.0 * (back / full).log10()).abs() < 0.5, "after release playback is back at 0 dB: {back}");
    call(&e, "transport.stop", json!({}));

    // Write: the whole pass is written; the master balance too (bus pass)
    call(&e, "automation.clear", json!({"owner_id": t}));
    call(&e, "automation.mode", json!({"owner_id": t, "mode": "write"}));
    call(&e, "automation.mode", json!({"owner_id": master, "mode": "latch"}));
    call(&e, "transport.play", json!({"from": sr}));
    sleep(200);
    call(&e, "automation.touch", json!({"owner_id": t, "param": "volume_db", "value": -3, "phase": "begin"}));
    call(&e, "automation.touch", json!({"owner_id": t, "param": "volume_db", "value": -3, "phase": "end"}));
    call(&e, "automation.touch", json!({"owner_id": master, "param": "pan", "value": 0.5, "phase": "begin"}));
    sleep(300);
    call(&e, "transport.stop", json!({}));
    let end_w = pos(&e);
    let st = call(&e, "state.get", json!({}));
    assert!(lane_at(&st, &t, "volume_db", sr + 1000).abs() < 0.01, "write starts from the value at play");
    assert!((lane_at(&st, &t, "volume_db", end_w - 2000) + 3.0).abs() < 0.01, "control value written to the stop");
    assert!((lane_at(&st, &master, "pan", end_w - 2000) - 0.5).abs() < 0.01, "bus (master balance) written");
    let pan_pts = st["automation"].as_array().unwrap().iter().filter(|x| x["owner_id"] == t.as_str() && x["param"] == "pan").count();
    assert!(pan_pts > 0, "write mode writes every lane of the track for the pass");
    // mode persists in the project
    assert_eq!(st["tracks"][0]["automation_mode"], "write");
}

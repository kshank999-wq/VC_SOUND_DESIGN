//! Plugin hosting through the engine API, with the test VST3 (`vcsd-testplug`)
//! in a real bundle: scan, insert, parameters, undo, save/reopen, latency
//! compensation, playback, missing plugins and export.
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use vcsd_core::mixer::{Mixer, PluginHost, RenderOptions};
use vcsd_engine::Engine;

fn call(e: &Arc<Engine>, cmd: &str, a: Value) -> Value {
    e.call(cmd, a).unwrap_or_else(|err| panic!("{cmd}: {err}"))
}

#[test]
fn plugins_end_to_end() {
    let root = std::env::temp_dir().join(format!("vcsd_plug_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let plugdir = root.join("My Plugins");
    vcsd_testplug::make_bundle(&plugdir.join("VC Studio"));
    let proj = root.join("proj");
    vcsd_engine::demo::run(&proj).unwrap();

    let e = Engine::with_prefs(Some(root.join("cfg/prefs.json")));
    e.player.use_software_clock(); // real-time logic, not the machine's audio hardware (see devices.rs)
    *e.plugin_scanner.lock().unwrap() = vcsd_plugins::scan::Runner::InProcess;
    // ---- scan: an added folder is remembered and searched
    let l = call(&e, "plugins.folders", json!({"folders": [plugdir]}));
    let names: Vec<&str> =
        l["plugins"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).filter(|n| n.starts_with("VCSD")).collect();
    assert_eq!(names, ["VCSD Test Delay", "VCSD Test Gain"], "{l}");
    let delay_id =
        l["plugins"].as_array().unwrap().iter().find(|p| p["name"] == "VCSD Test Delay").unwrap()["id"].as_str().unwrap().to_string();
    assert!(std::fs::read_to_string(root.join("cfg/plugins.json")).unwrap().contains("VCSD Test Delay"), "scan cached");

    call(&e, "project.open", json!({"dir": proj}));
    let st = call(&e, "state.get", json!({}));
    let master = st["buses"].as_array().unwrap().iter().find(|b| b["parent_id"].is_null()).unwrap()["id"].as_str().unwrap().to_string();

    // ---- insert on the master bus; it loads and reports its latency
    let r = call(&e, "insert.add", json!({"owner_id": master, "plugin_id": delay_id}));
    let slot = r["id"].as_str().unwrap().to_string();
    assert_eq!(r["status"]["loaded"], true, "{r}");
    assert_eq!(r["status"]["latency"], 480);
    let st = call(&e, "state.get", json!({}));
    assert_eq!(st["plugins"][&slot]["loaded"], true);

    // ---- parameters: set, undo, redo
    let p = call(&e, "plugin.params", json!({"id": slot}));
    assert_eq!(p["params"][0]["title"], "Gain");
    assert_eq!(p["params"][0]["display"], "0.0");
    let r = call(&e, "plugin.param", json!({"id": slot, "param_id": 0, "value": 0.25}));
    assert_eq!(r["param"]["display"], "-6.0");
    call(&e, "undo", json!({}));
    assert_eq!(call(&e, "plugin.params", json!({"id": slot}))["params"][0]["display"], "0.0", "undo puts the plugin back");
    call(&e, "redo", json!({}));
    assert_eq!(call(&e, "plugin.params", json!({"id": slot}))["params"][0]["display"], "-6.0");

    // ---- playback runs through the plugin
    call(&e, "transport.play", json!({"from": 48_000 * 2}));
    std::thread::sleep(Duration::from_millis(500));
    let t = call(&e, "transport.state", json!({}));
    call(&e, "transport.stop", json!({}));
    assert!(t["master_peak"].as_f64().unwrap() > 0.001, "audible through the plugin: {t}");

    // ---- save captures the plugin's state; reopening restores it
    call(&e, "project.save", json!({}));
    let saved = vcsd_core::Session::open(&proj).unwrap().state;
    let ins = &saved.buses[&master].inserts;
    let pi = ins.iter().find_map(|i| i.as_plugin()).unwrap();
    assert!(pi.state.as_deref().is_some_and(|s| s.len() > 8), "state stored with the project");
    assert_eq!(pi.params.get(&0), Some(&0.25));

    let e2 = Engine::with_prefs(Some(root.join("cfg/prefs.json"))); // knows the plugin from the cache
    call(&e2, "project.open", json!({"dir": proj}));
    assert_eq!(call(&e2, "plugin.params", json!({"id": slot}))["params"][0]["display"], "-6.0", "restored on open");

    // ---- the render is the dry mix at -6 dB, sample-aligned despite 480 samples of latency
    let n = 48_000 * 4;
    let dry = Mixer::new(&saved, RenderOptions::default()).render_master(0, n).unwrap();
    let host: Arc<dyn PluginHost> = Arc::new(e2.rack.offline_copy(&saved));
    let wet = Mixer::new(&saved, RenderOptions::default()).with_plugins(Some(host)).render_master(0, n).unwrap();
    assert!(dry.peak() > 0.01);
    let max = dry.data.iter().zip(&wet.data).map(|(d, w)| (d * 0.5 - w).abs()).fold(0.0f32, f32::max);
    assert!(max < 1e-5, "real VST3 in the path, latency compensated (max diff {max})");

    // ---- bypass, reorder, remove
    call(
        &e,
        "insert.add",
        json!({"owner_id": master, "plugin_id": l["plugins"].as_array().unwrap().iter().find(|p| p["name"] == "VCSD Test Gain").unwrap()["id"], "index": 0}),
    );
    call(&e, "insert.bypass", json!({"owner_id": master, "id": slot, "bypass": true}));
    call(&e, "insert.move", json!({"owner_id": master, "id": slot, "to": 0}));
    let st = call(&e, "state.get", json!({}));
    let mi = &st["buses"].as_array().unwrap().iter().find(|b| b["id"] == master.as_str()).unwrap()["inserts"];
    assert_eq!(mi[0]["id"], slot.as_str());
    assert_eq!(mi[0]["bypass"], true);
    assert_eq!(mi[1]["name"], "VCSD Test Gain");
    call(&e, "insert.remove", json!({"owner_id": master, "id": slot}));
    assert!(call(&e, "state.get", json!({}))["plugins"].get(&slot).is_none(), "instance released");

    // ---- a plugin that isn't installed: kept, passed through, reported by QA
    let track = st["tracks"].as_array().unwrap()[0]["id"].as_str().unwrap().to_string();
    let missing = json!({"type": "plugin", "id": "ins_missing", "plugin_id": format!("vst3:{}", "0".repeat(32)), "format": "vst3",
        "name": "Some Reverb", "vendor": "Elsewhere", "path": "/nowhere/Some Reverb.vst3", "state": "AAEC"});
    call(&e, "track.update", json!({"id": track, "inserts": [missing]}));
    let st = call(&e, "state.get", json!({}));
    assert_eq!(st["plugins"]["ins_missing"]["loaded"], false);
    assert!(st["plugins"]["ins_missing"]["error"].as_str().unwrap().contains("not installed"));
    let qa = call(&e, "qa.run", json!({}));
    assert!(
        qa.as_array().unwrap().iter().any(|i| i["code"] == "plugin_missing" && i["message"].as_str().unwrap().contains("Some Reverb")),
        "{qa}"
    );
    call(&e, "project.save", json!({}));
    let kept = vcsd_core::Session::open(&proj).unwrap().state.tracks[&track].inserts[0].as_plugin().unwrap().clone();
    assert_eq!(kept.state.as_deref(), Some("AAEC"), "saved state of a missing plugin is never lost");

    // ---- export renders through (offline) plugins and reports the missing one
    let x = call(&e, "export.run", json!({"profile": "quick_review", "force": true}));
    assert!(x["files"].as_u64().unwrap() > 0);
    assert!(x["qa"].as_array().unwrap().iter().any(|i| i["code"] == "plugin_missing"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn plugin_parameter_automation() {
    let root = std::env::temp_dir().join(format!("vcsd_pauto_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    vcsd_testplug::make_bundle(&root.join("plugs"));
    let proj = root.join("proj");
    vcsd_engine::demo::run(&proj).unwrap();
    let e = Engine::with_prefs(Some(root.join("cfg/prefs.json")));
    e.player.use_software_clock(); // real-time logic, not the machine's audio hardware (see devices.rs)
    *e.plugin_scanner.lock().unwrap() = vcsd_plugins::scan::Runner::InProcess;
    let l = call(&e, "plugins.folders", json!({"folders": [root.join("plugs")]}));
    let gain = l["plugins"].as_array().unwrap().iter().find(|p| p["name"] == "VCSD Test Gain").unwrap()["id"].clone();
    call(&e, "project.open", json!({"dir": proj}));
    let st = call(&e, "state.get", json!({}));
    let master = st["buses"].as_array().unwrap().iter().find(|b| b["parent_id"].is_null()).unwrap()["id"].as_str().unwrap().to_string();
    let slot = call(&e, "insert.add", json!({"owner_id": master, "plugin_id": gain}))["id"].as_str().unwrap().to_string();

    // ---- a drawn lane: gain ×0 at 1 s rising to ×2 at 3 s
    let sr = 48_000i64;
    call(&e, "automation.set", json!({"owner_id": slot, "param": "plugin:0", "sample": sr, "value": 0.0}));
    call(&e, "automation.set", json!({"owner_id": slot, "param": "plugin:0", "sample": 3 * sr, "value": 1.0}));
    let st = call(&e, "state.get", json!({}));
    let lane: Vec<&Value> = st["automation"].as_array().unwrap().iter().filter(|p| p["owner_id"] == slot.as_str()).collect();
    assert_eq!(lane.len(), 2);
    assert_eq!(lane[0]["param"], "plugin:0");

    // the mix follows the lane, sample by sample, through the real plugin
    call(&e, "project.save", json!({}));
    let saved = vcsd_core::Session::open(&proj).unwrap().state;
    let n = (4 * sr) as usize;
    let dry = Mixer::new(&saved, RenderOptions::default()).render_master(0, n).unwrap();
    let host: Arc<dyn PluginHost> = Arc::new(e.rack.offline_copy(&saved));
    let m = Mixer::new(&saved, RenderOptions::default()).with_plugins(Some(host));
    let mut wet = vec![];
    for b in (0..n).step_by(1024) {
        wet.extend(m.render_master(b as i64, 1024.min(n - b)).unwrap().data);
    }
    let gain_at = |f: i64| 2.0 * ((f - sr) as f32 / (2 * sr) as f32).clamp(0.0, 1.0);
    let max = (0..n).map(|f| (dry.data[f * 2] * gain_at(f as i64) - wet[f * 2]).abs()).fold(0.0f32, f32::max);
    assert!(dry.peak() > 0.01);
    assert!(max < 1e-4, "automated gain, sample-accurate across render blocks (max diff {max})");

    // the panel shows the automated value while it plays
    call(&e, "transport.play", json!({"from": 3 * sr}));
    std::thread::sleep(Duration::from_millis(300));
    let p = call(&e, "plugin.params", json!({"id": slot}));
    assert_eq!(p["params"][0]["display"], "6.0", "{p}");
    call(&e, "transport.stop", json!({}));

    // ---- writing: Latch on the master, move the plugin control while playing
    call(&e, "automation.clear", json!({"owner_id": slot}));
    call(&e, "automation.mode", json!({"owner_id": master, "mode": "latch"}));
    let r = call(&e, "automation.touch", json!({"owner_id": slot, "param": "plugin:0", "value": 0.8, "phase": "begin"}));
    assert_eq!(r["capturing"], false, "nothing is written while stopped");
    call(&e, "transport.play", json!({"from": sr}));
    std::thread::sleep(Duration::from_millis(150));
    call(&e, "automation.touch", json!({"owner_id": slot, "param": "plugin:0", "value": 0.8, "phase": "begin"}));
    std::thread::sleep(Duration::from_millis(150));
    let r = call(&e, "automation.touch", json!({"owner_id": slot, "param": "plugin:0", "value": 0.2, "phase": "end"}));
    assert_eq!(r["capturing"], true);
    let t = call(&e, "transport.state", json!({}));
    assert!(t["writing"].as_array().unwrap().iter().any(|w| w["owner_id"] == slot.as_str() && w["param"] == "plugin:0"), "{t}");
    std::thread::sleep(Duration::from_millis(150));
    call(&e, "transport.stop", json!({}));
    let st = call(&e, "state.get", json!({}));
    assert_eq!(st["undo"], "write automation");
    let pts: Vec<f64> = st["automation"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["owner_id"] == slot.as_str())
        .map(|p| p["value"].as_f64().unwrap())
        .collect();
    assert!(
        pts.len() >= 2 && pts.iter().any(|v| (v - 0.8).abs() < 1e-6) && (pts.last().unwrap() - 0.2).abs() < 1e-6,
        "latched to the last value: {pts:?}"
    );

    // ---- a plugin removed while it is being written: the rest of the pass is kept
    call(&e, "transport.play", json!({"from": sr}));
    std::thread::sleep(Duration::from_millis(100));
    call(&e, "automation.touch", json!({"owner_id": slot, "param": "plugin:0", "value": 0.6, "phase": "begin"}));
    call(&e, "automation.touch", json!({"owner_id": master, "param": "volume_db", "value": -6.0, "phase": "begin"}));
    std::thread::sleep(Duration::from_millis(100));
    let before = call(&e, "state.get", json!({}))["automation"].as_array().unwrap().len();
    call(&e, "insert.remove", json!({"owner_id": master, "id": slot}));
    call(&e, "transport.stop", json!({}));
    let st2 = call(&e, "state.get", json!({}));
    assert_eq!(st2["undo"], "write automation", "the master's volume pass still commits");
    assert!(st2["automation"].as_array().unwrap().iter().any(|p| p["owner_id"] == master.as_str()));
    assert!(st2["automation"].as_array().unwrap().len() < before + 10);
    call(&e, "undo", json!({}));
    call(&e, "undo", json!({}));
    call(&e, "automation.clear", json!({"owner_id": master}));

    // ---- removing the plugin removes its lanes; undo brings both back
    call(&e, "insert.remove", json!({"owner_id": master, "id": slot}));
    let gone = call(&e, "state.get", json!({}))["automation"].as_array().unwrap().iter().filter(|p| p["owner_id"] == slot.as_str()).count();
    assert_eq!(gone, 0);
    call(&e, "undo", json!({}));
    let back = call(&e, "state.get", json!({}))["automation"].as_array().unwrap().iter().filter(|p| p["owner_id"] == slot.as_str()).count();
    assert_eq!(back, pts.len());
    let _ = std::fs::remove_dir_all(&root);
}

//! Device selection end to end. On Linux this test gives ALSA virtual `null`
//! devices (via a private HOME/.asoundrc), so cpal enumerates, opens, plays
//! and records through real ALSA streams without sound hardware.
use serde_json::{json, Value};
use std::time::Duration;
use vcsd_engine::Engine;

fn call(e: &std::sync::Arc<Engine>, cmd: &str, a: Value) -> Value {
    e.call(cmd, a).unwrap_or_else(|err| panic!("{cmd}: {err}"))
}

#[test]
fn select_output_input_and_midi_ports() {
    let tmp = std::env::temp_dir().join(format!("vcsd_dev_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    if cfg!(target_os = "linux") {
        std::fs::write(
            tmp.join(".asoundrc"),
            "pcm.!default { type null }\npcm.studio_a { type null\n hint { show on description \"Studio Interface A\" } }\npcm.studio_b { type null\n hint { show on description \"Studio Interface B\" } }\n",
        )
        .unwrap();
        std::env::set_var("HOME", &tmp); // before any ALSA call in this process
    }
    let prefs = tmp.join("cfg/prefs.json");
    let e = Engine::with_prefs(Some(prefs.clone()));
    let d = call(&e, "devices.list", json!({}));
    let outs: Vec<String> = d["outputs"].as_array().unwrap().iter().map(|o| o["name"].as_str().unwrap().to_string()).collect();
    if !outs.iter().any(|n| n == "Studio Interface A") {
        eprintln!("no virtual ALSA devices on this platform/host; skipping ({outs:?})");
        return;
    }

    // a small project so the transport has something to play and record onto
    let proj = tmp.join("proj");
    call(&e, "project.new", json!({"dir": proj, "name": "dev test", "fps": "24", "start_tc": "01:00:00:00"}));
    call(&e, "scene.add", json!({"number": "1", "title": "t", "start": 0, "end": 48000 * 20}));
    call(&e, "marker.add", json!({"lane": "props", "kind": "range", "start": 48000, "end": 96000, "label": "door"}));
    let cue = call(&e, "cues.build", json!({}))["created"][0].as_str().unwrap().to_string();

    // output: switch to a named device, live
    let r = call(&e, "prefs.set", json!({"output_device": "Studio Interface A"}));
    assert!(r["active"]["output"].as_str().unwrap().starts_with("Studio Interface A"), "{}", r["active"]);
    assert!(r["active"]["output_warning"].is_null());
    let saved: Value = serde_json::from_slice(&std::fs::read(&prefs).unwrap()).unwrap();
    assert_eq!(saved["output_device"], "Studio Interface A");
    call(&e, "transport.play", json!({"from": 0}));
    std::thread::sleep(Duration::from_millis(600));
    let pos = call(&e, "transport.state", json!({}))["position"].as_i64().unwrap();
    call(&e, "transport.stop", json!({}));
    assert!(pos > 48000 / 5, "transport advanced on the selected ALSA device: {pos}");

    // an unplugged device falls back to the default and says so
    let r = call(&e, "prefs.set", json!({"output_device": "Unplugged Box"}));
    assert!(r["active"]["output_warning"].as_str().unwrap().contains("not connected"), "{}", r["active"]);
    let r = call(&e, "prefs.set", json!({"output_device": null}));
    assert!(r["prefs"]["output_device"].is_null());

    // input: validated, then used for recording
    assert!(e.call("prefs.set", json!({"input_device": "Nope"})).unwrap_err().contains("not connected"));
    call(&e, "prefs.set", json!({"input_device": "Studio Interface B"}));
    let rec = call(&e, "record.start", json!({"cue_id": cue, "from": 48000}));
    assert_eq!(rec["device"], "Studio Interface B");
    std::thread::sleep(Duration::from_millis(500));
    let take = call(&e, "record.stop", json!({}));
    let wav = std::path::PathBuf::from(take["path"].as_str().unwrap());
    let info = vcsd_core::wav::read_info(&wav).unwrap();
    assert!(info.frames > 4800, "captured audio: {} frames", info.frames);
    assert!(info.bext.unwrap().time_reference >= 3600 * 48000, "BWF stamped at timeline position");

    // MIDI: an absent controller is reported but remembered for next launch
    let r = call(&e, "prefs.set", json!({"midi_port": "Pad Controller"}));
    assert!(r["error"].is_string());
    assert_eq!(r["prefs"]["midi_port"], "Pad Controller");
    assert!(r["active"]["midi"].is_null());
    call(&e, "prefs.set", json!({"midi_port": null}));

    // test tone plays on the current output
    call(&e, "devices.test_tone", json!({}));
    assert!(call(&e, "transport.state", json!({}))["previewing"].as_bool().unwrap());

    // preferences are restored on the next launch
    call(&e, "prefs.set", json!({"output_device": "Studio Interface B"}));
    drop(e);
    let e2 = Engine::with_prefs(Some(prefs));
    std::thread::sleep(Duration::from_millis(300));
    let d = call(&e2, "devices.list", json!({}));
    assert_eq!(d["prefs"]["input_device"], "Studio Interface B");
    assert_eq!(d["prefs"]["output_device"], "Studio Interface B");
}

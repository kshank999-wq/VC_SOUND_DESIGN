use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use vcsd_engine::Engine;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vcsd_eng_{name}_{}_{}", std::process::id(), vcsd_core::ids::new_id("t")));
    d
}

/// One demo project shared by the tests in this binary (read-only use).
fn demo_dir() -> std::path::PathBuf {
    static D: OnceLock<std::path::PathBuf> = OnceLock::new();
    D.get_or_init(|| {
        let d = tmp("demo");
        vcsd_engine::demo::run(&d).unwrap();
        d
    })
    .clone()
}

fn copy_demo() -> std::path::PathBuf {
    let src = demo_dir();
    let dst = tmp("copy");
    std::fs::create_dir_all(&dst).unwrap();
    // project db only; media paths are absolute and stay valid
    std::fs::copy(src.join("Project.vcsd"), dst.join("Project.vcsd")).unwrap();
    for d in vcsd_core::layout::DIRS {
        std::fs::create_dir_all(dst.join(d)).unwrap();
    }
    dst
}

fn call(e: &Arc<Engine>, cmd: &str, a: Value) -> Value {
    e.call(cmd, a).unwrap_or_else(|err| panic!("{cmd}: {err}"))
}

#[test]
fn transport_runs_in_realtime_and_spotting_uses_heard_position() {
    let e = Engine::with_prefs(None);
    call(&e, "project.open", json!({"dir": copy_demo()}));
    let st = call(&e, "state.get", json!({}));
    assert!(st["cues"].as_array().unwrap().len() >= 9);
    let sr = 48_000i64;
    call(&e, "transport.play", json!({"from": 0}));
    std::thread::sleep(Duration::from_millis(700));
    let t = call(&e, "transport.state", json!({}));
    let pos = t["position"].as_i64().unwrap();
    assert!(t["playing"].as_bool().unwrap());
    assert!(pos > sr * 3 / 10 && pos < sr * 12 / 10, "~0.7 s of audio consumed, got {pos} ({})", t["device"]);
    assert!(t["master_peak"].as_f64().unwrap() > 0.0, "birds ambience is audible at the start");
    // live spot a glass hit while playing → lands at the heard position
    let before = st["markers"].as_array().unwrap().len();
    call(&e, "spot.tap", json!({"key": "key:KeyG"}));
    let st2 = call(&e, "state.get", json!({}));
    let m = st2["markers"].as_array().unwrap();
    assert_eq!(m.len(), before + 1);
    let newest = m.iter().max_by_key(|x| x["id"].as_str().unwrap().to_string()).unwrap();
    let mpos = newest["start_sample"].as_i64().unwrap();
    assert!((mpos - pos).abs() < sr / 2, "tap at heard position: marker {mpos} vs transport {pos}");
    call(&e, "transport.stop", json!({}));
    let p1 = call(&e, "transport.state", json!({}))["position"].as_i64().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let p2 = call(&e, "transport.state", json!({}))["position"].as_i64().unwrap();
    assert_eq!(p1, p2, "stopped transport does not move");
    // frame step lands on frame boundaries (24 fps @ 48k = 2000 samples)
    let r = call(&e, "transport.step", json!({"frames": 3}));
    assert_eq!(r["position"].as_i64().unwrap() % 2000, 0);
    // undo removes the spotted marker
    call(&e, "undo", json!({}));
    assert_eq!(call(&e, "state.get", json!({}))["markers"].as_array().unwrap().len(), before);
}

#[test]
fn perform_record_and_errors() {
    let e = Engine::with_prefs(None);
    call(&e, "project.open", json!({"dir": copy_demo()}));
    let st = call(&e, "state.get", json!({}));
    let cue =
        st["cues"].as_array().unwrap().iter().find(|c| c["description"] == "car pass by").unwrap()["id"].as_str().unwrap().to_string();
    let greg =
        st["identities"].as_array().unwrap().iter().find(|i| i["name"] == "Greg / Boots").unwrap()["id"].as_str().unwrap().to_string();
    call(&e, "perform.start", json!({"cue_id": cue, "pads": [{"key": "key:KeyJ", "identity_id": greg}]}));
    for (k, at) in [(0, 48_000 * 46), (1, 48_000 * 46 + 20_000), (2, 48_000 * 46 + 41_000)] {
        let _ = k;
        call(&e, "perform.trigger", json!({"pad": "key:KeyJ", "at": at, "velocity": 100}));
    }
    // MIDI injected while in perform mode goes to the recorder (unmapped note ignored)
    call(&e, "midi.message", json!({"bytes": [0x99, 36, 100]}));
    let r = call(&e, "perform.commit", json!({}));
    assert!(r["id"].as_str().unwrap().starts_with("prf_"));
    let st = call(&e, "state.get", json!({}));
    let clips: Vec<&Value> =
        st["clips"].as_array().unwrap().iter().filter(|c| c["cue_id"] == cue.as_str() && c["muted"] == false).collect();
    assert_eq!(clips.len(), 3);
    // Without an input device recording fails cleanly. Some CI machines have a virtual one (macOS):
    // there it starts and stops cleanly, with a take or "no audio captured" if the device is silent.
    match e.call("record.start", json!({"cue_id": cue})) {
        Err(err) => assert!(err.contains("input") || err.contains("device"), "{err}"),
        Ok(r) => {
            assert!(r["device"].is_string(), "{r}");
            assert!(e.call("record.start", json!({"cue_id": cue})).unwrap_err().contains("already recording"));
            std::thread::sleep(Duration::from_millis(200));
            if let Err(err) = e.call("record.stop", json!({})) {
                assert!(err.contains("no audio captured"), "{err}");
            }
            assert!(e.call("record.stop", json!({})).unwrap_err().contains("not recording"));
        }
    }
    assert!(!call(&e, "transport.state", json!({}))["playing"].as_bool().unwrap());
    assert!(e.call("nope", json!({})).unwrap_err().contains("unknown command"));
    // audition a library sound on top of a stopped transport
    let asset = st["clips"].as_array().unwrap()[0]["asset_id"].as_str().unwrap().to_string();
    call(&e, "audition", json!({"asset_id": asset}));
    assert!(call(&e, "transport.state", json!({}))["previewing"].as_bool().unwrap());
    let pk = call(&e, "asset.peaks", json!({"id": asset}));
    assert!(pk["data"].as_array().unwrap().len() > 10);
}

fn http(port: u16, req: &str) -> (String, Vec<u8>) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut buf = vec![];
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (String::from_utf8_lossy(&buf[..split]).to_string(), buf[split + 4..].to_vec())
}

#[test]
fn browser_mode_server_api_and_media_ranges() {
    let e = Engine::with_prefs(None);
    call(&e, "project.open", json!({"dir": copy_demo()}));
    let pic = call(&e, "state.get", json!({}))["picture"]["path"].as_str().unwrap().to_string();
    let ui = tmp("ui");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(ui.join("index.html"), "<html>vcsd</html>").unwrap();
    let port = 18_000 + (std::process::id() % 2000) as u16;
    let e2 = e.clone();
    std::thread::spawn(move || vcsd_engine::serve::serve(e2, port, ui));
    std::thread::sleep(Duration::from_millis(300));
    let body = r#"{}"#;
    let (h, b) = http(
        port,
        &format!("POST /api/state.get HTTP/1.0\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()),
    );
    assert!(h.starts_with("HTTP/1.0 200"));
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["ok"]["project"]["name"], "Gravel Road");
    let (_, b) = http(port, "POST /api/bogus HTTP/1.0\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    assert!(serde_json::from_slice::<Value>(&b).unwrap()["error"].is_string());
    // the picture is a real video with ffmpeg, else a short placeholder file: ranges inside it
    let whole = std::fs::read(&pic).unwrap();
    let range = |r: &str| {
        http(
            port,
            &format!("GET /media?path={} HTTP/1.0\r\nHost: x\r\nRange: bytes={r}\r\nConnection: close\r\n\r\n", pic.replace(' ', "%20")),
        )
    };
    let (h, b) = range("10-29");
    assert!(h.starts_with("HTTP/1.0 206") && h.contains(&format!("bytes 10-29/{}", whole.len())), "{h}");
    assert_eq!(b, whole[10..30]);
    let (h, _) = range(&format!("{}-", whole.len()));
    assert!(h.starts_with("HTTP/1.0 416") && h.contains(&format!("bytes */{}", whole.len())), "a range past the end: {h}");
    let (h, _) = http(port, "GET /media?path=/etc/passwd HTTP/1.0\r\nHost: x\r\nConnection: close\r\n\r\n");
    assert!(h.starts_with("HTTP/1.0 403"), "only project media is served: {h}");
    let (h, b) = http(port, "GET /anything HTTP/1.0\r\nHost: x\r\nConnection: close\r\n\r\n");
    assert!(h.starts_with("HTTP/1.0 200") && b == b"<html>vcsd</html>");
}

//! The VST3 host against a real (tiny) plugin: `vcsd-testplug`.
use vcsd_plugins::vst3host::{scan_factory, Vst3Instance};
use vcsd_plugins::{scan, Instance, PluginInfo, Setup};
use vst3::ComPtr;

fn infos() -> Vec<PluginInfo> {
    let f = unsafe { ComPtr::from_raw(vcsd_testplug::GetPluginFactory()) }.unwrap();
    scan_factory(&f, "(in process)").unwrap()
}

fn make(name: &str, setup: Setup) -> Vst3Instance {
    let info = infos().into_iter().find(|i| i.name == name).unwrap();
    unsafe { Vst3Instance::from_factory(&info, setup, vcsd_testplug::GetPluginFactory()) }.unwrap()
}

fn tone(frames: usize, ch: usize) -> Vec<f32> {
    (0..frames).flat_map(|f| (0..ch).map(move |c| ((f as f32 * 0.05).sin() * 0.5) * if c == 0 { 1.0 } else { -1.0 })).collect()
}

#[test]
fn scans_effect_classes_only() {
    let v = infos();
    let names: Vec<&str> = v.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["VCSD Test Gain", "VCSD Test Delay"], "the controller class is not listed");
    assert!(v.iter().all(|i| i.id.starts_with("vst3:") && i.id.len() == 37 && i.is_effect()));
    assert_eq!(v[1].category, "Fx|Delay");
    assert_eq!(v[0].vendor, "VC Studio (test)");
    assert_eq!(v[0].version, "1.0.0");
}

#[test]
fn processes_with_parameters_and_restores_state() {
    let g = make("VCSD Test Gain", Setup::default());
    assert_eq!(g.latency(), 0);
    let input = tone(3000, 2); // > max_block: split into several process calls
    let mut buf = input.clone();
    g.process(&mut buf, 2, 0);
    assert_eq!(buf, input, "unity at the default value");

    let p = g.params();
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].title.as_str(), p[0].units.as_str(), p[0].display.as_str()), ("Gain", "dB", "0.0"));
    g.set_param(0, 0.25).unwrap(); // ×0.5
    assert_eq!(g.params()[0].display, "-6.0");
    let mut buf = input.clone();
    g.process(&mut buf, 2, 3000);
    assert!(buf.iter().zip(&input).all(|(o, i)| (o - i * 0.5).abs() < 1e-6), "parameter reached the processor");

    // a change made while nothing plays still lands in the saved state (flushed)
    g.set_param(0, 0.3).unwrap();
    g.set_param(0, 0.25).unwrap();
    // state: a fresh instance picks up the saved gain, in processor and controller
    let st = g.get_state().unwrap();
    assert_eq!(f64::from_le_bytes(st[8..16].try_into().unwrap()), 0.25, "processor state is current");
    let h = make("VCSD Test Gain", Setup::default());
    h.set_state(&st).unwrap();
    assert!((h.params()[0].value - 0.25).abs() < 1e-9);
    let mut buf = input.clone();
    h.process(&mut buf, 2, 0);
    assert!(buf.iter().zip(&input).all(|(o, i)| (o - i * 0.5).abs() < 1e-6));
    assert!(h.set_state(b"nonsense").is_err());
}

#[test]
fn reports_latency_and_resets_on_a_jump() {
    let d = make("VCSD Test Delay", Setup { max_block: 256, ..Default::default() });
    assert_eq!(d.latency(), vcsd_testplug::DELAY_SAMPLES as u32);
    let n = 2048;
    let mut buf = vec![0.0f32; n * 2];
    buf[100 * 2] = 1.0;
    buf[100 * 2 + 1] = 1.0;
    d.process(&mut buf, 2, 0);
    let at = buf.iter().position(|v| *v != 0.0).unwrap() / 2;
    assert_eq!(at, 100 + vcsd_testplug::DELAY_SAMPLES, "delayed by exactly the reported latency");

    // an impulse right at the end is still in the delay line…
    let mut buf = vec![0.0f32; n * 2];
    buf[(n - 10) * 2] = 1.0;
    d.process(&mut buf, 2, n as i64);
    // …continuing normally it would come out; after a locate it must not
    let mut jumped = vec![0.0f32; n * 2];
    d.process(&mut jumped, 2, 999_999);
    assert!(jumped.iter().all(|v| *v == 0.0), "a jump in position resets the plugin");
}

#[test]
fn mono_slots_use_a_mono_layout() {
    let g = make("VCSD Test Gain", Setup { channels: 1, ..Default::default() });
    g.set_param(0, 1.0).unwrap(); // ×2
    let input = tone(500, 1);
    let mut buf = input.clone();
    g.process(&mut buf, 1, 0);
    assert!(buf.iter().zip(&input).all(|(o, i)| (o - i * 2.0).abs() < 1e-6));
}

#[test]
fn loads_a_real_bundle_and_reports_broken_ones() {
    let dir = std::env::temp_dir().join(format!("vcsd-vst3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    vcsd_testplug::make_bundle(&dir.join("Vendor"));
    std::fs::write(dir.join("Broken.vst3"), b"not a library").unwrap();

    let mut cache = scan::Cache::default();
    let rep = scan::scan(std::slice::from_ref(&dir), &mut cache, &scan::Runner::InProcess, false);
    assert_eq!(rep.bundles, 2, "nested vendor folders are searched");
    assert_eq!(rep.plugins.len(), 2);
    assert_eq!(rep.failed.len(), 1);
    assert!(rep.failed[0].path.ends_with("Broken.vst3"));
    // unchanged bundles come from the cache
    let again = scan::scan(std::slice::from_ref(&dir), &mut cache, &scan::Runner::InProcess, false);
    assert_eq!((again.scanned, again.plugins.len()), (0, 2));

    let delay = rep.plugins.iter().find(|p| p.name == "VCSD Test Delay").unwrap();
    let inst = vcsd_plugins::instantiate(delay, Setup::default()).unwrap();
    assert_eq!(inst.latency(), 480);
    let _ = std::fs::remove_dir_all(&dir);
}

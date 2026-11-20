//! Print the audio devices and MIDI ports the engine can see.
//! `cargo run -p vcsd-engine --example devices`
fn main() {
    let e = vcsd_engine::Engine::with_prefs(None);
    std::thread::sleep(std::time::Duration::from_millis(300));
    println!("{}", serde_json::to_string_pretty(&e.devices_json()).unwrap());
}

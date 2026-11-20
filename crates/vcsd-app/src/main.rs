//! VC Sound Design desktop app: a Tauri shell around `vcsd-engine`.
//!
//! The web UI calls one IPC command, `api(cmd, args)`, which runs the engine
//! command on a worker thread (never on the UI thread). Realtime audio,
//! recording and MIDI live in the engine, not in the webview.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use vcsd_engine::Engine;

#[tauri::command]
async fn api(engine: tauri::State<'_, Arc<Engine>>, cmd: String, args: serde_json::Value) -> Result<serde_json::Value, String> {
    let e = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || e.call(&cmd, args)).await.map_err(|x| x.to_string())?
}

fn main() {
    let engine = Engine::new();
    // Loopback media server (token-protected, Range-capable) for the picture viewer.
    if let Err(err) = vcsd_engine::serve::start_media_server(engine.clone()) {
        eprintln!("media server unavailable: {err}");
    }
    // `vc-sound-design /path/to/project` opens a project directly.
    if let Some(dir) = std::env::args().nth(1).filter(|a| !a.starts_with('-')) {
        if let Err(err) = engine.call("project.open", serde_json::json!({ "dir": dir })) {
            eprintln!("could not open {dir}: {err}");
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // remembers size, position and maximised state of every window (main + mixer)
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(engine)
        .invoke_handler(tauri::generate_handler![api])
        .run(tauri::generate_context!())
        .expect("error while running VC Sound Design");
}

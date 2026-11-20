//! VC Sound Design desktop app: a Tauri shell around `vcsd-engine`.
//!
//! The web UI calls one IPC command, `api(cmd, args)`, which runs the engine
//! command on a worker thread (never on the UI thread). Realtime audio,
//! recording and MIDI live in the engine, not in the webview. Plugins' own
//! editors open in native windows ([`editors`]).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod editors;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Whether the updater plugin actually initialised in this run.
struct Updates(AtomicBool);
use tauri::Manager;
use vcsd_engine::Engine;

#[tauri::command]
async fn api(engine: tauri::State<'_, Arc<Engine>>, cmd: String, args: serde_json::Value) -> Result<serde_json::Value, String> {
    let e = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || e.call(&cmd, args)).await.map_err(|x| x.to_string())?
}

/// What this build can do, for the UI: version, and whether it was built with
/// an update signing key (release builds) so in-app updates are available.
#[tauri::command]
fn build_info(app: tauri::AppHandle, updates: tauri::State<'_, Updates>) -> serde_json::Value {
    serde_json::json!({
        "version": app.package_info().version.to_string(),
        "updater": updates.0.load(Ordering::Relaxed),
        "identifier": app.config().identifier,
    })
}

fn main() {
    // started as the out-of-process plugin scanner: scan one bundle and exit
    vcsd_engine::vcsd_plugins::scan::child_main();
    editors::prefer_x11();
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
        .plugin(tauri_plugin_window_state::Builder::default().with_filter(|label| !label.starts_with("fx-")).build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Auto-update only in builds configured with an update public key
            // (release builds via scripts/release-config.mjs). Dev builds stay
            // fully functional without one.
            // A broken update configuration must never stop the app from opening a
            // project: log it and run without updates.
            if app.config().plugins.0.contains_key("updater") {
                match app.handle().plugin(tauri_plugin_updater::Builder::new().build()) {
                    Ok(()) => app.state::<Updates>().0.store(true, Ordering::Relaxed),
                    Err(e) => eprintln!("updates disabled: {e}"),
                }
            }
            Ok(())
        })
        .manage(engine)
        .manage(Updates(AtomicBool::new(false)))
        .invoke_handler(tauri::generate_handler![
            api,
            build_info,
            editors::plugin_editor_open,
            editors::plugin_editor_close,
            editors::plugin_editors_open
        ])
        .run(tauri::generate_context!())
        .expect("error while running VC Sound Design");
}

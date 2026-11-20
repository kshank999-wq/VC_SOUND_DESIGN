//! VC Sound Design application services.
//!
//! [`Engine`] owns the open project, the realtime [`player::Player`], live
//! spotting / performance capture, recording and MIDI input, and exposes all
//! of it through one JSON command API: `engine.call("cue.replace", {...})`.
//! The desktop shell (Tauri) and the browser-mode server ([`serve`]) are thin
//! transports over that single entry point, so both behave identically.

pub mod demo;
pub mod devices;
pub mod player;
pub mod recorder;
pub mod serve;

use player::Player;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use vcsd_core::builder::{populate, PopulateOptions, TrackPolicy};
use vcsd_core::editing::{self, ReplaceScope, TakeImport};
use vcsd_core::export::{ExportOptions, ExportProfile, ExportRange};
use vcsd_core::library::{self, AssetMeta, Query};
use vcsd_core::perform::{Pad, PadMap, PerformanceRecorder};
use vcsd_core::picture::{import_picture, PictureImport};
use vcsd_core::spotting::{self, LiveSpotter, NewIdentity, NewMarker, SpotBinding};
use vcsd_core::*;

pub type ApiResult = std::result::Result<Value, String>;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    Spot,
    Perform,
}

struct Inner {
    session: Option<Session>,
    version: u64,
    spotter: LiveSpotter,
    perf: Option<PerformanceRecorder>,
    mode: InputMode,
    last_autosave: Instant,
    peaks: HashMap<String, Arc<Value>>,
    log: Vec<String>,
}

pub struct Engine {
    inner: Mutex<Inner>,
    pub player: Player,
    recording: Mutex<Option<recorder::Recording>>,
    midi: Mutex<Option<midir::MidiInputConnection<()>>>,
    /// Set by the desktop shell when it runs the loopback media server.
    pub media_base: Mutex<Option<String>>,
    prefs: Mutex<devices::Prefs>,
    prefs_path: Option<PathBuf>,
    midi_port: Mutex<Option<String>>,
    midi_error: Mutex<Option<String>>,
    midi_last: Mutex<Option<(Instant, Vec<u8>)>>,
}

// ---------------------------------------------------------------- arg helpers

fn s<'a>(a: &'a Value, k: &str) -> std::result::Result<&'a str, String> {
    a.get(k).and_then(|v| v.as_str()).ok_or_else(|| format!("missing string argument '{k}'"))
}
fn os<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(|v| v.as_str()).filter(|v| !v.is_empty())
}
fn i(a: &Value, k: &str) -> std::result::Result<i64, String> {
    a.get(k)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)))
        .ok_or_else(|| format!("missing integer argument '{k}'"))
}
fn oi(a: &Value, k: &str) -> Option<i64> {
    a.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)))
}
fn of(a: &Value, k: &str) -> Option<f32> {
    a.get(k).and_then(|v| v.as_f64()).map(|f| f as f32)
}
fn ob(a: &Value, k: &str) -> bool {
    a.get(k).and_then(|v| v.as_bool()).unwrap_or(false)
}
fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}
fn kind(k: &str) -> std::result::Result<EventType, String> {
    match k {
        "point" => Ok(EventType::Point),
        "range" => Ok(EventType::Range),
        "repeated" => Ok(EventType::Repeated),
        _ => Err(format!("unknown event kind '{k}'")),
    }
}
fn mode(m: &str) -> VariationMode {
    match m {
        "random" => VariationMode::Random,
        "locked" => VariationMode::Locked,
        "alternate_feet" | "alternate" => VariationMode::AlternateFeet,
        _ => VariationMode::RoundRobin,
    }
}
/// Automation owner: `owner_id`, or `track_id` / `bus_id` for clarity.
fn auto_owner(a: &Value) -> std::result::Result<&str, String> {
    os(a, "owner_id").or_else(|| os(a, "bus_id")).or_else(|| os(a, "track_id")).ok_or_else(|| "missing owner_id (track or bus)".to_string())
}
fn auto_param(a: &Value) -> AutomationParam {
    if os(a, "param") == Some("pan") {
        AutomationParam::Pan
    } else {
        AutomationParam::VolumeDb
    }
}
fn today() -> String {
    ids::date_from_unix(ids::now_unix())
}

fn default_bindings(st: &ProjectState) -> LiveSpotter {
    let mut sp = LiveSpotter::new();
    let lane = |c: &str| st.find_lane(c).map(|l| l.id.clone()).ok();
    for (key, cat, kind, label) in [
        ("key:KeyF", "footsteps", EventType::Repeated, "footsteps"),
        ("key:KeyP", "props", EventType::Point, "prop"),
        ("key:KeyG", "fx", EventType::Point, "hit"),
        ("key:KeyB", "bg", EventType::Range, "ambience"),
        ("key:KeyD", "design", EventType::Range, "design"),
        ("key:KeyN", "dialogue", EventType::Point, "note"),
        ("key:KeyM", "music", EventType::Range, "music"),
        ("midi:10:36", "footsteps", EventType::Repeated, "footsteps"),
        ("midi:10:38", "props", EventType::Point, "prop"),
        ("midi:10:42", "fx", EventType::Point, "hit"),
    ] {
        if let Some(l) = lane(cat) {
            sp.bind(key, SpotBinding { lane_id: l, kind, label: label.into(), identity_id: None });
        }
    }
    sp
}

fn bindings_file(root: &Path) -> PathBuf {
    root.join("Spotting.json")
}

impl Engine {
    /// Engine using the per-user preferences file (devices, MIDI port).
    pub fn new() -> Arc<Engine> {
        Self::with_prefs(Some(devices::default_prefs_path()))
    }

    /// Engine with an explicit preferences file (`None` = defaults, never saved).
    pub fn with_prefs(prefs_path: Option<PathBuf>) -> Arc<Engine> {
        let prefs = prefs_path.as_deref().map(devices::Prefs::load).unwrap_or_default();
        let player = Player::start();
        player.set_output_device(prefs.output_device.clone());
        let midi_pref = prefs.midi_port.clone();
        let e = Arc::new(Engine {
            inner: Mutex::new(Inner {
                session: None,
                version: 0,
                spotter: LiveSpotter::new(),
                perf: None,
                mode: InputMode::Spot,
                last_autosave: Instant::now(),
                peaks: HashMap::new(),
                log: vec![],
            }),
            player,
            recording: Mutex::new(None),
            midi: Mutex::new(None),
            media_base: Mutex::new(None),
            prefs: Mutex::new(prefs),
            prefs_path,
            midi_port: Mutex::new(None),
            midi_error: Mutex::new(None),
            midi_last: Mutex::new(None),
        });
        if let Some(port) = midi_pref {
            // a missing controller at startup is reported, not fatal
            let _ = e.connect_midi(Some(&port));
        }
        e
    }

    /// Connect a MIDI input (first available when `port` is None). Must be
    /// called without holding the inner lock: replacing a connection joins its
    /// callback thread, which may be waiting on that lock.
    fn connect_midi(self: &Arc<Self>, port: Option<&str>) -> std::result::Result<String, String> {
        *self.midi.lock().unwrap() = None;
        *self.midi_port.lock().unwrap() = None;
        let r = (|| {
            let input = midir::MidiInput::new("VC Sound Design").map_err(e)?;
            let ports = input.ports();
            let p = ports
                .iter()
                .find(|p| port.is_none_or(|w| input.port_name(p).map(|n| n == w).unwrap_or(false)))
                .ok_or_else(|| match port {
                    Some(w) => format!("MIDI input '{w}' is not connected"),
                    None => "no MIDI inputs found".to_string(),
                })?
                .clone();
            let name = input.port_name(&p).unwrap_or_default();
            let me = self.clone();
            let conn = input.connect(&p, "vcsd-in", move |_, msg, _| me.handle_midi(msg), ()).map_err(e)?;
            Ok::<_, String>((name, conn))
        })();
        match r {
            Ok((name, conn)) => {
                *self.midi.lock().unwrap() = Some(conn);
                *self.midi_port.lock().unwrap() = Some(name.clone());
                *self.midi_error.lock().unwrap() = None;
                Ok(name)
            }
            Err(err) => {
                *self.midi_error.lock().unwrap() = Some(err.clone());
                Err(err)
            }
        }
    }

    fn disconnect_midi(&self) {
        *self.midi.lock().unwrap() = None;
        *self.midi_port.lock().unwrap() = None;
        *self.midi_error.lock().unwrap() = None;
    }

    fn save_prefs(&self) -> std::result::Result<(), String> {
        match &self.prefs_path {
            Some(p) => self.prefs.lock().unwrap().save(p),
            None => Ok(()),
        }
    }

    /// Devices, ports, saved choices and what is actually active.
    pub fn devices_json(&self) -> Value {
        let mut v = self.device_status_json();
        v["outputs"] = json!(devices::outputs());
        v["inputs"] = json!(devices::inputs());
        v["midi_inputs"] = json!(devices::midi_ports());
        v
    }

    /// Cheap status (no enumeration) for polling: saved prefs + active devices.
    pub fn device_status_json(&self) -> Value {
        let ts = self.player.state();
        json!({
            "prefs": *self.prefs.lock().unwrap(),
            "active": {
                "output": ts.device,
                "output_rate": ts.device_rate,
                "output_warning": ts.device_warning,
                "output_latency": ts.output_latency,
                "midi": *self.midi_port.lock().unwrap(),
                "midi_error": *self.midi_error.lock().unwrap(),
                "midi_last": self.midi_last.lock().unwrap().as_ref().map(|(t, b)| json!({"ms_ago": t.elapsed().as_millis() as u64, "bytes": b})),
            },
            "prefs_path": self.prefs_path.as_ref().map(|p| p.display().to_string()),
        })
    }

    /// Single entry point used by every front end.
    pub fn call(self: &Arc<Self>, cmd: &str, args: Value) -> ApiResult {
        let a = if args.is_null() { json!({}) } else { args };
        match self.dispatch(cmd, &a) {
            Ok(v) => Ok(v),
            Err(err) => {
                if let Ok(mut g) = self.inner.lock() {
                    g.log.push(format!("{cmd}: {err}"));
                }
                Err(err)
            }
        }
    }

    fn publish(&self, g: &mut Inner) {
        g.version += 1;
        let snap = g.session.as_ref().map(|s| Arc::new(s.state.clone()));
        self.player.set_project(snap, g.version);
        if let Some(sess) = &g.session {
            if g.last_autosave.elapsed().as_secs() >= 15 {
                let _ = sess.autosave();
                g.last_autosave = Instant::now();
            }
        }
    }

    fn open_session(&self, g: &mut Inner, sess: Session) {
        let root = sess.root.clone();
        g.spotter = root
            .as_ref()
            .and_then(|r| std::fs::read(bindings_file(r)).ok())
            .and_then(|b| serde_json::from_slice::<std::collections::BTreeMap<String, SpotBinding>>(&b).ok())
            .map(|b| {
                let mut sp = LiveSpotter::new();
                sp.bindings = b;
                sp
            })
            .unwrap_or_else(|| default_bindings(&sess.state));
        g.session = Some(sess);
        g.perf = None;
        g.peaks.clear();
        self.player.stop();
        self.player.clear_cache();
        self.player.locate(0);
        self.publish(g);
    }

    /// Run a mutating edit on the session (undoable) and publish the result.
    fn edit<T>(
        &self,
        g: &mut Inner,
        label: &str,
        f: impl FnOnce(&mut ProjectState) -> vcsd_core::Result<T>,
    ) -> std::result::Result<T, String> {
        let sess = g.session.as_mut().ok_or("no project open")?;
        let v = sess.edit(label, f).map_err(e)?;
        self.publish(g);
        Ok(v)
    }

    fn st(g: &Inner) -> std::result::Result<&ProjectState, String> {
        g.session.as_ref().map(|s| &s.state).ok_or_else(|| "no project open".to_string())
    }

    fn heard(&self) -> i64 {
        self.player.position()
    }

    /// MIDI input callback target.
    pub fn handle_midi(self: &Arc<Self>, msg: &[u8]) {
        *self.midi_last.lock().unwrap() = Some((Instant::now(), msg.to_vec()));
        let pos = self.heard();
        let mut g = self.inner.lock().unwrap();
        let changed = match g.mode {
            InputMode::Perform => g.perf.as_mut().map(|p| p.midi(msg, pos).unwrap_or(false)).unwrap_or(false),
            InputMode::Spot => {
                let Inner { session, spotter, .. } = &mut *g;
                match session.as_mut() {
                    Some(sess) => sess.edit("midi spot", |st| spotter.midi(st, msg, pos)).map(|o| o.is_some()).unwrap_or(false),
                    None => false,
                }
            }
        };
        if changed {
            self.publish(&mut g);
        }
    }

    fn dispatch(self: &Arc<Self>, cmd: &str, a: &Value) -> ApiResult {
        let mut guard = self.inner.lock().map_err(|_| "engine lock poisoned")?;
        let g = &mut *guard;
        match cmd {
            // ------------------------------------------------ project
            "app.info" => Ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "ffmpeg": std::process::Command::new("ffmpeg").arg("-version").output().is_ok(),
                "input_devices": devices::inputs().into_iter().map(|d| d.name).collect::<Vec<_>>(),
                "midi_inputs": devices::midi_ports(),
                "home": std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default(),
                "open": g.session.as_ref().and_then(|s| s.root.as_ref()).map(|r| r.display().to_string()),
                "media_base": self.media_base.lock().unwrap().clone(),
            })),
            "project.new" => {
                let dir = PathBuf::from(s(a, "dir")?);
                if layout::project_file(&dir).exists() {
                    return Err(format!("{} already contains a project", dir.display()));
                }
                let sess = Session::create(
                    &dir,
                    NewProject {
                        name: os(a, "name").unwrap_or("Untitled").into(),
                        frame_rate: FrameRate::parse(os(a, "fps").unwrap_or("24")).map_err(e)?,
                        sample_rate: oi(a, "sample_rate").unwrap_or(48_000) as u32,
                        start_tc: os(a, "start_tc").unwrap_or("01:00:00:00").into(),
                    },
                )
                .map_err(e)?;
                self.open_session(g, sess);
                Ok(json!({"ok": true}))
            }
            "project.open" => {
                let dir = PathBuf::from(s(a, "dir")?);
                let sess = Session::open(&dir).map_err(e)?;
                let recovered = Session::recover(&dir).ok().flatten().is_some();
                self.open_session(g, sess);
                Ok(json!({"ok": true, "autosave_available": recovered}))
            }
            "project.recover" => {
                let dir = PathBuf::from(s(a, "dir")?);
                let st = Session::recover(&dir).map_err(e)?.ok_or("no newer autosave")?;
                let mut sess = Session::new(st);
                sess.root = Some(dir);
                self.open_session(g, sess);
                Ok(json!({"ok": true}))
            }
            "project.demo" => {
                let dir = PathBuf::from(s(a, "dir")?);
                drop(guard);
                let log = demo::run(&dir).map_err(e)?;
                let mut guard = self.inner.lock().unwrap();
                let sess = Session::open(&dir).map_err(e)?;
                self.open_session(&mut guard, sess);
                Ok(json!({"log": log}))
            }
            "project.save" => {
                let sess = g.session.as_mut().ok_or("no project open")?;
                sess.save().map_err(e)?;
                if let Some(r) = &sess.root {
                    let _ = std::fs::write(bindings_file(r), serde_json::to_vec_pretty(&g.spotter.bindings).unwrap_or_default());
                }
                self.publish(g);
                Ok(json!({"saved": true}))
            }
            "project.close" => {
                g.session = None;
                self.player.stop();
                self.publish(g);
                Ok(json!({}))
            }
            "project.settings" => {
                self.edit(g, "project settings", |st| {
                    if let Some(n) = a.get("name").and_then(|v| v.as_str()) {
                        st.project.name = n.into();
                    }
                    if let Some(r) = a.get("reel").and_then(|v| v.as_str()) {
                        st.project.reel = r.into();
                    }
                    if let Some(b) = a.get("ai_enabled").and_then(|v| v.as_bool()) {
                        st.project.ai.enabled = b;
                    }
                    Ok(())
                })?;
                Ok(json!({}))
            }
            "undo" | "redo" => {
                let sess = g.session.as_mut().ok_or("no project open")?;
                let r = if cmd == "undo" { sess.undo() } else { sess.redo() }.map_err(e)?;
                self.publish(g);
                Ok(json!({"label": r}))
            }
            "state.get" => state_json(g, self),
            "log.get" => Ok(json!(g.log.iter().rev().take(50).collect::<Vec<_>>())),

            // ------------------------------------------------ files
            "fs.list" => {
                let dir = PathBuf::from(s(a, "dir")?);
                let mut out = vec![];
                for ent in std::fs::read_dir(&dir).map_err(e)?.flatten() {
                    let name = ent.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    let is_dir = ent.path().is_dir();
                    out.push(json!({"name": name, "path": ent.path().display().to_string(), "dir": is_dir, "project": is_dir && layout::project_file(&ent.path()).exists()}));
                }
                out.sort_by_key(|v| (!v["dir"].as_bool().unwrap_or(false), v["name"].as_str().unwrap_or("").to_lowercase()));
                Ok(json!({"dir": dir.display().to_string(), "parent": dir.parent().map(|p| p.display().to_string()), "entries": out}))
            }

            // ------------------------------------------------ picture
            "picture.import" => {
                let path = PathBuf::from(s(a, "path")?);
                let id = self.edit(g, "import picture", |st| {
                    import_picture(
                        st,
                        PictureImport {
                            path: &path,
                            label: os(a, "label").unwrap_or("v1").into(),
                            start_tc: os(a, "start_tc").map(String::from),
                            duration_frames: oi(a, "frames"),
                            change_notes: os(a, "notes").unwrap_or("").into(),
                        },
                    )
                })?;
                Ok(json!({"id": id}))
            }
            "picture.proxy" => {
                // Browser-playable H.264 proxy next to the project cache (spec §16 media services)
                let st = Self::st(g)?;
                let pic = st.current_picture().ok_or("no picture")?.clone();
                let root = g.session.as_ref().and_then(|x| x.root.clone()).ok_or("no project folder")?;
                let out = root.join("Cache").join(format!("proxy_{}.mp4", &pic.fingerprint[4..16]));
                if !out.exists() {
                    let tc = st.project.frame_rate.frames_to_tc(pic.source_start_frame);
                    let ok = std::process::Command::new("ffmpeg")
                        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
                        .arg(&pic.media_path)
                        .args([
                            "-vf",
                            "scale=-2:540",
                            "-c:v",
                            "libx264",
                            "-preset",
                            "veryfast",
                            "-crf",
                            "23",
                            "-g",
                            "12",
                            "-pix_fmt",
                            "yuv420p",
                            "-c:a",
                            "aac",
                            "-timecode",
                            &tc,
                        ])
                        .arg(&out)
                        .status()
                        .map_err(|x| format!("ffmpeg unavailable: {x}"))?;
                    if !ok.success() {
                        return Err("proxy transcode failed".into());
                    }
                }
                Ok(json!({"path": out.display().to_string()}))
            }

            // ------------------------------------------------ scenes / identities
            "scene.add" => {
                let id = self.edit(g, "add scene", |st| {
                    let tb = st.tb();
                    let a0 = match oi(a, "start") {
                        Some(x) => x,
                        None => tb.sample_at_tc(s(a, "start_tc").map_err(Error::Invalid)?)?,
                    };
                    let z0 = match oi(a, "end") {
                        Some(x) => x,
                        None => tb.sample_at_tc(s(a, "end_tc").map_err(Error::Invalid)?)?,
                    };
                    spotting::add_scene(st, s(a, "number").map_err(Error::Invalid)?, os(a, "title").unwrap_or(""), a0, z0)
                })?;
                Ok(json!({"id": id}))
            }
            "scene.update" => {
                self.edit(g, "edit scene", |st| {
                    let id = s(a, "id").map_err(Error::Invalid)?;
                    let sc = st.scenes.get_mut(id).ok_or_else(|| Error::NotFound(id.into()))?;
                    if let Some(t) = os(a, "title") {
                        sc.title = t.into();
                    }
                    if let (Some(x), Some(y)) = (oi(a, "start"), oi(a, "end")) {
                        if y <= x {
                            return Err(Error::Invalid("scene end must be after start".into()));
                        }
                        sc.start_sample = x;
                        sc.end_sample = y;
                    }
                    Ok(())
                })?;
                Ok(json!({}))
            }
            "identity.add" => {
                let id = self.edit(g, "add identity", |st| {
                    let name = s(a, "name").map_err(Error::Invalid)?.to_string();
                    let id = spotting::add_identity(
                        st,
                        NewIdentity {
                            name: name.clone(),
                            category: Category::parse(os(a, "category").unwrap_or("fx")),
                            character: os(a, "character").map(String::from),
                            surface: os(a, "surface").map(String::from),
                            prop: os(a, "prop").map(String::from),
                        },
                    )?;
                    if let Some(q) = os(a, "pool_query") {
                        let pool: Vec<String> =
                            library::search(st, &Query { text: q.into(), ..Default::default() }).iter().map(|x| x.id.clone()).collect();
                        if let Some(first) = pool.first().cloned() {
                            spotting::assign_identity_sound(st, &id, &first, pool, mode(os(a, "mode").unwrap_or("round_robin")))?;
                        }
                    }
                    Ok(id)
                })?;
                Ok(json!({"id": id}))
            }
            "identity.assign" => {
                self.edit(g, "assign identity sound", |st| {
                    let id = s(a, "id").map_err(Error::Invalid)?;
                    let pool: Vec<String> = a.get("pool").and_then(|p| serde_json::from_value(p.clone()).ok()).unwrap_or_default();
                    let def = os(a, "default")
                        .map(String::from)
                        .or_else(|| pool.first().cloned())
                        .ok_or_else(|| Error::Invalid("no sound given".into()))?;
                    spotting::assign_identity_sound(
                        st,
                        id,
                        &def,
                        if pool.is_empty() { vec![def.clone()] } else { pool },
                        mode(os(a, "mode").unwrap_or("round_robin")),
                    )
                })?;
                Ok(json!({}))
            }

            // ------------------------------------------------ markers / cues
            "marker.add" => {
                let id = self.edit(g, "add marker", |st| {
                    let k = kind(s(a, "kind").map_err(Error::Invalid)?).map_err(Error::Invalid)?;
                    let start = i(a, "start").map_err(Error::Invalid)?;
                    let subs: Vec<i64> = a.get("sub_events").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                    spotting::add_marker(
                        st,
                        NewMarker {
                            lane_id: st.find_lane(s(a, "lane").map_err(Error::Invalid)?)?.id.clone(),
                            scene_id: os(a, "scene_id").map(String::from),
                            kind: k,
                            start,
                            end: oi(a, "end").unwrap_or(start),
                            label: os(a, "label").unwrap_or("").into(),
                            identity_id: os(a, "identity_id").map(String::from),
                            sub_events: subs,
                            origin: TimingOrigin::Manual,
                        },
                    )
                })?;
                Ok(json!({"id": id}))
            }
            "marker.update" => {
                self.edit(g, "move marker", |st| {
                    let subs: Option<Vec<i64>> = a.get("sub_events").and_then(|v| serde_json::from_value(v.clone()).ok());
                    spotting::update_marker(
                        st,
                        s(a, "id").map_err(Error::Invalid)?,
                        i(a, "start").map_err(Error::Invalid)?,
                        i(a, "end").map_err(Error::Invalid)?,
                        subs,
                    )
                })?;
                Ok(json!({}))
            }
            "marker.delete" => {
                self.edit(g, "delete marker", |st| {
                    let id = s(a, "id").map_err(Error::Invalid)?;
                    let m = st.markers.remove(id).ok_or_else(|| Error::NotFound(id.into()))?;
                    if let Some(c) = m.cue_id {
                        if let Some(cue) = st.cues.get_mut(&c) {
                            cue.marker_id = None;
                        }
                    }
                    Ok(())
                })?;
                Ok(json!({}))
            }
            "cues.build" => {
                let ids = self.edit(g, "build cues", |st| spotting::markers_to_cues(st, os(a, "scene_id")))?;
                Ok(json!({"created": ids}))
            }
            "cue.update" => {
                self.edit(g, "edit cue", |st| {
                    let c = st.cue_mut(s(a, "id").map_err(Error::Invalid)?)?;
                    if let Some(v) = a.get("description").and_then(|v| v.as_str()) {
                        c.description = v.into();
                    }
                    if let Some(v) = a.get("notes").and_then(|v| v.as_str()) {
                        c.notes = v.into();
                    }
                    if let Some(v) = a.get("owner").and_then(|v| v.as_str()) {
                        c.owner = v.into();
                    }
                    if let Some(v) = a.get("intent").and_then(|v| v.as_str()) {
                        c.intent = v.into();
                    }
                    if let Some(v) = a.get("status") {
                        c.status = serde_json::from_value(v.clone())?;
                    }
                    if let Some(v) = a.get("priority") {
                        c.priority = serde_json::from_value(v.clone())?;
                    }
                    if let Some(v) = a.get("identity_id") {
                        c.identity_id = v.as_str().filter(|x| !x.is_empty()).map(String::from);
                    }
                    if let (Some(x), Some(y)) = (oi(a, "start"), oi(a, "end")) {
                        let d = x - c.start_sample;
                        c.start_sample = x;
                        c.end_sample = y.max(x);
                        let id = c.id.clone();
                        for cl in st.clips.values_mut().filter(|cl| cl.cue_id.as_deref() == Some(id.as_str()) && cl.sub_index.is_none()) {
                            cl.timeline_start += d;
                        }
                    }
                    Ok(())
                })?;
                Ok(json!({}))
            }
            "cue.delete" => {
                self.edit(g, "delete cue", |st| {
                    let id = s(a, "id").map_err(Error::Invalid)?.to_string();
                    let c = st.cues.remove(&id).ok_or_else(|| Error::NotFound(id.clone()))?;
                    st.clips.retain(|_, cl| cl.cue_id.as_deref() != Some(id.as_str()));
                    if let Some(m) = c.marker_id.and_then(|m| st.markers.get_mut(&m)) {
                        m.cue_id = None;
                        m.approved = false;
                    }
                    Ok(())
                })?;
                Ok(json!({}))
            }
            "populate" => {
                let opts = PopulateOptions {
                    policy: match os(a, "policy").unwrap_or("") {
                        "category_per_scene" => TrackPolicy::CategoryPerScene,
                        "identity_global" => TrackPolicy::IdentityGlobal,
                        _ => TrackPolicy::IdentityPerScene,
                    },
                    scene_id: os(a, "scene_id").map(String::from),
                    cue_ids: a.get("cue_ids").and_then(|v| serde_json::from_value(v.clone()).ok()),
                    repopulate: ob(a, "repopulate"),
                };
                let rep = if ob(a, "dry_run") {
                    builder::preview(Self::st(g)?, &opts).map_err(e)?
                } else {
                    self.edit(g, "populate placeholders", |st| populate(st, &opts))?
                };
                Ok(serde_json::to_value(rep).map_err(e)?)
            }
            "cue.replace" => {
                let changed = self.edit(g, "replace sound", |st| {
                    let scope = match os(a, "scope").unwrap_or("cue") {
                        "scene" => ReplaceScope::IdentityInScene,
                        "all" => ReplaceScope::IdentityEverywhere,
                        _ => ReplaceScope::Cue,
                    };
                    editing::replace_cue_asset(
                        st,
                        s(a, "id").map_err(Error::Invalid)?,
                        s(a, "asset_id").map_err(Error::Invalid)?,
                        scope,
                        ob(a, "confirm"),
                    )
                })?;
                Ok(json!({"changed": changed}))
            }
            "cue.revert" => {
                self.edit(g, "revert to placeholder", |st| editing::revert_to_placeholder(st, s(a, "id").map_err(Error::Invalid)?))?;
                Ok(json!({}))
            }

            // ------------------------------------------------ clips / tracks / mixer
            "clip.update" => {
                self.edit(g, "edit clip", |st| {
                    let id = s(a, "id").map_err(Error::Invalid)?;
                    if let Some(x) = oi(a, "timeline_start") {
                        editing::move_clip(st, id, x)?;
                    }
                    if let Some(x) = oi(a, "trim_start") {
                        editing::trim_start(st, id, x)?;
                    }
                    if let Some(x) = oi(a, "length") {
                        editing::trim_end(st, id, x)?;
                    }
                    if let Some(x) = of(a, "gain_db") {
                        editing::set_clip_gain(st, id, x)?;
                    }
                    if oi(a, "fade_in").is_some() || oi(a, "fade_out").is_some() {
                        let c = st.clips.get(id).ok_or_else(|| Error::NotFound(id.into()))?;
                        let (fi, fo) = (oi(a, "fade_in").unwrap_or(c.fade_in), oi(a, "fade_out").unwrap_or(c.fade_out));
                        editing::set_fades(st, id, fi, fo)?;
                    }
                    if let Some(m) = a.get("muted").and_then(|v| v.as_bool()) {
                        st.clip_mut(id)?.muted = m;
                    }
                    if let Some(asset) = os(a, "asset_id") {
                        editing::replace_clip_asset(st, id, asset)?;
                    }
                    Ok(())
                })?;
                Ok(json!({}))
            }
            "clip.split" => {
                let id = self.edit(g, "split clip", |st| {
                    editing::split(st, s(a, "id").map_err(Error::Invalid)?, i(a, "at").map_err(Error::Invalid)?)
                })?;
                Ok(json!({"id": id}))
            }
            "clip.delete" => {
                self.edit(g, "delete clip", |st| editing::delete_clip(st, s(a, "id").map_err(Error::Invalid)?))?;
                Ok(json!({}))
            }
            "track.update" => {
                self.edit(g, "track", |st| {
                    let id = s(a, "id").map_err(Error::Invalid)?;
                    let bus = os(a, "bus_id").map(String::from);
                    if let Some(b) = &bus {
                        if !st.buses.contains_key(b) {
                            return Err(Error::NotFound(b.clone()));
                        }
                    }
                    let t = st.tracks.get_mut(id).ok_or_else(|| Error::NotFound(id.into()))?;
                    if let Some(x) = of(a, "gain_db") {
                        t.gain_db = x.clamp(-96.0, 12.0);
                    }
                    if let Some(x) = of(a, "pan") {
                        t.pan = x.clamp(-1.0, 1.0);
                    }
                    if let Some(x) = a.get("mute").and_then(|v| v.as_bool()) {
                        t.mute = x;
                    }
                    if let Some(x) = a.get("solo").and_then(|v| v.as_bool()) {
                        t.solo = x;
                    }
                    if let Some(x) = a.get("armed").and_then(|v| v.as_bool()) {
                        t.armed = x;
                    }
                    if let Some(x) = os(a, "name") {
                        t.name = x.into();
                    }
                    if let Some(b) = bus {
                        t.bus_id = b;
                    }
                    if let Some(ins) = a.get("inserts") {
                        t.inserts = serde_json::from_value(ins.clone())?;
                    }
                    Ok(())
                })?;
                Ok(json!({}))
            }
            "bus.update" => {
                self.edit(g, "bus", |st| {
                    let id = s(a, "id").map_err(Error::Invalid)?;
                    let b = st.buses.get_mut(id).ok_or_else(|| Error::NotFound(id.into()))?;
                    if let Some(x) = of(a, "gain_db") {
                        b.gain_db = x.clamp(-96.0, 12.0);
                    }
                    if let Some(x) = a.get("mute").and_then(|v| v.as_bool()) {
                        b.mute = x;
                    }
                    if let Some(ins) = a.get("inserts") {
                        b.inserts = serde_json::from_value(ins.clone())?;
                    }
                    Ok(())
                })?;
                Ok(json!({}))
            }
            "automation.set" => {
                let id = self.edit(g, "add automation point", |st| {
                    editing::set_automation(
                        st,
                        auto_owner(a).map_err(Error::Invalid)?,
                        auto_param(a),
                        i(a, "sample").map_err(Error::Invalid)?,
                        of(a, "value").unwrap_or(0.0),
                    )
                })?;
                Ok(json!({"id": id}))
            }
            "automation.update" => {
                self.edit(g, "move automation point", |st| {
                    let id = s(a, "id").map_err(Error::Invalid)?;
                    let p = st.automation.get(id).ok_or_else(|| Error::NotFound(id.into()))?.clone();
                    editing::move_automation(st, id, oi(a, "sample").unwrap_or(p.sample), of(a, "value").unwrap_or(p.value))
                })?;
                Ok(json!({}))
            }
            "automation.delete" => {
                self.edit(g, "delete automation point", |st| editing::delete_automation(st, s(a, "id").map_err(Error::Invalid)?))?;
                Ok(json!({}))
            }
            "automation.clear" => {
                let n = self.edit(g, "clear automation", |st| {
                    let p = os(a, "param").map(|_| auto_param(a));
                    editing::clear_automation(st, auto_owner(a).map_err(Error::Invalid)?, p)
                })?;
                Ok(json!({"removed": n}))
            }

            // ------------------------------------------------ library
            "library.import" => {
                let path = PathBuf::from(s(a, "path")?);
                let placeholder = ob(a, "placeholder");
                let rep = self.edit(g, "import library", |st| {
                    if path.is_file() {
                        let id = library::import_wav(st, &path, AssetMeta { license: None, ..Default::default() })?;
                        Ok(library::ImportReport { imported: vec![id], failed: vec![] })
                    } else if ob(a, "pack") || placeholder || path.join("pack.json").exists() {
                        Ok(library::import_pack(st, &path, placeholder)?.1)
                    } else {
                        library::import_directory(st, &path, AssetMeta::default())
                    }
                })?;
                Ok(json!({"imported": rep.imported.len(), "failed": rep.failed}))
            }
            "library.install_starter" => {
                let dir = match os(a, "dir") {
                    Some(d) => PathBuf::from(d),
                    None => {
                        g.session.as_ref().and_then(|x| x.root.clone()).ok_or("no project open")?.join("Audio/LibraryRefs/VC_Starter_Pack")
                    }
                };
                synth::write_starter_pack(&dir, Self::st(g)?.project.sample_rate).map_err(e)?;
                let n = self.edit(g, "install starter pack", |st| Ok(library::import_pack(st, &dir, true)?.1.imported.len()))?;
                Ok(json!({"imported": n, "dir": dir.display().to_string()}))
            }
            "library.search" => {
                let st = Self::st(g)?;
                let q = Query {
                    text: os(a, "text").unwrap_or("").into(),
                    category: os(a, "category").map(Category::parse),
                    ..Default::default()
                };
                let res: Vec<Value> =
                    library::search(st, &q).into_iter().take(oi(a, "limit").unwrap_or(300) as usize).map(asset_json).collect();
                Ok(json!(res))
            }
            "asset.peaks" => {
                let id = s(a, "id")?.to_string();
                if let Some(p) = g.peaks.get(&id) {
                    return Ok((**p).clone());
                }
                let st = Self::st(g)?;
                let asset = st.asset(&id).map_err(e)?.clone();
                drop(guard);
                let v = Arc::new(peaks(&asset)?);
                self.inner.lock().unwrap().peaks.insert(id, v.clone());
                Ok((*v).clone())
            }
            "audition" => {
                let st = Self::st(g)?;
                let id = s(a, "asset_id")?;
                let m = vcsd_core::mixer::Mixer::with_cache(st, Default::default(), self.player.shared.cache.clone());
                let buf = m.source(id).map_err(e)?;
                self.player.preview(Some(buf));
                Ok(json!({}))
            }
            "audition.stop" => {
                self.player.preview(None);
                Ok(json!({}))
            }

            // ------------------------------------------------ transport
            "transport.play" => {
                self.player.play(oi(a, "from"));
                Ok(json!({}))
            }
            "transport.stop" => {
                self.player.stop();
                Ok(json!({}))
            }
            "transport.toggle" => {
                if self.player.is_playing() {
                    self.player.stop()
                } else {
                    self.player.play(None)
                }
                Ok(json!({"playing": self.player.is_playing()}))
            }
            "transport.locate" => {
                self.player.locate(i(a, "sample")?);
                Ok(json!({}))
            }
            "transport.step" => {
                let st = Self::st(g)?;
                let tb = st.tb();
                let n = oi(a, "frames").unwrap_or(1);
                let to = tb.sample_at_frame(tb.frame_at(self.player.position()) + n);
                self.player.stop();
                self.player.locate(to);
                Ok(json!({"position": to}))
            }
            "transport.loop" => {
                self.player.set_loop(match (oi(a, "start"), oi(a, "end")) {
                    (Some(x), Some(y)) if y > x => Some((x, y)),
                    _ => None,
                });
                Ok(json!({}))
            }
            "transport.state" => {
                let mut v = serde_json::to_value(self.player.state()).map_err(e)?;
                v["version"] = json!(g.version);
                v["recording"] = json!(self.recording.lock().unwrap().as_ref().map(|r| r.device.clone()));
                v["mode"] = json!(g.mode);
                v["perf_triggers"] = json!(g.perf.as_ref().map(|p| p.len()));
                v["open_spots"] = json!(g.spotter.bindings.keys().filter(|k| g.spotter.is_open(k)).collect::<Vec<_>>());
                Ok(v)
            }

            // ------------------------------------------------ live spotting
            "spot.tap" => {
                let pos = oi(a, "at").unwrap_or_else(|| self.heard());
                let key = s(a, "key")?.to_string();
                let Inner { session, spotter, .. } = &mut *g;
                let sess = session.as_mut().ok_or("no project open")?;
                let out = sess.edit("spot", |st| spotter.tap(st, &key, pos)).map_err(e)?;
                self.publish(g);
                Ok(json!({"outcome": format!("{out:?}")}))
            }
            "spot.finish" => {
                let pos = oi(a, "at").unwrap_or_else(|| self.heard());
                let keys: Vec<String> = match os(a, "key") {
                    Some(k) => vec![k.into()],
                    None => g.spotter.bindings.keys().filter(|k| g.spotter.is_open(k)).cloned().collect(),
                };
                let Inner { session, spotter, .. } = &mut *g;
                let sess = session.as_mut().ok_or("no project open")?;
                let mut made = vec![];
                for k in keys {
                    made.push(sess.edit("spot", |st| spotter.finish(st, &k, pos)).map_err(e)?);
                }
                self.publish(g);
                Ok(json!({"markers": made}))
            }
            "spot.bindings" => Ok(serde_json::to_value(&g.spotter.bindings).map_err(e)?),
            "spot.bind" => {
                let key = s(a, "key")?.to_string();
                if ob(a, "remove") {
                    g.spotter.bindings.remove(&key);
                } else {
                    let st = Self::st(g)?;
                    let b = SpotBinding {
                        lane_id: st.find_lane(s(a, "lane")?).map_err(e)?.id.clone(),
                        kind: kind(s(a, "kind")?)?,
                        label: os(a, "label").unwrap_or("").into(),
                        identity_id: os(a, "identity_id").map(String::from),
                    };
                    g.spotter.bind(&key, b);
                }
                Ok(json!({}))
            }
            "input.mode" => {
                g.mode = if os(a, "mode") == Some("perform") { InputMode::Perform } else { InputMode::Spot };
                Ok(json!({}))
            }

            // ------------------------------------------------ perform
            "perform.start" => {
                let st = Self::st(g)?;
                let mut pads = PadMap::default();
                let target = os(a, "cue_id").and_then(|c| st.cues.get(c));
                for p in a.get("pads").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                    let key = p["key"].as_str().unwrap_or_default().to_string();
                    let assets: Vec<String> = match p["identity_id"].as_str() {
                        // the cue's scene-local override wins (e.g. wooden kitchen floor)
                        Some(iid) if target.is_some_and(|c| c.identity_id.as_deref() == Some(iid) && c.asset_override_id.is_some()) => {
                            vec![target.unwrap().asset_override_id.clone().unwrap()]
                        }
                        Some(iid) => {
                            let ident = st.identity(iid).map_err(e)?;
                            PadMap::from_identity(ident).assets
                        }
                        None => serde_json::from_value(p["assets"].clone()).unwrap_or_default(),
                    };
                    if assets.is_empty() {
                        return Err(format!("pad {key} has no sounds"));
                    }
                    pads.set(
                        &key,
                        Pad {
                            name: p["name"].as_str().unwrap_or(&key).into(),
                            assets,
                            mode: mode(p["mode"].as_str().unwrap_or("round_robin")),
                            ..Default::default()
                        },
                    );
                }
                let cue = os(a, "cue_id").map(String::from);
                let track = match os(a, "track_id") {
                    Some(t) => t.to_string(),
                    None => {
                        let c = cue.as_ref().ok_or("perform needs a track or a cue")?;
                        st.clips_for_cue(c).first().map(|x| x.track_id.clone()).ok_or("cue has no track yet — populate first")?
                    }
                };
                let mut rec = PerformanceRecorder::new(&track, cue.as_deref(), pads);
                rec.latency_compensation = oi(a, "latency").unwrap_or(0);
                g.perf = Some(rec);
                g.mode = InputMode::Perform;
                Ok(json!({"track_id": track}))
            }
            "perform.trigger" => {
                let pos = oi(a, "at").unwrap_or_else(|| self.heard());
                let p = g.perf.as_mut().ok_or("no performance armed")?;
                p.trigger(s(a, "pad")?, pos, oi(a, "velocity").unwrap_or(100).clamp(1, 127) as u8).map_err(e)?;
                Ok(json!({"count": p.len()}))
            }
            "perform.commit" => {
                let rec = g.perf.take().ok_or("no performance armed")?;
                g.mode = InputMode::Spot;
                let id = self.edit(g, "performance take", |st| rec.commit(st))?;
                Ok(json!({"id": id}))
            }
            "perform.cancel" => {
                g.perf = None;
                g.mode = InputMode::Spot;
                Ok(json!({}))
            }
            "perform.select" => {
                self.edit(g, "select take", |st| perform::select_take(st, s(a, "id").map_err(Error::Invalid)?))?;
                Ok(json!({}))
            }

            // ------------------------------------------------ recording
            "record.start" => {
                let st = Self::st(g)?;
                let rate = st.project.sample_rate;
                let cue = os(a, "cue_id").map(String::from);
                let track = os(a, "track_id").map(String::from);
                if cue.is_none() && track.is_none() {
                    return Err("choose a cue or an armed track to record onto".into());
                }
                if self.recording.lock().unwrap().is_some() {
                    return Err("already recording".into());
                }
                let input = self.prefs.lock().unwrap().input_device.clone();
                let rec = recorder::start(self.player.shared.clone(), rate, input, cue, track)?;
                let dev = rec.device.clone();
                *self.recording.lock().unwrap() = Some(rec);
                let from = oi(a, "from").unwrap_or_else(|| self.player.position()) - oi(a, "preroll").unwrap_or(0);
                self.player.play(Some(from));
                Ok(json!({"device": dev}))
            }
            "record.stop" => {
                let rec = self.recording.lock().unwrap().take().ok_or("not recording")?;
                let (cue, track) = (rec.cue_id.clone(), rec.track_id.clone());
                self.player.stop();
                let cap = rec.finish()?;
                if cap.data.is_empty() {
                    return Err("no audio captured".into());
                }
                let root = g.session.as_ref().and_then(|x| x.root.clone()).ok_or("no project folder")?;
                let st = Self::st(g)?;
                let tb = st.tb();
                let label = cue.as_ref().and_then(|c| st.cues.get(c)).map(|c| c.code.clone()).unwrap_or_else(|| "track".into());
                let path = root.join("Audio/RecordedFoley").join(format!(
                    "{}_{}_{}.wav",
                    layout::safe_name(&label),
                    today(),
                    &ids::new_id("r")[24..]
                ));
                let buf = wav::AudioBuffer { sample_rate: cap.sample_rate, channels: cap.channels, data: cap.data };
                let bext = wav::Bext {
                    description: format!("Foley take for {label} @ {}", tb.tc_at(cap.start_pos)),
                    originator: "VC Sound Design".into(),
                    time_reference: (tb.origin_since_midnight() + cap.start_pos).max(0) as u64,
                    coding_history: format!("A=PCM,F={},W=32,T={}\r\n", cap.sample_rate, cap.device),
                    ..Default::default()
                };
                wav::write(&path, &buf, wav::SampleFormat::Float32, Some(&bext)).map_err(e)?;
                let start = cap.start_pos;
                let clip = self.edit(g, "record take", |st| {
                    let take = editing::add_take(
                        st,
                        TakeImport {
                            wav: &path,
                            cue_id: cue.clone(),
                            track_id: track.clone(),
                            slate: String::new(),
                            timeline_start: Some(start),
                            sync_offset: 0,
                        },
                    )?;
                    match &cue {
                        Some(c) => editing::use_take_for_cue(st, c, &take),
                        None => Ok(take),
                    }
                })?;
                Ok(json!({"path": path.display().to_string(), "clip": clip}))
            }
            "take.add" => {
                let path = PathBuf::from(s(a, "path")?);
                let clip = self.edit(g, "add take", |st| {
                    let cue = s(a, "cue_id").map_err(Error::Invalid)?.to_string();
                    let take = editing::add_take(
                        st,
                        TakeImport {
                            wav: &path,
                            cue_id: Some(cue.clone()),
                            track_id: None,
                            slate: os(a, "slate").unwrap_or("").into(),
                            timeline_start: oi(a, "at"),
                            sync_offset: oi(a, "sync_offset").unwrap_or(0),
                        },
                    )?;
                    match os(a, "clip_id") {
                        Some(cl) => editing::use_take_for_clip(st, cl, &take).map(|_| cl.to_string()),
                        None => editing::use_take_for_cue(st, &cue, &take),
                    }
                })?;
                Ok(json!({"clip": clip}))
            }

            // ------------------------------------------------ MIDI
            "midi.connect" => {
                drop(guard);
                let name = self.connect_midi(os(a, "port"))?;
                Ok(json!({"port": name}))
            }
            "midi.disconnect" => {
                drop(guard);
                self.disconnect_midi();
                Ok(json!({}))
            }
            "devices.list" => Ok(self.devices_json()),
            "devices.status" => Ok(self.device_status_json()),
            "prefs.set" => {
                // keys present in `args` change; null = system default / no MIDI
                drop(guard);
                let has = |k: &str| a.as_object().is_some_and(|o| o.contains_key(k));
                let val = |k: &str| a.get(k).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(String::from);
                let mut midi_err = None;
                if has("output_device") {
                    let v = val("output_device");
                    self.prefs.lock().unwrap().output_device = v.clone();
                    self.player.set_output_device(v);
                }
                if has("input_device") {
                    let v = val("input_device");
                    if let Some(n) = &v {
                        if !devices::inputs().iter().any(|d| &d.name == n) {
                            return Err(format!("audio input '{n}' is not connected"));
                        }
                    }
                    self.prefs.lock().unwrap().input_device = v;
                }
                if has("midi_port") {
                    let v = val("midi_port");
                    match &v {
                        Some(p) => {
                            if let Err(x) = self.connect_midi(Some(p)) {
                                midi_err = Some(x);
                            }
                        }
                        None => self.disconnect_midi(),
                    }
                    // remember the choice even if the controller is unplugged right now
                    self.prefs.lock().unwrap().midi_port = v;
                }
                self.save_prefs()?;
                std::thread::sleep(std::time::Duration::from_millis(60)); // let the output reopen
                let mut out = self.devices_json();
                if let Some(x) = midi_err {
                    out["error"] = json!(x);
                }
                Ok(out)
            }
            "devices.test_tone" => {
                // 1 kHz at -18 dBFS for 0.6 s with short ramps, on the current output
                let sr = self.player.shared.project_rate.load(std::sync::atomic::Ordering::Relaxed).max(8000);
                let n = (sr as f32 * 0.6) as usize;
                let mut data = Vec::with_capacity(n * 2);
                let amp = 10f32.powf(-18.0 / 20.0);
                for k in 0..n {
                    let env = (k.min(n - 1 - k) as f32 / (sr as f32 * 0.01)).min(1.0);
                    let v = (2.0 * std::f32::consts::PI * 1000.0 * k as f32 / sr as f32).sin() * amp * env;
                    data.push(v);
                    data.push(v);
                }
                self.player.preview(Some(Arc::new(wav::AudioBuffer { sample_rate: sr, channels: 2, data })));
                Ok(json!({}))
            }
            "midi.message" => {
                // for on-screen pads / testing: inject a raw MIDI message
                let bytes: Vec<u8> = serde_json::from_value(a.get("bytes").cloned().unwrap_or_default()).map_err(e)?;
                drop(guard);
                self.handle_midi(&bytes);
                Ok(json!({}))
            }

            // ------------------------------------------------ AI
            "ai.run" => {
                let ids: Vec<String> = serde_json::from_value(a.get("cue_ids").cloned().unwrap_or_default()).map_err(e)?;
                let r = self.edit(g, "ai timing assist", |st| match os(a, "assistant").unwrap_or("visual") {
                    "cadence" => ai::run(st, &ai::CadenceHeuristic { step_secs: of(a, "step_secs").unwrap_or(0.5) as f64 }, &ids),
                    _ => ai::run(st, &ai::VisualOnsetAssistant::default(), &ids),
                })?;
                Ok(json!({"proposals": r}))
            }
            "ai.accept" => {
                self.edit(g, "accept AI proposal", |st| ai::accept(st, s(a, "id").map_err(Error::Invalid)?, None))?;
                Ok(json!({}))
            }
            "ai.reject" => {
                self.edit(g, "reject AI proposal", |st| ai::reject(st, s(a, "id").map_err(Error::Invalid)?))?;
                Ok(json!({}))
            }

            // ------------------------------------------------ reports / QA / export
            "report.active_at" => Ok(serde_json::to_value(report::active_at(Self::st(g)?, i(a, "sample")?)).map_err(e)?),
            "report.summary" => Ok(serde_json::to_value(report::scene_summary(Self::st(g)?, s(a, "scene_id")?).map_err(e)?).map_err(e)?),
            "report.write" => {
                let st = Self::st(g)?;
                let path = PathBuf::from(s(a, "path")?);
                let sc = os(a, "scene_id");
                match path.extension().and_then(|x| x.to_str()).unwrap_or("pdf") {
                    "csv" => std::fs::write(&path, report::rows_to_csv(st, &report::cue_rows(st, sc))),
                    "json" => {
                        std::fs::write(&path, serde_json::to_vec_pretty(&report::manifest_json(st, sc, &today())).unwrap_or_default())
                    }
                    _ => std::fs::write(&path, pdf::cue_report(st, sc, &today())),
                }
                .map_err(e)?;
                Ok(json!({"path": path.display().to_string()}))
            }
            "qa.run" => {
                Ok(serde_json::to_value(qa::run(Self::st(g)?, &qa::QaOptions { verify_hashes: ob(a, "hashes"), exclude_temp_refs: true }))
                    .map_err(e)?)
            }
            "export.run" => {
                let st = Self::st(g)?;
                let mut o = ExportOptions::new(ExportProfile::parse(os(a, "profile").unwrap_or("resolve")).map_err(e)?, &today());
                o.handle_samples = oi(a, "handles_ms").unwrap_or(0) * st.project.sample_rate as i64 / 1000;
                o.force = ob(a, "force");
                if ob(a, "float") {
                    o.format = wav::SampleFormat::Float32;
                }
                if let Some(sc) = os(a, "scene_id") {
                    o.range = ExportRange::Scene(sc.into());
                }
                let root = g.session.as_ref().and_then(|x| x.root.clone());
                let r = self.edit(g, "export", |st| export::export(st, root.as_deref(), &o))?;
                if let Some(sess) = g.session.as_mut() {
                    let _ = sess.save();
                }
                Ok(json!({"dir": r.dir.display().to_string(), "files": r.files.len(), "qa": r.qa}))
            }
            "relink.run" => {
                let dirs: Vec<PathBuf> = serde_json::from_value(a.get("dirs").cloned().unwrap_or_default()).map_err(e)?;
                let rep = self.edit(g, "relink", |st| relink::relink_by_hash(st, &dirs.iter().map(|p| p.as_path()).collect::<Vec<_>>()))?;
                self.player.clear_cache();
                g.peaks.clear();
                Ok(serde_json::to_value(rep).map_err(e)?)
            }
            "score.import" => {
                let path = PathBuf::from(s(a, "path")?);
                let id = self.edit(g, "import score", |st| integrations::import_score(st, &path))?;
                Ok(json!({"id": id}))
            }
            "script.import" => {
                let text = std::fs::read_to_string(s(a, "path")?).map_err(e)?;
                let r = self.edit(g, "import script breakdown", |st| integrations::import_film_studio(st, &text))?;
                Ok(serde_json::to_value(r).map_err(e)?)
            }
            "tc.parse" => {
                let st = Self::st(g)?;
                Ok(json!({"sample": st.tb().sample_at_tc(s(a, "tc")?).map_err(e)?}))
            }
            _ => Err(format!("unknown command '{cmd}'")),
        }
    }

    /// Paths the browser-mode server may stream (picture + referenced media).
    pub fn media_allowed(&self, p: &Path) -> bool {
        let g = self.inner.lock().unwrap();
        let Some(sess) = &g.session else { return false };
        let ps = p.to_string_lossy();
        sess.state.pictures.values().any(|x| x.media_path == ps)
            || sess.state.assets.values().any(|x| x.path == ps)
            || sess.root.as_ref().is_some_and(|r| std::fs::canonicalize(r).map(|r| p.starts_with(r)).unwrap_or(false))
    }
}

fn asset_json(x: &Asset) -> Value {
    json!({
        "id": x.id, "name": x.name, "path": x.path, "kind": x.kind, "category": x.category,
        "sample_rate": x.sample_rate, "channels": x.channels, "length": x.length_samples,
        "tags": x.tags, "license": x.license.as_ref().map(|l| l.name.clone()),
        "redistributable": x.license.as_ref().map(|l| l.redistributable), "loopable": x.loopable,
        "missing": !Path::new(&x.path).exists(),
    })
}

/// Waveform overview: per 256-sample bucket min/max (mono sum) as i8.
fn peaks(asset: &Asset) -> std::result::Result<Value, String> {
    const PER: usize = 256;
    let (_, b) = wav::read(Path::new(&asset.path)).map_err(e)?;
    let ch = b.channels as usize;
    let n = b.frames();
    let mut data = Vec::with_capacity(n / PER * 2 + 2);
    let mut f = 0;
    while f < n {
        let end = (f + PER).min(n);
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for k in f..end {
            let v = (0..ch).map(|c| b.data[k * ch + c]).sum::<f32>() / ch as f32;
            lo = lo.min(v);
            hi = hi.max(v);
        }
        data.push((lo.clamp(-1.0, 1.0) * 127.0) as i8);
        data.push((hi.clamp(-1.0, 1.0) * 127.0) as i8);
        f = end;
    }
    Ok(json!({"per": PER, "sample_rate": b.sample_rate, "data": data}))
}

fn state_json(g: &Inner, eng: &Engine) -> ApiResult {
    let Some(sess) = &g.session else { return Ok(json!({"open": false, "version": g.version})) };
    let st = &sess.state;
    let p = &st.project;
    let pic = st.current_picture();
    let mut tracks: Vec<&Track> = st.tracks.values().collect();
    tracks.sort_by(|a, b| {
        (a.scene_id.as_ref().and_then(|x| st.scenes.get(x)).map(|x| x.start_sample).unwrap_or(i64::MAX), &a.folder, a.order).cmp(&(
            b.scene_id.as_ref().and_then(|x| st.scenes.get(x)).map(|x| x.start_sample).unwrap_or(i64::MAX),
            &b.folder,
            b.order,
        ))
    });
    let mut buses: Vec<&Bus> = st.buses.values().collect();
    buses.sort_by_key(|b| (b.parent_id.is_none(), b.order));
    let mut scenes: Vec<&Scene> = st.scenes.values().collect();
    scenes.sort_by_key(|x| x.start_sample);
    let mut lanes: Vec<&MarkerLane> = st.lanes.values().collect();
    lanes.sort_by_key(|l| l.order);
    let assets: Vec<Value> = st
        .assets
        .values()
        .filter(|x| st.clips.values().any(|c| c.asset_id == x.id) || x.kind == AssetKind::Recorded)
        .map(asset_json)
        .collect();
    let _ = eng;
    Ok(json!({
        "open": true,
        "version": g.version,
        "root": sess.root.as_ref().map(|r| r.display().to_string()),
        "dirty": sess.dirty,
        "undo": sess.undo_label(), "redo": sess.redo_label(),
        "project": {
            "name": p.name, "reel": p.reel, "fps_num": p.frame_rate.num, "fps_den": p.frame_rate.den, "drop_frame": p.frame_rate.drop_frame,
            "fps_label": p.frame_rate.label(), "sample_rate": p.sample_rate, "start_frame": p.start_frame, "ai_enabled": p.ai.enabled,
        },
        "picture": pic.map(|x| json!({"id": x.id, "label": x.label, "path": x.media_path, "start_frame": x.source_start_frame, "duration_frames": x.duration_frames})),
        "pictures": st.pictures.values().map(|x| json!({"id": x.id, "label": x.label})).collect::<Vec<_>>(),
        "scenes": scenes,
        "lanes": lanes,
        "markers": st.markers.values().collect::<Vec<_>>(),
        "cues": st.sorted_cues(None),
        "chart": report::cue_rows(st, None),
        "identities": st.identities.values().collect::<Vec<_>>(),
        "tracks": tracks,
        "automation": st.automation.values().collect::<Vec<_>>(),
        "buses": buses,
        "clips": st.clips.values().collect::<Vec<_>>(),
        "assets": assets,
        "takes": st.takes.values().collect::<Vec<_>>(),
        "performances": st.performances.values().map(|x| json!({"id": x.id, "track_id": x.track_id, "cue_id": x.cue_id, "take": x.take})).collect::<Vec<_>>(),
        "proposals": st.proposals.values().collect::<Vec<_>>(),
        "scores": st.scores.values().collect::<Vec<_>>(),
        "script": st.script_refs.values().collect::<Vec<_>>(),
        "exports": st.exports.values().collect::<Vec<_>>(),
        "bindings": g.spotter.bindings,
        "qa": { "errors": 0 },
    }))
}

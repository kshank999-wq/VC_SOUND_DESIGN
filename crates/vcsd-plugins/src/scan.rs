//! Finding plugins.
//!
//! VST3 bundles are found in the platform's standard folders plus any the user
//! adds (and `VCSD_VST3_PATH`). Each new or changed bundle is opened in a
//! child process (`<exe> --vcsd-plugin-scan <bundle>`), so a plugin that
//! crashes or hangs while loading is reported, not fatal. Results are cached by
//! modification time. Audio Units are listed through the system's component
//! registry, which doesn't load plugin code.

use crate::PluginInfo;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Argument that turns an app binary into a one-bundle scanner (see [`child_main`]).
pub const SCAN_ARG: &str = "--vcsd-plugin-scan";
const TIMEOUT: Duration = Duration::from_secs(30);

/// Standard VST3 folders for this platform.
pub fn default_vst3_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut v = vec![];
    if cfg!(target_os = "macos") {
        if let Some(h) = env("HOME") {
            v.push(h.join("Library/Audio/Plug-Ins/VST3"));
        }
        v.push("/Library/Audio/Plug-Ins/VST3".into());
    } else if cfg!(target_os = "windows") {
        if let Some(c) = env("COMMONPROGRAMFILES") {
            v.push(c.join("VST3"));
        }
        if let Some(l) = env("LOCALAPPDATA") {
            v.push(l.join("Programs/Common/VST3"));
        }
    } else {
        if let Some(h) = env("HOME") {
            v.push(h.join(".vst3"));
        }
        v.push("/usr/lib/vst3".into());
        v.push("/usr/local/lib/vst3".into());
    }
    if let Some(p) = std::env::var_os("VCSD_VST3_PATH") {
        v.extend(std::env::split_paths(&p));
    }
    v
}

/// All `.vst3` bundles (or single-file modules) under `dirs`.
pub fn find_bundles(dirs: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(d: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let is_vst3 = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vst3"));
            if is_vst3 {
                out.push(p);
            } else if p.is_dir() && depth < 4 {
                walk(&p, depth + 1, out); // vendors nest bundles in folders
            }
        }
    }
    let mut out = vec![];
    for d in dirs {
        walk(d, 0, &mut out);
    }
    out.dedup();
    out
}

/// If this process was started as a scanner, scan and exit. Call first thing in `main`.
pub fn child_main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(SCAN_ARG) {
        return;
    }
    let r = match args.get(2) {
        Some(p) => crate::vst3host::scan_module(Path::new(p)),
        None => Err("no bundle given".into()),
    };
    let out = match r {
        Ok(v) => serde_json::json!({ "plugins": v }),
        Err(e) => serde_json::json!({ "error": e }),
    };
    println!("{out}");
    std::process::exit(0);
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    pub modified: u64,
    #[serde(default)]
    pub plugins: Vec<PluginInfo>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Scan results by bundle path, persisted between runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    pub bundles: BTreeMap<String, Entry>,
}

impl Cache {
    pub fn load(path: &Path) -> Cache {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap_or_default()).map_err(|e| e.to_string())
    }
}

/// How bundles are opened during a scan.
#[derive(Debug, Clone)]
pub enum Runner {
    /// `exe [args…] --vcsd-plugin-scan <bundle>`, one process per bundle.
    Child { exe: PathBuf, args: Vec<String> },
    /// Load in this process (tests; a crashing plugin takes the process down).
    InProcess,
}

impl Runner {
    /// This executable as the scanner (it must call [`child_main`]).
    pub fn current_exe() -> Runner {
        match std::env::current_exe() {
            Ok(exe) => Runner::Child { exe, args: vec![] },
            Err(_) => Runner::InProcess,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Failed {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub plugins: Vec<PluginInfo>,
    pub failed: Vec<Failed>,
    pub bundles: usize,
    /// Bundles opened this time (the rest came from the cache).
    pub scanned: usize,
}

fn modified(p: &Path) -> u64 {
    let m = |p: &Path| {
        std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs())
    };
    crate::vst3host::binary_path(p).ok().and_then(|b| m(&b)).or_else(|| m(p)).unwrap_or(0)
}

fn scan_child(exe: &Path, args: &[String], bundle: &Path) -> Entry {
    let mut e = Entry { modified: modified(bundle), ..Default::default() };
    let child =
        Command::new(exe).args(args).arg(SCAN_ARG).arg(bundle).stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null()).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(x) => {
            e.error = Some(format!("could not start the scanner: {x}"));
            return e;
        }
    };
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let t0 = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if t0.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let out = reader.join().unwrap_or_default();
    let v: Option<serde_json::Value> = out.lines().rev().find_map(|l| serde_json::from_str(l).ok());
    match (status, v) {
        (None, _) => e.error = Some(format!("did not finish loading within {} s", TIMEOUT.as_secs())),
        (Some(_), Some(v)) if v.get("plugins").is_some() => {
            e.plugins = serde_json::from_value(v["plugins"].clone()).unwrap_or_default();
        }
        (Some(_), Some(v)) => e.error = Some(v["error"].as_str().unwrap_or("failed to load").to_string()),
        (Some(s), None) => e.error = Some(format!("crashed while loading ({s})")),
    }
    e
}

/// Scan `dirs`, reusing cached results for unchanged bundles (all of them again with `force`).
pub fn scan(dirs: &[PathBuf], cache: &mut Cache, runner: &Runner, force: bool) -> Report {
    let bundles = find_bundles(dirs);
    let mut rep = Report { bundles: bundles.len(), ..Default::default() };
    let mut fresh = BTreeMap::new();
    for b in &bundles {
        let key = b.display().to_string();
        let m = modified(b);
        let entry = match cache.bundles.get(&key) {
            Some(e) if !force && e.modified == m => e.clone(),
            _ => {
                rep.scanned += 1;
                match runner {
                    Runner::Child { exe, args } => scan_child(exe, args, b),
                    Runner::InProcess => match crate::vst3host::scan_module(b) {
                        Ok(plugins) => Entry { modified: m, plugins, error: None },
                        Err(err) => Entry { modified: m, plugins: vec![], error: Some(err) },
                    },
                }
            }
        };
        if let Some(err) = &entry.error {
            rep.failed.push(Failed { path: key.clone(), error: err.clone() });
        }
        rep.plugins.extend(entry.plugins.iter().cloned());
        fresh.insert(key, entry);
    }
    cache.bundles = fresh; // forget bundles that were removed
    #[cfg(target_os = "macos")]
    rep.plugins.extend(crate::au::list());
    rep.plugins.sort_by_key(|p| (p.vendor.to_lowercase(), p.name.to_lowercase()));
    rep.plugins.dedup_by(|a, b| a.id == b.id);
    rep
}

//! Plugin hosting for VC Sound Design.
//!
//! - **VST3** on macOS, Windows and Linux ([`vst3host`]).
//! - **Audio Units** (v2 API, which also loads most v3 effects) on macOS ([`au`]).
//!
//! Scanning loads third-party code, so [`scan`] runs each VST3 bundle in a child
//! process: a plugin that crashes while being scanned is listed as failed
//! instead of taking the app down. Instances are created in-process and run on
//! the engine's render thread (never in the device callback).
//!
//! The API is format-neutral: an [`Instance`] processes interleaved f32 audio
//! in place, exposes parameters as normalised (0–1) values, and saves/restores
//! an opaque state blob.

pub mod scan;
pub mod vst3host;

#[cfg(target_os = "macos")]
pub mod au;

use serde::{Deserialize, Serialize};
use std::rc::Rc;
use std::sync::Arc;

/// One plugin (class) found by the scanner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginInfo {
    /// Stable id: `vst3:<class id hex>` or `au:<type>:<subtype>:<manufacturer>`.
    pub id: String,
    /// `vst3` or `au`.
    pub format: String,
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub version: String,
    /// VST3 sub-categories (`Fx|EQ`, `Instrument|Synth`, …) or the AU type.
    #[serde(default)]
    pub category: String,
    /// Bundle / module path (VST3); empty for Audio Units.
    #[serde(default)]
    pub path: String,
}

impl PluginInfo {
    /// Instruments and generators make no sense as inserts.
    pub fn is_effect(&self) -> bool {
        let c = self.category.to_ascii_lowercase();
        !(c.contains("instrument") || c.contains("generator") || c == "aumu")
    }
}

/// A plugin parameter, values normalised 0–1.
#[derive(Debug, Clone, Serialize)]
pub struct ParamInfo {
    pub id: u32,
    pub title: String,
    pub units: String,
    /// 0 = continuous, n = n+1 discrete steps.
    pub step_count: i32,
    pub default: f64,
    pub value: f64,
    /// The plugin's own text for the current value.
    pub display: String,
    pub automatable: bool,
    pub read_only: bool,
    pub is_bypass: bool,
}

/// How an instance will be run.
#[derive(Debug, Clone, Copy)]
pub struct Setup {
    pub sample_rate: f64,
    /// Channel count of the audio it will process (1 or 2 in this app).
    pub channels: usize,
    /// Largest block passed to `process` at once (bigger calls are split).
    pub max_block: usize,
    /// Offline (export) rendering: plugins may use higher-quality modes.
    pub offline: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Setup { sample_rate: 48_000.0, channels: 2, max_block: 1024, offline: false }
    }
}

/// A running plugin.
pub trait Instance: Send + Sync {
    fn info(&self) -> &PluginInfo;
    /// Reported processing latency in samples.
    fn latency(&self) -> u32;
    fn params(&self) -> Vec<ParamInfo>;
    /// Set a parameter (normalised); takes effect from the next processed block.
    fn set_param(&self, id: u32, value: f64) -> Result<(), String>;
    fn get_state(&self) -> Result<Vec<u8>, String>;
    fn set_state(&self, data: &[u8]) -> Result<(), String>;
    /// Process interleaved audio in place. `pos` is the timeline position of the
    /// first frame; when it doesn't follow on from the previous call the plugin
    /// is reset first (locate, loop, a second export pass).
    fn process(&self, buf: &mut [f32], channels: usize, pos: i64);
    /// Clear internal state (tails, delay lines).
    fn reset(&self);
    /// Position the next `process` call continues from (None after a reset).
    fn next_pos(&self) -> Option<i64>;
    /// True once after the plugin's own editor changed something (state worth saving).
    fn take_dirty(&self) -> bool {
        false
    }
    /// Show the plugin's own editor inside `parent`. Main (GUI) thread only.
    /// Dropping the returned editor detaches it; keep the instance alive meanwhile.
    fn open_editor(&self, _parent: ParentWindow, _host: Rc<dyn EditorHost>) -> Result<Box<dyn Editor>, String> {
        Err(format!("{} has no editor of its own", self.info().name))
    }
}

/// The native window a plugin editor is embedded in.
#[derive(Debug, Clone, Copy)]
pub enum ParentWindow {
    /// Windows `HWND`.
    Win32(*mut std::ffi::c_void),
    /// macOS `NSView` (the window's content view).
    AppKit(*mut std::ffi::c_void),
    /// X11 window id (Linux).
    X11(u64),
}

/// What the app provides to an open editor (main thread).
pub trait EditorHost {
    /// The plugin asks for a new editor size (in the units of [`Editor::size`]).
    /// Return true once the window has been resized.
    fn resize(&self, width: u32, height: u32) -> bool;
    /// UI scale (1.0, 2.0, …) for plugins that draw at a scale (Windows/Linux).
    fn scale(&self) -> f64 {
        1.0
    }
    /// Linux: the GUI event loop that plugin file descriptors and timers join.
    fn run_loop(&self) -> Option<Rc<dyn RunLoop>> {
        None
    }
}

/// Linux GUI event loop hooks (VST3 `IRunLoop`); everything runs on the main thread.
pub trait RunLoop {
    /// Call `f` whenever `fd` is readable. Returns an id for [`RunLoop::cancel`].
    fn watch_fd(&self, fd: i32, f: Box<dyn Fn()>) -> u64;
    /// Call `f` every `ms` milliseconds.
    fn every(&self, ms: u32, f: Box<dyn Fn()>) -> u64;
    fn cancel(&self, id: u64);
}

/// An open plugin editor. Dropping it removes the view from its window.
pub trait Editor {
    /// Current size: physical pixels on Windows/Linux, points on macOS.
    fn size(&self) -> (u32, u32);
    fn resizable(&self) -> bool;
    /// The user resized the window: returns the size the editor actually took.
    fn set_size(&self, width: u32, height: u32) -> (u32, u32);
}

/// Create an instance of a scanned plugin.
pub fn instantiate(info: &PluginInfo, setup: Setup) -> Result<Arc<dyn Instance>, String> {
    match info.format.as_str() {
        "vst3" => Ok(Arc::new(vst3host::Vst3Instance::new(info, setup)?)),
        #[cfg(target_os = "macos")]
        "au" => Ok(Arc::new(au::AuInstance::new(info, setup)?)),
        f => Err(format!("{f} plugins are not supported on this platform")),
    }
}

/// Map `channels` of interleaved audio onto a plugin bus of `bus_ch` channels.
/// Mono into a stereo plugin is duplicated; stereo into a mono plugin is summed to mono.
pub(crate) fn split_in(buf: &[f32], channels: usize, frames: usize, bus: &mut [Vec<f32>]) {
    let bus_ch = bus.len();
    for (c, dst) in bus.iter_mut().enumerate() {
        for f in 0..frames {
            dst[f] = if bus_ch >= channels {
                buf[f * channels + c.min(channels - 1)]
            } else {
                (0..channels).map(|k| buf[f * channels + k]).sum::<f32>() / channels as f32
            };
        }
    }
}

/// The reverse of [`split_in`]: a stereo plugin feeding a mono slot is averaged.
pub(crate) fn join_out(bus: &[Vec<f32>], buf: &mut [f32], channels: usize, frames: usize) {
    let bus_ch = bus.len();
    if bus_ch == 0 {
        return;
    }
    for f in 0..frames {
        for c in 0..channels {
            buf[f * channels + c] = if channels == 1 && bus_ch > 1 { (bus[0][f] + bus[1][f]) * 0.5 } else { bus[c.min(bus_ch - 1)][f] };
        }
    }
}

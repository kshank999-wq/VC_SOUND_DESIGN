//! Audio device / MIDI port enumeration and persisted user preferences.

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const COMMON_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub max_channels: u16,
    /// Common sample rates the device can run at natively.
    pub rates: Vec<u32>,
}

fn dev_name(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| "unnamed device".into())
}

fn info(d: &cpal::Device, default: &Option<String>, input: bool) -> DeviceInfo {
    let name = dev_name(d);
    let ranges: Vec<cpal::SupportedStreamConfigRange> = if input {
        d.supported_input_configs().map(|c| c.collect()).unwrap_or_default()
    } else {
        d.supported_output_configs().map(|c| c.collect()).unwrap_or_default()
    };
    let rates =
        COMMON_RATES.iter().copied().filter(|r| ranges.iter().any(|c| c.min_sample_rate() <= *r && *r <= c.max_sample_rate())).collect();
    DeviceInfo {
        is_default: default.as_deref() == Some(name.as_str()),
        max_channels: ranges.iter().map(|c| c.channels()).max().unwrap_or(0),
        rates,
        name,
    }
}

pub fn outputs() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let def = host.default_output_device().map(|d| dev_name(&d));
    host.output_devices().map(|it| it.map(|d| info(&d, &def, false)).collect()).unwrap_or_default()
}

pub fn inputs() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let def = host.default_input_device().map(|d| dev_name(&d));
    host.input_devices().map(|it| it.map(|d| info(&d, &def, true)).collect()).unwrap_or_default()
}

/// Output device by name, or the system default when `name` is None.
/// Returns (device, warning) — the warning explains a fallback to default.
pub fn find_output(name: Option<&str>) -> Result<(cpal::Device, Option<String>), String> {
    let host = cpal::default_host();
    if let Some(n) = name {
        if let Some(d) = host.output_devices().ok().and_then(|mut it| it.find(|d| dev_name(d) == n)) {
            return Ok((d, None));
        }
        let d = host.default_output_device().ok_or_else(|| format!("output '{n}' not found and no default output device"))?;
        return Ok((d, Some(format!("'{n}' is not connected — using system default"))));
    }
    host.default_output_device().map(|d| (d, None)).ok_or_else(|| "no default output device".into())
}

pub fn find_input(name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match name {
        Some(n) => host
            .input_devices()
            .ok()
            .and_then(|mut it| it.find(|d| dev_name(d) == n))
            .ok_or_else(|| format!("audio input '{n}' is not connected")),
        None => host.default_input_device().ok_or_else(|| "no audio input device".into()),
    }
}

pub fn device_name(d: &cpal::Device) -> String {
    dev_name(d)
}

pub fn midi_ports() -> Vec<String> {
    midir::MidiInput::new("VC Sound Design probe")
        .map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
        .unwrap_or_default()
}

/// Per-user preferences (not per project: devices belong to the machine).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Prefs {
    /// None = follow the system default output.
    #[serde(default)]
    pub output_device: Option<String>,
    #[serde(default)]
    pub input_device: Option<String>,
    /// MIDI input to connect at startup; None = no MIDI.
    #[serde(default)]
    pub midi_port: Option<String>,
    /// Window/panel layout owned by the UI (opaque JSON object).
    #[serde(default)]
    pub ui_layout: serde_json::Value,
}

/// `$VCSD_CONFIG_DIR`, else the platform config folder.
pub fn default_prefs_path() -> PathBuf {
    let dir = if let Ok(d) = std::env::var("VCSD_CONFIG_DIR") {
        PathBuf::from(d)
    } else if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into())).join("VC Sound Design")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join("Library/Application Support/VC Sound Design")
    } else {
        std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config"))
            .join("vc-sound-design")
    };
    dir.join("prefs.json")
}

impl Prefs {
    pub fn load(path: &std::path::Path) -> Prefs {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap_or_default()).map_err(|e| e.to_string())
    }
}

/// Merge `patch` into `base`: objects merge recursively, `null` deletes a key,
/// anything else replaces. Used for the UI layout so independent parts of the
/// UI (panels, per-project timeline views) can save without clobbering each other.
pub fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base.as_object_mut(), patch.as_object()) {
        (Some(b), Some(p)) => {
            for (k, v) in p {
                if v.is_null() {
                    b.remove(k);
                } else if v.is_object() && b.get(k).is_some_and(|x| x.is_object()) {
                    merge_json(b.get_mut(k).unwrap(), v);
                } else {
                    b.insert(k.clone(), v.clone());
                }
            }
        }
        _ => *base = patch.clone(),
    }
}

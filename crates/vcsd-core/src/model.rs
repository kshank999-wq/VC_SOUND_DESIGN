//! Domain entities (spec §8.1, §17). Every entity carries an immutable,
//! prefixed ID; all positions are integer timeline samples.

use crate::timecode::{FrameRate, TimeBase};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------- project

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub frame_rate: FrameRate,
    pub sample_rate: u32,
    pub bit_depth: u16,
    /// Production/sequence start timecode, frames since midnight. Timeline sample 0.
    pub start_frame: i64,
    #[serde(default)]
    pub editorial_version: String,
    #[serde(default)]
    pub output_profile: String,
    #[serde(default)]
    pub reel: String,
    #[serde(default)]
    pub current_picture_id: Option<String>,
    #[serde(default)]
    pub ai: AiSettings,
    #[serde(default)]
    pub contacts: Vec<Contact>,
    pub schema_version: u32,
    pub created_at: i64,
}

impl Project {
    pub fn timebase(&self) -> TimeBase {
        TimeBase::new(self.frame_rate, self.sample_rate, self.start_frame)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "Untitled".into(),
            frame_rate: FrameRate::FPS_24,
            sample_rate: 48_000,
            bit_depth: 24,
            start_frame: 0,
            editorial_version: String::new(),
            output_profile: String::new(),
            reel: String::new(),
            current_picture_id: None,
            ai: AiSettings::default(),
            contacts: vec![],
            schema_version: SCHEMA_VERSION,
            created_at: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contact {
    pub role: String,
    pub name: String,
    #[serde(default)]
    pub email: String,
}

/// AI is opt-in and offline-first (spec §10). Cloud processing needs explicit consent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiSettings {
    pub enabled: bool,
    pub allow_cloud: bool,
    #[serde(default)]
    pub cloud_consent_at: Option<i64>,
}

// ---------------------------------------------------------------- picture

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PictureVersion {
    pub id: String,
    pub label: String,
    pub media_path: String,
    /// Fast content fingerprint (size + sampled SHA-256) used as AI cache key.
    pub fingerprint: String,
    /// Source timecode of the first frame of the media, frames since midnight.
    pub source_start_frame: i64,
    pub duration_frames: i64,
    #[serde(default)]
    pub media_frame_rate: Option<FrameRate>,
    #[serde(default)]
    pub has_reference_audio: bool,
    #[serde(default)]
    pub previous_id: Option<String>,
    #[serde(default)]
    pub change_notes: String,
    pub imported_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    pub number: String,
    pub title: String,
    pub start_sample: i64,
    pub end_sample: i64,
    #[serde(default)]
    pub picture_id: Option<String>,
    #[serde(default)]
    pub handle_samples: i64,
    /// External ID from VC Film Studio, when imported.
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub next_cue_seq: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptReference {
    pub id: String,
    pub scene_id: String,
    pub external_id: Option<String>,
    pub order: u32,
    pub character: Option<String>,
    pub text: String,
}

// ---------------------------------------------------------------- categories

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Footsteps,
    Props,
    HardFx,
    Background,
    DesignFx,
    Dialogue,
    Music,
    Custom(String),
}

impl Category {
    pub fn parse(s: &str) -> Category {
        match s.trim().to_ascii_lowercase().replace([' ', '-', '/'], "_").as_str() {
            "footsteps" | "foley" | "feet" | "steps" => Category::Footsteps,
            "props" | "prop" | "foley_props" => Category::Props,
            "fx" | "hard_fx" | "hardfx" | "effects" | "sfx" => Category::HardFx,
            "bg" | "background" | "backgrounds" | "ambience" | "atmos" | "ambiance" | "environment" => Category::Background,
            "design" | "design_fx" | "designfx" | "special_effects" => Category::DesignFx,
            "dialogue" | "dia" | "dx" | "dialogue_notes" => Category::Dialogue,
            "music" | "mus" | "score" => Category::Music,
            other => Category::Custom(other.to_string()),
        }
    }

    /// Short department prefix used in track labels and cue codes.
    pub fn prefix(&self) -> String {
        match self {
            Category::Footsteps => "FOLEY".into(),
            Category::Props => "PROPS".into(),
            Category::HardFx => "FX".into(),
            Category::Background => "BG".into(),
            Category::DesignFx => "DESIGN".into(),
            Category::Dialogue => "DIA".into(),
            Category::Music => "MUS".into(),
            Category::Custom(s) => s.to_ascii_uppercase(),
        }
    }

    pub fn code(&self) -> String {
        match self {
            Category::Footsteps => "FST".into(),
            Category::Props => "PRP".into(),
            Category::HardFx => "FX".into(),
            Category::Background => "BG".into(),
            Category::DesignFx => "DFX".into(),
            Category::Dialogue => "DIA".into(),
            Category::Music => "MUS".into(),
            Category::Custom(s) => s.chars().filter(|c| c.is_ascii_alphanumeric()).take(4).collect::<String>().to_ascii_uppercase(),
        }
    }

    pub fn display(&self) -> String {
        match self {
            Category::Footsteps => "Foley / Footsteps".into(),
            Category::Props => "Foley / Props".into(),
            Category::HardFx => "Hard FX".into(),
            Category::Background => "Backgrounds / Atmos".into(),
            Category::DesignFx => "Design FX".into(),
            Category::Dialogue => "Dialogue notes".into(),
            Category::Music => "Music".into(),
            Category::Custom(s) => s.clone(),
        }
    }

    /// Default department bus name.
    pub fn default_bus(&self) -> &'static str {
        match self {
            Category::Footsteps | Category::Props => "Foley",
            Category::HardFx => "Hard FX",
            Category::Background => "Backgrounds/Atmos",
            Category::DesignFx | Category::Custom(_) => "Design FX",
            Category::Dialogue => "Dialogue Reference",
            Category::Music => "Music/Score",
        }
    }

    /// Whether range cues of this category are naturally filled by looping.
    pub fn loops_naturally(&self) -> bool {
        matches!(self, Category::Background | Category::Music)
    }

    pub fn default_channels(&self) -> u16 {
        if matches!(self, Category::Background | Category::Music) {
            2
        } else {
            1
        }
    }
}

// ---------------------------------------------------------------- identities

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VariationMode {
    #[default]
    RoundRobin,
    /// Deterministic pseudo-random (seeded by cue/event id) — renders are reproducible.
    Random,
    /// Always use the locked asset.
    Locked,
    /// Alternate between left/right pools (footsteps).
    AlternateFeet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundIdentity {
    pub id: String,
    /// e.g. "Greg / Boots / Gravel"
    pub name: String,
    pub category: Category,
    #[serde(default)]
    pub character: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub prop: Option<String>,
    #[serde(default)]
    pub default_asset_id: Option<String>,
    #[serde(default)]
    pub variation_pool: Vec<String>,
    /// Second pool for AlternateFeet (right foot); `variation_pool` is left.
    #[serde(default)]
    pub alt_pool: Vec<String>,
    #[serde(default)]
    pub variation_mode: VariationMode,
    /// Optional shared bus routing for all instances of this identity.
    #[serde(default)]
    pub bus_id: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub external_id: Option<String>,
}

impl SoundIdentity {
    pub fn label_parts(&self) -> Vec<String> {
        self.name.split(['/', '•', '|']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    }
}

// ---------------------------------------------------------------- markers / cues

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Point,
    Range,
    Repeated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerLane {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub order: u32,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub id: String,
    pub lane_id: String,
    pub scene_id: String,
    pub kind: EventType,
    pub start_sample: i64,
    pub end_sample: i64,
    pub label: String,
    #[serde(default)]
    pub identity_id: Option<String>,
    /// Tapped or inferred sub-events of a repeated-event marker (timeline samples).
    #[serde(default)]
    pub sub_events: Vec<i64>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub cue_id: Option<String>,
    #[serde(default)]
    pub origin: TimingOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimingOrigin {
    #[default]
    Manual,
    AiSuggested,
    MidiPerformed,
    Recorded,
    Imported,
    Inferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[default]
    Unreviewed,
    Accepted,
    Rejected,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CueStatus {
    /// From script only: no picture timing yet.
    Conceptual,
    #[default]
    Spotted,
    Placeholder,
    Replaced,
    NeedsRecording,
    Recorded,
    Approved,
    Missing,
    Omitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cue {
    pub id: String,
    pub scene_id: String,
    /// Human-readable code, e.g. `12-FST-003`. Unique within the project.
    pub code: String,
    #[serde(default)]
    pub identity_id: Option<String>,
    pub category: Category,
    pub event_type: EventType,
    #[serde(default)]
    pub lane_id: Option<String>,
    pub start_sample: i64,
    pub end_sample: i64,
    #[serde(default)]
    pub sub_events: Vec<i64>,
    pub description: String,
    #[serde(default)]
    pub intent: String,
    #[serde(default)]
    pub script_ref_id: Option<String>,
    #[serde(default)]
    pub character: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub prop: Option<String>,
    /// Placeholder asset placed by the auto-track builder.
    #[serde(default)]
    pub placeholder_asset_id: Option<String>,
    /// Scene-local override of the identity's default asset.
    #[serde(default)]
    pub asset_override_id: Option<String>,
    #[serde(default)]
    pub timing_origin: TimingOrigin,
    #[serde(default)]
    pub review_status: ReviewStatus,
    #[serde(default)]
    pub status: CueStatus,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub marker_id: Option<String>,
    #[serde(default)]
    pub picture_id: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    /// Proposed timing shift from a reconform, awaiting user review.
    #[serde(default)]
    pub pending_shift: Option<i64>,
}

impl Cue {
    pub fn duration(&self) -> i64 {
        (self.end_sample - self.start_sample).max(0)
    }
    /// Interval used for "active at" queries; point events cover one frame.
    pub fn covers(&self, s: i64, one_frame: i64) -> bool {
        let end = if self.end_sample > self.start_sample { self.end_sample } else { self.start_sample + one_frame };
        s >= self.start_sample && s < end
    }
}

// ---------------------------------------------------------------- assets

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Placeholder,
    #[default]
    Library,
    Recorded,
    Rendered,
    Imported,
    Score,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct License {
    pub name: String,
    #[serde(default)]
    pub redistributable: bool,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub attribution: String,
    /// Temp/reference material that must not be baked into final deliverables.
    #[serde(default)]
    pub temp_reference: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub length_samples: i64,
    pub kind: AssetKind,
    #[serde(default)]
    pub category: Option<Category>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default)]
    pub pack_id: Option<String>,
    /// BWF time reference (samples since midnight) when present.
    #[serde(default)]
    pub time_reference: Option<u64>,
    #[serde(default)]
    pub peak_dbfs: Option<f32>,
    #[serde(default)]
    pub loopable: bool,
    pub imported_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundPack {
    pub id: String,
    pub name: String,
    pub version: String,
    pub root: String,
    pub license: License,
    #[serde(default)]
    pub placeholder: bool,
}

// ---------------------------------------------------------------- tracks / buses / clips

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bus {
    pub id: String,
    pub name: String,
    pub order: u32,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub mute: bool,
    /// Rendered as its own stem on export.
    #[serde(default = "yes")]
    pub stem: bool,
    #[serde(default)]
    pub inserts: Vec<Insert>,
    #[serde(default)]
    pub automation_mode: AutoMode,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub scene_id: Option<String>,
    #[serde(default)]
    pub identity_id: Option<String>,
    pub category: Category,
    pub bus_id: String,
    pub channels: u16,
    pub order: u32,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub armed: bool,
    #[serde(default)]
    pub frozen_asset_id: Option<String>,
    #[serde(default)]
    pub inserts: Vec<Insert>,
    #[serde(default)]
    pub automation_mode: AutoMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FillMode {
    /// Play source once; silence after it ends.
    #[default]
    OneShot,
    /// Loop the source region with a short crossfade to fill the clip length.
    Loop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetHistoryEntry {
    pub asset_id: String,
    pub origin: String,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipInstance {
    pub id: String,
    pub track_id: String,
    #[serde(default)]
    pub cue_id: Option<String>,
    pub asset_id: String,
    pub timeline_start: i64,
    /// Offset into the source asset (slip).
    #[serde(default)]
    pub source_offset: i64,
    pub length: i64,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub fade_in: i64,
    #[serde(default)]
    pub fade_out: i64,
    #[serde(default)]
    pub fill: FillMode,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub take_id: Option<String>,
    #[serde(default)]
    pub performance_event_id: Option<String>,
    /// Index of the sub-event (repeated cues) this clip realises.
    #[serde(default)]
    pub sub_index: Option<u32>,
    #[serde(default)]
    pub is_placeholder: bool,
    /// Every asset this clip has used, oldest first (spec §8.3 history).
    #[serde(default)]
    pub history: Vec<AssetHistoryEntry>,
}

impl ClipInstance {
    pub fn end(&self) -> i64 {
        self.timeline_start + self.length
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub id: String,
    /// Track or bus (incl. the master bus) this breakpoint automates, or a
    /// plugin slot for plugin parameters.
    /// Projects saved before bus automation used `track_id`.
    #[serde(alias = "track_id")]
    pub owner_id: String,
    pub param: AutomationParam,
    pub sample: i64,
    pub value: f32,
}

/// How a track/bus responds to its controls during playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoMode {
    /// Play existing automation; control moves change the static setting.
    #[default]
    Read,
    /// Write while a control is held; on release return to the existing automation.
    Touch,
    /// Start writing on first touch and keep the last value until stop.
    Latch,
    /// Write the control value for the whole pass, from play to stop.
    Write,
}

/// What an automation lane controls. Serialised as `"volume_db"`, `"pan"`, or
/// `"plugin:<parameter id>"` (owner = the plugin slot; values normalised 0–1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutomationParam {
    VolumeDb,
    Pan,
    Plugin(u32),
}

impl AutomationParam {
    pub fn key(&self) -> String {
        match self {
            AutomationParam::VolumeDb => "volume_db".into(),
            AutomationParam::Pan => "pan".into(),
            AutomationParam::Plugin(id) => format!("plugin:{id}"),
        }
    }
    pub fn parse(s: &str) -> Option<AutomationParam> {
        match s {
            "volume_db" => Some(AutomationParam::VolumeDb),
            "pan" => Some(AutomationParam::Pan),
            _ => s.strip_prefix("plugin:").and_then(|n| n.parse().ok()).map(AutomationParam::Plugin),
        }
    }
}

impl Serialize for AutomationParam {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.key())
    }
}

impl<'de> Deserialize<'de> for AutomationParam {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        AutomationParam::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown automation parameter '{s}'")))
    }
}

/// Insert processors: built-in EQ, dynamics and filters (spec §12), or a
/// hosted VST3/AU plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Insert {
    Gain { db: f32 },
    HighPass { hz: f32, q: f32 },
    LowPass { hz: f32, q: f32 },
    Peak { hz: f32, q: f32, db: f32 },
    Compressor { threshold_db: f32, ratio: f32, attack_ms: f32, release_ms: f32, makeup_db: f32 },
    Plugin(PluginInsert),
}

/// A third-party plugin in an insert slot. Everything needed to recreate it
/// on another machine travels with the project; if the plugin is not
/// installed there, the slot is kept (state intact) and passed through.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginInsert {
    /// Slot id (unique in the project).
    pub id: String,
    /// Scanner id: `vst3:<class id>` or `au:<type>:<subtype>:<manufacturer>`.
    pub plugin_id: String,
    pub format: String,
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    /// Where it was found when inserted (a hint; the id is what matters).
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub bypass: bool,
    /// Opaque plugin state (base64), captured when the project is saved.
    #[serde(default)]
    pub state: Option<String>,
    /// Parameter values set from the app (normalised 0–1), applied after `state`.
    #[serde(default, with = "param_map")]
    pub params: std::collections::BTreeMap<u32, f64>,
}

/// Parameter ids as JSON object keys. (Serde can't parse numeric keys inside
/// an internally tagged enum such as [`Insert`], so this is done by hand.)
mod param_map {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(m: &BTreeMap<u32, f64>, s: S) -> Result<S::Ok, S::Error> {
        m.iter().map(|(k, v)| (k.to_string(), *v)).collect::<BTreeMap<String, f64>>().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<u32, f64>, D::Error> {
        BTreeMap::<String, f64>::deserialize(d)?
            .into_iter()
            .map(|(k, v)| k.parse().map(|k| (k, v)).map_err(|_| serde::de::Error::custom(format!("bad parameter id '{k}'"))))
            .collect()
    }
}

impl Insert {
    pub fn as_plugin(&self) -> Option<&PluginInsert> {
        match self {
            Insert::Plugin(p) => Some(p),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- takes / performance

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Take {
    pub id: String,
    #[serde(default)]
    pub cue_id: Option<String>,
    pub track_id: String,
    pub asset_id: String,
    pub number: u32,
    #[serde(default)]
    pub slate: String,
    /// Timeline sample where the take's first sample belongs.
    pub timeline_start: i64,
    /// Explicit sync offset applied (latency compensation etc.).
    #[serde(default)]
    pub sync_offset: i64,
    #[serde(default)]
    pub notes: String,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Performance {
    pub id: String,
    pub track_id: String,
    #[serde(default)]
    pub cue_id: Option<String>,
    pub take: u32,
    pub pad_map: crate::perform::PadMap,
    pub latency_compensation: i64,
    #[serde(default)]
    pub quantize_frames: Option<u32>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceEvent {
    pub id: String,
    pub performance_id: String,
    pub pad: String,
    /// Raw captured timeline sample (before latency compensation).
    pub raw_sample: i64,
    /// Placed timeline sample.
    pub sample: i64,
    pub velocity: u8,
    pub index: u32,
}

// ---------------------------------------------------------------- AI

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProposal {
    pub id: String,
    pub cue_id: String,
    pub assistant: String,
    pub picture_fingerprint: String,
    pub start_sample: i64,
    pub end_sample: i64,
    #[serde(default)]
    pub sub_events: Vec<i64>,
    pub confidence: f32,
    pub reason: String,
    pub status: ProposalStatus,
    pub created_at: i64,
    /// Snapshot of the cue timing at proposal time (before/after compare).
    pub before_start: i64,
    pub before_end: i64,
    #[serde(default)]
    pub before_sub_events: Vec<i64>,
}

// ---------------------------------------------------------------- score

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoMark {
    pub bar: u32,
    pub beat: u32,
    pub sample: i64,
    pub bpm: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreImport {
    pub id: String,
    pub title: String,
    pub asset_id: String,
    pub declared_start_tc: String,
    pub declared_frame_rate: FrameRate,
    pub declared_sample_rate: u32,
    pub timeline_start: i64,
    pub track_id: String,
    #[serde(default)]
    pub tempo_map: Vec<TempoMark>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub imported_at: i64,
}

// ---------------------------------------------------------------- audit / export

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub seq: u64,
    pub at: i64,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportJob {
    pub id: String,
    pub profile: String,
    pub output_dir: String,
    pub created_at: i64,
    pub file_count: usize,
    pub qa_errors: usize,
    pub qa_warnings: usize,
}

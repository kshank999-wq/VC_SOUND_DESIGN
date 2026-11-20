export type Category = string | { custom: string };
export type EventType = "point" | "range" | "repeated";

export interface ProjectInfo {
  name: string;
  reel: string;
  fps_num: number;
  fps_den: number;
  drop_frame: boolean;
  fps_label: string;
  sample_rate: number;
  start_frame: number;
  ai_enabled: boolean;
}
export interface Scene { id: string; number: string; title: string; start_sample: number; end_sample: number }
export interface Lane { id: string; name: string; category: Category; order: number; color: string }
export interface Marker {
  id: string; lane_id: string; scene_id: string; kind: EventType; start_sample: number; end_sample: number;
  label: string; identity_id: string | null; sub_events: number[]; cue_id: string | null;
}
export interface Cue {
  id: string; scene_id: string; code: string; identity_id: string | null; category: Category; event_type: EventType;
  lane_id: string | null; start_sample: number; end_sample: number; sub_events: number[]; description: string;
  intent: string; notes: string; owner: string; status: string; priority: string; timing_origin: string;
  review_status: string; placeholder_asset_id: string | null; asset_override_id: string | null; marker_id: string | null;
  pending_shift: number | null; script_ref_id: string | null;
}
export interface ChartRow {
  cue_id: string; code: string; scene: string; category: string; identity: string; description: string;
  event_type: EventType; start_tc: string; end_tc: string; duration_tc: string; start_sample: number; end_sample: number;
  sub_events: number; track: string; media: string; source_status: string; status: string; owner: string; notes: string;
  media_missing: boolean; timing_origin: string; priority: string;
}
export interface Identity {
  id: string; name: string; category: Category; character: string | null; surface: string | null; prop: string | null;
  default_asset_id: string | null; variation_pool: string[]; variation_mode: string;
}
export type Insert =
  | { type: "gain"; db: number }
  | { type: "high_pass"; hz: number; q: number }
  | { type: "low_pass"; hz: number; q: number }
  | { type: "peak"; hz: number; q: number; db: number }
  | { type: "compressor"; threshold_db: number; ratio: number; attack_ms: number; release_ms: number; makeup_db: number }
  | PluginSlot;
/** A hosted VST3/AU plugin in an insert slot. */
export interface PluginSlot {
  type: "plugin"; id: string; plugin_id: string; format: string; name: string; vendor: string; path: string;
  bypass: boolean; state: string | null; params: Record<string, number>;
}
export interface PluginInfo { id: string; format: string; name: string; vendor: string; version: string; category: string; path: string }
export interface PluginParam {
  id: number; title: string; units: string; step_count: number; default: number; value: number; display: string;
  automatable: boolean; read_only: boolean; is_bypass: boolean;
}
export interface PluginStatus { loaded: boolean; error: string | null; latency: number }
export interface Track {
  id: string; name: string; scene_id: string | null; identity_id: string | null; category: Category; bus_id: string;
  channels: number; order: number; folder: string; gain_db: number; pan: number; mute: boolean; solo: boolean; armed: boolean;
  inserts: Insert[];
  automation_mode?: "read" | "touch" | "latch" | "write";
}
export interface Bus { id: string; name: string; order: number; parent_id: string | null; gain_db: number; mute: boolean; stem: boolean; inserts: Insert[]; automation_mode?: "read" | "touch" | "latch" | "write" }
export interface HistoryEntry { asset_id: string; origin: string; at: number }
export interface Clip {
  id: string; track_id: string; cue_id: string | null; asset_id: string; timeline_start: number; source_offset: number;
  length: number; gain_db: number; fade_in: number; fade_out: number; fill: "one_shot" | "loop"; muted: boolean;
  take_id: string | null; performance_event_id: string | null; sub_index: number | null; is_placeholder: boolean;
  history: HistoryEntry[];
}
export interface Asset {
  id: string; name: string; path: string; kind: string; category: Category | null; sample_rate: number; channels: number;
  length: number; tags: string[]; license: string | null; redistributable: boolean | null; loopable: boolean; missing: boolean;
}
export interface Take { id: string; cue_id: string | null; track_id: string; asset_id: string; number: number; slate: string; timeline_start: number }
export interface Proposal {
  id: string; cue_id: string; assistant: string; start_sample: number; end_sample: number; sub_events: number[];
  confidence: number; reason: string; status: "pending" | "accepted" | "rejected"; before_start: number;
}
export interface ScriptLine { id: string; scene_id: string; order: number; character: string | null; text: string }
export interface Binding { lane_id: string; kind: EventType; label: string; identity_id: string | null }
export interface ProjectView {
  open: boolean;
  version: number;
  root?: string;
  dirty?: boolean;
  undo?: string | null;
  redo?: string | null;
  project: ProjectInfo;
  picture: { id: string; label: string; path: string; start_frame: number; duration_frames: number } | null;
  pictures: { id: string; label: string }[];
  scenes: Scene[];
  lanes: Lane[];
  markers: Marker[];
  cues: Cue[];
  chart: ChartRow[];
  identities: Identity[];
  tracks: Track[];
  automation?: { id: string; owner_id: string; param: "volume_db" | "pan"; sample: number; value: number }[];
  buses: Bus[];
  clips: Clip[];
  assets: Asset[];
  takes: Take[];
  performances: { id: string; track_id: string; cue_id: string | null; take: number }[];
  proposals: Proposal[];
  scores: { id: string; title: string; timeline_start: number; warnings: string[] }[];
  script: ScriptLine[];
  exports: { id: string; profile: string; output_dir: string; file_count: number; qa_errors: number; qa_warnings: number }[];
  bindings: Record<string, Binding>;
  /** Live plugin instances by slot id. */
  plugins?: Record<string, PluginStatus>;
}
export interface TransportState {
  playing: boolean;
  position: number;
  dropouts: number;
  load: number;
  master_peak: number;
  meters: Record<string, number>;
  device: string;
  device_rate: number;
  output_latency: number;
  loop_range: [number, number] | null;
  previewing: boolean;
  device_warning: string | null;
  version: number;
  recording: string | null;
  mode: "spot" | "perform";
  perf_triggers: number | null;
  open_spots: string[];
  writing?: { owner_id: string; param: "volume_db" | "pan"; mode: string; touching: boolean; value: number | null }[];
}
export interface QaIssue { severity: "error" | "warning" | "info"; code: string; message: string; entity?: string }
export type Selection =
  | { kind: "cue"; id: string }
  | { kind: "clip"; id: string }
  | { kind: "marker"; id: string }
  | { kind: "track"; id: string }
  | { kind: "folder"; id: string }
  | null;

const CAT_KEYS = ["footsteps", "props", "hard_fx", "background", "design_fx", "dialogue", "music"] as const;
const CAT_LABEL: Record<string, string> = {
  footsteps: "Foley / Footsteps",
  props: "Foley / Props",
  hard_fx: "Hard FX",
  background: "Backgrounds / Atmos",
  design_fx: "Design FX",
  dialogue: "Dialogue notes",
  music: "Music",
};
const CAT_COLOR: Record<string, string> = {
  footsteps: "#6aa84f",
  props: "#93c47d",
  hard_fx: "#e06666",
  background: "#6fa8dc",
  design_fx: "#8e7cc3",
  dialogue: "#f1c232",
  music: "#e69138",
};
export const CATEGORIES = CAT_KEYS;
export function catKey(c: Category | null | undefined): string {
  if (!c) return "custom";
  return typeof c === "string" ? c : "custom";
}
export function catLabel(c: Category | null | undefined): string {
  if (!c) return "—";
  return typeof c === "string" ? CAT_LABEL[c] ?? c : c.custom;
}
export function catColor(c: Category | null | undefined): string {
  return CAT_COLOR[catKey(c)] ?? "#999999";
}

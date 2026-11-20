//! Scenes, marker lanes, spotting markers, live keyboard/MIDI spotting,
//! sound identities and marker → cue conversion (spec §6.2, §8).

use crate::error::{invalid, not_found, Result};
use crate::ids::new_id;
use crate::model::*;
use crate::state::ProjectState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn create_default_lanes(s: &mut ProjectState) {
    let lanes = [
        ("Foley • Footsteps", Category::Footsteps, "#4e9a06"),
        ("Foley • Props", Category::Props, "#73d216"),
        ("Hard FX", Category::HardFx, "#cc0000"),
        ("Backgrounds / Atmos", Category::Background, "#3465a4"),
        ("Design FX", Category::DesignFx, "#75507b"),
        ("Dialogue notes", Category::Dialogue, "#c4a000"),
        ("Music", Category::Music, "#ce5c00"),
    ];
    for (i, (name, cat, color)) in lanes.into_iter().enumerate() {
        add_lane(s, name, cat, color, i as u32);
    }
}

pub fn add_lane(s: &mut ProjectState, name: &str, category: Category, color: &str, order: u32) -> String {
    let id = new_id("lan");
    s.lanes.insert(id.clone(), MarkerLane { id: id.clone(), name: name.into(), category, order, color: color.into(), hidden: false });
    id
}

// ---------------------------------------------------------------- scenes

pub fn add_scene(s: &mut ProjectState, number: &str, title: &str, start: i64, end: i64) -> Result<String> {
    if end <= start {
        return Err(invalid("scene end must be after start"));
    }
    if s.scenes.values().any(|sc| sc.number.eq_ignore_ascii_case(number)) {
        return Err(crate::error::Error::Conflict(format!("scene number {number} already exists")));
    }
    let id = new_id("scn");
    s.scenes.insert(
        id.clone(),
        Scene {
            id: id.clone(),
            number: number.into(),
            title: title.into(),
            start_sample: start,
            end_sample: end,
            picture_id: s.project.current_picture_id.clone(),
            handle_samples: 0,
            external_id: None,
            next_cue_seq: 1,
        },
    );
    Ok(id)
}

// ---------------------------------------------------------------- identities

pub struct NewIdentity {
    pub name: String,
    pub category: Category,
    pub character: Option<String>,
    pub surface: Option<String>,
    pub prop: Option<String>,
}

pub fn add_identity(s: &mut ProjectState, n: NewIdentity) -> Result<String> {
    if let Ok(existing) = s.find_identity(&n.name) {
        return Err(crate::error::Error::Conflict(format!("identity '{}' already exists ({})", n.name, existing.id)));
    }
    let id = new_id("idn");
    s.identities.insert(
        id.clone(),
        SoundIdentity {
            id: id.clone(),
            name: n.name,
            category: n.category,
            character: n.character,
            surface: n.surface,
            prop: n.prop,
            default_asset_id: None,
            variation_pool: vec![],
            alt_pool: vec![],
            variation_mode: VariationMode::RoundRobin,
            bus_id: None,
            notes: String::new(),
            external_id: None,
        },
    );
    Ok(id)
}

/// Assign the identity's default sound (and optionally its variation pool).
pub fn assign_identity_sound(
    s: &mut ProjectState,
    identity_id: &str,
    default_asset: &str,
    pool: Vec<String>,
    mode: VariationMode,
) -> Result<()> {
    s.asset(default_asset)?;
    for a in &pool {
        s.asset(a)?;
    }
    let i = s.identities.get_mut(identity_id).ok_or_else(|| not_found("identity", identity_id))?;
    i.default_asset_id = Some(default_asset.to_string());
    i.variation_pool = pool;
    i.variation_mode = mode;
    Ok(())
}

/// "Show all Greg's boots cues": every cue of an identity across all scenes.
pub fn cues_for_identity<'a>(s: &'a ProjectState, identity_id: &str) -> Vec<&'a Cue> {
    let mut v: Vec<&Cue> = s.cues.values().filter(|c| c.identity_id.as_deref() == Some(identity_id)).collect();
    v.sort_by(|a, b| (a.start_sample, &a.code).cmp(&(b.start_sample, &b.code)));
    v
}

// ---------------------------------------------------------------- markers

pub struct NewMarker {
    pub lane_id: String,
    pub scene_id: Option<String>,
    pub kind: EventType,
    pub start: i64,
    pub end: i64,
    pub label: String,
    pub identity_id: Option<String>,
    pub sub_events: Vec<i64>,
    pub origin: TimingOrigin,
}

pub fn add_marker(s: &mut ProjectState, m: NewMarker) -> Result<String> {
    if !s.lanes.contains_key(&m.lane_id) {
        return Err(not_found("marker lane", &m.lane_id));
    }
    if let Some(i) = &m.identity_id {
        s.identity(i)?;
    }
    let end = if m.kind == EventType::Point { m.start } else { m.end };
    if end < m.start {
        return Err(invalid("marker end precedes start"));
    }
    let scene_id = match m.scene_id {
        Some(id) => s.scene(&id)?.id.clone(),
        None => {
            s.scene_at(m.start).map(|sc| sc.id.clone()).ok_or_else(|| invalid("marker is not inside any scene; create a scene first"))?
        }
    };
    let mut subs = m.sub_events;
    subs.sort();
    let id = new_id("mrk");
    s.markers.insert(
        id.clone(),
        Marker {
            id: id.clone(),
            lane_id: m.lane_id,
            scene_id,
            kind: m.kind,
            start_sample: m.start,
            end_sample: end,
            label: m.label,
            identity_id: m.identity_id,
            sub_events: subs,
            approved: true,
            cue_id: None,
            origin: m.origin,
        },
    );
    Ok(id)
}

/// Edit a marker's timing; the linked cue (if any) follows (spec §6.2).
pub fn update_marker(s: &mut ProjectState, id: &str, start: i64, end: i64, sub_events: Option<Vec<i64>>) -> Result<()> {
    let m = s.markers.get_mut(id).ok_or_else(|| not_found("marker", id))?;
    let end = if m.kind == EventType::Point { start } else { end };
    if end < start {
        return Err(invalid("marker end precedes start"));
    }
    let delta = start - m.start_sample;
    m.start_sample = start;
    m.end_sample = end;
    if let Some(se) = &sub_events {
        m.sub_events = se.clone();
    }
    if let Some(cid) = m.cue_id.clone() {
        let cue = s.cue_mut(&cid)?;
        cue.start_sample = start;
        cue.end_sample = end;
        if let Some(se) = sub_events {
            cue.sub_events = se;
        }
        for c in s.clips.values_mut().filter(|c| c.cue_id.as_deref() == Some(cid.as_str()) && c.sub_index.is_none()) {
            c.timeline_start += delta;
        }
    }
    Ok(())
}

/// Next human cue code within a scene, e.g. `12-FST-004`.
pub fn next_cue_code(s: &mut ProjectState, scene_id: &str, cat: &Category) -> Result<String> {
    let sc = s.scenes.get_mut(scene_id).ok_or_else(|| not_found("scene", scene_id))?;
    let seq = sc.next_cue_seq.max(1);
    sc.next_cue_seq = seq + 1;
    Ok(format!("{}-{}-{:03}", sc.number, cat.code(), seq))
}

/// Convert approved markers without cues into cue records. Returns new cue IDs.
pub fn markers_to_cues(s: &mut ProjectState, scene_id: Option<&str>) -> Result<Vec<String>> {
    let mut ids: Vec<(i64, String)> = s
        .markers
        .values()
        .filter(|m| m.approved && m.cue_id.is_none() && scene_id.is_none_or(|sc| m.scene_id == sc))
        .map(|m| (m.start_sample, m.id.clone()))
        .collect();
    ids.sort();
    let mut out = vec![];
    for (_, mid) in ids {
        let m = s.markers[&mid].clone();
        let lane = s.lanes.get(&m.lane_id).ok_or_else(|| not_found("lane", &m.lane_id))?.clone();
        let ident = m.identity_id.as_ref().and_then(|i| s.identities.get(i)).cloned();
        let category = ident.as_ref().map(|i| i.category.clone()).unwrap_or(lane.category.clone());
        let code = next_cue_code(s, &m.scene_id, &category)?;
        let id = new_id("cue");
        s.cues.insert(
            id.clone(),
            Cue {
                id: id.clone(),
                scene_id: m.scene_id.clone(),
                code,
                identity_id: m.identity_id.clone(),
                category,
                event_type: m.kind,
                lane_id: Some(m.lane_id.clone()),
                start_sample: m.start_sample,
                end_sample: m.end_sample,
                sub_events: m.sub_events.clone(),
                description: m.label.clone(),
                intent: String::new(),
                script_ref_id: None,
                character: ident.as_ref().and_then(|i| i.character.clone()),
                surface: ident.as_ref().and_then(|i| i.surface.clone()),
                prop: ident.as_ref().and_then(|i| i.prop.clone()),
                placeholder_asset_id: None,
                asset_override_id: None,
                timing_origin: m.origin,
                review_status: ReviewStatus::Unreviewed,
                status: CueStatus::Spotted,
                priority: Priority::Normal,
                owner: String::new(),
                notes: String::new(),
                marker_id: Some(m.id.clone()),
                picture_id: s.project.current_picture_id.clone(),
                external_id: None,
                pending_shift: None,
            },
        );
        s.markers.get_mut(&mid).unwrap().cue_id = Some(id.clone());
        out.push(id);
    }
    Ok(out)
}

/// Give a script-only (conceptual) cue its picture timing after spotting.
pub fn spot_conceptual_cue(s: &mut ProjectState, cue_id: &str, start: i64, end: i64, kind: EventType) -> Result<()> {
    let one = s.one_frame();
    let scene = s.scene_at(start).map(|x| x.id.clone());
    let c = s.cue_mut(cue_id)?;
    c.start_sample = start;
    c.end_sample = if kind == EventType::Point { start } else { end.max(start + one) };
    c.event_type = kind;
    c.status = CueStatus::Spotted;
    if let Some(sc) = scene {
        c.scene_id = sc;
    }
    Ok(())
}

// ---------------------------------------------------------------- live spotting

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotBinding {
    pub lane_id: String,
    pub kind: EventType,
    pub label: String,
    #[serde(default)]
    pub identity_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpotOutcome {
    Created(String),
    RangeOpened,
    SubEvent(usize),
}

/// Frame-accurate live spotting from hotkeys or MIDI pads while picture plays.
/// Point bindings create a one-frame marker per tap. Range bindings open on
/// the first tap and close on the second. Repeated bindings open a container
/// on the first tap and record every tap as a sub-event until `finish`.
#[derive(Debug, Default)]
pub struct LiveSpotter {
    pub bindings: BTreeMap<String, SpotBinding>,
    open: BTreeMap<String, (i64, Vec<i64>)>,
    /// When true, taps snap to the first sample of their frame.
    pub snap_to_frame: bool,
}

impl LiveSpotter {
    pub fn new() -> Self {
        Self { snap_to_frame: true, ..Default::default() }
    }

    pub fn bind(&mut self, key: &str, b: SpotBinding) {
        self.bindings.insert(key.to_string(), b);
    }

    pub fn is_open(&self, key: &str) -> bool {
        self.open.contains_key(key)
    }

    fn pos(&self, s: &ProjectState, sample: i64) -> i64 {
        if self.snap_to_frame {
            s.tb().snap(sample)
        } else {
            sample
        }
    }

    pub fn tap(&mut self, s: &mut ProjectState, key: &str, sample: i64) -> Result<SpotOutcome> {
        let b = self.bindings.get(key).cloned().ok_or_else(|| not_found("spot binding", key))?;
        let t = self.pos(s, sample);
        match b.kind {
            EventType::Point => Ok(SpotOutcome::Created(self.create(s, &b, t, t, vec![])?)),
            EventType::Range => match self.open.remove(key) {
                None => {
                    self.open.insert(key.to_string(), (t, vec![]));
                    Ok(SpotOutcome::RangeOpened)
                }
                Some((start, _)) => {
                    let (a, z) = if t >= start { (start, t) } else { (t, start) };
                    Ok(SpotOutcome::Created(self.create(s, &b, a, z, vec![])?))
                }
            },
            EventType::Repeated => {
                let e = self.open.entry(key.to_string()).or_insert((t, vec![]));
                e.1.push(t);
                Ok(SpotOutcome::SubEvent(e.1.len()))
            }
        }
    }

    /// Close an open range/repeated container at `end_sample`.
    pub fn finish(&mut self, s: &mut ProjectState, key: &str, end_sample: i64) -> Result<String> {
        let b = self.bindings.get(key).cloned().ok_or_else(|| not_found("spot binding", key))?;
        let (start, subs) = self.open.remove(key).ok_or_else(|| invalid(format!("no open spot for '{key}'")))?;
        let end = self.pos(s, end_sample).max(subs.last().copied().unwrap_or(start));
        self.create(s, &b, start, end, subs)
    }

    /// Handle a raw MIDI message: note-on (velocity > 0) maps to binding `midi:<ch>:<note>`.
    pub fn midi(&mut self, s: &mut ProjectState, msg: &[u8], sample: i64) -> Result<Option<SpotOutcome>> {
        match MidiNote::parse(msg) {
            Some(n) if n.velocity > 0 => {
                let key = n.key();
                if self.bindings.contains_key(&key) {
                    self.tap(s, &key, sample).map(Some)
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    fn create(&self, s: &mut ProjectState, b: &SpotBinding, start: i64, end: i64, subs: Vec<i64>) -> Result<String> {
        add_marker(
            s,
            NewMarker {
                lane_id: b.lane_id.clone(),
                scene_id: None,
                kind: b.kind,
                start,
                end,
                label: b.label.clone(),
                identity_id: b.identity_id.clone(),
                sub_events: subs,
                origin: TimingOrigin::Manual,
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiNote {
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
}

impl MidiNote {
    /// Parse a note-on message (status 0x9n). Note-off returns velocity 0.
    pub fn parse(msg: &[u8]) -> Option<MidiNote> {
        if msg.len() < 3 {
            return None;
        }
        let status = msg[0] & 0xF0;
        let channel = msg[0] & 0x0F;
        match status {
            0x90 => Some(MidiNote { channel: channel + 1, note: msg[1] & 0x7F, velocity: msg[2] & 0x7F }),
            0x80 => Some(MidiNote { channel: channel + 1, note: msg[1] & 0x7F, velocity: 0 }),
            _ => None,
        }
    }
    pub fn key(&self) -> String {
        format!("midi:{}:{}", self.channel, self.note)
    }
}

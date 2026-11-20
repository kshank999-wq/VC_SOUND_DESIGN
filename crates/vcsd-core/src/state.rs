//! In-memory project state and the editing session (undo/redo, audit, autosave).
//!
//! All mutations go through [`Session::edit`], which snapshots state first so
//! a failed operation leaves the project untouched and every successful one is
//! undoable (spec §2: non-destructive and reversible).

use crate::error::{not_found, Result};
use crate::ids::{new_id, now_unix};
use crate::model::*;
use crate::timecode::{FrameRate, TimeBase};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub type Map<T> = BTreeMap<String, T>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectState {
    pub project: Project,
    #[serde(default)]
    pub pictures: Map<PictureVersion>,
    #[serde(default)]
    pub scenes: Map<Scene>,
    #[serde(default)]
    pub characters: Map<Character>,
    #[serde(default)]
    pub script_refs: Map<ScriptReference>,
    #[serde(default)]
    pub identities: Map<SoundIdentity>,
    #[serde(default)]
    pub lanes: Map<MarkerLane>,
    #[serde(default)]
    pub markers: Map<Marker>,
    #[serde(default)]
    pub cues: Map<Cue>,
    #[serde(default)]
    pub assets: Map<Asset>,
    #[serde(default)]
    pub packs: Map<SoundPack>,
    #[serde(default)]
    pub buses: Map<Bus>,
    #[serde(default)]
    pub tracks: Map<Track>,
    #[serde(default)]
    pub clips: Map<ClipInstance>,
    #[serde(default)]
    pub automation: Map<AutomationPoint>,
    #[serde(default)]
    pub takes: Map<Take>,
    #[serde(default)]
    pub performances: Map<Performance>,
    #[serde(default)]
    pub performance_events: Map<PerformanceEvent>,
    #[serde(default)]
    pub proposals: Map<AiProposal>,
    #[serde(default)]
    pub scores: Map<ScoreImport>,
    #[serde(default)]
    pub exports: Map<ExportJob>,
    #[serde(default)]
    pub audit: Vec<AuditEvent>,
}

pub struct NewProject {
    pub name: String,
    pub frame_rate: FrameRate,
    pub sample_rate: u32,
    pub start_tc: String,
}

impl ProjectState {
    pub fn new(p: NewProject) -> Result<Self> {
        let start_frame = p.frame_rate.tc_to_frames(&p.start_tc)?;
        if !(8_000..=384_000).contains(&p.sample_rate) {
            return Err(crate::error::invalid("sample rate must be between 8 kHz and 384 kHz"));
        }
        let mut s = ProjectState {
            project: Project {
                id: new_id("prj"),
                name: p.name,
                frame_rate: p.frame_rate,
                sample_rate: p.sample_rate,
                start_frame,
                created_at: now_unix(),
                ..Default::default()
            },
            ..Default::default()
        };
        crate::spotting::create_default_lanes(&mut s);
        crate::mixer::create_default_buses(&mut s);
        Ok(s)
    }

    pub fn tb(&self) -> TimeBase {
        self.project.timebase()
    }

    pub fn one_frame(&self) -> i64 {
        let tb = self.tb();
        tb.sample_at_frame(tb.start_frame + 1) - tb.sample_at_frame(tb.start_frame)
    }

    pub fn scene(&self, id: &str) -> Result<&Scene> {
        self.scenes.get(id).ok_or_else(|| not_found("scene", id))
    }
    pub fn cue(&self, id: &str) -> Result<&Cue> {
        self.cues.get(id).ok_or_else(|| not_found("cue", id))
    }
    pub fn cue_mut(&mut self, id: &str) -> Result<&mut Cue> {
        self.cues.get_mut(id).ok_or_else(|| not_found("cue", id))
    }
    pub fn asset(&self, id: &str) -> Result<&Asset> {
        self.assets.get(id).ok_or_else(|| not_found("asset", id))
    }
    pub fn track(&self, id: &str) -> Result<&Track> {
        self.tracks.get(id).ok_or_else(|| not_found("track", id))
    }
    pub fn identity(&self, id: &str) -> Result<&SoundIdentity> {
        self.identities.get(id).ok_or_else(|| not_found("identity", id))
    }
    pub fn clip_mut(&mut self, id: &str) -> Result<&mut ClipInstance> {
        self.clips.get_mut(id).ok_or_else(|| not_found("clip", id))
    }

    /// Resolve a scene by ID or by scene number ("12").
    pub fn find_scene(&self, key: &str) -> Result<&Scene> {
        self.scenes
            .get(key)
            .or_else(|| self.scenes.values().find(|s| s.number.eq_ignore_ascii_case(key)))
            .ok_or_else(|| not_found("scene", key))
    }
    /// Resolve a cue by ID or human code ("12-FST-001").
    pub fn find_cue(&self, key: &str) -> Result<&Cue> {
        self.cues.get(key).or_else(|| self.cues.values().find(|c| c.code.eq_ignore_ascii_case(key))).ok_or_else(|| not_found("cue", key))
    }
    /// Resolve an identity by ID or by (case-insensitive) name.
    pub fn find_identity(&self, key: &str) -> Result<&SoundIdentity> {
        let norm = |s: &str| s.to_ascii_lowercase().split(['/', '•', '|']).map(|p| p.trim().to_string()).collect::<Vec<_>>().join("/");
        self.identities
            .get(key)
            .or_else(|| self.identities.values().find(|i| norm(&i.name) == norm(key)))
            .ok_or_else(|| not_found("identity", key))
    }
    pub fn find_lane(&self, key: &str) -> Result<&MarkerLane> {
        let cat = Category::parse(key);
        self.lanes
            .get(key)
            .or_else(|| self.lanes.values().find(|l| l.name.eq_ignore_ascii_case(key)))
            .or_else(|| {
                let mut v: Vec<_> = self.lanes.values().filter(|l| l.category == cat).collect();
                v.sort_by_key(|l| l.order);
                v.into_iter().next()
            })
            .ok_or_else(|| not_found("marker lane", key))
    }
    /// Resolve an asset by ID, sha256, path, file name, or search words that
    /// match exactly one asset.
    pub fn find_asset(&self, key: &str) -> Result<&Asset> {
        if let Some(a) = self
            .assets
            .get(key)
            .or_else(|| self.assets.values().find(|a| a.sha256 == key || a.path == key))
            .or_else(|| self.assets.values().find(|a| a.name.eq_ignore_ascii_case(key)))
        {
            return Ok(a);
        }
        let hits = crate::library::search(self, &crate::library::Query { text: key.into(), ..Default::default() });
        match hits.as_slice() {
            [one] => Ok(one),
            [] => Err(not_found("asset", key)),
            many => Err(crate::error::invalid(format!(
                "'{key}' matches {} assets ({}…); be more specific",
                many.len(),
                many.iter().take(3).map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
            ))),
        }
    }

    pub fn clips_for_cue(&self, cue_id: &str) -> Vec<&ClipInstance> {
        let mut v: Vec<_> = self.clips.values().filter(|c| c.cue_id.as_deref() == Some(cue_id)).collect();
        v.sort_by(|a, b| (a.timeline_start, &a.id).cmp(&(b.timeline_start, &b.id)));
        v
    }

    /// Scene that contains timeline sample `s`, if any.
    pub fn scene_at(&self, s: i64) -> Option<&Scene> {
        self.scenes.values().find(|sc| s >= sc.start_sample && s < sc.end_sample)
    }

    pub fn current_picture(&self) -> Option<&PictureVersion> {
        self.project.current_picture_id.as_ref().and_then(|id| self.pictures.get(id))
    }

    /// Cues sorted deterministically: start sample, then code, then id.
    pub fn sorted_cues<'a>(&'a self, scene_id: Option<&str>) -> Vec<&'a Cue> {
        let mut v: Vec<&Cue> = self.cues.values().filter(|c| scene_id.is_none_or(|s| c.scene_id == s)).collect();
        v.sort_by(|a, b| (a.start_sample, &a.code, &a.id).cmp(&(b.start_sample, &b.code, &b.id)));
        v
    }

    pub(crate) fn audit(&mut self, action: impl Into<String>) {
        let seq = self.audit.last().map(|a| a.seq + 1).unwrap_or(1);
        self.audit.push(AuditEvent { seq, at: now_unix(), action: action.into() });
    }
}

const UNDO_LIMIT: usize = 200;

/// An open project with undo/redo history and an optional on-disk location.
pub struct Session {
    pub state: ProjectState,
    /// Project folder (contains `Project.vcsd`).
    pub root: Option<PathBuf>,
    undo: Vec<(String, Vec<u8>)>,
    redo: Vec<(String, Vec<u8>)>,
    pub dirty: bool,
}

impl Session {
    pub fn new(state: ProjectState) -> Self {
        Self { state, root: None, undo: vec![], redo: vec![], dirty: true }
    }

    /// Create a project folder with the standard layout (spec §15.3) and save.
    pub fn create(root: &Path, p: NewProject) -> Result<Self> {
        let state = ProjectState::new(p)?;
        crate::layout::create_project_dirs(root)?;
        let mut s = Self::new(state);
        s.root = Some(root.to_path_buf());
        s.save()?;
        Ok(s)
    }

    pub fn open(root: &Path) -> Result<Self> {
        let state = crate::store::load(&crate::layout::project_file(root))?;
        Ok(Self { state, root: Some(root.to_path_buf()), undo: vec![], redo: vec![], dirty: false })
    }

    pub fn save(&mut self) -> Result<()> {
        let root = self.root.clone().ok_or_else(|| crate::error::invalid("session has no project folder"))?;
        crate::store::save(&crate::layout::project_file(&root), &self.state)?;
        let _ = std::fs::remove_file(crate::layout::autosave_file(&root));
        self.dirty = false;
        Ok(())
    }

    /// Write a crash-recovery copy next to the project without touching it.
    pub fn autosave(&self) -> Result<()> {
        if let Some(root) = &self.root {
            crate::store::save(&crate::layout::autosave_file(root), &self.state)?;
        }
        Ok(())
    }

    /// If an autosave newer than the project exists, load it for recovery.
    pub fn recover(root: &Path) -> Result<Option<ProjectState>> {
        let a = crate::layout::autosave_file(root);
        let p = crate::layout::project_file(root);
        if !a.exists() {
            return Ok(None);
        }
        let newer = match (std::fs::metadata(&a)?.modified(), std::fs::metadata(&p).and_then(|m| m.modified())) {
            (Ok(am), Ok(pm)) => am > pm,
            _ => true,
        };
        if newer {
            Ok(Some(crate::store::load(&a)?))
        } else {
            Ok(None)
        }
    }

    /// Run a mutation atomically: on error the state is restored; on success
    /// it becomes one undo step and is recorded in the audit log.
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut ProjectState) -> Result<T>) -> Result<T> {
        let snap = serde_json::to_vec(&self.state)?;
        match f(&mut self.state) {
            Ok(v) => {
                self.undo.push((label.to_string(), snap));
                if self.undo.len() > UNDO_LIMIT {
                    self.undo.remove(0);
                }
                self.redo.clear();
                self.state.audit(label);
                self.dirty = true;
                Ok(v)
            }
            Err(e) => {
                self.state = serde_json::from_slice(&snap)?;
                Err(e)
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn undo(&mut self) -> Result<Option<String>> {
        let Some((label, snap)) = self.undo.pop() else { return Ok(None) };
        let cur = serde_json::to_vec(&self.state)?;
        self.state = serde_json::from_slice(&snap)?;
        self.state.audit(format!("undo: {label}"));
        self.redo.push((label.clone(), cur));
        self.dirty = true;
        Ok(Some(label))
    }

    pub fn redo(&mut self) -> Result<Option<String>> {
        let Some((label, snap)) = self.redo.pop() else { return Ok(None) };
        let cur = serde_json::to_vec(&self.state)?;
        self.state = serde_json::from_slice(&snap)?;
        self.state.audit(format!("redo: {label}"));
        self.undo.push((label.clone(), cur));
        self.dirty = true;
        Ok(Some(label))
    }

    pub fn root(&self) -> Result<&Path> {
        self.root.as_deref().ok_or_else(|| crate::error::invalid("session has no project folder"))
    }
}

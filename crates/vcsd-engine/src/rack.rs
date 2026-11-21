//! Live plugin instances for the project's insert slots.
//!
//! The project holds what each slot *is* (plugin id, saved state, parameter
//! values); the rack holds the running instances. [`Rack::sync`] runs after
//! every edit: it creates instances for new slots (restoring their state),
//! drops removed ones and pushes parameter values that changed in the model
//! (app edits, undo). The render thread only calls [`PluginHost::process`].
//!
//! A slot whose plugin isn't installed keeps its saved state untouched and
//! passes audio through; QA reports it before export.

use base64::Engine as _;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use vcsd_core::mixer::PluginHost;
use vcsd_core::qa::{QaIssue, Severity};
use vcsd_core::wav::AudioBuffer;
use vcsd_core::{Insert, PluginInsert, ProjectState};
use vcsd_plugins::{Instance, PluginInfo, Setup};

struct Slot {
    /// What the instance was made for; a change means a new instance.
    key: String,
    inst: Option<Arc<dyn Instance>>,
    error: Option<String>,
    /// Parameter values last pushed to the instance from the model.
    applied: BTreeMap<u32, f64>,
}

#[derive(Default)]
pub struct Rack {
    slots: Mutex<HashMap<String, Slot>>,
    /// Scanned plugins by id (where to load them from).
    catalog: Mutex<HashMap<String, PluginInfo>>,
    offline: bool,
}

pub fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Every plugin slot in the project with the channel count it processes.
pub fn plugin_slots(st: &ProjectState) -> Vec<(&PluginInsert, usize, String)> {
    let mut v = vec![];
    for t in st.tracks.values() {
        for p in t.inserts.iter().filter_map(Insert::as_plugin) {
            v.push((p, t.channels.clamp(1, 2) as usize, t.name.clone()));
        }
    }
    for b in st.buses.values() {
        for p in b.inserts.iter().filter_map(Insert::as_plugin) {
            v.push((p, 2, b.name.clone()));
        }
    }
    v
}

impl Rack {
    pub fn new() -> Rack {
        Rack::default()
    }

    pub fn set_catalog(&self, plugins: &[PluginInfo]) {
        *self.catalog.lock().unwrap() = plugins.iter().map(|p| (p.id.clone(), p.clone())).collect();
    }

    pub fn info(&self, plugin_id: &str) -> Option<PluginInfo> {
        self.catalog.lock().unwrap().get(plugin_id).cloned()
    }

    fn make(&self, p: &PluginInsert, channels: usize, rate: u32) -> Result<Arc<dyn Instance>, String> {
        // the scan knows where it lives now; fall back to where it was when inserted
        let info = self.info(&p.plugin_id).unwrap_or_else(|| PluginInfo {
            id: p.plugin_id.clone(),
            format: p.format.clone(),
            name: p.name.clone(),
            vendor: p.vendor.clone(),
            version: String::new(),
            category: String::new(),
            path: p.path.clone(),
        });
        if info.format == "vst3" && !std::path::Path::new(&info.path).exists() {
            return Err(format!("{} is not installed on this computer", p.name));
        }
        let setup = Setup { sample_rate: rate as f64, channels, max_block: if self.offline { 4096 } else { 1024 }, offline: self.offline };
        let inst = vcsd_plugins::instantiate(&info, setup)?;
        if let Some(s) = &p.state {
            let bytes = base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| format!("saved state unreadable: {e}"))?;
            inst.set_state(&bytes)?;
        }
        Ok(inst)
    }

    /// Bring instances in line with the project.
    pub fn sync(&self, st: &ProjectState) {
        let rate = st.project.sample_rate;
        let wanted = plugin_slots(st);
        let key = |p: &PluginInsert, ch: usize| format!("{}|{ch}|{rate}", p.plugin_id);
        let todo: Vec<(&PluginInsert, usize)> = {
            let mut slots = self.slots.lock().unwrap();
            slots.retain(|id, _| wanted.iter().any(|(p, ..)| &p.id == id));
            wanted.iter().filter(|(p, ch, _)| slots.get(&p.id).is_none_or(|s| s.key != key(p, *ch))).map(|(p, ch, _)| (*p, *ch)).collect()
        };
        // load plugins without holding the lock the render thread uses
        let made: Vec<(String, Slot)> = todo
            .into_iter()
            .map(|(p, ch)| {
                let (inst, error) = match self.make(p, ch, rate) {
                    Ok(i) => (Some(i), None),
                    Err(e) => (None, Some(e)),
                };
                (p.id.clone(), Slot { key: key(p, ch), inst, error, applied: BTreeMap::new() })
            })
            .collect();
        let mut slots = self.slots.lock().unwrap();
        slots.extend(made);
        for (p, ..) in wanted {
            let Some(s) = slots.get_mut(&p.id) else { continue };
            if let Some(inst) = &s.inst {
                for (id, v) in &p.params {
                    if s.applied.get(id) != Some(v) {
                        let _ = inst.set_param(*id, *v);
                        s.applied.insert(*id, *v);
                    }
                }
            }
        }
    }

    pub fn instance(&self, slot_id: &str) -> Option<Arc<dyn Instance>> {
        self.slots.lock().unwrap().get(slot_id).and_then(|s| s.inst.clone())
    }

    /// Touches made in plugins' own windows since the last call, by slot id.
    pub fn take_gestures(&self) -> Vec<(String, vcsd_plugins::Gesture)> {
        let insts: Vec<(String, Arc<dyn Instance>)> =
            self.slots.lock().unwrap().iter().filter_map(|(k, s)| s.inst.clone().map(|i| (k.clone(), i))).collect();
        insts.into_iter().flat_map(|(k, i)| i.take_gestures().into_iter().map(move |g| (k.clone(), g))).collect()
    }

    /// True if any plugin was edited in its own window since the last call.
    pub fn poll_dirty(&self) -> bool {
        let insts: Vec<Arc<dyn Instance>> = self.slots.lock().unwrap().values().filter_map(|s| s.inst.clone()).collect();
        insts.iter().fold(false, |any, i| i.take_dirty() | any) // check (and reset) them all
    }

    /// Note a parameter value already sent to the instance (so sync doesn't resend it).
    pub fn mark_applied(&self, slot_id: &str, param: u32, v: f64) {
        if let Some(s) = self.slots.lock().unwrap().get_mut(slot_id) {
            s.applied.insert(param, v);
        }
    }

    /// Per slot: loaded or why not, and latency.
    pub fn status(&self) -> serde_json::Value {
        let slots = self.slots.lock().unwrap();
        serde_json::Value::Object(
            slots
                .iter()
                .map(|(id, s)| {
                    (
                        id.clone(),
                        serde_json::json!({
                            "loaded": s.inst.is_some(),
                            "error": s.error,
                            "latency": s.inst.as_ref().map(|i| i.latency()).unwrap_or(0),
                        }),
                    )
                })
                .collect(),
        )
    }

    /// Store every running plugin's current state in the project (before save/export).
    /// Returns true if anything changed.
    pub fn capture(&self, st: &mut ProjectState) -> bool {
        let slots = self.slots.lock().unwrap();
        let mut changed = false;
        let mut one = |ins: &mut Vec<Insert>| {
            for i in ins.iter_mut() {
                let Insert::Plugin(p) = i else { continue };
                let Some(inst) = slots.get(&p.id).and_then(|s| s.inst.as_ref()) else { continue };
                if let Ok(bytes) = inst.get_state() {
                    let s = b64(&bytes);
                    if p.state.as_deref() != Some(s.as_str()) {
                        p.state = Some(s);
                        changed = true;
                    }
                }
                // keep app-set values in step with the plugin (its own editor may have moved them)
                if !p.params.is_empty() {
                    let now: HashMap<u32, f64> = inst.params().into_iter().map(|x| (x.id, x.value)).collect();
                    for (id, v) in p.params.iter_mut() {
                        if let Some(n) = now.get(id) {
                            if (*n - *v).abs() > 1e-9 {
                                *v = *n;
                                changed = true;
                            }
                        }
                    }
                }
            }
        };
        for t in st.tracks.values_mut() {
            one(&mut t.inserts);
        }
        for b in st.buses.values_mut() {
            one(&mut b.inserts);
        }
        drop(slots);
        // values written back above must not be re-sent
        let slots_now: Vec<(String, BTreeMap<u32, f64>)> =
            plugin_slots(st).into_iter().map(|(p, ..)| (p.id.clone(), p.params.clone())).collect();
        for (id, params) in slots_now {
            for (k, v) in params {
                self.mark_applied(&id, k, v);
            }
        }
        changed
    }

    /// Fresh offline instances (from the project's saved states) for an export,
    /// so rendering never disturbs playback instances.
    pub fn offline_copy(&self, st: &ProjectState) -> Rack {
        let r = Rack { offline: true, ..Default::default() };
        *r.catalog.lock().unwrap() = self.catalog.lock().unwrap().clone();
        r.sync(st);
        r
    }

    /// QA findings: plugins that can't be loaded here.
    pub fn qa(&self, st: &ProjectState) -> Vec<QaIssue> {
        let slots = self.slots.lock().unwrap();
        plugin_slots(st)
            .into_iter()
            .filter(|(p, ..)| !p.bypass)
            .filter_map(|(p, _, owner)| {
                let err = match slots.get(&p.id) {
                    Some(s) if s.inst.is_some() => return None,
                    Some(s) => s.error.clone().unwrap_or_default(),
                    None => "not loaded".into(),
                };
                Some(QaIssue::new(
                    Severity::Warning,
                    "plugin_missing",
                    format!("Plugin '{}' on {owner} can't be loaded ({err}); it is passed through in exports", p.name),
                    Some(&p.id),
                ))
            })
            .collect()
    }
}

impl PluginHost for Rack {
    fn process(
        &self,
        slot: &PluginInsert,
        buf: &mut AudioBuffer,
        pos: i64,
        automation: &[vcsd_core::mixer::ParamCurve],
    ) -> Result<(), String> {
        let inst = {
            let slots = self.slots.lock().unwrap();
            match slots.get(&slot.id) {
                Some(s) => s.inst.clone().ok_or_else(|| s.error.clone().unwrap_or_else(|| "not loaded".into()))?,
                None => return Err("not loaded yet".into()),
            }
        };
        let curves: Vec<vcsd_plugins::ParamCurve> =
            automation.iter().map(|c| vcsd_plugins::ParamCurve { id: c.id, points: c.points.clone() }).collect();
        inst.process(&mut buf.data, buf.channels as usize, pos, &curves);
        Ok(())
    }

    fn latency(&self, slot: &PluginInsert) -> i64 {
        self.slots.lock().unwrap().get(&slot.id).and_then(|s| s.inst.as_ref()).map(|i| i.latency() as i64).unwrap_or(0)
    }

    fn continues_at(&self, slot: &PluginInsert) -> Option<i64> {
        self.instance(&slot.id).and_then(|i| i.next_pos())
    }
}

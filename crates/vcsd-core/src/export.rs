//! Export profiles and the studio handoff package (spec §15).
//!
//! Tier 1 (guaranteed): sample-aligned BWF WAV stems/tracks/cues plus a
//! machine-readable cue manifest that lets a receiving editor rebuild the
//! intended layout without any interchange adapter. Tier 2 (AAF etc.) plugs
//! in through [`InterchangeAdapter`] once validated against a pinned target.

use crate::error::{Error, Result};
use crate::ids::{new_id, now_unix};
use crate::layout::safe_name;
use crate::mixer::{Mixer, RenderOptions};
use crate::model::*;
use crate::qa::{QaIssue, QaOptions, Severity};
use crate::state::ProjectState;
use crate::wav::{AudioBuffer, Bext, SampleFormat};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportProfile {
    QuickReview,
    Resolve,
    ProStudio,
    Archival,
}

impl ExportProfile {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "quick" | "quickreview" | "review" => Self::QuickReview,
            "resolve" | "fairlight" => Self::Resolve,
            "pro" | "prostudio" | "studio" => Self::ProStudio,
            "archive" | "archival" => Self::Archival,
            _ => return Err(crate::error::invalid(format!("unknown export profile '{s}' (quick|resolve|pro|archive)"))),
        })
    }
    pub fn slug(&self) -> &'static str {
        match self {
            Self::QuickReview => "quick_review",
            Self::Resolve => "resolve_fairlight",
            Self::ProStudio => "pro_studio",
            Self::Archival => "archival",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ExportRange {
    All,
    Scene(String),
    Custom(i64, i64),
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub profile: ExportProfile,
    pub range: ExportRange,
    /// Handles added around discrete cue files (Pro profile).
    pub handle_samples: i64,
    pub format: SampleFormat,
    /// Date stamp used in folder names and reports (keeps output reproducible).
    pub date: String,
    /// Export despite QA errors (they are still written to the QA report).
    pub force: bool,
    /// Defaults to true for final deliverables (spec §15.2).
    pub exclude_temp_refs: bool,
    pub out_dir: Option<PathBuf>,
    /// Hosted plugins for the render (None: plugin slots pass through, with a QA warning).
    pub plugins: Option<crate::mixer::PluginHostRef>,
}

impl ExportOptions {
    pub fn new(profile: ExportProfile, date: &str) -> Self {
        Self {
            profile,
            range: ExportRange::All,
            handle_samples: 0,
            format: SampleFormat::Int24,
            date: date.into(),
            force: false,
            exclude_temp_refs: profile != ExportProfile::QuickReview,
            out_dir: None,
            plugins: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub dir: PathBuf,
    pub files: Vec<String>,
    pub qa: Vec<QaIssue>,
    pub job_id: String,
}

/// Optional Tier-2 timeline interchange (AAF, OTIO, …). Implementations must
/// report every feature they cannot carry so the README documents the loss.
pub trait InterchangeAdapter {
    fn name(&self) -> &str;
    fn validated_targets(&self) -> Vec<String>;
    fn write(&self, s: &ProjectState, package_dir: &Path, manifest: &serde_json::Value) -> Result<Vec<String>>;
}

pub fn resolve_range(s: &ProjectState, r: &ExportRange) -> Result<(i64, i64)> {
    Ok(match r {
        ExportRange::Custom(a, z) => (*a, *z),
        ExportRange::Scene(id) => {
            let sc = s.find_scene(id)?;
            (sc.start_sample - sc.handle_samples, sc.end_sample + sc.handle_samples)
        }
        ExportRange::All => {
            let mut a = s.scenes.values().map(|x| x.start_sample).min();
            let mut z = s.scenes.values().map(|x| x.end_sample).max();
            for c in s.clips.values().filter(|c| !c.muted) {
                a = Some(a.map_or(c.timeline_start, |v| v.min(c.timeline_start)));
                z = Some(z.map_or(c.end(), |v| v.max(c.end())));
            }
            (a.unwrap_or(0), z.unwrap_or(0))
        }
    })
}

struct Writer<'a> {
    dir: PathBuf,
    files: Vec<String>,
    s: &'a ProjectState,
    format: SampleFormat,
    qa: Vec<QaIssue>,
}

impl<'a> Writer<'a> {
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.dir).unwrap_or(p).to_string_lossy().replace('\\', "/")
    }
    fn wav(&mut self, rel: &str, buf: &AudioBuffer, start: i64, desc: &str) -> Result<String> {
        let p = self.dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap())?;
        let tb = self.s.tb();
        let bext = Bext {
            description: format!("{desc} @ {} {}", tb.tc_at(start), self.s.project.frame_rate.label()),
            originator: "VC Sound Design".into(),
            originator_reference: safe_name(&self.s.project.name),
            origination_date: crate::ids::date_from_unix(now_unix()),
            origination_time: crate::ids::time_from_unix(now_unix()),
            time_reference: (tb.origin_since_midnight() + start).max(0) as u64,
            coding_history: format!("A=PCM,F={},W={},T=VC Sound Design export\r\n", buf.sample_rate, self.format.bits()),
        };
        let peak = buf.peak();
        if peak > 1.0 && self.format != SampleFormat::Float32 {
            self.qa.push(QaIssue::new(
                Severity::Warning,
                "clipping",
                format!("{rel} clips (peak {:+.2} dBFS); integer output is clamped", 20.0 * peak.log10()),
                None,
            ));
        }
        crate::wav::write(&p, buf, self.format, Some(&bext))?;
        let r = self.rel(&p);
        self.files.push(r.clone());
        Ok(r)
    }
    fn text(&mut self, rel: &str, body: &[u8]) -> Result<String> {
        let p = self.dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(&p, body)?;
        let r = self.rel(&p);
        self.files.push(r.clone());
        Ok(r)
    }
}

fn unique_dir(base: PathBuf) -> PathBuf {
    if !base.exists() {
        return base;
    }
    (2..).map(|i| PathBuf::from(format!("{}_{i}", base.display()))).find(|p| !p.exists()).unwrap()
}

/// Build an export package. QA errors block the export unless `force`.
pub fn export(s: &mut ProjectState, root: Option<&Path>, o: &ExportOptions) -> Result<ExportResult> {
    let mut qa = crate::qa::run(s, &QaOptions { verify_hashes: true, exclude_temp_refs: o.exclude_temp_refs });
    let errors: Vec<&QaIssue> = qa.iter().filter(|i| i.severity == Severity::Error).collect();
    if !errors.is_empty() && !o.force {
        return Err(Error::QaBlocked(errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")));
    }
    let (start, end) = resolve_range(s, &o.range)?;
    if end <= start {
        return Err(crate::error::invalid("export range is empty"));
    }
    let frames = (end - start) as usize;
    let base = match &o.out_dir {
        Some(d) => d.clone(),
        None => {
            root.ok_or_else(|| crate::error::invalid("no output folder"))?.join("Exports").join(format!("{}_{}", o.date, o.profile.slug()))
        }
    };
    let dir = unique_dir(base);
    std::fs::create_dir_all(&dir)?;
    let st: &ProjectState = s;
    let mixer = Mixer::new(st, RenderOptions { exclude_temp_refs: o.exclude_temp_refs, honor_solo: false })
        .with_plugins(o.plugins.as_ref().map(|p| p.0.clone()));
    let mut w = Writer { dir: dir.clone(), files: vec![], s: st, format: o.format, qa: vec![] };
    let tb = st.tb();
    let prefix = safe_name(&st.project.name);
    let scene_filter: Option<String> = match &o.range {
        ExportRange::Scene(k) => Some(st.find_scene(k)?.id.clone()),
        _ => None,
    };

    // ---- review mix (all profiles except archival)
    let mut mix_file = None;
    if o.profile != ExportProfile::Archival {
        let mix = mixer.render_master(start, frames)?;
        mix_file = Some(w.wav(&format!("Audio/Mix/{prefix}_ReviewMix.wav"), &mix, start, "Review mix")?);
    }

    // ---- stems + consolidated tracks
    let mut stems_meta = vec![];
    let mut tracks_meta = vec![];
    if matches!(o.profile, ExportProfile::Resolve | ExportProfile::ProStudio) {
        let mut buses: Vec<&Bus> = st.buses.values().filter(|b| b.stem).collect();
        buses.sort_by_key(|b| (b.order, b.id.clone()));
        for b in buses {
            let has = st.tracks.values().any(|t| t.bus_id == b.id && st.clips.values().any(|c| c.track_id == t.id));
            if !has {
                continue;
            }
            let buf = mixer.render_bus(&b.id, start, frames)?;
            let f = w.wav(&format!("Audio/Stems/{prefix}_STEM_{}.wav", safe_name(&b.name)), &buf, start, &format!("Stem {}", b.name))?;
            stems_meta.push(serde_json::json!({ "bus_id": b.id, "bus": b.name, "file": f, "channels": 2, "start_sample": start, "start_tc": tb.tc_at(start) }));
        }
        let mut tracks: Vec<&Track> = st
            .tracks
            .values()
            .filter(|t| st.clips.values().any(|c| c.track_id == t.id && !c.muted && c.timeline_start < end && c.end() > start))
            .collect();
        tracks.sort_by_key(|t| (t.order, t.id.clone()));
        for (i, t) in tracks.iter().enumerate() {
            let buf = mixer.render_track_print(&t.id, start, frames)?;
            let f = w.wav(&format!("Audio/Tracks/{:03}_{}.wav", i + 1, safe_name(&t.name)), &buf, start, &format!("Track {}", t.name))?;
            tracks_meta.push(serde_json::json!({
                "track_id": t.id, "name": t.name, "file": f, "channels": t.channels, "start_sample": start, "start_tc": tb.tc_at(start),
                "bus": st.buses.get(&t.bus_id).map(|b| b.name.clone()), "folder": t.folder, "order": i + 1,
                "scene": t.scene_id.as_ref().and_then(|x| st.scenes.get(x)).map(|x| x.number.clone()),
                "identity": t.identity_id.as_ref().and_then(|x| st.identities.get(x)).map(|x| x.name.clone()),
                "category": t.category.display(), "muted": t.mute, "gain_db": t.gain_db, "pan": t.pan,
            }));
        }
    }

    // ---- discrete cue files with handles (pro)
    let mut discrete = vec![];
    if o.profile == ExportProfile::ProStudio {
        let h = o.handle_samples.max(0);
        let mut clips: Vec<&ClipInstance> = st.clips.values().filter(|c| !c.muted && c.timeline_start < end && c.end() > start).collect();
        clips.sort_by_key(|c| (c.timeline_start, c.id.clone()));
        for c in clips {
            if !mixer.opts.exclude_temp_refs
                || st.assets.get(&c.asset_id).is_none_or(|a| !a.license.as_ref().is_some_and(|l| l.temp_reference))
            {
                let head = h.min(c.source_offset);
                let mut ext = c.clone();
                ext.timeline_start -= head;
                ext.source_offset -= head;
                ext.length += head + h;
                ext.fade_in = 0;
                ext.fade_out = 0;
                let ch = st.assets.get(&c.asset_id).map(|a| a.channels).unwrap_or(1).clamp(1, 2);
                let mut buf = AudioBuffer::silent(st.project.sample_rate, ch, ext.length as usize);
                mixer.render_clip_into(&ext, &mut buf, ext.timeline_start)?;
                let code = c.cue_id.as_ref().and_then(|x| st.cues.get(x)).map(|x| x.code.clone()).unwrap_or_else(|| "NOCUE".into());
                let f = w.wav(
                    &format!(
                        "Audio/Cues/{}_{}_{}.wav",
                        safe_name(&code),
                        c.sub_index.map(|i| format!("{:02}", i + 1)).unwrap_or_else(|| "00".into()),
                        &c.id[c.id.len() - 8..]
                    ),
                    &buf,
                    ext.timeline_start,
                    &format!("Cue {code}"),
                )?;
                discrete.push(serde_json::json!({ "clip_id": c.id, "cue_code": code, "file": f, "start_sample": ext.timeline_start, "start_tc": tb.tc_at(ext.timeline_start), "head_handle": head, "tail_handle": h, "clip_start_sample": c.timeline_start }));
            }
        }
    }

    // ---- manifests
    let manifest = cue_manifest(st, start, end, o, &tracks_meta, &stems_meta, &discrete, mix_file.as_deref());
    w.text("Manifests/cue_manifest.json", serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    w.text("Manifests/media_inventory.csv", media_inventory(st).as_bytes())?;
    w.text("Manifests/license_manifest.json", serde_json::to_string_pretty(&license_manifest(st))?.as_bytes())?;
    if o.profile != ExportProfile::QuickReview {
        w.text("Manifests/routing_map.json", serde_json::to_string_pretty(&routing_map(st))?.as_bytes())?;
        w.text("Manifests/picture_history.json", serde_json::to_string_pretty(&picture_history(st))?.as_bytes())?;
    }
    if o.profile == ExportProfile::Archival {
        crate::store::save(&dir.join("Project/Project.vcsd"), st)?;
        w.files.push("Project/Project.vcsd".into());
    }

    // ---- reports
    let rows_all = crate::report::cue_rows(st, scene_filter.as_deref());
    w.text("Reports/cue_inventory.csv", crate::report::rows_to_csv(st, &rows_all).as_bytes())?;
    w.text(
        "Reports/cue_inventory.json",
        serde_json::to_string_pretty(&crate::report::manifest_json(st, scene_filter.as_deref(), &o.date))?.as_bytes(),
    )?;
    w.text("Reports/cue_inventory.pdf", &crate::pdf::cue_report(st, scene_filter.as_deref(), &o.date))?;
    let mut scenes: Vec<&Scene> = st.scenes.values().filter(|x| scene_filter.as_ref().is_none_or(|f| &x.id == f)).collect();
    scenes.sort_by_key(|x| x.start_sample);
    if o.profile != ExportProfile::QuickReview {
        for sc in scenes {
            let n = safe_name(&format!("Scene_{}", sc.number));
            w.text(&format!("Reports/{n}/{n}_cue_report.pdf"), &crate::pdf::cue_report(st, Some(&sc.id), &o.date))?;
            let rows = crate::report::cue_rows(st, Some(&sc.id));
            w.text(&format!("Reports/{n}/{n}_cues.csv"), crate::report::rows_to_csv(st, &rows).as_bytes())?;
            w.text(
                &format!("Reports/{n}/{n}_cues.json"),
                serde_json::to_string_pretty(&crate::report::manifest_json(st, Some(&sc.id), &o.date))?.as_bytes(),
            )?;
            w.text(
                &format!("Reports/{n}/{n}_summary.json"),
                serde_json::to_string_pretty(&crate::report::scene_summary(st, &sc.id)?)?.as_bytes(),
            )?;
        }
    }

    // ---- QA, README, checksums
    for wq in mixer.warnings.borrow().iter() {
        qa.push(QaIssue::new(Severity::Warning, "render", wq.clone(), None));
    }
    qa.append(&mut w.qa);
    w.text("README.md", readme(st, o, start, end, &qa).as_bytes())?;
    w.text("Reports/delivery_qa.md", crate::qa::to_markdown(&qa).as_bytes())?;
    w.text("Reports/delivery_qa.json", serde_json::to_string_pretty(&qa)?.as_bytes())?;
    let mut sums = String::new();
    let mut files = w.files.clone();
    files.sort();
    files.dedup();
    for f in &files {
        sums.push_str(&format!("{}  {f}\n", crate::hash::sha256_file(&dir.join(f))?));
    }
    std::fs::write(dir.join("checksums.sha256"), &sums)?;
    let bad = verify_checksums(&dir)?;
    if !bad.is_empty() {
        return Err(crate::error::invalid(format!("checksum verification failed for {bad:?}")));
    }
    files.push("checksums.sha256".into());

    let job_id = new_id("exp");
    let (errs, warns) = (crate::qa::count(&qa, Severity::Error), crate::qa::count(&qa, Severity::Warning));
    drop(mixer);
    s.exports.insert(
        job_id.clone(),
        ExportJob {
            id: job_id.clone(),
            profile: o.profile.slug().into(),
            output_dir: dir.to_string_lossy().to_string(),
            created_at: now_unix(),
            file_count: files.len(),
            qa_errors: errs,
            qa_warnings: warns,
        },
    );
    Ok(ExportResult { dir, files, qa, job_id })
}

/// Re-hash every file listed in `checksums.sha256`; returns mismatches.
pub fn verify_checksums(dir: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(dir.join("checksums.sha256"))?;
    let mut bad = vec![];
    for line in text.lines() {
        let Some((h, f)) = line.split_once("  ") else { continue };
        let p = dir.join(f);
        if !p.exists() || crate::hash::sha256_file(&p)? != h {
            bad.push(f.to_string());
        }
    }
    Ok(bad)
}

#[allow(clippy::too_many_arguments)]
fn cue_manifest(
    s: &ProjectState,
    start: i64,
    end: i64,
    o: &ExportOptions,
    tracks: &[serde_json::Value],
    stems: &[serde_json::Value],
    discrete: &[serde_json::Value],
    mix: Option<&str>,
) -> serde_json::Value {
    let tb = s.tb();
    let cues: Vec<serde_json::Value> = s
        .sorted_cues(None)
        .into_iter()
        .map(|c| {
            let clips: Vec<serde_json::Value> = s
                .clips_for_cue(&c.id)
                .into_iter()
                .map(|cl| {
                    let a = s.assets.get(&cl.asset_id);
                    serde_json::json!({
                        "clip_id": cl.id, "track_id": cl.track_id,
                        "track": s.tracks.get(&cl.track_id).map(|t| t.name.clone()),
                        "timeline_start": cl.timeline_start, "start_tc": tb.tc_at(cl.timeline_start),
                        "length": cl.length, "source_offset": cl.source_offset, "gain_db": cl.gain_db,
                        "fade_in": cl.fade_in, "fade_out": cl.fade_out, "fill": cl.fill, "muted": cl.muted,
                        "sub_index": cl.sub_index, "take_id": cl.take_id, "performance_event_id": cl.performance_event_id,
                        "asset": a.map(|a| serde_json::json!({"name": a.name, "sha256": a.sha256, "source_path": a.path, "kind": a.kind, "sample_rate": a.sample_rate, "channels": a.channels, "license": a.license})),
                        "history": cl.history.iter().map(|h| serde_json::json!({"asset": s.assets.get(&h.asset_id).map(|a| a.name.clone()), "sha256": s.assets.get(&h.asset_id).map(|a| a.sha256.clone()), "origin": h.origin, "at": h.at})).collect::<Vec<_>>(),
                    })
                })
                .collect();
            serde_json::json!({
                "cue_id": c.id, "code": c.code,
                "scene": s.scenes.get(&c.scene_id).map(|x| x.number.clone()),
                "identity": c.identity_id.as_ref().and_then(|i| s.identities.get(i)).map(|i| i.name.clone()),
                "identity_id": c.identity_id,
                "category": c.category.display(), "event_type": c.event_type,
                "start_sample": c.start_sample, "end_sample": c.end_sample,
                "start_tc": tb.tc_at(c.start_sample), "end_tc": tb.tc_at(c.end_sample),
                "sub_events": c.sub_events, "description": c.description, "notes": c.notes,
                "status": c.status, "timing_origin": c.timing_origin, "review": c.review_status,
                "owner": c.owner, "priority": c.priority,
                "marker_id": c.marker_id, "picture_id": c.picture_id,
                "in_range": c.start_sample < end && c.end_sample.max(c.start_sample + 1) > start,
                "clips": clips,
            })
        })
        .collect();
    let markers: Vec<serde_json::Value> = s
        .markers
        .values()
        .map(|m| serde_json::json!({"marker_id": m.id, "lane": s.lanes.get(&m.lane_id).map(|l| l.name.clone()), "kind": m.kind, "start_tc": tb.tc_at(m.start_sample), "end_tc": tb.tc_at(m.end_sample), "start_sample": m.start_sample, "end_sample": m.end_sample, "label": m.label, "cue_id": m.cue_id, "sub_events": m.sub_events}))
        .collect();
    serde_json::json!({
        "schema": "vc.sounddesign.handoff-manifest",
        "version": 1,
        "project": {
            "id": s.project.id, "name": s.project.name, "reel": s.project.reel,
            "frame_rate": s.project.frame_rate.label(), "frame_rate_num": s.project.frame_rate.num, "frame_rate_den": s.project.frame_rate.den,
            "drop_frame": s.project.frame_rate.drop_frame, "sample_rate": s.project.sample_rate,
            "start_tc": s.project.frame_rate.frames_to_tc(s.project.start_frame),
            "timeline_origin_samples_since_midnight": tb.origin_since_midnight(),
            "editorial_version": s.project.editorial_version,
        },
        "export": {
            "profile": o.profile.slug(), "date": o.date, "bit_depth": o.format.bits(), "float": o.format == SampleFormat::Float32,
            "range_start_sample": start, "range_end_sample": end, "range_start_tc": tb.tc_at(start), "range_end_tc": tb.tc_at(end),
            "handles_samples": o.handle_samples, "temp_references_excluded": o.exclude_temp_refs,
            "alignment": "every WAV in Audio/Tracks, Audio/Stems and Audio/Mix starts at range_start_tc; BWF time_reference = timeline_origin + range_start_sample",
        },
        "picture": s.current_picture().map(|p| serde_json::json!({"label": p.label, "fingerprint": p.fingerprint, "start_tc": s.project.frame_rate.frames_to_tc(p.source_start_frame), "duration_frames": p.duration_frames})),
        "review_mix": mix,
        "tracks": tracks,
        "stems": stems,
        "discrete_cues": discrete,
        "identities": s.identities.values().map(|i| serde_json::json!({"identity_id": i.id, "name": i.name, "category": i.category.display(), "default_asset": i.default_asset_id.as_ref().and_then(|a| s.assets.get(a)).map(|a| a.name.clone()), "variation_mode": i.variation_mode})).collect::<Vec<_>>(),
        "markers": markers,
        "cues": cues,
        "scores": s.scores.values().map(|sc| serde_json::json!({"title": sc.title, "declared_start_tc": sc.declared_start_tc, "declared_frame_rate": sc.declared_frame_rate.label(), "timeline_start": sc.timeline_start, "warnings": sc.warnings})).collect::<Vec<_>>(),
    })
}

fn media_inventory(s: &ProjectState) -> String {
    use crate::report::csv_escape as e;
    let mut out = String::from("asset_id,name,kind,sha256,sample_rate,channels,bits,length_samples,license,redistributable,temp_reference,pack,source_path,used_by_clips\n");
    for a in s.assets.values() {
        let used = s.clips.values().filter(|c| c.asset_id == a.id).count();
        let l = a.license.clone().unwrap_or_default();
        out.push_str(&format!(
            "{},{},{:?},{},{},{},{},{},{},{},{},{},{},{}\n",
            a.id,
            e(&a.name),
            a.kind,
            a.sha256,
            a.sample_rate,
            a.channels,
            a.bits_per_sample,
            a.length_samples,
            e(&l.name),
            l.redistributable,
            l.temp_reference,
            e(&a.pack_id.as_ref().and_then(|p| s.packs.get(p)).map(|p| p.name.clone()).unwrap_or_default()),
            e(&a.path),
            used
        ));
    }
    out
}

fn license_manifest(s: &ProjectState) -> serde_json::Value {
    let used: std::collections::BTreeSet<&str> = s.clips.values().map(|c| c.asset_id.as_str()).collect();
    serde_json::json!({
        "schema": "vc.sounddesign.license-manifest", "version": 1,
        "note": "Third-party sounds are never assumed redistributable. Review every entry with redistributable=false before onward delivery.",
        "assets": s.assets.values().filter(|a| used.contains(a.id.as_str())).map(|a| serde_json::json!({
            "name": a.name, "sha256": a.sha256, "kind": a.kind, "license": a.license,
            "pack": a.pack_id.as_ref().and_then(|p| s.packs.get(p)).map(|p| serde_json::json!({"name": p.name, "version": p.version})),
        })).collect::<Vec<_>>(),
    })
}

fn routing_map(s: &ProjectState) -> serde_json::Value {
    let mut buses: Vec<&Bus> = s.buses.values().collect();
    buses.sort_by_key(|b| (b.order, b.id.clone()));
    let tb = s.tb();
    // Automation is already baked into the rendered audio; it is listed here
    // so the receiving engineer can see (and rebuild) the moves.
    let automation = |owner: &str| {
        // the owner's own lanes, plus those of the plugins in its inserts
        let slots: Vec<(String, String)> = s
            .tracks
            .get(owner)
            .map(|t| &t.inserts)
            .or_else(|| s.buses.get(owner).map(|b| &b.inserts))
            .map(|ins| ins.iter().filter_map(Insert::as_plugin).map(|p| (p.id.clone(), p.name.clone())).collect())
            .unwrap_or_default();
        let mut pts: Vec<&AutomationPoint> =
            s.automation.values().filter(|p| p.owner_id == owner || slots.iter().any(|(id, _)| *id == p.owner_id)).collect();
        pts.sort_by_key(|p| (p.sample, p.id.clone()));
        pts.iter()
            .map(|p| {
                let unit = match p.param {
                    AutomationParam::Pan => "pan -1..1",
                    AutomationParam::VolumeDb => "dB relative to fader",
                    AutomationParam::Plugin(_) => "normalised 0..1",
                };
                let plugin = slots.iter().find(|(id, _)| *id == p.owner_id).map(|(_, n)| n.clone());
                serde_json::json!({"param": p.param, "plugin": plugin, "tc": tb.tc_at(p.sample), "sample": p.sample, "value": p.value, "unit": unit})
            })
            .collect::<Vec<_>>()
    };
    serde_json::json!({
        "note": "Volume/pan and plugin parameter automation is rendered into stems, tracks and mix; breakpoints are listed per track and bus for reference.",
        "buses": buses.iter().map(|b| serde_json::json!({"bus_id": b.id, "name": b.name, "parent": b.parent_id.as_ref().and_then(|p| s.buses.get(p)).map(|p| p.name.clone()), "gain_db": b.gain_db, "inserts": b.inserts, "stem": b.stem, "automation": automation(&b.id)})).collect::<Vec<_>>(),
        "tracks": s.tracks.values().map(|t| serde_json::json!({"track_id": t.id, "name": t.name, "bus": s.buses.get(&t.bus_id).map(|b| b.name.clone()), "folder": t.folder, "channels": t.channels, "gain_db": t.gain_db, "pan": t.pan, "mute": t.mute, "inserts": t.inserts, "automation": automation(&t.id)})).collect::<Vec<_>>(),
    })
}

fn picture_history(s: &ProjectState) -> serde_json::Value {
    let fr = s.project.frame_rate;
    let mut v: Vec<&PictureVersion> = s.pictures.values().collect();
    v.sort_by_key(|p| (p.imported_at, p.id.clone()));
    serde_json::json!(v.iter().map(|p| serde_json::json!({"label": p.label, "file": Path::new(&p.media_path).file_name().map(|f| f.to_string_lossy().to_string()), "fingerprint": p.fingerprint, "start_tc": fr.frames_to_tc(p.source_start_frame), "duration_frames": p.duration_frames, "previous": p.previous_id.as_ref().and_then(|x| s.pictures.get(x)).map(|x| x.label.clone()), "change_notes": p.change_notes, "current": s.project.current_picture_id.as_deref() == Some(p.id.as_str())})).collect::<Vec<_>>())
}

fn readme(s: &ProjectState, o: &ExportOptions, start: i64, end: i64, qa: &[QaIssue]) -> String {
    let tb = s.tb();
    let fr = s.project.frame_rate;
    let contacts = if s.project.contacts.is_empty() {
        "- (none entered)".to_string()
    } else {
        s.project.contacts.iter().map(|c| format!("- {}: {} {}", c.role, c.name, c.email)).collect::<Vec<_>>().join("\n")
    };
    format!(
        r#"# {name} — Sound Editorial Handoff ({profile})

Generated by VC Sound Design on {date}.

## Technical specification
- Frame rate: **{fr}** ({df})
- Sample rate: **{sr} Hz**, {bits}-bit {kind} WAV (BWF `bext` time reference embedded)
- Range: **{a} – {b}** (timeline samples {sa}–{sb})
- Picture: {pic}
- Temp/reference material excluded from audio: {temp}

## Alignment
Every file in `Audio/Mix`, `Audio/Stems` and `Audio/Tracks` starts at **{a}** and runs to {b}
(silence included). Import them at {a} in Resolve/Fairlight (or use "Place at BWF timecode").
Discrete cue files in `Audio/Cues` (Pro profile) carry their own BWF time references.

## Rebuilding the layout without an interchange file
`Manifests/cue_manifest.json` lists every track (file, bus, folder, channel count), every cue
(code, identity, in/out TC + samples, status, notes, clips, source asset hashes and replacement
history) and every marker. A receiving editor can reconstruct the intended track layout from the
WAVs and this manifest alone. Bus routing is in `Manifests/routing_map.json`.

## Interchange (Tier 2)
No timeline interchange (AAF/OTIO) is included in this package. Routing, plugins, clip fades,
automation and markers are therefore delivered as rendered audio plus the JSON/CSV manifests,
not as editable Fairlight timeline objects.

## Contents
- `Audio/` rendered audio · `Manifests/` cue manifest, media inventory, license manifest, routing map, picture history
- `Reports/` cue inventory (CSV/JSON/landscape PDF), per-scene reports, delivery QA
- `checksums.sha256` SHA-256 of every file (verify with `sha256sum -c checksums.sha256`)

## Rights
See `Manifests/license_manifest.json`. Assets not marked redistributable must be cleared before onward delivery.

## Delivery QA summary
Errors: {qe} · Warnings: {qw} · Info: {qi} — details in `Reports/delivery_qa.md`.

## Contacts
{contacts}
"#,
        name = s.project.name,
        profile = o.profile.slug(),
        date = o.date,
        fr = fr.label(),
        df = if fr.drop_frame { "drop-frame" } else { "non-drop-frame" },
        sr = s.project.sample_rate,
        bits = o.format.bits(),
        kind = if o.format == SampleFormat::Float32 { "float" } else { "PCM" },
        a = tb.tc_at(start),
        b = tb.tc_at(end),
        sa = start,
        sb = end,
        pic = s.current_picture().map(|p| p.label.clone()).unwrap_or_else(|| "(none)".into()),
        temp = o.exclude_temp_refs,
        qe = crate::qa::count(qa, Severity::Error),
        qw = crate::qa::count(qa, Severity::Warning),
        qi = crate::qa::count(qa, Severity::Info),
    )
}

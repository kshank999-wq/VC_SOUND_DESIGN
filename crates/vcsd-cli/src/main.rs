//! `vcsd` — command-line front end for the VC Sound Design engine.

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use vcsd_core::builder::{populate, PopulateOptions, TrackPolicy};
use vcsd_core::editing::{self, ReplaceScope, TakeImport};
use vcsd_core::export::{ExportOptions, ExportProfile, ExportRange};
use vcsd_core::library::{self, AssetMeta, Query};
use vcsd_core::picture::{import_picture, PictureImport};
use vcsd_core::spotting::{self, NewIdentity, NewMarker};
use vcsd_core::*;

#[derive(Parser)]
#[command(name = "vcsd", version, about = "VC Sound Design — sound-design-first DAW engine (CLI)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a project folder (frame rate, sample rate, start TC)
    New {
        dir: PathBuf,
        #[arg(long, default_value = "Untitled")]
        name: String,
        #[arg(long, default_value = "24")]
        fps: String,
        #[arg(long, default_value_t = 48_000)]
        sr: u32,
        #[arg(long, default_value = "01:00:00:00")]
        start_tc: String,
    },
    /// Project overview
    Info { dir: PathBuf },
    /// Import a picture reference version (never modified)
    Picture {
        dir: PathBuf,
        file: PathBuf,
        #[arg(long, default_value = "v1")]
        label: String,
        #[arg(long)]
        start_tc: Option<String>,
        #[arg(long)]
        frames: Option<i64>,
    },
    /// Add a scene: NUMBER IN OUT
    Scene {
        dir: PathBuf,
        number: String,
        r#in: String,
        out: String,
        #[arg(long, default_value = "")]
        title: String,
    },
    /// Create a sound identity, optionally assigning its default sound / variation pool
    Identity {
        dir: PathBuf,
        name: String,
        #[arg(long, default_value = "footsteps")]
        category: String,
        /// Asset id/name/sha for the default sound
        #[arg(long)]
        sound: Option<String>,
        /// Variation pool search words (all matches join the pool)
        #[arg(long)]
        pool: Option<String>,
        #[arg(long, default_value = "round-robin")]
        mode: String,
    },
    /// Index a WAV library folder or sound pack
    Lib {
        dir: PathBuf,
        folder: PathBuf,
        /// Register as a sound pack (reads pack.json)
        #[arg(long)]
        pack: bool,
        /// Pack provides placeholders
        #[arg(long)]
        placeholder: bool,
    },
    /// Search the library
    Search {
        dir: PathBuf,
        query: String,
        #[arg(long)]
        category: Option<String>,
    },
    /// Add a spotting marker
    Spot {
        dir: PathBuf,
        #[arg(long)]
        lane: String,
        #[arg(long, default_value = "point")]
        kind: String,
        #[arg(long)]
        r#in: String,
        #[arg(long)]
        out: Option<String>,
        #[arg(long, default_value = "")]
        label: String,
        #[arg(long)]
        identity: Option<String>,
        /// Comma-separated sub-event TCs for repeated markers
        #[arg(long)]
        steps: Option<String>,
    },
    /// Convert approved markers to cues and list the cue chart
    Cues {
        dir: PathBuf,
        #[arg(long)]
        scene: Option<String>,
    },
    /// Populate placeholder audio at cue positions
    Populate {
        dir: PathBuf,
        #[arg(long)]
        scene: Option<String>,
        /// identity-per-scene | category-per-scene | identity-global
        #[arg(long, default_value = "identity-per-scene")]
        policy: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Replace a cue's sound (scope: cue | scene | all)
    Replace {
        dir: PathBuf,
        cue: String,
        asset: String,
        #[arg(long, default_value = "cue")]
        scope: String,
        /// Confirm a film-wide ripple
        #[arg(long)]
        yes: bool,
    },
    /// Revert a cue to its placeholder (takes and history are kept)
    Revert { dir: PathBuf, cue: String },
    /// Register a recorded take WAV for a cue and use it
    Take {
        dir: PathBuf,
        cue: String,
        wav: PathBuf,
        #[arg(long)]
        at: Option<String>,
        #[arg(long, default_value = "")]
        slate: String,
        #[arg(long, default_value_t = 0)]
        sync_offset: i64,
    },
    /// Everything active at a timecode
    At { dir: PathBuf, tc: String },
    /// Scene summary: gaps, overlaps, missing media, sync warnings
    Summary { dir: PathBuf, scene: String },
    /// Write a cue report (.pdf landscape, .csv or .json by extension)
    Report {
        dir: PathBuf,
        out: PathBuf,
        #[arg(long)]
        scene: Option<String>,
    },
    /// Delivery QA
    Qa {
        dir: PathBuf,
        #[arg(long)]
        hashes: bool,
    },
    /// Export a deliverable package (quick | resolve | pro | archive)
    Export {
        dir: PathBuf,
        #[arg(long, default_value = "resolve")]
        profile: String,
        #[arg(long)]
        scene: Option<String>,
        #[arg(long, default_value_t = 0)]
        handles_ms: u32,
        #[arg(long)]
        float: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Also write an AAF of the editable timeline (linked media in AAF/Media)
        #[arg(long)]
        aaf: bool,
    },
    /// Relink missing media by content hash
    Relink { dir: PathBuf, search: Vec<PathBuf> },
    /// Import a VC Composer timecoded score (JSON contract)
    Score { dir: PathBuf, json: PathBuf },
    /// Import a VC Film Studio script breakdown (JSON contract)
    Script { dir: PathBuf, json: PathBuf },
    /// Run AI timing assist on cues (suggestions only)
    Ai {
        dir: PathBuf,
        cues: Vec<String>,
        /// visual | cadence
        #[arg(long, default_value = "visual")]
        assistant: String,
        /// Turn on local AI assistance for this project
        #[arg(long)]
        enable: bool,
    },
    /// List, accept or reject AI proposals
    Proposals {
        dir: PathBuf,
        #[arg(long)]
        accept: Option<String>,
        #[arg(long)]
        reject: Option<String>,
    },
    /// Rebuild a project from an export package (receiving editor)
    Reconstruct { package: PathBuf, dir: PathBuf },
    /// Timecode conversion: vcsd tc 29.97df 01:00:00;00
    Tc { fps: String, value: String },
    /// Generate the procedural starter sound pack
    Pack { out: PathBuf },
    /// Build a complete demo project end-to-end
    Demo { dir: PathBuf },
    /// Run the app in a web browser: serves the UI and engine on localhost
    Serve {
        /// Project to open on start
        dir: Option<PathBuf>,
        #[arg(long, default_value_t = 7878)]
        port: u16,
        /// Built UI folder (ui/dist)
        #[arg(long, default_value = "ui/dist")]
        ui: PathBuf,
    },
    /// List installed VST3 (and on macOS, Audio Unit) effects
    Plugins {
        /// Open every bundle again instead of using the scan cache
        #[arg(long)]
        rescan: bool,
        /// Show failed bundles and plugin ids
        #[arg(long, short)]
        verbose: bool,
    },
}

fn today() -> String {
    ids::date_from_unix(ids::now_unix())
}

fn open(dir: &Path) -> Result<Session> {
    if let Some(_rec) = Session::recover(dir)? {
        eprintln!("note: a newer autosave exists ({}); open it from the app to recover", dir.join(layout::AUTOSAVE_FILE).display());
    }
    Session::open(dir)
}

fn mutate<T>(dir: &Path, label: &str, f: impl FnOnce(&mut ProjectState) -> Result<T>) -> Result<(Session, T)> {
    let mut s = open(dir)?;
    let v = s.edit(label, f)?;
    s.save()?;
    Ok((s, v))
}

fn parse_kind(k: &str) -> Result<EventType> {
    Ok(match k.to_ascii_lowercase().as_str() {
        "point" | "p" => EventType::Point,
        "range" | "r" => EventType::Range,
        "repeated" | "repeat" | "x" => EventType::Repeated,
        _ => return Err(Error::Invalid(format!("unknown marker kind '{k}'"))),
    })
}

fn parse_mode(m: &str) -> VariationMode {
    match m.to_ascii_lowercase().replace('_', "-").as_str() {
        "random" => VariationMode::Random,
        "locked" => VariationMode::Locked,
        "alternate" | "alternate-feet" | "feet" => VariationMode::AlternateFeet,
        _ => VariationMode::RoundRobin,
    }
}

fn print_cues(s: &ProjectState, scene: Option<&str>) {
    let rows = report::cue_rows(s, scene);
    println!("{:<12} {:<20} {:<26} {:<12} {:<12} {:<9} {:<16} TRACK", "CUE", "IDENTITY", "DESCRIPTION", "IN", "OUT", "EVENTS", "SOURCE");
    for r in rows {
        println!(
            "{:<12} {:<20} {:<26} {:<12} {:<12} {:<9} {:<16} {}",
            r.code,
            trunc(&r.identity, 20),
            trunc(&r.description, 26),
            r.start_tc,
            r.end_tc,
            if r.sub_events > 0 { format!("{} subs", r.sub_events) } else { format!("{:?}", r.event_type).to_lowercase() },
            trunc(&r.source_status, 16),
            r.track
        );
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::New { dir, name, fps, sr, start_tc } => {
            let s = Session::create(&dir, NewProject { name, frame_rate: FrameRate::parse(&fps)?, sample_rate: sr, start_tc })?;
            println!("created {} ({} fps, {} Hz, start {})", dir.display(), s.state.project.frame_rate, sr, s.state.tb().tc_at(0));
        }
        Cmd::Info { dir } => {
            let s = open(&dir)?;
            let st = &s.state;
            let p = &st.project;
            println!("{} — {} @ {} Hz, start {}", p.name, p.frame_rate.label(), p.sample_rate, st.tb().tc_at(0));
            println!("picture: {}", st.current_picture().map(|x| x.label.clone()).unwrap_or_else(|| "(none)".into()));
            println!(
                "scenes {} · identities {} · markers {} · cues {} · tracks {} · clips {} · assets {} · takes {} · performances {}",
                st.scenes.len(),
                st.identities.len(),
                st.markers.len(),
                st.cues.len(),
                st.tracks.len(),
                st.clips.len(),
                st.assets.len(),
                st.takes.len(),
                st.performances.len()
            );
            let mut scenes: Vec<&Scene> = st.scenes.values().collect();
            scenes.sort_by_key(|x| x.start_sample);
            for sc in scenes {
                println!("  scene {:<5} {} – {}  {}", sc.number, st.tb().tc_at(sc.start_sample), st.tb().tc_at(sc.end_sample), sc.title);
            }
            let perf = transport::perf_probe(st, 5.0)?;
            println!("engine: {:.0}× realtime render, ~{:.1} MB decoded audio", perf.realtime_factor, perf.decoded_mb_estimate);
        }
        Cmd::Picture { dir, file, label, start_tc, frames } => {
            let (s, id) = mutate(&dir, "import picture", |st| {
                import_picture(st, PictureImport { path: &file, label, start_tc, duration_frames: frames, change_notes: String::new() })
            })?;
            let p = &s.state.pictures[&id];
            println!(
                "picture {} starts {} ({} frames) fingerprint {}",
                p.label,
                s.state.project.frame_rate.frames_to_tc(p.source_start_frame),
                p.duration_frames,
                &p.fingerprint[..20]
            );
        }
        Cmd::Scene { dir, number, r#in, out, title } => {
            mutate(&dir, "add scene", |st| {
                let tb = st.tb();
                spotting::add_scene(st, &number, &title, tb.sample_at_tc(&r#in)?, tb.sample_at_tc(&out)?)
            })?;
            println!("scene {number} {} – {}", r#in, out);
        }
        Cmd::Identity { dir, name, category, sound, pool, mode } => {
            let (_, id) = mutate(&dir, "add identity", |st| {
                let id = spotting::add_identity(
                    st,
                    NewIdentity {
                        name: name.clone(),
                        category: Category::parse(&category),
                        character: name.split('/').next().map(|x| x.trim().to_string()),
                        surface: None,
                        prop: None,
                    },
                )?;
                let pool_ids: Vec<String> = match &pool {
                    Some(q) => library::search(st, &Query { text: q.clone(), ..Default::default() }).iter().map(|a| a.id.clone()).collect(),
                    None => vec![],
                };
                let def = match &sound {
                    Some(k) => Some(st.find_asset(k)?.id.clone()),
                    None => pool_ids.first().cloned(),
                };
                if let Some(d) = def {
                    spotting::assign_identity_sound(st, &id, &d, pool_ids, parse_mode(&mode))?;
                }
                Ok(id)
            })?;
            println!("identity '{name}' {id}");
        }
        Cmd::Lib { dir, folder, pack, placeholder } => {
            let (_, rep) = mutate(&dir, "import library", |st| {
                if pack || placeholder {
                    Ok(library::import_pack(st, &folder, placeholder)?.1)
                } else {
                    library::import_directory(st, &folder, AssetMeta::default())
                }
            })?;
            println!("indexed {} files ({} failed)", rep.imported.len(), rep.failed.len());
            for (f, e) in rep.failed {
                println!("  ! {f}: {e}");
            }
        }
        Cmd::Search { dir, query, category } => {
            let s = open(&dir)?;
            for a in
                library::search(&s.state, &Query { text: query, category: category.map(|c| Category::parse(&c)), ..Default::default() })
            {
                println!(
                    "{:<38} {:<34} {:>6.2}s {}ch {:?} {}",
                    a.id,
                    a.name,
                    a.length_samples as f64 / a.sample_rate as f64,
                    a.channels,
                    a.kind,
                    a.license.as_ref().map(|l| l.name.as_str()).unwrap_or("(no license)")
                );
            }
        }
        Cmd::Spot { dir, lane, kind, r#in, out, label, identity, steps } => {
            let (_, id) = mutate(&dir, "spot", |st| {
                let tb = st.tb();
                let k = parse_kind(&kind)?;
                let a = tb.sample_at_tc(&r#in)?;
                let z = match &out {
                    Some(o) => tb.sample_at_tc(o)?,
                    None => a,
                };
                let subs = match &steps {
                    Some(list) => list.split(',').map(|t| tb.sample_at_tc(t.trim())).collect::<Result<Vec<_>>>()?,
                    None => vec![],
                };
                let ident = match &identity {
                    Some(i) => Some(st.find_identity(i)?.id.clone()),
                    None => None,
                };
                spotting::add_marker(
                    st,
                    NewMarker {
                        lane_id: st.find_lane(&lane)?.id.clone(),
                        scene_id: None,
                        kind: k,
                        start: a,
                        end: z,
                        label: label.clone(),
                        identity_id: ident,
                        sub_events: subs,
                        origin: TimingOrigin::Manual,
                    },
                )
            })?;
            println!("marker {id}");
        }
        Cmd::Cues { dir, scene } => {
            let (s, new) = mutate(&dir, "markers to cues", |st| {
                let sc = match &scene {
                    Some(k) => Some(st.find_scene(k)?.id.clone()),
                    None => None,
                };
                spotting::markers_to_cues(st, sc.as_deref())
            })?;
            if !new.is_empty() {
                println!("{} new cues", new.len());
            }
            let sc = scene.as_ref().map(|k| s.state.find_scene(k).map(|x| x.id.clone())).transpose()?;
            print_cues(&s.state, sc.as_deref());
        }
        Cmd::Populate { dir, scene, policy, dry_run } => {
            let mut s = open(&dir)?;
            let opts = PopulateOptions {
                policy: match policy.as_str() {
                    "category-per-scene" => TrackPolicy::CategoryPerScene,
                    "identity-global" => TrackPolicy::IdentityGlobal,
                    _ => TrackPolicy::IdentityPerScene,
                },
                scene_id: scene.as_ref().map(|k| s.state.find_scene(k).map(|x| x.id.clone())).transpose()?,
                ..Default::default()
            };
            let rep = if dry_run { builder::preview(&s.state, &opts)? } else { s.edit("populate", |st| populate(st, &opts))? };
            println!(
                "{}{} cues · {} clips · {} new tracks",
                if dry_run { "[preview] " } else { "" },
                rep.cues_populated.len(),
                rep.clips_created,
                rep.tracks_created.len()
            );
            for w in &rep.warnings {
                println!("  ⚠ {}: {}", w.cue_code, w.message);
            }
            if !dry_run {
                s.save()?;
            }
        }
        Cmd::Replace { dir, cue, asset, scope, yes } => {
            let (_, changed) = mutate(&dir, "replace", |st| {
                let c = st.find_cue(&cue)?.id.clone();
                let a = st.find_asset(&asset)?.id.clone();
                let sc = match scope.as_str() {
                    "scene" => ReplaceScope::IdentityInScene,
                    "all" => ReplaceScope::IdentityEverywhere,
                    _ => ReplaceScope::Cue,
                };
                editing::replace_cue_asset(st, &c, &a, sc, yes)
            })?;
            println!("replaced sound on {} cue(s)", changed.len());
        }
        Cmd::Revert { dir, cue } => {
            mutate(&dir, "revert", |st| {
                let c = st.find_cue(&cue)?.id.clone();
                editing::revert_to_placeholder(st, &c)
            })?;
            println!("reverted {cue} to placeholder");
        }
        Cmd::Take { dir, cue, wav, at, slate, sync_offset } => {
            let (_, clip) = mutate(&dir, "take", |st| {
                let c = st.find_cue(&cue)?.id.clone();
                let pos = match &at {
                    Some(t) => Some(st.tb().sample_at_tc(t)?),
                    None => None,
                };
                let take = editing::add_take(
                    st,
                    TakeImport {
                        wav: &wav,
                        cue_id: Some(c.clone()),
                        track_id: None,
                        slate: slate.clone(),
                        timeline_start: pos,
                        sync_offset,
                    },
                )?;
                editing::use_take_for_cue(st, &c, &take)
            })?;
            println!("take placed as clip {clip}");
        }
        Cmd::At { dir, tc } => {
            let s = open(&dir)?;
            let at = s.state.tb().sample_at_tc(&tc)?;
            let v = report::active_at(&s.state, at);
            println!("{} active at {tc}", v.len());
            for a in v {
                println!(
                    "  {:<12} {:<22} {} – {}  [{}] {} | {}",
                    a.code,
                    trunc(if a.identity.is_empty() { &a.description } else { &a.identity }, 22),
                    a.start_tc,
                    a.end_tc,
                    a.category,
                    a.tracks.join(", "),
                    a.buses.join(", ")
                );
            }
        }
        Cmd::Summary { dir, scene } => {
            let s = open(&dir)?;
            let id = s.state.find_scene(&scene)?.id.clone();
            println!("{}", serde_json::to_string_pretty(&report::scene_summary(&s.state, &id)?)?);
        }
        Cmd::Report { dir, out, scene } => {
            let s = open(&dir)?;
            let sc = scene.as_ref().map(|k| s.state.find_scene(k).map(|x| x.id.clone())).transpose()?;
            let date = today();
            match out.extension().and_then(|e| e.to_str()).unwrap_or("pdf") {
                "csv" => std::fs::write(&out, report::rows_to_csv(&s.state, &report::cue_rows(&s.state, sc.as_deref())))?,
                "json" => std::fs::write(&out, serde_json::to_string_pretty(&report::manifest_json(&s.state, sc.as_deref(), &date))?)?,
                _ => std::fs::write(&out, pdf::cue_report(&s.state, sc.as_deref(), &date))?,
            }
            println!("wrote {}", out.display());
        }
        Cmd::Qa { dir, hashes } => {
            let s = open(&dir)?;
            let v = qa::run(&s.state, &qa::QaOptions { verify_hashes: hashes, exclude_temp_refs: true });
            print!("{}", qa::to_markdown(&v));
        }
        Cmd::Export { dir, profile, scene, handles_ms, float, force, date, out, aaf } => {
            let mut s = open(&dir)?;
            let mut o = ExportOptions::new(ExportProfile::parse(&profile)?, &date.unwrap_or_else(today));
            o.handle_samples = handles_ms as i64 * s.state.project.sample_rate as i64 / 1000;
            o.force = force;
            o.out_dir = out;
            o.aaf = aaf;
            if float {
                o.format = wav::SampleFormat::Float32;
            }
            if let Some(sc) = scene {
                o.range = ExportRange::Scene(sc);
            }
            let root = dir.clone();
            let r = s.edit("export", |st| export::export(st, Some(&root), &o))?;
            s.save()?;
            println!("exported {} files to {}", r.files.len(), r.dir.display());
            println!(
                "QA: {} errors, {} warnings, {} info (see Reports/delivery_qa.md)",
                qa::count(&r.qa, qa::Severity::Error),
                qa::count(&r.qa, qa::Severity::Warning),
                qa::count(&r.qa, qa::Severity::Info)
            );
        }
        Cmd::Relink { dir, search } => {
            let (_, rep) =
                mutate(&dir, "relink", |st| relink::relink_by_hash(st, &search.iter().map(|p| p.as_path()).collect::<Vec<_>>()))?;
            println!("scanned {} files · relinked {} · still missing {}", rep.scanned_files, rep.relinked.len(), rep.still_missing.len());
        }
        Cmd::Score { dir, json } => {
            let (s, id) = mutate(&dir, "import score", |st| integrations::import_score(st, &json))?;
            let sc = &s.state.scores[&id];
            println!("score '{}' at {}", sc.title, s.state.tb().tc_at(sc.timeline_start));
            for w in &sc.warnings {
                println!("  ⚠ {w}");
            }
        }
        Cmd::Script { dir, json } => {
            let text = std::fs::read_to_string(&json)?;
            let (_, r) = mutate(&dir, "import script", |st| integrations::import_film_studio(st, &text))?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        Cmd::Ai { dir, cues, assistant, enable } => {
            let (s, ids) = mutate(&dir, "ai assist", |st| {
                if enable {
                    st.project.ai.enabled = true;
                }
                let ids = cues.iter().map(|c| st.find_cue(c).map(|x| x.id.clone())).collect::<Result<Vec<_>>>()?;
                match assistant.as_str() {
                    "cadence" => ai::run(st, &ai::CadenceHeuristic { step_secs: 0.5 }, &ids),
                    _ => ai::run(st, &ai::VisualOnsetAssistant::default(), &ids),
                }
            })?;
            for id in ids {
                let p = &s.state.proposals[&id];
                println!(
                    "{id}  {} conf {:.2}  {} → {}  ({} subs) {}",
                    s.state.cues[&p.cue_id].code,
                    p.confidence,
                    s.state.tb().tc_at(p.before_start),
                    s.state.tb().tc_at(p.start_sample),
                    p.sub_events.len(),
                    p.reason
                );
            }
        }
        Cmd::Proposals { dir, accept, reject } => {
            let mut s = open(&dir)?;
            if let Some(id) = accept {
                s.edit("accept proposal", |st| ai::accept(st, &id, None))?;
                s.save()?;
            }
            if let Some(id) = reject {
                s.edit("reject proposal", |st| ai::reject(st, &id))?;
                s.save()?;
            }
            for p in s.state.proposals.values() {
                println!(
                    "{} {:?} {} conf {:.2} {}",
                    p.id,
                    p.status,
                    s.state.cues.get(&p.cue_id).map(|c| c.code.as_str()).unwrap_or("?"),
                    p.confidence,
                    p.reason
                );
            }
        }
        Cmd::Reconstruct { package, dir } => {
            let (state, rep) = handoff::reconstruct(&package)?;
            layout::create_project_dirs(&dir)?;
            let mut s = Session::new(state);
            s.root = Some(dir.clone());
            s.save()?;
            println!(
                "rebuilt {} tracks and {} cues into {} (checksum failures: {})",
                rep.tracks,
                rep.cues,
                dir.display(),
                rep.checksum_failures.len()
            );
        }
        Cmd::Tc { fps, value } => {
            let r = FrameRate::parse(&fps)?;
            match value.parse::<i64>() {
                Ok(frames) => println!("{}", r.frames_to_tc(frames)),
                Err(_) => {
                    let f = r.tc_to_frames(&value)?;
                    println!(
                        "{f} frames · {:.6} s · {} samples @48k",
                        f as f64 * r.den as f64 / r.num as f64,
                        r.sample_at_frame(f, 48_000)
                    );
                }
            }
        }
        Cmd::Pack { out } => {
            synth::write_starter_pack(&out, 48_000)?;
            println!("wrote starter pack to {}", out.display());
        }
        Cmd::Demo { dir } => {
            for line in vcsd_engine::demo::run(&dir)? {
                println!("{line}");
            }
        }
        Cmd::Plugins { rescan, verbose } => {
            let engine = vcsd_engine::Engine::new();
            let v = engine.call("plugins.scan", serde_json::json!({"force": rescan})).map_err(Error::Invalid)?;
            for p in v["plugins"].as_array().into_iter().flatten() {
                let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
                print!("{:<5} {:<32} {:<24} {}", s("format"), s("name"), s("vendor"), s("category"));
                if verbose {
                    print!("  {}  {}", s("id"), s("path"));
                }
                println!();
            }
            let failed = v["failed"].as_array().map(|a| a.len()).unwrap_or(0);
            println!(
                "{} effects in {} bundles ({} scanned now, {} failed){}",
                v["plugins"].as_array().map(|a| a.len()).unwrap_or(0),
                v["bundles"],
                v["scanned"],
                failed,
                if failed > 0 && !verbose { " — use -v for details" } else { "" }
            );
            if verbose {
                for f in v["failed"].as_array().into_iter().flatten() {
                    println!("failed: {}: {}", f["path"].as_str().unwrap_or(""), f["error"].as_str().unwrap_or(""));
                }
            }
        }
        Cmd::Serve { dir, port, ui } => {
            let engine = vcsd_engine::Engine::new();
            if let Some(d) = dir {
                engine.call("project.open", serde_json::json!({"dir": d})).map_err(Error::Invalid)?;
            }
            println!("VC Sound Design (browser mode) → http://127.0.0.1:{port}/");
            vcsd_engine::serve::serve(engine, port, ui)?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    // `vcsd --vcsd-plugin-scan <bundle>`: the out-of-process plugin scanner
    vcsd_engine::vcsd_plugins::scan::child_main();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

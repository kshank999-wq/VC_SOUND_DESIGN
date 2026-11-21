//! Sound library: WAV indexing, sound packs, search, and variation selection (spec §9).

use crate::error::{invalid, Result};
use crate::ids::{new_id, now_unix};
use crate::model::*;
use crate::state::ProjectState;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct AssetMeta {
    pub kind: AssetKind,
    pub category: Option<Category>,
    pub tags: Vec<String>,
    pub license: Option<License>,
    pub pack_id: Option<String>,
    pub loopable: Option<bool>,
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1 && !t.chars().all(|c| c.is_ascii_digit()))
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Index one WAV file. Identical content (same SHA-256) is deduplicated and
/// the existing asset is returned, so re-importing never forks identities.
pub fn import_wav(s: &mut ProjectState, path: &Path, meta: AssetMeta) -> Result<String> {
    let abs = std::fs::canonicalize(path)?;
    let info = crate::wav::read_info(&abs)?;
    let sha = crate::hash::sha256_file(&abs)?;
    if let Some(a) = s.assets.values().find(|a| a.sha256 == sha) {
        return Ok(a.id.clone());
    }
    let name = abs.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut tags = meta.tags.clone();
    for t in tokens(&name) {
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    let lower = name.to_ascii_lowercase();
    let loopable =
        meta.loopable.unwrap_or(lower.contains("loop") || lower.contains("amb") || matches!(meta.category, Some(Category::Background)));
    let id = new_id("ast");
    s.assets.insert(
        id.clone(),
        Asset {
            id: id.clone(),
            name,
            path: abs.to_string_lossy().to_string(),
            sha256: sha,
            size_bytes: std::fs::metadata(&abs)?.len(),
            sample_rate: info.sample_rate,
            channels: info.channels,
            bits_per_sample: info.format.bits(),
            length_samples: info.frames as i64,
            kind: meta.kind,
            category: meta.category,
            tags,
            license: meta.license,
            pack_id: meta.pack_id,
            time_reference: info.bext.as_ref().map(|b| b.time_reference),
            peak_dbfs: None,
            loopable,
            imported_at: now_unix(),
        },
    );
    Ok(id)
}

fn walk_wavs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk_wavs(&p, out)?;
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav") || e.eq_ignore_ascii_case("bwf")) {
            out.push(p);
        }
    }
    Ok(())
}

pub fn find_wavs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v = vec![];
    walk_wavs(dir, &mut v)?;
    Ok(v)
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Index a library folder. The first directory level under `dir` is used as
/// the category hint (e.g. `footsteps/`, `doors/`, `ambience/`), and every
/// path component becomes a search tag.
pub fn import_directory(s: &mut ProjectState, dir: &Path, base: AssetMeta) -> Result<ImportReport> {
    if !dir.is_dir() {
        return Err(invalid(format!("{} is not a directory", dir.display())));
    }
    let mut rep = ImportReport::default();
    for p in find_wavs(dir)? {
        let rel = p.strip_prefix(dir).unwrap_or(&p);
        let comps: Vec<String> =
            rel.parent().map(|pp| pp.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect()).unwrap_or_default();
        let mut meta = base.clone();
        if meta.category.is_none() {
            meta.category = comps.first().map(|c| category_hint(c));
        }
        for c in &comps {
            for t in tokens(c) {
                if !meta.tags.contains(&t) {
                    meta.tags.push(t);
                }
            }
        }
        match import_wav(s, &p, meta) {
            Ok(id) => rep.imported.push(id),
            Err(e) => rep.failed.push((p.display().to_string(), e.to_string())),
        }
    }
    Ok(rep)
}

fn category_hint(dir: &str) -> Category {
    let d = dir.to_ascii_lowercase();
    if d.contains("foot") || d.contains("step") {
        Category::Footsteps
    } else if ["door", "latch", "prop", "handle", "cloth", "fabric", "key"].iter().any(|k| d.contains(k)) {
        Category::Props
    } else if ["amb", "bg", "atmos", "room", "wind", "bird", "tree", "exterior"].iter().any(|k| d.contains(k)) {
        Category::Background
    } else if d.contains("music") || d.contains("score") {
        Category::Music
    } else if d.contains("design") {
        Category::DesignFx
    } else {
        Category::HardFx
    }
}

/// Register a downloadable/starter sound pack (kept outside app binaries) and
/// index its contents. Pack root may contain an optional `pack.json`
/// `{ "name", "version", "license": {..} }`.
pub fn import_pack(s: &mut ProjectState, root: &Path, placeholder: bool) -> Result<(String, ImportReport)> {
    let root = std::fs::canonicalize(root)?;
    let manifest = root.join("pack.json");
    let (name, version, license) = if manifest.exists() {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest)?)?;
        (
            v["name"].as_str().unwrap_or("Pack").to_string(),
            v["version"].as_str().unwrap_or("1").to_string(),
            serde_json::from_value(v["license"].clone()).unwrap_or_default(),
        )
    } else {
        (root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), "1".into(), License::default())
    };
    let id = s.packs.values().find(|p| p.root == root.to_string_lossy()).map(|p| p.id.clone()).unwrap_or_else(|| new_id("pck"));
    s.packs.insert(
        id.clone(),
        SoundPack { id: id.clone(), name, version, root: root.to_string_lossy().to_string(), license: license.clone(), placeholder },
    );
    let rep = import_directory(
        s,
        &root,
        AssetMeta {
            kind: if placeholder { AssetKind::Placeholder } else { AssetKind::Library },
            license: Some(license),
            pack_id: Some(id.clone()),
            ..Default::default()
        },
    )?;
    Ok((id, rep))
}

#[derive(Debug, Clone, Default)]
pub struct Query {
    pub text: String,
    pub category: Option<Category>,
    pub kind: Option<AssetKind>,
    pub tags: Vec<String>,
    pub redistributable_only: bool,
}

/// Search assets. All query words must match the name, tags or file name
/// (not the folders above it: a library in "SFX 2024-02" must not match "02").
pub fn search<'a>(s: &'a ProjectState, q: &Query) -> Vec<&'a Asset> {
    // unlike tag extraction, queries keep numeric words ("smash 02")
    let words: Vec<String> =
        q.text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(|t| t.to_ascii_lowercase()).collect();
    let mut v: Vec<&Asset> = s
        .assets
        .values()
        .filter(|a| q.category.as_ref().is_none_or(|c| a.category.as_ref() == Some(c)))
        .filter(|a| q.kind.is_none_or(|k| a.kind == k))
        .filter(|a| !q.redistributable_only || a.license.as_ref().is_some_and(|l| l.redistributable))
        .filter(|a| q.tags.iter().all(|t| a.tags.iter().any(|x| x.eq_ignore_ascii_case(t))))
        .filter(|a| {
            let file = std::path::Path::new(&a.path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let hay = format!("{} {} {}", a.name, a.tags.join(" "), file).to_ascii_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect();
    v.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    v
}

/// Small deterministic hash (FNV-1a) used for reproducible "random" variation picks.
pub fn stable_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Choose the asset for occurrence `index` of an identity (spec §9 variation pools).
pub fn pick_variation(ident: &SoundIdentity, index: usize, seed: &str) -> Option<String> {
    let pool: &[String] = &ident.variation_pool;
    match ident.variation_mode {
        VariationMode::Locked => ident.default_asset_id.clone().or_else(|| pool.first().cloned()),
        _ if pool.is_empty() => ident.default_asset_id.clone(),
        VariationMode::RoundRobin => Some(pool[index % pool.len()].clone()),
        VariationMode::Random => Some(pool[(stable_hash(&format!("{seed}:{index}")) % pool.len() as u64) as usize].clone()),
        // Without a separate right-foot pool, alternate through the single pool.
        VariationMode::AlternateFeet if ident.alt_pool.is_empty() => Some(pool[index % pool.len()].clone()),
        VariationMode::AlternateFeet => {
            let side = if index.is_multiple_of(2) { pool } else { &ident.alt_pool[..] };
            Some(side[(index / 2) % side.len()].clone())
        }
    }
}

/// Best placeholder for a category when no identity assignment exists:
/// prefer placeholder-pack assets of that category, then any library asset.
pub fn default_placeholder(s: &ProjectState, cat: &Category, hint: &str) -> Option<String> {
    let words = tokens(hint);
    let score =
        |a: &Asset| words.iter().filter(|w| a.tags.iter().any(|t| t == *w) || a.name.to_ascii_lowercase().contains(w.as_str())).count();
    let mut cands: Vec<&Asset> = s
        .assets
        .values()
        .filter(|a| a.category.as_ref() == Some(cat) && matches!(a.kind, AssetKind::Placeholder | AssetKind::Library))
        .collect();
    cands.sort_by(|a, b| {
        (score(b), (b.kind == AssetKind::Placeholder) as u8, &a.name).cmp(&(score(a), (a.kind == AssetKind::Placeholder) as u8, &b.name))
    });
    cands.first().map(|a| a.id.clone())
}

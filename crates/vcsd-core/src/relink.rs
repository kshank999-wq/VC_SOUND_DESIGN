//! Missing-media detection and relink by content hash (spec §9, §12).

use crate::error::Result;
use crate::state::ProjectState;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default, Serialize)]
pub struct RelinkReport {
    pub relinked: Vec<(String, String)>,
    pub still_missing: Vec<String>,
    pub scanned_files: usize,
}

pub fn missing_assets(s: &ProjectState) -> Vec<String> {
    s.assets.values().filter(|a| !Path::new(&a.path).exists()).map(|a| a.id.clone()).collect()
}

/// Scan folders for WAVs whose SHA-256 matches a missing asset and repoint
/// it. Cue and clip data are untouched — only `asset.path` changes. Files
/// are pre-filtered by byte size so only plausible candidates are hashed.
pub fn relink_by_hash(s: &mut ProjectState, search_dirs: &[&Path]) -> Result<RelinkReport> {
    let mut rep = RelinkReport::default();
    let missing = missing_assets(s);
    if missing.is_empty() {
        return Ok(rep);
    }
    let sizes: HashMap<u64, Vec<String>> = missing.iter().fold(HashMap::new(), |mut m, id| {
        m.entry(s.assets[id].size_bytes).or_default().push(id.clone());
        m
    });
    let mut found: HashMap<String, String> = HashMap::new();
    for d in search_dirs {
        for f in crate::library::find_wavs(d)? {
            rep.scanned_files += 1;
            let Ok(meta) = std::fs::metadata(&f) else { continue };
            let Some(cands) = sizes.get(&meta.len()) else { continue };
            let h = crate::hash::sha256_file(&f)?;
            for id in cands {
                if s.assets[id].sha256 == h && !found.contains_key(id) {
                    found.insert(id.clone(), std::fs::canonicalize(&f)?.to_string_lossy().to_string());
                }
            }
        }
    }
    for id in missing {
        match found.remove(&id) {
            Some(p) => {
                let a = s.assets.get_mut(&id).unwrap();
                a.path = p.clone();
                rep.relinked.push((id, p));
            }
            None => rep.still_missing.push(id),
        }
    }
    Ok(rep)
}

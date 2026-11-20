//! Project folder conventions (spec §15.3).

use crate::error::Result;
use std::path::{Path, PathBuf};

pub const PROJECT_FILE: &str = "Project.vcsd";
pub const AUTOSAVE_FILE: &str = "Project.vcsd.autosave";

pub const DIRS: &[&str] = &[
    "Picture",
    "Scripts",
    "Audio/Imported",
    "Audio/LibraryRefs",
    "Audio/RecordedFoley",
    "Audio/Rendered",
    "Cache",
    "Reports",
    "Exports",
    "Manifests",
];

pub fn create_project_dirs(root: &Path) -> Result<()> {
    for d in DIRS {
        std::fs::create_dir_all(root.join(d))?;
    }
    Ok(())
}

pub fn project_file(root: &Path) -> PathBuf {
    root.join(PROJECT_FILE)
}

pub fn autosave_file(root: &Path) -> PathBuf {
    root.join(AUTOSAVE_FILE)
}

/// Make a string safe for use as a file name component.
pub fn safe_name(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

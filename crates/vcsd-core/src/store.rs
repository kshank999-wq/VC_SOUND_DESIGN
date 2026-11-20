//! Native project format: a versioned SQLite database (`Project.vcsd`) holding
//! one table per entity (id + JSON document) plus a `meta` table with the
//! schema version. Media is always referenced externally, never embedded.
//!
//! Saves are atomic: the database is written to a temporary file and renamed
//! over the original, so a crash mid-save never corrupts the project.

use crate::error::{invalid, Result};
use crate::model::SCHEMA_VERSION;
use crate::state::{Map, ProjectState};
use rusqlite::{params, Connection};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;

macro_rules! collections {
    ($m:ident) => {
        $m!(
            pictures,
            scenes,
            characters,
            script_refs,
            identities,
            lanes,
            markers,
            cues,
            assets,
            packs,
            buses,
            tracks,
            clips,
            automation,
            takes,
            performances,
            performance_events,
            proposals,
            scores,
            exports
        )
    };
}

fn put_all<T: Serialize>(c: &Connection, table: &str, m: &Map<T>) -> Result<()> {
    c.execute_batch(&format!("CREATE TABLE {table} (id TEXT PRIMARY KEY NOT NULL, data TEXT NOT NULL);"))?;
    let mut st = c.prepare(&format!("INSERT INTO {table} (id, data) VALUES (?1, ?2)"))?;
    for (id, v) in m {
        st.execute(params![id, serde_json::to_string(v)?])?;
    }
    Ok(())
}

fn get_all<T: DeserializeOwned>(c: &Connection, table: &str) -> Result<Map<T>> {
    let exists: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1", [table], |r| r.get(0))?;
    let mut out = Map::new();
    if exists == 0 {
        return Ok(out);
    }
    let mut st = c.prepare(&format!("SELECT id, data FROM {table} ORDER BY id"))?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, data) = row?;
        out.insert(id, serde_json::from_str(&data)?);
    }
    Ok(out)
}

pub fn save(path: &Path, s: &ProjectState) -> Result<()> {
    let tmp = path.with_extension("vcsd.tmp");
    let _ = std::fs::remove_file(&tmp);
    {
        let mut c = Connection::open(&tmp)?;
        let tx = c.transaction()?;
        tx.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);")?;
        tx.execute("INSERT INTO meta VALUES ('schema_version', ?1)", [SCHEMA_VERSION.to_string()])?;
        tx.execute("INSERT INTO meta VALUES ('app', 'vc-sound-design')", [])?;
        tx.execute("INSERT INTO meta VALUES ('project', ?1)", [serde_json::to_string(&s.project)?])?;
        macro_rules! put { ($($t:ident),*) => { $( put_all(&tx, stringify!($t), &s.$t)?; )* } }
        collections!(put);
        tx.execute_batch("CREATE TABLE audit (seq INTEGER PRIMARY KEY, at INTEGER NOT NULL, action TEXT NOT NULL);")?;
        {
            let mut st = tx.prepare("INSERT INTO audit VALUES (?1, ?2, ?3)")?;
            for a in &s.audit {
                st.execute(params![a.seq as i64, a.at, a.action])?;
            }
        }
        tx.commit()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<ProjectState> {
    if !path.exists() {
        return Err(crate::error::not_found("project file", &path.display().to_string()));
    }
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let ver: String = c.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0))?;
    let ver: u32 = ver.parse().map_err(|_| invalid("corrupt schema_version"))?;
    if ver > SCHEMA_VERSION {
        return Err(invalid(format!("project schema v{ver} is newer than this build supports (v{SCHEMA_VERSION})")));
    }
    let project: String = c.query_row("SELECT value FROM meta WHERE key='project'", [], |r| r.get(0))?;
    let mut s = ProjectState { project: serde_json::from_str(&project)?, ..Default::default() };
    macro_rules! get { ($($t:ident),*) => { $( s.$t = get_all(&c, stringify!($t))?; )* } }
    collections!(get);
    let mut st = c.prepare("SELECT seq, at, action FROM audit ORDER BY seq")?;
    let rows = st.query_map([], |r| Ok(crate::model::AuditEvent { seq: r.get::<_, i64>(0)? as u64, at: r.get(1)?, action: r.get(2)? }))?;
    for r in rows {
        s.audit.push(r?);
    }
    migrate(&mut s, ver);
    Ok(s)
}

/// Forward migrations between schema versions run here.
fn migrate(s: &mut ProjectState, _from: u32) {
    s.project.schema_version = SCHEMA_VERSION;
}

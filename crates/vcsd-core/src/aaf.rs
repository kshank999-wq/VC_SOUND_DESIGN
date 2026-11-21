//! AAF export (Tier-2 interchange, spec §15.2).
//!
//! Writes an AAF file (SMPTE 377, structured storage with 4 KiB sectors) that
//! rebuilds the editable timeline in another editor:
//!
//! - one composition with a timecode track at the project frame rate;
//! - one sound track per channel of every project track (stereo → L/R),
//!   overlapping clips split onto extra tracks, each clip a `SourceClip` into
//!   linked external WAV media (Avid AMA-style master → file → import mobs),
//!   with linear fades and clip gain (track fader folded in) as the standard
//!   Mono Audio Gain effect;
//! - track volume automation as a time-varying Mono Audio Gain around the
//!   track (Amplitude `VaryingValue`, linear control points);
//! - pan: Mono Audio Pan on mono tracks (static or automated); stereo tracks
//!   as hard-panned L/R with the balance in each side's gain;
//! - bus levels: AAF has no buses, so each bus's static gain, volume lane and
//!   balance lane (down to the master) are folded into the tracks it carries;
//! - cue markers (`CommentMarker`s with code and description).
//!
//! The fixed parts of the file — the meta-dictionary (class and type
//! definitions) and the dictionary (data/container/operation/parameter
//! definitions) — come from `assets/aaf_template.aaf`, generated with pyaaf2
//! by `scripts/aaf/make_template.py`. Everything else is written here.
//!
//! Optionally, tracks with plugins or insert effects are printed: rendered
//! pre-fader with their inserts and plugin automation, and linked as one clip
//! (fader, volume, pan and bus levels stay editable around it).
//!
//! What AAF can't carry (unprinted plugins and their automation, bus routing and effects, loop
//! crossfades, mutes…) is listed in the [`AafReport`]: the rendered stems in
//! the same package include all of it.

use crate::error::{invalid, Result};
use crate::layout::safe_name;
use crate::model::*;
use crate::state::ProjectState;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const TEMPLATE: &[u8] = include_bytes!("../assets/aaf_template.aaf");

// ---------------------------------------------------------------- object model

pub(crate) mod class {
    pub const CONTENT_STORAGE: &str = "0d010101-0101-1800-060e-2b3402060101";
    pub const COMPOSITION_MOB: &str = "0d010101-0101-3500-060e-2b3402060101";
    pub const MASTER_MOB: &str = "0d010101-0101-3600-060e-2b3402060101";
    pub const SOURCE_MOB: &str = "0d010101-0101-3700-060e-2b3402060101";
    pub const TIMELINE_SLOT: &str = "0d010101-0101-3b00-060e-2b3402060101";
    pub const EVENT_SLOT: &str = "0d010101-0101-3900-060e-2b3402060101";
    pub const SEQUENCE: &str = "0d010101-0101-0f00-060e-2b3402060101";
    pub const SOURCE_CLIP: &str = "0d010101-0101-1100-060e-2b3402060101";
    pub const FILLER: &str = "0d010101-0101-0900-060e-2b3402060101";
    pub const TIMECODE: &str = "0d010101-0101-1400-060e-2b3402060101";
    pub const OPERATION_GROUP: &str = "0d010101-0101-0a00-060e-2b3402060101";
    pub const CONSTANT_VALUE: &str = "0d010101-0101-3d00-060e-2b3402060101";
    pub const VARYING_VALUE: &str = "0d010101-0101-3e00-060e-2b3402060101";
    pub const CONTROL_POINT: &str = "0d010101-0101-1900-060e-2b3402060101";
    pub const COMMENT_MARKER: &str = "0d010101-0101-0800-060e-2b3402060101";
    pub const WAVE_DESCRIPTOR: &str = "0d010101-0101-2c00-060e-2b3402060101";
    pub const IMPORT_DESCRIPTOR: &str = "0d010101-0101-4a00-060e-2b3402060101";
    pub const NETWORK_LOCATOR: &str = "0d010101-0101-3200-060e-2b3402060101";
}

/// Definition keys (AUIDs as stored, i.e. little-endian GUID bytes).
pub(crate) mod key {
    pub const SOUND: &str = "0202030100020000060e2b3404010101";
    pub const TIMECODE: &str = "0102030100010000060e2b3404010101";
    pub const DESCRIPTIVE: &str = "0102030100100000060e2b3404010101";
    pub const CONTAINER_AAF: &str = "71b51343bad8d211809b006008143e6f";
    pub const OP_MONO_AUDIO_GAIN: &str = "94a82e9d6809d3118a380050040ef7d2";
    pub const PARAM_AMPLITUDE: &str = "212396e46722d3118a4c0050040ef7d2";
    pub const OP_MONO_AUDIO_PAN: &str = "93a82e9d6809d3118a380050040ef7d2";
    pub const PARAM_PAN: &str = "222396e46722d3118a4c0050040ef7d2";
    pub const TYPE_RATIONAL: &str = "0001010300000000060e2b3401040101";
    pub const INTERP_LINEAR: &str = "a4856c5bde0ed31180a9006008143e6f";
}

/// Dictionary sets that weak references point into (property id under Dictionary-3b04).
#[derive(Clone, Copy)]
enum Defs {
    Data = 0x2605,
    Operation = 0x2603,
    Container = 0x2608,
    Interpolation = 0x2609,
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

enum Val {
    Data(Vec<u8>),
    Strong(Box<Obj>),
    Vector(Vec<Obj>),
    /// Strong reference set keyed by a property (`MobID` for mobs).
    Set {
        key_pid: u16,
        items: Vec<(Vec<u8>, Obj)>,
    },
    Weak {
        defs: Defs,
        key: Vec<u8>,
    },
}

struct Obj {
    class: &'static str,
    props: Vec<(u16, &'static str, Val)>,
}

impl Obj {
    fn new(class: &'static str) -> Obj {
        Obj { class, props: vec![] }
    }
    fn data(mut self, pid: u16, name: &'static str, v: Vec<u8>) -> Obj {
        self.props.push((pid, name, Val::Data(v)));
        self
    }
    fn strong(mut self, pid: u16, name: &'static str, o: Obj) -> Obj {
        self.props.push((pid, name, Val::Strong(Box::new(o))));
        self
    }
    fn vector(mut self, pid: u16, name: &'static str, v: Vec<Obj>) -> Obj {
        self.props.push((pid, name, Val::Vector(v)));
        self
    }
    fn weak(mut self, pid: u16, name: &'static str, defs: Defs, key: &str) -> Obj {
        self.props.push((pid, name, Val::Weak { defs, key: hex(key) }));
        self
    }
}

// ---------------------------------------------------------------- encodings

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()).collect()
}
fn u8v(v: u8) -> Vec<u8> {
    vec![v]
}
fn u16v(v: u16) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}
fn u32v(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}
fn i64v(v: i64) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}
fn rational(num: i32, den: i32) -> Vec<u8> {
    [num.to_le_bytes(), den.to_le_bytes()].concat()
}

/// AAF TimeStamp: year u16, month, day, hour, minute, second, 1/250 s.
fn timestamp(unix: i64) -> Vec<u8> {
    let (y, m, d) = crate::ids::civil_from_days(unix.div_euclid(86_400));
    let s = unix.rem_euclid(86_400);
    let mut v = (y as u16).to_le_bytes().to_vec();
    v.extend([m as u8, d as u8, (s / 3600) as u8, ((s / 60) % 60) as u8, (s % 60) as u8, 0]);
    v
}

/// SMPTE UMID-style MobID (32 bytes): the AAF label prefix + a unique material number.
fn new_mob_id() -> Vec<u8> {
    let mut v = hex("060a2b340101010501010f2013000000");
    v.extend(Uuid::now_v7().to_bytes_le());
    v
}

/// pyaaf2's `mangle_name`: property name + "-" + pid, squeezed to fit `size`
/// (32 for single references; 22 for vectors and sets, whose children get `{n}`).
fn mangle(name: &str, pid: u16, size: usize) -> String {
    let p = format!("{pid:x}");
    let max = size - p.len() - 2;
    let chars: Vec<char> = name.chars().collect();
    let squeezed: String = if chars.len() <= max {
        name.to_string()
    } else {
        let half = max / 2;
        (0..max)
            .map(|i| match i.cmp(&half) {
                std::cmp::Ordering::Less => chars[i],
                std::cmp::Ordering::Equal => '-',
                std::cmp::Ordering::Greater => chars[chars.len() - (max - i)],
            })
            .collect()
    };
    format!("{squeezed}-{p}")
}

// ---------------------------------------------------------------- storage

/// Weak-reference table of the template (`/referenced properties`): pid paths.
fn weak_table<F: Read + Seek>(cf: &mut cfb::CompoundFile<F>) -> Result<Vec<Vec<u16>>> {
    let mut b = vec![];
    cf.open_stream("/referenced properties")?.read_to_end(&mut b)?;
    if b.len() < 7 || b[0] != 0x4c {
        return Err(invalid("AAF template: bad weak reference table"));
    }
    let count = u16::from_le_bytes([b[1], b[2]]) as usize;
    let mut out = vec![];
    let mut cur = vec![];
    let mut i = 7;
    while i + 1 < b.len() && out.len() < count {
        let pid = u16::from_le_bytes([b[i], b[i + 1]]);
        i += 2;
        if pid == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(pid);
        }
    }
    Ok(out)
}

/// Properties stream ↔ list of (pid, stored form, data).
pub(crate) fn parse_props(b: &[u8]) -> Result<Vec<(u16, u16, Vec<u8>)>> {
    if b.len() < 4 {
        return Err(invalid("AAF: short properties stream"));
    }
    let n = u16::from_le_bytes([b[2], b[3]]) as usize;
    let mut heads = vec![];
    for k in 0..n {
        let o = 4 + k * 6;
        let r = |a: usize| u16::from_le_bytes([b[o + a], b[o + a + 1]]);
        heads.push((r(0), r(2), r(4) as usize));
    }
    let mut off = 4 + n * 6;
    let mut out = vec![];
    for (pid, fmt, len) in heads {
        out.push((pid, fmt, b.get(off..off + len).ok_or_else(|| invalid("AAF: truncated properties"))?.to_vec()));
        off += len;
    }
    Ok(out)
}

fn props_bytes(props: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut v = vec![0x4c, 0x20];
    v.extend((props.len() as u16).to_le_bytes());
    for (pid, fmt, d) in props {
        v.extend(pid.to_le_bytes());
        v.extend(fmt.to_le_bytes());
        v.extend((d.len() as u16).to_le_bytes());
    }
    for (_, _, d) in props {
        v.extend(d);
    }
    v
}

struct Writer<'a, F: Read + Write + Seek> {
    cf: &'a mut cfb::CompoundFile<F>,
    weak: &'a [Vec<u16>],
}

impl<F: Read + Write + Seek> Writer<'_, F> {
    fn weak_index(&self, defs: Defs) -> Result<u16> {
        self.weak
            .iter()
            .position(|p| p.as_slice() == [2, 0x3b04, defs as u16])
            .map(|i| i as u16)
            .ok_or_else(|| invalid("AAF template lacks a dictionary weak reference target"))
    }

    fn stream(&mut self, path: &str, data: &[u8]) -> Result<()> {
        self.cf.create_stream(path)?.write_all(data)?;
        Ok(())
    }

    fn object(&mut self, path: &str, o: &Obj) -> Result<()> {
        if !self.cf.is_storage(path) {
            self.cf.create_storage(path)?;
        }
        self.cf.set_storage_clsid(path, Uuid::parse_str(o.class).unwrap())?;
        let mut props = vec![];
        for (pid, name, v) in &o.props {
            let size = if matches!(v, Val::Vector(_) | Val::Set { .. }) { 22 } else { 32 };
            let m = mangle(name, *pid, size);
            match v {
                Val::Data(d) => props.push((*pid, 0x82, d.clone())),
                Val::Weak { defs, key } => {
                    let mut d = self.weak_index(*defs)?.to_le_bytes().to_vec();
                    d.extend(0x1b01u16.to_le_bytes()); // keyed by DefinitionObject Identification
                    d.push(16);
                    d.extend(key);
                    props.push((*pid, 0x02, d));
                }
                Val::Strong(child) => {
                    props.push((*pid, 0x22, utf16z(&m)));
                    self.object(&format!("{path}/{m}"), child)?;
                }
                Val::Vector(items) => {
                    props.push((*pid, 0x32, utf16z(&m)));
                    let n = items.len() as u32;
                    let mut idx = [n.to_le_bytes(), n.to_le_bytes(), u32::MAX.to_le_bytes()].concat();
                    for (i, child) in items.iter().enumerate() {
                        idx.extend((i as u32).to_le_bytes());
                        self.object(&format!("{path}/{m}{{{i:x}}}"), child)?;
                    }
                    self.stream(&format!("{path}/{m} index"), &idx)?;
                }
                Val::Set { key_pid, items } => {
                    props.push((*pid, 0x3a, utf16z(&m)));
                    let n = items.len() as u32;
                    let mut idx = [n.to_le_bytes(), n.to_le_bytes(), u32::MAX.to_le_bytes()].concat();
                    idx.extend(key_pid.to_le_bytes());
                    idx.push(items.first().map(|x| x.0.len() as u8).unwrap_or(0));
                    for (i, (k, child)) in items.iter().enumerate() {
                        idx.extend((i as u32).to_le_bytes());
                        idx.extend(1u32.to_le_bytes()); // reference count
                        idx.extend(k);
                        self.object(&format!("{path}/{m}{{{i:x}}}"), child)?;
                    }
                    self.stream(&format!("{path}/{m} index"), &idx)?;
                }
            }
        }
        self.stream(&format!("{path}/properties"), &props_bytes(&props))
    }
}

const CONTENT_PATH: &str = "/Header-2/Content-3b03";

/// A complete AAF file: the template's fixed parts plus `content`.
fn write_file(content: &Obj, unix_now: i64) -> Result<Vec<u8>> {
    let mut src = cfb::CompoundFile::open(Cursor::new(TEMPLATE))?;
    let weak = weak_table(&mut src)?;
    let entries: Vec<(PathBuf, bool, Uuid)> = src.walk().map(|e| (e.path().to_path_buf(), e.is_storage(), *e.clsid())).collect();
    let mut dst = cfb::CompoundFile::create_with_version(cfb::Version::V4, Cursor::new(Vec::new()))?;
    for (path, storage, clsid) in entries {
        let p = path.to_string_lossy().replace('\\', "/");
        if p.starts_with(CONTENT_PATH) {
            continue; // the template's demo content is replaced
        }
        if storage {
            if p != "/" {
                dst.create_storage(&p)?;
            }
            dst.set_storage_clsid(&p, clsid)?;
            continue;
        }
        let mut data = vec![];
        src.open_stream(&p)?.read_to_end(&mut data)?;
        // stamp the header and our identification with this export's time and version
        if p == "/Header-2/properties" || (p.starts_with("/Header-2/Identifi") && p.ends_with("{0}/properties")) {
            let mut props = parse_props(&data)?;
            for (pid, _, d) in props.iter_mut() {
                match *pid {
                    0x3b02 | 0x3c06 => *d = timestamp(unix_now),
                    0x3c04 => *d = utf16z(env!("CARGO_PKG_VERSION")),
                    _ => {}
                }
            }
            data = props_bytes(&props);
        }
        dst.create_stream(&p)?.write_all(&data)?;
    }
    Writer { cf: &mut dst, weak: &weak }.object(CONTENT_PATH, content)?;
    dst.flush()?;
    Ok(dst.into_inner().into_inner())
}

// ---------------------------------------------------------------- timeline

/// One source file referenced by the AAF.
#[derive(Debug, Clone)]
pub struct AafMedia {
    /// Absolute path of the WAV the AAF links to.
    pub path: PathBuf,
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: i64,
}

/// What was written and what AAF could not carry.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AafReport {
    pub file: String,
    pub composition: String,
    pub tracks: usize,
    pub clips: usize,
    pub markers: usize,
    pub media_files: usize,
    /// Each kind of loss once, with how often it applies.
    pub losses: Vec<String>,
    /// Tracks whose volume automation is in the AAF (as time-varying track gain).
    pub automated_tracks: usize,
    /// Tracks with a pan (static or automated) in the AAF.
    pub panned_tracks: usize,
    /// Tracks printed with their plugins and insert effects.
    pub printed_tracks: usize,
}

/// RIFF header + `fmt ` chunk of a WAV (the WAVEDescriptor `Summary`).
fn wave_summary(path: &Path) -> Result<Vec<u8>> {
    let b = std::fs::read(path)?;
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(invalid(format!("{} is not a WAV file", path.display())));
    }
    let mut i = 12;
    while i + 8 <= b.len() {
        let size = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        if &b[i..i + 4] == b"fmt " {
            let mut v = b[0..12].to_vec();
            v.extend(&b[i..(i + 8 + size).min(b.len())]);
            return Ok(v);
        }
        i += 8 + size + (size & 1);
    }
    Err(invalid(format!("{} has no fmt chunk", path.display())))
}

fn mob_head(o: Obj, name: &str, id: &[u8], now: i64) -> Obj {
    o.data(0x4402, "Name", utf16z(name)).data(0x4401, "MobID", id.to_vec()).data(0x4405, "CreationTime", timestamp(now)).data(
        0x4404,
        "LastModified",
        timestamp(now),
    )
}

fn slot(id: u32, name: &str, rate: (i32, i32), segment: Obj) -> Obj {
    Obj::new(class::TIMELINE_SLOT)
        .data(0x4801, "SlotID", u32v(id))
        .data(0x4802, "SlotName", utf16z(name))
        .data(0x4b02, "Origin", i64v(0))
        .data(0x4b01, "EditRate", rational(rate.0, rate.1))
        .strong(0x4803, "Segment", segment)
}

fn source_clip(len: i64, start: i64, source: &[u8], source_slot: u32) -> Obj {
    Obj::new(class::SOURCE_CLIP)
        .weak(0x0201, "DataDefinition", Defs::Data, key::SOUND)
        .data(0x0202, "Length", i64v(len))
        .data(0x1201, "StartTime", i64v(start))
        .data(0x1101, "SourceID", source.to_vec())
        .data(0x1102, "SourceMobSlotID", u32v(source_slot))
}

fn sequence(def: &str, len: i64, items: Vec<Obj>) -> Obj {
    Obj::new(class::SEQUENCE).weak(0x0201, "DataDefinition", Defs::Data, def).data(0x0202, "Length", i64v(len)).vector(
        0x1001,
        "Components",
        items,
    )
}

fn filler(len: i64) -> Obj {
    Obj::new(class::FILLER).weak(0x0201, "DataDefinition", Defs::Data, key::SOUND).data(0x0202, "Length", i64v(len))
}

type MobId = Vec<u8>;
/// A mob and its MobID (the key of the content's mob set).
type KeyedMob = (MobId, Obj);

/// Master, file and import mobs for one linked WAV (Avid AMA layout).
/// Returns the master mob id (slots 1..=channels) and the three mobs.
fn link_media(m: &AafMedia, now: i64) -> Result<(MobId, Vec<KeyedMob>)> {
    let name = m.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let rate = (m.sample_rate as i32, 1);
    let chans = 1..=m.channels.max(1) as u32;
    let (tape_id, file_id, master_id) = (new_mob_id(), new_mob_id(), new_mob_id());
    let tape = mob_head(Obj::new(class::SOURCE_MOB), &format!("{name} <TAPE MOB>"), &tape_id, now)
        .vector(
            0x4403,
            "Slots",
            chans.clone().map(|c| slot(c, "", rate, sequence(key::SOUND, m.frames, vec![source_clip(m.frames, 0, &[0; 32], 0)]))).collect(),
        )
        .strong(0x4701, "EssenceDescription", Obj::new(class::IMPORT_DESCRIPTOR));
    let url = url_for(&m.path);
    let desc = Obj::new(class::WAVE_DESCRIPTOR)
        .data(0x3001, "SampleRate", rational(rate.0, 1))
        .data(0x3801, "Summary", wave_summary(&m.path)?)
        .data(0x3002, "Length", i64v(m.frames))
        .weak(0x3004, "ContainerFormat", Defs::Container, key::CONTAINER_AAF)
        .vector(0x2f01, "Locator", vec![Obj::new(class::NETWORK_LOCATOR).data(0x4001, "URLString", utf16z(&url))]);
    let linked = |id: &[u8], refs: &[u8], label: &str| {
        mob_head(Obj::new(if label.is_empty() { class::MASTER_MOB } else { class::SOURCE_MOB }), &format!("{name}{label}"), id, now).vector(
            0x4403,
            "Slots",
            chans
                .clone()
                .map(|c| {
                    let s = slot(c, "", rate, sequence(key::SOUND, m.frames, vec![source_clip(m.frames, 0, refs, c)]));
                    let mut s = s;
                    s.props.push((0x4804, "PhysicalTrackNumber", Val::Data(u32v(c))));
                    s
                })
                .collect(),
        )
    };
    let file = linked(&file_id, &tape_id, " <SOURCE MOB>").strong(0x4701, "EssenceDescription", desc);
    let master = linked(&master_id, &file_id, "");
    Ok((master_id.clone(), vec![(tape_id, tape), (file_id, file), (master_id, master)]))
}

/// `file://` URL of an absolute path (percent-encoding what URLs can't hold).
pub fn url_for(path: &Path) -> String {
    let raw = path.to_string_lossy();
    // Windows `canonicalize` gives verbatim paths (\\?\C:\…); URLs want C:/…
    let raw =
        raw.strip_prefix(r"\\?\UNC\").map(|r| format!(r"\\{r}")).unwrap_or_else(|| raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string());
    let s = raw.replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/'); // C:/… on Windows
    }
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A piece of a clip on one AAF track: source range, fades, gain.
#[derive(Debug, Clone)]
struct Piece {
    at: i64,
    len: i64,
    source: i64,
    fade_in: i64,
    fade_out: i64,
    gain: f32,
    asset: String,
}

fn piece_obj(p: &Piece, master: &[u8], channel: u32) -> Obj {
    let mut clip = source_clip(p.len, p.source, master, channel);
    if p.fade_in > 0 {
        clip = clip.data(0x1202, "FadeInLength", i64v(p.fade_in)).data(0x1203, "FadeInType", u8v(1));
    }
    if p.fade_out > 0 {
        clip = clip.data(0x1204, "FadeOutLength", i64v(p.fade_out)).data(0x1205, "FadeOutType", u8v(1));
    }
    if (p.gain - 1.0).abs() < 1e-6 {
        return clip;
    }
    // clip gain: Mono Audio Gain with a constant Amplitude
    let cv = Obj::new(class::CONSTANT_VALUE).data(0x4c01, "Definition", hex(key::PARAM_AMPLITUDE)).data(
        0x4d01,
        "Value",
        amplitude(p.gain as f64),
    );
    gain_group(p.len, clip, cv)
}

/// An indirect Rational amplitude value (linear gain).
fn amplitude(g: f64) -> Vec<u8> {
    let den = 1_000_000i32;
    let mut v = vec![0x4c];
    v.extend(hex(key::TYPE_RATIONAL));
    v.extend(rational((g * den as f64).round().clamp(0.0, i32::MAX as f64) as i32, den));
    v
}

/// `input` inside a Mono Audio Gain operation with Amplitude `param`
/// (a ConstantValue or a VaryingValue).
fn gain_group(len: i64, input: Obj, param: Obj) -> Obj {
    effect(key::OP_MONO_AUDIO_GAIN, key::PARAM_AMPLITUDE, len, input, param)
}

/// `input` inside a one-parameter audio effect (Mono Audio Gain / Pan).
fn effect(op: &str, param_def: &str, len: i64, input: Obj, param: Obj) -> Obj {
    let mut g = Obj::new(class::OPERATION_GROUP)
        .weak(0x0201, "DataDefinition", Defs::Data, key::SOUND)
        .data(0x0202, "Length", i64v(len))
        .weak(0x0b01, "Operation", Defs::Operation, op)
        .vector(0x0b02, "InputSegments", vec![input]);
    g.props.push((0x0b03, "Parameters", Val::Set { key_pid: 0x4c01, items: vec![(hex(param_def), param)] }));
    g
}

/// A parameter's value over a track: one constant, or control points `(timeline sample, value)`.
#[derive(Debug, Clone, PartialEq)]
enum Curve {
    Const(f64),
    Points(Vec<(i64, f64)>),
}

impl Curve {
    /// Points that all hold one value become a constant.
    fn from_points(p: Vec<(i64, f64)>) -> Curve {
        match p.first() {
            Some(&(_, v)) if p.iter().all(|x| (x.1 - v).abs() < 1e-9) => Curve::Const(v),
            _ => Curve::Points(p),
        }
    }
    fn param(&self, def: &str, len: i64, start: i64) -> Obj {
        match self {
            Curve::Const(v) => Obj::new(class::CONSTANT_VALUE).data(0x4c01, "Definition", hex(def)).data(0x4d01, "Value", amplitude(*v)),
            Curve::Points(p) => {
                let cps = p
                    .iter()
                    .map(|&(t, v)| {
                        let rel = t - start;
                        let time = if len <= i32::MAX as i64 {
                            rational(rel as i32, len as i32)
                        } else {
                            rational((rel as f64 / len as f64 * 1e9) as i32, 1_000_000_000)
                        };
                        Obj::new(class::CONTROL_POINT).data(0x1a03, "Time", time).data(0x1a02, "Value", amplitude(v))
                    })
                    .collect();
                Obj::new(class::VARYING_VALUE)
                    .data(0x4c01, "Definition", hex(def))
                    .weak(0x4e01, "Interpolation", Defs::Interpolation, key::INTERP_LINEAR)
                    .vector(0x4e02, "PointList", cps)
            }
        }
    }
}

/// Lane value in dB at `s` (linear between breakpoints, held at the ends), as the mixer plays it.
fn lane_db(pts: &[(i64, f32)], s: i64) -> f64 {
    let i = pts.partition_point(|p| p.0 <= s);
    let v = match (i.checked_sub(1).map(|k| pts[k]), pts.get(i)) {
        (Some(a), Some(b)) => a.1 + (b.1 - a.1) * ((s - a.0) as f32 / (b.0 - a.0).max(1) as f32),
        (Some(a), None) => a.1,
        (None, Some(b)) => b.1,
        (None, None) => 0.0,
    };
    v as f64
}

/// The mixer's gain for a lane value: −96 dB and below is silence.
fn db_amp(db: f64) -> f64 {
    if db <= -96.0 {
        0.0
    } else {
        10f64.powf(db / 20.0)
    }
}

/// Control points `(offset, amplitude)` over `[start, end)` for a dB volume lane.
/// AAF interpolates linearly in amplitude while the mixer ramps in dB, so
/// segments are split until the two agree within 0.1 dB (or 0.001 absolute
/// near silence).
#[cfg(test)]
fn volume_points(pts: &[(i64, f32)], start: i64, end: i64) -> Vec<(i64, f64)> {
    let keys: Vec<i64> = pts.iter().map(|p| p.0).collect();
    gain_points(&keys, start, end, &|t| db_amp(lane_db(pts, t)))
}

/// Control points for any gain curve `amp(t)` with corners at `keys`: split until
/// amplitude-linear interpolation stays within 0.1 dB of it (0.001 absolute near silence).
fn gain_points(keys: &[i64], start: i64, end: i64, amp: &dyn Fn(i64) -> f64) -> Vec<(i64, f64)> {
    fit(keys, start, end, amp, &|lin, v| {
        if v > 0.001 && lin > 0.001 {
            (20.0 * (lin / v).log10()).abs() > 0.1
        } else {
            (lin - v).abs() > 0.001
        }
    })
}

/// Control points for `f` with corners at `keys`, split until linear interpolation
/// between neighbours is no longer `bad(interpolated, actual)` at the midpoints.
pub(crate) fn fit(keys: &[i64], start: i64, end: i64, f: &dyn Fn(i64) -> f64, bad: &dyn Fn(f64, f64) -> bool) -> Vec<(i64, f64)> {
    let last = end - 1;
    let mut ks = vec![start];
    ks.extend(keys.iter().copied().filter(|&t| t > start && t < last));
    ks.push(last);
    ks.sort();
    ks.dedup();
    fn split(a: i64, b: i64, f: &dyn Fn(i64) -> f64, bad: &dyn Fn(f64, f64) -> bool, out: &mut Vec<(i64, f64)>, depth: u32) {
        if b - a > 1 && depth < 12 {
            let m = a + (b - a) / 2;
            let (va, vb) = (f(a), f(b));
            let lin = va + (vb - va) * (m - a) as f64 / (b - a) as f64;
            if bad(lin, f(m)) {
                split(a, m, f, bad, out, depth + 1);
                split(m, b, f, bad, out, depth + 1);
                return;
            }
        }
        out.push((b, f(b)));
    }
    let mut out = vec![(start, f(start))];
    for w in ks.windows(2) {
        split(w[0], w[1], f, bad, &mut out, 0);
    }
    out
}

/// Gain and pan of one AAF track (one channel of one layer of a project track).
#[derive(Debug, Clone, Default, PartialEq)]
struct Mix {
    gain: Option<Curve>,
    pan: Option<Curve>,
}

/// The mixer's stereo balance (centre = unity on both sides): (left, right).
fn balance(p: f64) -> (f64, f64) {
    let p = p.clamp(-1.0, 1.0);
    if p <= 0.0 {
        (1.0, 1.0 + p)
    } else {
        (1.0 - p, 1.0)
    }
}

/// Where a pan/balance lane crosses the centre (the balance law has a corner there).
fn centre_crossings(lane: &[(i64, f32)]) -> Vec<i64> {
    lane.windows(2)
        .filter(|w| (w[0].1 < 0.0) != (w[1].1 < 0.0) && (w[1].1 - w[0].1).abs() > 1e-9)
        .map(|w| w[0].0 + ((0.0 - w[0].1) / (w[1].1 - w[0].1) * (w[1].0 - w[0].0) as f32) as i64)
        .collect()
}

fn owner_lane(s: &ProjectState, owner: &str, param: AutomationParam) -> Vec<(i64, f32)> {
    let mut v: Vec<(i64, f32)> =
        s.automation.values().filter(|p| p.owner_id == owner && p.param == param).map(|p| (p.sample, p.value)).collect();
    v.sort_by_key(|p| p.0);
    v
}

/// Everything a track passes through after its own fader: its bus, that bus's
/// parent, … the master — static gains, volume lanes and balance lanes.
struct BusChain {
    gain: f64,
    vols: Vec<Vec<(i64, f32)>>,
    bals: Vec<Vec<(i64, f32)>>,
    muted: bool,
}

impl BusChain {
    fn of(s: &ProjectState, bus: &str) -> BusChain {
        let mut c = BusChain { gain: 1.0, vols: vec![], bals: vec![], muted: false };
        let mut cur = s.buses.get(bus);
        let mut guard = 0;
        while let Some(b) = cur {
            c.gain *= 10f64.powf(b.gain_db as f64 / 20.0);
            c.muted |= b.mute;
            let (v, p) = (owner_lane(s, &b.id, AutomationParam::VolumeDb), owner_lane(s, &b.id, AutomationParam::Pan));
            if !v.is_empty() {
                c.vols.push(v);
            }
            if !p.is_empty() {
                c.bals.push(p);
            }
            cur = b.parent_id.as_ref().and_then(|p| s.buses.get(p));
            guard += 1;
            if guard > 64 {
                break; // a routing loop can't be rendered either
            }
        }
        c
    }
    /// Level at `x` (static gains × volume lanes; −96 dB is silence).
    fn amp(&self, x: i64) -> f64 {
        self.vols.iter().fold(self.gain, |g, l| g * db_amp(lane_db(l, x)))
    }
    /// Balance at `x` as (left, right) factors.
    fn bal(&self, x: i64) -> (f64, f64) {
        self.bals.iter().fold((1.0, 1.0), |(l, r), lane| {
            let (bl, br) = balance(lane_db(lane, x));
            (l * bl, r * br)
        })
    }
    fn keys(&self) -> Vec<i64> {
        let mut k: Vec<i64> = self.vols.iter().chain(&self.bals).flatten().map(|p| p.0).collect();
        for l in &self.bals {
            k.extend(centre_crossings(l));
        }
        k
    }
    fn is_flat(&self) -> bool {
        self.vols.is_empty() && self.bals.is_empty() && (self.gain - 1.0).abs() < 1e-9
    }
}

/// Gain and pan for each channel of a project track over `[start, end)`,
/// including what its buses do to it (AAF tracks are flat: no buses).
///
/// Mono tracks: track volume × bus levels → gain; the track's pan → Mono Audio
/// Pan (exact: both sides interpolate pan linearly). A bus balance moves the
/// panned signal's left/right levels; with a −3 dB constant-power pan law that
/// is exactly a new pan plus a gain, which is what gets written.
/// Stereo tracks: L/R hard panned; volume, balance and bus levels/balance
/// become each side's gain. Curves are fitted within 0.1 dB (pan within 0.002).
/// `fader`: also apply the track's static fader (printed tracks, whose clips
/// can't carry it as clip gain).
fn track_mix(s: &ProjectState, t: &Track, start: i64, end: i64, fader: bool) -> Vec<Mix> {
    let (vol, pan) = (owner_lane(s, &t.id, AutomationParam::VolumeDb), owner_lane(s, &t.id, AutomationParam::Pan));
    let mut bus = BusChain::of(s, &t.bus_id);
    if fader {
        bus.gain *= 10f64.powf(t.gain_db as f64 / 20.0);
    }
    let pan_at = |x: i64| if pan.is_empty() { t.pan as f64 } else { lane_db(&pan, x) };
    let vol_at = |x: i64| if vol.is_empty() { 1.0 } else { db_amp(lane_db(&vol, x)) };
    let mut keys: Vec<i64> = vol.iter().chain(&pan).map(|p| p.0).collect();
    keys.extend(bus.keys());
    let as_gain = |c: Curve| match c {
        Curve::Const(v) if (v - 1.0).abs() < 1e-9 => None,
        c => Some(c),
    };
    if t.channels < 2 {
        if bus.bals.is_empty() {
            // no bus balance: the track's pan maps exactly; levels multiply
            let gain = if vol.is_empty() && bus.vols.is_empty() {
                as_gain(Curve::Const(bus.gain))
            } else {
                as_gain(Curve::from_points(gain_points(&keys, start, end, &|x| vol_at(x) * bus.amp(x))))
            };
            let pan_curve = if pan.is_empty() {
                (t.pan.abs() > 0.001).then(|| Curve::Const((t.pan as f64 + 1.0) / 2.0))
            } else {
                let last = end - 1;
                let mut ks = vec![start];
                ks.extend(pan.iter().map(|p| p.0).filter(|&x| x > start && x < last));
                ks.push(last);
                ks.dedup();
                Some(Curve::from_points(ks.into_iter().map(|x| (x, (pan_at(x).clamp(-1.0, 1.0) + 1.0) / 2.0)).collect()))
            };
            return vec![Mix { gain, pan: pan_curve }];
        }
        // the panned signal's left/right after the buses' balance
        let lr = |x: i64| {
            let (gl, gr) = crate::mixer::pan_gains(pan_at(x) as f32);
            let (bl, br) = bus.bal(x);
            (gl as f64 * bl, gr as f64 * br)
        };
        let gain = as_gain(Curve::from_points(gain_points(&keys, start, end, &|x| {
            let (l, r) = lr(x);
            vol_at(x) * bus.amp(x) * (l * l + r * r).sqrt()
        })));
        let pan_param = |x: i64| {
            let (l, r) = lr(x);
            if l <= 0.0 && r <= 0.0 {
                0.5
            } else {
                r.atan2(l) / std::f64::consts::FRAC_PI_2
            }
        };
        let pan_curve = Curve::from_points(fit(&keys, start, end, &pan_param, &|lin, v| (lin - v).abs() > 0.002));
        return vec![Mix { gain, pan: Some(pan_curve).filter(|c| *c != Curve::Const(0.5)) }];
    }
    let side = |left: bool| {
        let f = |x: i64| {
            let (tl, tr) = balance(pan_at(x));
            let (bl, br) = bus.bal(x);
            vol_at(x) * bus.amp(x) * if left { tl * bl } else { tr * br }
        };
        let mut ks = keys.clone();
        ks.extend(centre_crossings(&pan));
        let gain = if vol.is_empty() && pan.is_empty() && t.pan.abs() < 0.001 && bus.is_flat() {
            None
        } else {
            as_gain(Curve::from_points(gain_points(&ks, start, end, &f)))
        };
        Mix { gain, pan: Some(Curve::Const(if left { 0.0 } else { 1.0 })) }
    };
    vec![side(true), side(false)]
}

/// What to export and how to name it.
#[derive(Debug, Clone)]
pub struct AafOptions {
    pub name: String,
    /// Timeline range (project samples); AAF time 0 is `start`.
    pub start: i64,
    pub end: i64,
    pub exclude_temp_refs: bool,
    /// Tracks rendered with their plugins and insert effects (pre-fader), by
    /// track id: each goes into the AAF as one clip of this file.
    pub prints: BTreeMap<String, AafMedia>,
}

#[derive(Default)]
struct Losses(BTreeMap<&'static str, usize>);

impl Losses {
    fn add(&mut self, what: &'static str, n: usize) {
        if n > 0 {
            *self.0.entry(what).or_default() += n;
        }
    }
    fn list(&self) -> Vec<String> {
        self.0.iter().map(|(k, n)| format!("{k} ({n})")).collect()
    }
}

/// Pieces of one clip on the timeline (loops expanded into butt-spliced repeats).
fn clip_pieces(s: &ProjectState, c: &ClipInstance, track_gain_db: f32, losses: &mut Losses) -> Vec<Piece> {
    let gain = 10f32.powf((c.gain_db + track_gain_db) / 20.0);
    let frames = s.assets.get(&c.asset_id).map(|a| a.length_samples).unwrap_or(c.length);
    let one = |at: i64, len: i64, source: i64, fade_in: i64, fade_out: i64| Piece {
        at,
        len,
        source,
        fade_in,
        fade_out,
        gain,
        asset: c.asset_id.clone(),
    };
    if c.fill == FillMode::OneShot {
        return vec![one(c.timeline_start, c.length, c.source_offset, c.fade_in, c.fade_out)];
    }
    // the mixer loops with a short crossfade; AAF gets plain repeats of the same span
    let loop_len = (frames - c.source_offset).max(1);
    let xf = (loop_len / 4).min(s.project.sample_rate as i64 / 20);
    let period = (loop_len - xf).max(1);
    if c.length > period {
        losses.add("looped clips: loop crossfades not carried, repeats are butt-spliced", 1);
    }
    let mut v = vec![];
    let mut p = 0;
    while p < c.length {
        let len = period.min(c.length - p);
        v.push(one(c.timeline_start + p, len, c.source_offset, if p == 0 { c.fade_in.min(len) } else { 0 }, 0));
        p += len;
    }
    if let Some(last) = v.last_mut() {
        last.fade_out = c.fade_out.min(last.len);
    }
    v
}

/// Trim a piece to `[start, end)`; None when it lies outside.
fn trim(mut p: Piece, start: i64, end: i64) -> Option<Piece> {
    if p.at + p.len <= start || p.at >= end {
        return None;
    }
    if p.at < start {
        let d = start - p.at;
        p.at = start;
        p.len -= d;
        p.source += d;
        p.fade_in = (p.fade_in - d).max(0);
    }
    if p.at + p.len > end {
        let d = p.at + p.len - end;
        p.len -= d;
        p.fade_out = (p.fade_out - d).max(0);
    }
    p.fade_in = p.fade_in.min(p.len);
    p.fade_out = p.fade_out.min(p.len - p.fade_in);
    Some(p)
}

/// Build the AAF and write it to `out`. `media` maps asset ids to the linked WAVs.
pub fn write(s: &ProjectState, out: &Path, media: &BTreeMap<String, AafMedia>, o: &AafOptions) -> Result<AafReport> {
    let now = crate::ids::now_unix();
    // printed tracks link to their render like any other media
    let print_key = |tid: &str| format!("print:{tid}");
    let mut media = media.clone();
    for (tid, m) in &o.prints {
        media.insert(print_key(tid), m.clone());
    }
    let media = &media;
    let printed = |track: &str| o.prints.contains_key(track);
    // the track a plugin lane's slot sits on
    let slot_track = |slot: &str| crate::editing::plugin_slot(s, slot).map(|(_, owner)| owner.to_string());
    let sr = s.project.sample_rate as i32;
    let (start, end) = (o.start, o.end.max(o.start + 1));
    let len = end - start;
    let mut losses = Losses::default();
    let mut mobs: Vec<KeyedMob> = vec![];
    let mut masters: BTreeMap<String, MobId> = BTreeMap::new();

    // ---- what the timeline can't hold
    let n_auto = |f: &dyn Fn(&AutomationPoint) -> bool| s.automation.values().filter(|p| f(p)).count();
    losses.add(
        "plugin parameter automation not carried (on unprinted tracks and buses)",
        n_auto(&|p| matches!(p.param, AutomationParam::Plugin(_)) && !slot_track(&p.owner_id).is_some_and(|t| printed(&t))),
    );
    losses.add("tracks printed with their plugins and insert effects: one rendered clip each, not editable clip by clip", o.prints.len());
    let pans = |t: &Track| t.pan.abs() > 0.001 || s.automation.values().any(|p| p.owner_id == t.id && p.param == AutomationParam::Pan);
    losses.add(
        "mono track pan uses the receiving app's pan law (VC Sound Design: −3 dB constant power)",
        s.tracks.values().filter(|t| !t.mute && t.channels < 2 && pans(t)).count(),
    );
    let inserts =
        s.tracks.values().filter(|t| !printed(&t.id)).map(|t| t.inserts.len()).chain(s.buses.values().map(|b| b.inserts.len())).sum();
    losses.add("insert effects and plugins not carried (rendered in stems and mix)", inserts);
    losses.add(
        "bus routing not carried: tracks are flat, with bus levels and automation folded into each track",
        s.buses.values().filter(|b| b.parent_id.is_some()).count(),
    );
    losses.add(
        "bus balance on mono tracks folded into their pan and gain (exact with a −3 dB constant-power pan law)",
        s.tracks.values().filter(|t| !t.mute && t.channels < 2 && !BusChain::of(s, &t.bus_id).bals.is_empty()).count(),
    );
    let static_gain = s.tracks.values().filter(|t| t.gain_db.abs() > 0.001 && !printed(&t.id)).count();
    losses.add("track fader gain folded into clip gain", static_gain);

    // ---- tracks → slots of pieces
    let mut tracks: Vec<&Track> = s.tracks.values().collect();
    tracks.sort_by(|a, b| (&a.folder, a.order, &a.name).cmp(&(&b.folder, b.order, &b.name)));
    // (name, source channel, pieces, owning track)
    let mut audio_slots: Vec<(String, u32, Vec<Piece>, String)> = vec![];
    let mut clip_count = 0;
    for t in tracks {
        if BusChain::of(s, &t.bus_id).muted {
            losses.add("tracks on muted buses left out", 1);
            continue;
        }
        if t.mute {
            losses.add("muted tracks left out", 1);
            continue;
        }
        if printed(&t.id) {
            // one clip of the track's render, across the range, on one layer
            let p = Piece { at: start, len, source: 0, fade_in: 0, fade_out: 0, gain: 1.0, asset: print_key(&t.id) };
            clip_count += 1;
            let chans = t.channels.clamp(1, 2) as u32;
            for ch in 1..=chans {
                let mut name = t.name.clone();
                if chans == 2 {
                    name.push_str(if ch == 1 { " L" } else { " R" });
                }
                audio_slots.push((name, ch, vec![p.clone()], t.id.clone()));
            }
            continue;
        }
        let mut pieces = vec![];
        let mut clips: Vec<&ClipInstance> = s.clips.values().filter(|c| c.track_id == t.id).collect();
        clips.sort_by(|a, b| (a.timeline_start, &a.id).cmp(&(b.timeline_start, &b.id)));
        for c in clips {
            if c.muted {
                losses.add("muted clips left out", 1);
                continue;
            }
            let Some(a) = s.assets.get(&c.asset_id) else { continue };
            if o.exclude_temp_refs && a.license.as_ref().is_some_and(|l| l.temp_reference) {
                losses.add("temp-reference clips left out", 1);
                continue;
            }
            if !media.contains_key(&c.asset_id) {
                losses.add("clips whose media could not be linked left out", 1);
                continue;
            }
            let before = pieces.len();
            pieces.extend(clip_pieces(s, c, t.gain_db, &mut losses).into_iter().filter_map(|p| trim(p, start, end)));
            if pieces.len() > before {
                clip_count += 1;
            }
            if t.channels == 1 && a.channels > 1 {
                losses.add("stereo sources on mono tracks: left channel only", 1);
            }
        }
        if pieces.is_empty() {
            continue;
        }
        // overlapping clips → extra layers (AAF sequences can't overlap)
        let mut lanes: Vec<Vec<Piece>> = vec![];
        for p in pieces {
            match lanes.iter_mut().find(|l| l.last().is_none_or(|q| q.at + q.len <= p.at)) {
                Some(l) => l.push(p),
                None => lanes.push(vec![p]),
            }
        }
        if lanes.len() > 1 {
            losses.add("overlapping clips split onto extra tracks", lanes.len() - 1);
        }
        let chans = t.channels.clamp(1, 2) as u32;
        for (li, lane) in lanes.into_iter().enumerate() {
            for ch in 1..=chans {
                let mut name = t.name.clone();
                if chans == 2 {
                    name.push_str(if ch == 1 { " L" } else { " R" });
                }
                if li > 0 {
                    name.push_str(&format!(" ({})", li + 1));
                }
                audio_slots.push((name, ch, lane.clone(), t.id.clone()));
            }
        }
    }

    // ---- media mobs for the assets in use
    for (_, _, pieces, _) in &audio_slots {
        for p in pieces {
            if !masters.contains_key(&p.asset) {
                let (id, ms) = link_media(&media[&p.asset], now)?;
                masters.insert(p.asset.clone(), id);
                mobs.extend(ms);
            }
        }
    }

    // ---- composition
    let tb = s.tb();
    let fr = s.project.frame_rate;
    let frames = fr.frame_at_sample(len, s.project.sample_rate) + 1;
    let tc = Obj::new(class::TIMECODE)
        .weak(0x0201, "DataDefinition", Defs::Data, key::TIMECODE)
        .data(0x0202, "Length", i64v(frames))
        .data(0x1501, "Start", i64v(tb.frame_at(start)))
        .data(0x1502, "FPS", u16v(fr.nominal() as u16))
        .data(0x1503, "Drop", u8v(fr.drop_frame as u8));
    let mut slots = vec![slot(1, "TC1", (fr.num as i32, fr.den as i32), tc)];
    // gain and pan of every channel of every track, from its static pan and lanes
    let mut mixes: BTreeMap<String, Vec<Mix>> = BTreeMap::new();
    for (_, _, _, tid) in &audio_slots {
        if !mixes.contains_key(tid) {
            mixes.insert(tid.clone(), track_mix(s, &s.tracks[tid], start, end, printed(tid)));
        }
    }
    for (i, (name, ch, pieces, tid)) in audio_slots.iter().enumerate() {
        let mut items = vec![];
        let mut at = start;
        for p in pieces {
            if p.at > at {
                items.push(filler(p.at - at));
            }
            let asset_ch = media[&p.asset].channels as u32;
            items.push(piece_obj(p, &masters[&p.asset], (*ch).min(asset_ch.max(1))));
            at = p.at + p.len;
        }
        if end > at {
            items.push(filler(end - at));
        }
        let mut seq = sequence(key::SOUND, len, items);
        let mix = &mixes[tid][(*ch as usize - 1).min(mixes[tid].len() - 1)];
        if let Some(g) = &mix.gain {
            seq = effect(key::OP_MONO_AUDIO_GAIN, key::PARAM_AMPLITUDE, len, seq, g.param(key::PARAM_AMPLITUDE, len, start));
        }
        if let Some(p) = &mix.pan {
            seq = effect(key::OP_MONO_AUDIO_PAN, key::PARAM_PAN, len, seq, p.param(key::PARAM_PAN, len, start));
        }
        let mut sl = slot(i as u32 + 2, name, (sr, 1), seq);
        sl.props.push((0x4804, "PhysicalTrackNumber", Val::Data(u32v(i as u32 + 1))));
        slots.push(sl);
    }
    // cue markers
    let mut cues: Vec<&Cue> = s.cues.values().filter(|c| c.start_sample >= start && c.start_sample < end).collect();
    cues.sort_by(|a, b| (a.start_sample, &a.code).cmp(&(b.start_sample, &b.code)));
    let markers = cues.len();
    if !cues.is_empty() {
        let items = cues
            .iter()
            .map(|c| {
                let text = if c.description.is_empty() { c.code.clone() } else { format!("{} {}", c.code, c.description) };
                Obj::new(class::COMMENT_MARKER)
                    .weak(0x0201, "DataDefinition", Defs::Data, key::DESCRIPTIVE)
                    .data(0x0202, "Length", i64v(0))
                    .data(0x0601, "Position", i64v(c.start_sample - start))
                    .data(0x0602, "Comment", utf16z(&text))
            })
            .collect();
        let ev = Obj::new(class::EVENT_SLOT)
            .data(0x4802, "SlotName", utf16z("Cues"))
            .data(0x4801, "SlotID", u32v(audio_slots.len() as u32 + 2))
            .data(0x4901, "EditRate", rational(sr, 1))
            .strong(0x4803, "Segment", sequence(key::DESCRIPTIVE, 0, items));
        slots.push(ev);
    }
    let comp_id = new_mob_id();
    let comp = mob_head(Obj::new(class::COMPOSITION_MOB), &o.name, &comp_id, now).vector(0x4403, "Slots", slots);
    mobs.push((comp_id, comp));

    let report = AafReport {
        file: out.display().to_string(),
        composition: o.name.clone(),
        tracks: audio_slots.len(),
        clips: clip_count,
        markers,
        media_files: masters.len(),
        printed_tracks: o.prints.len(),
        automated_tracks: s
            .automation
            .values()
            .filter(|p| mixes.contains_key(&p.owner_id) && p.param == AutomationParam::VolumeDb)
            .map(|p| &p.owner_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        panned_tracks: mixes.keys().filter(|k| pans(&s.tracks[k.as_str()])).count(),
        losses: losses.list(),
    };
    let content = Obj { class: class::CONTENT_STORAGE, props: vec![(0x1901, "Mobs", Val::Set { key_pid: 0x4401, items: mobs })] };
    let bytes = write_file(&content, now)?;
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(out, bytes)?;
    Ok(report)
}

/// Copy (or, at another sample rate, convert) the media of `asset_ids` into
/// `dir` for the AAF to link to. Returns the linked files by asset id and notes.
pub fn prepare_media(s: &ProjectState, asset_ids: &[String], dir: &Path) -> Result<(BTreeMap<String, AafMedia>, Vec<String>)> {
    std::fs::create_dir_all(dir)?;
    let mut out = BTreeMap::new();
    let mut notes = vec![];
    let mut used_names = std::collections::HashSet::new();
    for id in asset_ids {
        let Some(a) = s.assets.get(id) else { continue };
        let src = Path::new(&a.path);
        if !src.exists() {
            notes.push(format!("media missing, not linked: {}", a.path));
            continue;
        }
        let mut stem = safe_name(&a.name);
        let mut k = 2;
        while !used_names.insert(stem.clone()) {
            stem = format!("{}_{k}", safe_name(&a.name));
            k += 1;
        }
        let path = dir.join(format!("{stem}.wav"));
        let (info, buf) = crate::wav::read(src)?;
        if info.sample_rate != s.project.sample_rate {
            let conv = crate::mixer::resample_linear(&buf, s.project.sample_rate);
            crate::wav::write(&path, &conv, crate::wav::SampleFormat::Int24, None)?;
            notes.push(format!("{} converted to {} Hz for the AAF (linear)", a.name, s.project.sample_rate));
            out.insert(
                id.clone(),
                AafMedia { path: abs(&path), channels: conv.channels, sample_rate: s.project.sample_rate, frames: conv.frames() as i64 },
            );
        } else {
            std::fs::copy(src, &path)?;
            out.insert(
                id.clone(),
                AafMedia { path: abs(&path), channels: buf.channels, sample_rate: buf.sample_rate, frames: buf.frames() as i64 },
            );
        }
    }
    Ok((out, notes))
}

fn abs(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Tracks worth printing over `[start, end)`: audible (not muted, not on a
/// muted bus), with an active insert (a plugin that isn't bypassed, or a
/// built-in effect) and at least one audible clip in the range.
pub fn tracks_to_print(s: &ProjectState, start: i64, end: i64) -> Vec<String> {
    let mut v: Vec<&Track> = s
        .tracks
        .values()
        .filter(|t| !t.mute && !BusChain::of(s, &t.bus_id).muted)
        .filter(|t| t.inserts.iter().any(|i| i.as_plugin().is_none_or(|p| !p.bypass)))
        .filter(|t| s.clips.values().any(|c| c.track_id == t.id && !c.muted && c.timeline_start < end && c.end() > start))
        .collect();
    v.sort_by(|a, b| (&a.folder, a.order, &a.name).cmp(&(&b.folder, b.order, &b.name)));
    v.into_iter().map(|t| t.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_ramps_become_amplitude_points_within_a_tenth_of_a_db() {
        // 0 dB → −40 dB over one second, then held; −96 is silence
        let lane = [(48_000i64, 0.0f32), (96_000, -40.0), (150_000, -96.0)];
        let pts = volume_points(&lane, 0, 200_000);
        assert_eq!(pts.first().unwrap(), &(0, 1.0), "held before the first breakpoint");
        assert_eq!(pts.last().unwrap().1, 0.0, "silence at the end");
        assert!(pts.iter().any(|p| p.0 == 48_000) && pts.iter().any(|p| p.0 == 96_000), "breakpoints kept");
        assert!(pts.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(pts.len() > 8 && pts.len() < 200, "ramp densified, not oversampled: {}", pts.len());
        // amplitude-linear interpolation between the points matches the dB-linear lane
        for t in (0..150_000).step_by(997) {
            let i = pts.partition_point(|p| p.0 <= t) - 1;
            let (a, b) = (pts[i], pts[(i + 1).min(pts.len() - 1)]);
            let lin = if b.0 > a.0 { a.1 + (b.1 - a.1) * (t - a.0) as f64 / (b.0 - a.0) as f64 } else { a.1 };
            let want = db_amp(lane_db(&lane, t));
            if want > 0.01 {
                assert!((20.0 * (lin / want).log10()).abs() < 0.11, "t={t}: {lin} vs {want}");
            } else {
                assert!((lin - want).abs() < 0.0011, "t={t}");
            }
        }
        // a lane with one point: a flat level
        assert!(volume_points(&[(10, -6.0)], 0, 100).iter().all(|p| p.1 == db_amp(-6.0)));
    }

    #[test]
    fn names_and_encodings_match_pyaaf2() {
        assert_eq!(mangle("Segment", 0x4803, 32), "Segment-4803");
        assert_eq!(mangle("EssenceDescription", 0x4701, 32), "EssenceDescription-4701");
        assert_eq!(mangle("ContainerDefinitions", 0x2608, 22), "Containe-nitions-2608");
        assert_eq!(mangle("IdentificationList", 0x3b06, 22), "Identifi-ionList-3b06");
        assert_eq!(mangle("InputSegments", 0x0b02, 22), "InputSegments-b02");
        // 2026-11-21 00:59:21 UTC, as pyaaf2 wrote it in the template
        assert_eq!(timestamp(1_795_222_761), hex("ea070b15003b1500"));
        assert_eq!(utf16z("ab"), vec![b'a', 0, b'b', 0, 0, 0]);
        assert_eq!(new_mob_id().len(), 32);
        assert_eq!(url_for(Path::new("/a b/c.wav")), "file:///a%20b/c.wav");
        assert_eq!(url_for(Path::new(r"\\?\C:\Films\x.wav")), "file:///C:/Films/x.wav");
    }

    #[test]
    fn template_has_every_definition_we_reference() {
        let mut cf = cfb::CompoundFile::open(Cursor::new(TEMPLATE)).unwrap();
        let t = weak_table(&mut cf).unwrap();
        for d in [Defs::Data, Defs::Operation, Defs::Container, Defs::Interpolation] {
            assert!(t.iter().any(|p| p.as_slice() == [2, 0x3b04, d as u16]));
        }
        // class ids read back as pyaaf2 prints them
        let h = cf.walk().find(|e| e.path() == Path::new("/Header-2")).unwrap();
        assert_eq!(h.clsid().to_string(), "0d010101-0101-2f00-060e-2b3402060101");
    }
}

//! AAF import (spec §15.2): bring an editor's timeline into the project.
//!
//! Reads the AAF object tree from structured storage (files from the AAF SDK —
//! Media Composer, Pro Tools, Premiere, Resolve… — pyaaf2, or VC Sound Design)
//! and rebuilds the top-level composition as tracks of clips:
//!
//! - every sound track becomes a project track, in a folder for the import; a
//!   pair of tracks that play channels 1 and 2 of the same stereo files,
//!   hard-panned left/right (or named "… L"/"… R"), becomes one stereo track;
//! - every `SourceClip` becomes a clip, followed through master and file mobs
//!   to its media. Linked WAV/BWF is used where it is (searched for by name
//!   when it moved). Embedded essence is extracted. AIFF and MXF
//!   (PCM) are converted, as is media at another sample rate (linear). A
//!   channel of a multichannel file becomes a mono extract. Everything written
//!   goes into the import's media folder;
//! - clip fades, and transitions as crossfades; clip gain (Mono Audio Gain);
//! - track gain and volume automation (Mono Audio Gain around the track: AAF
//!   interpolates linear amplitude, so the dB lane is fitted within 0.1 dB);
//!   pan (Mono Audio Pan) as track pan or pan automation; the two sides'
//!   gains of a stereo pair as volume and balance;
//! - comment markers become spotting markers in an "AAF markers" lane (those
//!   inside a scene).
//!
//! The composition's timecode places it on the timeline (unless `at` is
//! given). What can't be brought in is listed in the [`AafImportReport`].

use crate::aaf::{fit, key, parse_props};
use crate::error::{invalid, Result};
use crate::ids::new_id;
use crate::layout::safe_name;
use crate::library::{import_wav, AssetMeta};
use crate::model::*;
use crate::state::ProjectState;
use crate::wav::{self, SampleFormat};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use uuid::Uuid;

// ---------------------------------------------------------------- options and report

#[derive(Debug, Clone, Default)]
pub struct AafImportOptions {
    /// Where extracted, converted and per-channel media are written.
    pub media_dir: PathBuf,
    /// Folders searched (recursively, by file name) for linked media that moved;
    /// the AAF's own folder is always searched.
    pub search_dirs: Vec<PathBuf>,
    /// Timeline sample of the composition's start; `None` = its timecode.
    pub at: Option<i64>,
    /// Composition to import, by name; `None` = the top-level one.
    pub composition: Option<String>,
    /// Track folder for the new tracks; `None` = "AAF/<composition>".
    pub folder: Option<String>,
    /// Bus levels in an AAF from VC Sound Design (its "VCSD" comment):
    /// restore the buses, or keep them folded into the tracks.
    pub buses: crate::otio_import::BusLevels,
}

impl AafImportOptions {
    /// Defaults for a project at `root`: media written to `Audio/Imported/AAF/<file name>`.
    pub fn for_project(root: &Path, aaf: &Path) -> AafImportOptions {
        let stem = aaf.file_stem().map(|s| safe_name(&s.to_string_lossy())).filter(|s| !s.is_empty()).unwrap_or_else(|| "AAF".into());
        AafImportOptions { media_dir: root.join("Audio/Imported/AAF").join(stem), ..Default::default() }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AafImportReport {
    pub composition: String,
    /// Timecode of the composition's start, as placed on the timeline.
    pub start_tc: String,
    pub tracks: Vec<String>,
    pub stereo_tracks: usize,
    pub clips: usize,
    pub markers: usize,
    pub automated_tracks: usize,
    pub panned_tracks: usize,
    /// How bus levels came in: "buses" (restored from VC Sound Design's
    /// description) or "fold" (in the tracks, as AAF carries them).
    pub bus_levels: String,
    pub buses_restored: usize,
    pub buses_created: usize,
    /// Project buses that already held the file's levels (left as they were).
    pub buses_matched: usize,
    /// Tracks whose solo came back (from VC Sound Design's description).
    pub soloed_tracks: usize,
    /// Track inserts rebuilt from VC Sound Design's description (built-in effects
    /// and plugins), plugins among them, and plugin parameters with automation.
    pub inserts: usize,
    pub plugins: usize,
    pub automated_parameters: usize,
    /// Media files used where they are.
    pub media_linked: usize,
    /// Media written to the import folder (extracted, converted, channel extracts).
    pub media_written: usize,
    /// Clips left out because their media couldn't be found or read: why, per file.
    pub missing_media: Vec<String>,
    /// What the project can't hold, with counts.
    pub notes: Vec<String>,
}

#[derive(Default)]
struct Notes(BTreeMap<String, usize>);

impl Notes {
    fn add(&mut self, what: impl Into<String>, n: usize) {
        if n > 0 {
            *self.0.entry(what.into()).or_default() += n;
        }
    }
    fn list(&self) -> Vec<String> {
        self.0.iter().map(|(k, n)| format!("{k} ({n})")).collect()
    }
}

// ---------------------------------------------------------------- structured storage

const FMT_DATA: u16 = 0x82;
const FMT_STREAM: u16 = 0x42;
const FMT_STRONG: u16 = 0x22;
const FMT_VECTOR: u16 = 0x32;
const FMT_SET: u16 = 0x3a;
const FMT_WEAK: u16 = 0x02;
const FMT_WEAK_ID: u16 = 0x03;

/// Baseline class numbers (`0d010101-0101-NN00-060e-2b3402060101`).
mod c {
    pub const ESSENCE_GROUP: u8 = 0x05;
    pub const COMMENT_MARKER: u8 = 0x08;
    pub const FILLER: u8 = 0x09;
    pub const OPERATION_GROUP: u8 = 0x0a;
    pub const NESTED_SCOPE: u8 = 0x0b;
    pub const SELECTOR: u8 = 0x0e;
    pub const SEQUENCE: u8 = 0x0f;
    pub const SOURCE_CLIP: u8 = 0x11;
    pub const TIMECODE: u8 = 0x14;
    pub const TRANSITION: u8 = 0x17;
    pub const FILE_DESCRIPTOR: u8 = 0x25;
    pub const COMPOSITION_MOB: u8 = 0x35;
    pub const SOURCE_MOB: u8 = 0x37;
    pub const EVENT_SLOT: u8 = 0x39;
    pub const CONSTANT_VALUE: u8 = 0x3d;
    pub const VARYING_VALUE: u8 = 0x3e;
}

/// Baseline subclasses this importer needs to recognise: (class, parent).
const BUILTIN_PARENTS: &[(u8, u8)] = &[
    (0x41, 0x08), // DescriptiveMarker → CommentMarker
    (0x47, 0x48), // AES3PCMDescriptor → PCMDescriptor
    (0x48, 0x42), // PCMDescriptor → SoundDescriptor
    (0x42, 0x25), // SoundDescriptor → FileDescriptor
    (0x2c, 0x25), // WAVEDescriptor
    (0x26, 0x25), // AIFCDescriptor
    (0x44, 0x25), // MultipleDescriptor
];

fn cls(n: u8) -> Uuid {
    Uuid::parse_str(&format!("0d010101-0101-{n:02x}00-060e-2b3402060101")).unwrap()
}

/// Stereo Audio Gain, and the Level parameter some apps give gain instead of Amplitude (as stored).
const OP_STEREO_AUDIO_GAIN: &str = "95a82e9d6809d3118a380050040ef7d2";
const PARAM_LEVEL: &str = "202396e46722d3118a4c0050040ef7d2";

/// Interpolation definitions (as stored): constant (hold) and none.
const INTERP_CONSTANT: &str = "a5856c5bde0ed31180a9006008143e6f";
const INTERP_NONE: &str = "a3856c5bde0ed31180a9006008143e6f";

/// Legacy (pre-SMPTE) sound data definition, still written by some editors.
const LEGACY_SOUND: &str = "e1ebe178ef6cd211807d006008143e6f";

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn utf16(b: &[u8]) -> String {
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&u)
}

fn join(path: &str, name: &str) -> String {
    if path == "/" {
        format!("/{name}")
    } else {
        format!("{path}/{name}")
    }
}

/// An object: its storage path, class and properties `(pid, stored form, bytes)`.
#[derive(Clone)]
struct Node {
    path: String,
    class: Uuid,
    props: Rc<Vec<(u16, u16, Vec<u8>)>>,
}

impl Node {
    fn raw(&self, pid: u16) -> Option<(u16, &[u8])> {
        self.props.iter().find(|p| p.0 == pid).map(|p| (p.1, p.2.as_slice()))
    }
    fn data(&self, pid: u16) -> Option<&[u8]> {
        self.raw(pid).filter(|p| p.0 == FMT_DATA).map(|p| p.1)
    }
    fn i64(&self, pid: u16) -> Option<i64> {
        let d = self.data(pid)?;
        Some(match d.len() {
            8 => i64::from_le_bytes(d.try_into().ok()?),
            4 => i32::from_le_bytes(d.try_into().ok()?) as i64,
            _ => return None,
        })
    }
    fn u32(&self, pid: u16) -> Option<u32> {
        let d = self.data(pid)?;
        match d.len() {
            4 => Some(u32::from_le_bytes(d.try_into().ok()?)),
            2 => Some(u16::from_le_bytes(d.try_into().ok()?) as u32),
            1 => Some(d[0] as u32),
            _ => None,
        }
    }
    fn rational(&self, pid: u16) -> Option<(i64, i64)> {
        rational(self.data(pid)?)
    }
    fn string(&self, pid: u16) -> Option<String> {
        self.data(pid).map(utf16)
    }
    /// Key of a weak reference (definitions are keyed by their AUID).
    fn weak(&self, pid: u16) -> Option<&[u8]> {
        let (fmt, d) = self.raw(pid)?;
        if fmt != FMT_WEAK && fmt != FMT_WEAK_ID || d.len() < 5 {
            return None;
        }
        let n = d[4] as usize;
        d.get(5..5 + n)
    }
    fn len(&self) -> i64 {
        self.i64(0x0202).unwrap_or(0)
    }
}

fn rational(d: &[u8]) -> Option<(i64, i64)> {
    if d.len() < 8 {
        return None;
    }
    let n = i32::from_le_bytes(d[0..4].try_into().ok()?) as i64;
    let den = i32::from_le_bytes(d[4..8].try_into().ok()?) as i64;
    (den != 0).then_some((n, den))
}

/// An indirect value (byte order, type AUID, value) as a number: Rational only.
/// An indirect value (byte order, type AUID, value) as a number: Rational, or
/// any of AAF's integer types.
fn indirect(d: &[u8]) -> Option<f64> {
    if d.len() < 18 {
        return None;
    }
    let (ty, v) = (hexs(&d[1..17]), &d[17..]);
    if ty == key::TYPE_RATIONAL {
        return rational(v).map(|(n, den)| n as f64 / den as f64);
    }
    // aafUInt8 … aafInt64: 0101nn00-0000-0000-060e-2b3401040101, stored as GUID bytes
    let n = u8::from_str_radix(ty.get(2..4)?, 16).ok()?;
    if !(ty.starts_with("00") && &ty[4..] == "010100000000060e2b3401040101") || !(1..=8).contains(&n) {
        return None;
    }
    let (size, signed) = [(1, false), (2, false), (4, false), (8, false), (1, true), (2, true), (4, true), (8, true)][n as usize - 1];
    let b = v.get(..size)?;
    let mut raw = [0u8; 8];
    raw[..size].copy_from_slice(b);
    let u = u64::from_le_bytes(raw);
    Some(if signed {
        let shift = 64 - 8 * size as u32;
        ((u << shift) as i64 >> shift) as f64
    } else {
        u as f64
    })
}

struct Aaf {
    cf: RefCell<cfb::CompoundFile<std::fs::File>>,
    /// Class → parent class, from the file's meta-dictionary.
    parents: HashMap<Uuid, Uuid>,
}

impl Aaf {
    fn open(path: &Path) -> Result<Aaf> {
        let cf = cfb::open(path).map_err(|e| invalid(format!("{} is not an AAF file ({e})", path.display())))?;
        let mut a = Aaf { cf: RefCell::new(cf), parents: HashMap::new() };
        let root = a.node("/")?;
        if let Some(meta) = a.child(&root, 0x0001) {
            for def in a.children(&meta, 0x0003) {
                let (Some(id), Some(parent)) = (def.data(0x0005), def.weak(0x0008)) else { continue };
                if let (Ok(id), Ok(parent)) = (<[u8; 16]>::try_from(id), <[u8; 16]>::try_from(parent)) {
                    let (id, parent) = (Uuid::from_bytes_le(id), Uuid::from_bytes_le(parent));
                    if id != parent {
                        a.parents.insert(id, parent);
                    }
                }
            }
        }
        Ok(a)
    }

    fn node(&self, path: &str) -> Result<Node> {
        let mut cf = self.cf.borrow_mut();
        let class = *cf.entry(path).map_err(|e| invalid(format!("AAF: {path}: {e}")))?.clsid();
        let mut b = vec![];
        cf.open_stream(join(path, "properties"))?.read_to_end(&mut b)?;
        if b.first() != Some(&0x4c) {
            return Err(invalid("AAF: big-endian files are not supported"));
        }
        Ok(Node { path: path.into(), class, props: Rc::new(parse_props(&b)?) })
    }

    fn child(&self, n: &Node, pid: u16) -> Option<Node> {
        let (fmt, d) = n.raw(pid)?;
        if fmt != FMT_STRONG {
            return None;
        }
        self.node(&join(&n.path, &utf16(d))).ok()
    }

    /// Members of a strong-reference vector or set, in stored order.
    fn children(&self, n: &Node, pid: u16) -> Vec<Node> {
        let Some((fmt, d)) = n.raw(pid) else { return vec![] };
        if fmt != FMT_VECTOR && fmt != FMT_SET {
            return vec![];
        }
        let name = utf16(d);
        let mut idx = vec![];
        let ok = self.cf.borrow_mut().open_stream(join(&n.path, &format!("{name} index"))).and_then(|mut s| s.read_to_end(&mut idx));
        if ok.is_err() || idx.len() < 12 {
            return vec![];
        }
        let count = u32::from_le_bytes(idx[0..4].try_into().unwrap()) as usize;
        let (head, entry) = if fmt == FMT_VECTOR {
            (12, 4)
        } else {
            match idx.get(14) {
                Some(&ks) => (15, 8 + ks as usize),
                None => return vec![],
            }
        };
        (0..count)
            .filter_map(|i| {
                let o = head + i * entry;
                let local = u32::from_le_bytes(idx.get(o..o + 4)?.try_into().ok()?);
                self.node(&join(&n.path, &format!("{name}{{{local:x}}}"))).ok()
            })
            .collect()
    }

    fn stream(&self, n: &Node, pid: u16) -> Option<Vec<u8>> {
        let (fmt, d) = n.raw(pid)?;
        if fmt != FMT_STREAM || d.is_empty() {
            return None;
        }
        let mut out = vec![];
        self.cf.borrow_mut().open_stream(join(&n.path, &utf16(&d[1..]))).ok()?.read_to_end(&mut out).ok()?;
        Some(out)
    }

    fn is(&self, n: &Node, class: u8) -> bool {
        let target = cls(class);
        let mut c = n.class;
        for _ in 0..32 {
            if c == target {
                return true;
            }
            c = match self.parents.get(&c) {
                Some(p) => *p,
                None => match BUILTIN_PARENTS.iter().find(|(k, _)| cls(*k) == c) {
                    Some((_, p)) => cls(*p),
                    None => return false,
                },
            };
        }
        false
    }
}

// ---------------------------------------------------------------- timeline model

type Rate = (i64, i64);

/// `v` edit units at `from` → edit units at `to` (rounded).
fn conv(v: i64, from: Rate, to: Rate) -> i64 {
    let mut n = v as i128 * from.1 as i128 * to.0 as i128;
    let mut d = from.0 as i128 * to.1 as i128;
    if d == 0 {
        return v;
    }
    if d < 0 {
        (n, d) = (-n, -d);
    }
    // round half away from zero
    (if n >= 0 { (2 * n + d) / (2 * d) } else { -((-2 * n + d) / (2 * d)) }) as i64
}

/// Where a clip's audio comes from: a file mob, the channel in it, and the
/// offset (at the file's sample rate).
#[derive(Debug, Clone, PartialEq)]
struct Src {
    file_mob: Vec<u8>,
    channel: u32,
    offset: i64,
}

#[derive(Debug, Clone)]
enum Curve {
    Const(f64),
    Points(Vec<(i64, f64)>),
}

impl Curve {
    fn at(&self, t: i64) -> f64 {
        match self {
            Curve::Const(v) => *v,
            Curve::Points(p) => {
                let i = p.partition_point(|x| x.0 <= t);
                match (i.checked_sub(1).map(|k| p[k]), p.get(i)) {
                    (Some(a), Some(b)) => a.1 + (b.1 - a.1) * (t - a.0) as f64 / (b.0 - a.0).max(1) as f64,
                    (Some(a), None) => a.1,
                    (None, Some(b)) => b.1,
                    (None, None) => 1.0,
                }
            }
        }
    }
}

/// A gain or pan curve over `[start, end)` (samples from the composition start).
#[derive(Debug, Clone)]
struct Span {
    start: i64,
    end: i64,
    curve: Curve,
}

fn covers(s: &Span, t: i64) -> bool {
    s.start <= t && t < s.end
}

#[derive(Debug, Clone)]
struct RawClip {
    at: i64,
    len: i64,
    src: std::result::Result<Src, String>,
    fade_in: i64,
    fade_out: i64,
    gain: f64,
    /// Inside a zero clip gain: silenced on export (a muted clip) or genuinely at −∞.
    zeroed: bool,
    muted: bool,
}

#[derive(Debug, Clone, Default)]
struct Lane {
    name: String,
    /// The AAF track (slot) id.
    slot: u32,
    clips: Vec<RawClip>,
    gains: Vec<Span>,
    pans: Vec<Span>,
    /// Zero gains around the track: our export silencing it, or genuinely silent.
    zero_spans: Vec<Span>,
    /// Muted (a zero gain around it, in another app's AAF).
    muted: bool,
}

impl Lane {
    fn gain(&self, t: i64) -> f64 {
        self.gains.iter().filter(|s| covers(s, t)).map(|s| s.curve.at(t)).product()
    }
    /// Pan parameter 0 (left) … 1 (right); 0.5 when not panned.
    fn pan(&self, t: i64) -> f64 {
        self.pans.iter().rev().find(|s| covers(s, t)).map(|s| s.curve.at(t)).unwrap_or(0.5)
    }
    /// Where the clips play: `[first start, last end)`.
    fn extent(&self) -> (i64, i64) {
        let lo = self.clips.iter().map(|c| c.at).min().unwrap_or(0);
        let hi = self.clips.iter().map(|c| c.at + c.len).max().unwrap_or(lo + 1);
        (lo, hi.max(lo + 1))
    }
    fn keys(&self) -> Vec<i64> {
        let mut k = vec![];
        for s in self.gains.iter().chain(&self.pans) {
            k.extend([s.start - 1, s.start, s.end - 1, s.end]);
            if let Curve::Points(p) = &s.curve {
                k.extend(p.iter().map(|x| x.0));
            }
        }
        k
    }
}

struct Walk<'a> {
    lane: Lane,
    rate: Rate,
    sr: Rate,
    pending_in: i64,
    /// Walking inside a clip's zero gain.
    zero_clip: bool,
    /// Walking an inactive alternate of a selector (a switched-off clip).
    inactive: bool,
    notes: &'a mut Notes,
}

impl Walk<'_> {
    fn smp(&self, v: i64) -> i64 {
        conv(v, self.rate, self.sr)
    }
}

// ---------------------------------------------------------------- reading the content

struct Content {
    aaf: Aaf,
    mobs: HashMap<Vec<u8>, Node>,
    essence: HashMap<Vec<u8>, Node>,
}

impl Content {
    fn read(path: &Path) -> Result<Content> {
        let aaf = Aaf::open(path)?;
        let root = aaf.node("/")?;
        let header = aaf.child(&root, 0x0002).ok_or_else(|| invalid("AAF: no header"))?;
        let content = aaf.child(&header, 0x3b03).ok_or_else(|| invalid("AAF: no content storage"))?;
        let mut mobs = HashMap::new();
        for m in aaf.children(&content, 0x1901) {
            if let Some(id) = m.data(0x4401) {
                mobs.insert(id.to_vec(), m.clone());
            }
        }
        let mut essence = HashMap::new();
        for e in aaf.children(&content, 0x1902) {
            if let Some(id) = e.data(0x2701) {
                essence.insert(id.to_vec(), e.clone());
            }
        }
        Ok(Content { aaf, mobs, essence })
    }

    fn slot(&self, mob: &Node, id: u32) -> Option<Node> {
        self.aaf.children(mob, 0x4403).into_iter().find(|s| s.u32(0x4801) == Some(id))
    }

    fn slot_rate(slot: &Node) -> Option<Rate> {
        slot.rational(0x4b01).or_else(|| slot.rational(0x4901)).filter(|r| r.0 > 0)
    }

    fn is_sound(&self, seg: &Node) -> bool {
        seg.weak(0x0201).map(hexs).is_some_and(|k| k == key::SOUND || k == LEGACY_SOUND)
    }

    /// The component under `pos` in a segment, with its start: through
    /// sequences, effects, selectors and essence groups down to a source clip.
    fn find_at(&self, seg: &Node, pos: i64, depth: u32) -> Option<(Node, i64)> {
        let a = &self.aaf;
        if depth > 32 {
            return None;
        }
        if a.is(seg, c::SOURCE_CLIP) {
            return Some((seg.clone(), 0));
        }
        let inner = if a.is(seg, c::SEQUENCE) {
            let comps = a.children(seg, 0x1001);
            let mut p = 0;
            for (i, comp) in comps.iter().enumerate() {
                let l = comp.len();
                if a.is(comp, c::TRANSITION) {
                    p -= l;
                    continue;
                }
                if pos < p + l || i + 1 == comps.len() {
                    return self.find_at(comp, pos - p, depth + 1).map(|(n, q)| (n, q + p));
                }
                p += l;
            }
            return None;
        } else if a.is(seg, c::OPERATION_GROUP) {
            a.children(seg, 0x0b02).into_iter().next()
        } else if a.is(seg, c::SELECTOR) {
            a.child(seg, 0x0f01)
        } else if a.is(seg, c::ESSENCE_GROUP) {
            a.children(seg, 0x0501).into_iter().next()
        } else if a.is(seg, c::NESTED_SCOPE) {
            a.children(seg, 0x0c01).into_iter().last()
        } else {
            None
        };
        self.find_at(&inner?, pos, depth + 1)
    }

    /// Follow `mob_id`/`slot_id` at `start` (that slot's edit units) to a file.
    fn resolve(&self, mob_id: &[u8], slot_id: u32, start: i64, depth: u32) -> std::result::Result<Src, String> {
        let a = &self.aaf;
        let mob = self.mobs.get(mob_id).ok_or("media not in the AAF (a reference to a mob that isn't there)")?;
        let name = mob.string(0x4402).unwrap_or_default();
        let slot = self.slot(mob, slot_id).ok_or_else(|| format!("{name}: no track {slot_id}"))?;
        let rate = Self::slot_rate(&slot).unwrap_or((1, 1));
        let pos = start + slot.i64(0x4b02).unwrap_or(0);
        if a.is(mob, c::SOURCE_MOB) {
            if let Some(desc) = a.child(mob, 0x4701) {
                if a.is(&desc, c::FILE_DESCRIPTOR) {
                    let file_rate = desc.rational(0x3d03).or_else(|| desc.rational(0x3001)).filter(|r| r.0 > 0).unwrap_or(rate);
                    // a file's sample rate in whole samples per second
                    let fs = (file_rate.0 as f64 / file_rate.1 as f64).round() as i64;
                    return Ok(Src {
                        file_mob: mob_id.to_vec(),
                        channel: slot.u32(0x4804).filter(|&c| c > 0).unwrap_or(1),
                        offset: conv(pos, rate, (fs, 1)),
                    });
                }
            }
        }
        if depth > 16 {
            return Err(format!("{name}: media chain too deep"));
        }
        let seg = a.child(&slot, 0x4803).ok_or_else(|| format!("{name}: empty track"))?;
        let (clip, at) = self.find_at(&seg, pos, 0).ok_or_else(|| format!("{name}: no media"))?;
        let next = clip.data(0x1101).unwrap_or(&[]).to_vec();
        if next.iter().all(|&b| b == 0) {
            return Err(format!("{name}: offline (no source media)"));
        }
        let next_slot = clip.u32(0x1102).unwrap_or(1);
        let next_rate = self.mobs.get(&next).and_then(|m| self.slot(m, next_slot)).and_then(|s| Self::slot_rate(&s)).unwrap_or(rate);
        let st = clip.i64(0x1201).unwrap_or(0) + conv(pos - at, rate, next_rate);
        self.resolve(&next, next_slot, st, depth + 1).map_err(|e| if e.starts_with("media not in") { format!("{name}: {e}") } else { e })
    }

    /// A gain/pan parameter as a curve over `[pos, pos+len)` (slot units).
    fn curve(&self, p: &Node, pos: i64, len: i64, w: &mut Walk) -> Option<Curve> {
        let a = &self.aaf;
        if a.is(p, c::CONSTANT_VALUE) {
            return p.data(0x4d01).and_then(indirect).map(Curve::Const);
        }
        if !a.is(p, c::VARYING_VALUE) {
            return None;
        }
        // constant (and "none") interpolation holds each value until the next point
        let interp = p.weak(0x4e01).map(hexs).unwrap_or_else(|| key::INTERP_LINEAR.to_string());
        let steps = interp == INTERP_CONSTANT || interp == INTERP_NONE;
        if !steps && interp != key::INTERP_LINEAR {
            w.notes.add("gain or pan curves with curved interpolation (B-spline, log, power) read as linear", 1);
        }
        let (s0, slen) = (w.smp(pos), w.smp(len));
        let mut pts: Vec<(i64, f64)> = a
            .children(p, 0x4e02)
            .iter()
            .filter_map(|cp| {
                let (n, d) = cp.rational(0x1a03)?;
                let v = cp.data(0x1a02).and_then(indirect)?;
                Some((s0 + (n as f64 / d as f64 * slen as f64).round() as i64, v))
            })
            .collect();
        pts.sort_by_key(|x| x.0);
        if steps {
            let mut held = vec![];
            for (i, &(t, v)) in pts.iter().enumerate() {
                held.push((t, v));
                if let Some(&(next, nv)) = pts.get(i + 1) {
                    if next - 1 > t && nv != v {
                        held.push((next - 1, v));
                    }
                }
            }
            pts = held;
        }
        (!pts.is_empty()).then_some(Curve::Points(pts))
    }

    /// True when `n` is one clip (possibly inside effects), not a sequence of them.
    fn clip_level(&self, n: &Node) -> bool {
        let a = &self.aaf;
        if a.is(n, c::SOURCE_CLIP) {
            return true;
        }
        if a.is(n, c::OPERATION_GROUP) {
            return a.children(n, 0x0b02).first().is_some_and(|i| self.clip_level(i));
        }
        false
    }

    fn walk(&self, n: &Node, pos: i64, gain: f64, w: &mut Walk, depth: u32) {
        let a = &self.aaf;
        if depth > 64 {
            return;
        }
        let len = n.len();
        if a.is(n, c::SEQUENCE) {
            let mut p = pos;
            for comp in a.children(n, 0x1001) {
                let l = comp.len();
                if a.is(&comp, c::TRANSITION) {
                    p -= l;
                    let xf = w.smp(l);
                    let cut = w.smp(p + l);
                    if let Some(last) = w.lane.clips.last_mut() {
                        if (last.at + last.len - cut).abs() <= 1 {
                            last.fade_out = last.fade_out.max(xf);
                        }
                    }
                    w.pending_in = xf;
                    continue;
                }
                self.walk(&comp, p, gain, w, depth + 1);
                p += l;
            }
        } else if a.is(n, c::SOURCE_CLIP) {
            let (at, l) = (w.smp(pos), w.smp(len));
            let id = n.data(0x1101).unwrap_or(&[]).to_vec();
            let src = if id.iter().all(|&b| b == 0) {
                Err("offline clip (no source media)".to_string())
            } else {
                self.resolve(&id, n.u32(0x1102).unwrap_or(1), n.i64(0x1201).unwrap_or(0), 0)
            };
            let fade_in = w.smp(n.i64(0x1202).unwrap_or(0)).max(std::mem::take(&mut w.pending_in));
            let fade_out = w.smp(n.i64(0x1204).unwrap_or(0));
            if l > 0 {
                w.lane.clips.push(RawClip { at, len: l, src, fade_in, fade_out, gain, zeroed: w.zero_clip, muted: w.inactive });
            }
        } else if a.is(n, c::OPERATION_GROUP) {
            let op = n.weak(0x0b01).map(hexs).unwrap_or_default();
            let inputs = a.children(n, 0x0b02);
            let Some(input) = inputs.first() else { return };
            // gain: Mono or Stereo Audio Gain, its level in Amplitude (or Level, as some
            // apps write it); pan: Mono Audio Pan
            let (param_defs, is_gain): (&[&str], bool) = match op.as_str() {
                k if k == key::OP_MONO_AUDIO_GAIN || k == OP_STEREO_AUDIO_GAIN => (&[key::PARAM_AMPLITUDE, PARAM_LEVEL], true),
                k if k == key::OP_MONO_AUDIO_PAN => (&[key::PARAM_PAN], false),
                _ => {
                    w.notes.add("audio effects other than gain and pan left out", 1);
                    return self.walk(input, pos, gain, w, depth + 1);
                }
            };
            let params = a.children(n, 0x0b03);
            let param = param_defs.iter().find_map(|d| params.iter().find(|p| p.data(0x4c01).map(hexs).as_deref() == Some(*d)).cloned());
            let curve = param.and_then(|p| self.curve(&p, pos, len, w));
            // the project's levels stop at +12 dB (a volume lane, clip gain)
            let peak = match &curve {
                Some(Curve::Const(v)) => *v,
                Some(Curve::Points(p)) => p.iter().map(|x| x.1).fold(0.0, f64::max),
                None => 0.0,
            };
            if is_gain && peak > 3.982 {
                w.notes.add("gain above +12 dB clamped to +12 dB", 1);
            }
            let span = |curve| Span { start: w.smp(pos), end: w.smp(pos + len), curve };
            match curve {
                // a zero gain: kept apart, since it may be our export silencing a muted clip or
                // track (the description says which); otherwise it is applied as −∞ later
                Some(Curve::Const(v)) if is_gain && v == 0.0 && self.clip_level(input) => {
                    let was = std::mem::replace(&mut w.zero_clip, true);
                    self.walk(input, pos, gain, w, depth + 1);
                    w.zero_clip = was;
                    return;
                }
                Some(Curve::Const(v)) if is_gain && v == 0.0 => w.lane.zero_spans.push(span(Curve::Const(0.0))),
                Some(Curve::Const(v)) if is_gain && self.clip_level(input) => return self.walk(input, pos, gain * v, w, depth + 1),
                Some(cv) => {
                    let s = span(cv);
                    if is_gain {
                        w.lane.gains.push(s);
                    } else {
                        w.lane.pans.push(s);
                    }
                }
                None => w.notes.add("gain or pan effects with unreadable values left out", 1),
            }
            self.walk(input, pos, gain, w, depth + 1);
        } else if a.is(n, c::SELECTOR) {
            let sel = a.child(n, 0x0f01);
            // nothing selected (filler) but a clip among the alternates: a switched-off clip,
            // in the timeline but silent — it comes in muted
            if sel.as_ref().is_none_or(|x| a.is(x, c::FILLER)) {
                if let Some(alt) = a.children(n, 0x0f02).into_iter().find(|x| self.count_clips(x, 0) > 0) {
                    w.notes.add("inactive clips (alternates of an empty selector) imported muted", 1);
                    let was = std::mem::replace(&mut w.inactive, true);
                    self.walk(&alt, pos, gain, w, depth + 1);
                    w.inactive = was;
                    return;
                }
            }
            if let Some(sel) = sel {
                self.walk(&sel, pos, gain, w, depth + 1);
            }
        } else if a.is(n, c::ESSENCE_GROUP) {
            if let Some(ch) = a.children(n, 0x0501).first() {
                self.walk(ch, pos, gain, w, depth + 1);
            }
        } else if a.is(n, c::NESTED_SCOPE) {
            if let Some(last) = a.children(n, 0x0c01).last() {
                self.walk(last, pos, gain, w, depth + 1);
            }
        } else if a.is(n, c::FILLER) {
            w.pending_in = 0;
        }
    }

    /// Source clips (sound) under a segment: how much of the timeline a composition holds.
    fn count_clips(&self, n: &Node, depth: u32) -> usize {
        let a = &self.aaf;
        if depth > 64 {
            return 0;
        }
        if a.is(n, c::SOURCE_CLIP) {
            return 1;
        }
        [0x1001, 0x0b02, 0x0501, 0x0c01]
            .iter()
            .flat_map(|&p| a.children(n, p))
            .chain(a.child(n, 0x0f01))
            .map(|x| self.count_clips(&x, depth + 1))
            .sum()
    }

    /// The composition to import: by name, else the one with the most sound
    /// clips among those no other composition uses.
    fn pick_composition(&self, name: Option<&str>) -> Result<Node> {
        let a = &self.aaf;
        let comps: Vec<&Node> = self.mobs.values().filter(|m| a.is(m, c::COMPOSITION_MOB)).collect();
        if let Some(want) = name {
            return comps
                .into_iter()
                .find(|m| m.string(0x4402).as_deref() == Some(want))
                .cloned()
                .ok_or_else(|| invalid(format!("the AAF has no composition named '{want}'")));
        }
        let clips = |m: &Node| -> usize {
            a.children(m, 0x4403)
                .iter()
                .filter_map(|s| a.child(s, 0x4803))
                .filter(|g| self.is_sound(g))
                .map(|g| self.count_clips(&g, 0))
                .sum()
        };
        let mut best: Option<(&Node, usize)> = None;
        for m in comps {
            let n = clips(m);
            if best.is_none_or(|(b, k)| n > k || (n == k && m.string(0x4402) < b.string(0x4402))) {
                best = Some((m, n));
            }
        }
        best.map(|b| b.0.clone()).ok_or_else(|| invalid("the AAF has no composition (sequence) to import"))
    }
}

// ---------------------------------------------------------------- media

/// Find, extract or convert the media of `file_mob` into a readable WAV.
/// Returns its path and whether it was written (vs used in place).
struct Media<'a> {
    content: &'a Content,
    o: &'a AafImportOptions,
    aaf_dir: PathBuf,
    index: Option<HashMap<String, PathBuf>>,
    files: HashMap<Vec<u8>, std::result::Result<(PathBuf, bool), String>>,
    channels: HashMap<(PathBuf, u32), std::result::Result<PathBuf, String>>,
    resampled: HashMap<PathBuf, std::result::Result<PathBuf, String>>,
    written: usize,
}

pub(crate) fn decode_url(u: &str) -> String {
    let rest = u.strip_prefix("file://localhost").or_else(|| u.strip_prefix("file://")).unwrap_or(u);
    let b = rest.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    let s = String::from_utf8_lossy(&out).into_owned();
    // file:///C:/… → C:/…
    let bytes = s.as_bytes();
    if bytes.len() > 3 && bytes[0] == b'/' && bytes[2] == b':' && bytes[1].is_ascii_alphabetic() {
        return s[1..].to_string();
    }
    s
}

pub(crate) fn file_name_of(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

pub(crate) fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

pub(crate) fn index_dir(dir: &Path, depth: u32, out: &mut HashMap<String, PathBuf>) {
    if depth > 5 || out.len() > 50_000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            index_dir(&p, depth + 1, out);
        } else if let Some(n) = p.file_name() {
            out.entry(n.to_string_lossy().to_lowercase()).or_insert(p);
        }
    }
}

impl Media<'_> {
    fn out_path(&self, name: &str, mob: &[u8], suffix: &str) -> PathBuf {
        let stem = safe_name(name);
        let stem = if stem.is_empty() { "media".to_string() } else { stem };
        // named after the mob, so importing again reuses what was written
        self.o.media_dir.join(format!("{stem}_{}{suffix}.wav", hexs(&mob[mob.len().saturating_sub(4)..])))
    }

    fn find(&mut self, url: &str) -> Option<PathBuf> {
        let p = PathBuf::from(decode_url(url));
        if p.is_file() {
            return Some(p);
        }
        let name = file_name_of(&decode_url(url));
        if name.is_empty() {
            return None;
        }
        let near = self.aaf_dir.join(&name);
        if near.is_file() {
            return Some(near);
        }
        if self.index.is_none() {
            let mut idx = HashMap::new();
            for d in self.o.search_dirs.iter().chain(std::iter::once(&self.aaf_dir)) {
                index_dir(d, 0, &mut idx);
            }
            self.index = Some(idx);
        }
        self.index.as_ref().unwrap().get(&name.to_lowercase()).cloned()
    }

    fn file(&mut self, mob_id: &[u8]) -> std::result::Result<(PathBuf, bool), String> {
        if let Some(r) = self.files.get(mob_id) {
            return r.clone();
        }
        let r = self.load(mob_id);
        if matches!(r, Ok((_, true))) {
            self.written += 1;
        }
        self.files.insert(mob_id.to_vec(), r.clone());
        r
    }

    fn load(&mut self, mob_id: &[u8]) -> std::result::Result<(PathBuf, bool), String> {
        let a = &self.content.aaf;
        let mob = self.content.mobs.get(mob_id).ok_or("no file mob")?.clone();
        let desc = a.child(&mob, 0x4701).ok_or("no essence descriptor")?;
        let name = mob.string(0x4402).unwrap_or_default();
        let name = name.trim_end_matches(" <SOURCE MOB>").trim_end_matches(".PHYS").to_string();
        let pcm = pcm_params(&desc);
        // embedded essence
        if let Some(ed) = self.content.essence.get(mob_id) {
            let data = a.stream(ed, 0x2702).ok_or_else(|| format!("{name}: embedded media unreadable"))?;
            let out = self.out_path(&name, mob_id, "");
            if !out.is_file() {
                std::fs::create_dir_all(&self.o.media_dir).map_err(|e| e.to_string())?;
                if data.starts_with(b"RIFF") {
                    std::fs::write(&out, &data).map_err(|e| e.to_string())?;
                } else if data.starts_with(b"FORM") {
                    aiff_to_wav(&data, &out)?;
                } else {
                    let (sr, ch, bits) = pcm.ok_or_else(|| format!("{name}: embedded media of an unknown format"))?;
                    write_pcm(&out, sr, ch, bits, &data)?;
                }
            }
            return Ok((out, true));
        }
        // linked
        let urls: Vec<String> = a
            .children(&desc, 0x2f01)
            .iter()
            .filter_map(|l| l.string(0x4001).or_else(|| l.string(0x4101)))
            .filter(|u| !u.is_empty())
            .collect();
        let Some(url) = urls.first().cloned() else { return Err(format!("{name}: no media location in the AAF")) };
        let Some(path) = urls.iter().find_map(|u| self.find(u)) else {
            return Err(format!("{} not found ({})", file_name_of(&decode_url(&url)), decode_url(&url)));
        };
        match ext_of(&path).as_str() {
            "wav" | "bwf" | "wave" => {
                wav::read_info(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok((path, false))
            }
            "aif" | "aiff" | "aifc" => {
                let out = self.out_path(&name, mob_id, "");
                if !out.is_file() {
                    std::fs::create_dir_all(&self.o.media_dir).map_err(|e| e.to_string())?;
                    aiff_to_wav(&std::fs::read(&path).map_err(|e| e.to_string())?, &out)?;
                }
                Ok((out, true))
            }
            "mxf" => {
                let out = self.out_path(&name, mob_id, "");
                if !out.is_file() {
                    std::fs::create_dir_all(&self.o.media_dir).map_err(|e| e.to_string())?;
                    mxf_to_wav(&std::fs::read(&path).map_err(|e| e.to_string())?, pcm, &out)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                }
                Ok((out, true))
            }
            e => Err(format!("{}: .{e} media can't be read (WAV, AIFF and PCM/AES3 MXF can)", path.display())),
        }
    }

    /// One channel of a multichannel file as a mono WAV.
    fn channel(&mut self, path: &Path, mob_id: &[u8], ch: u32) -> std::result::Result<PathBuf, String> {
        let k = (path.to_path_buf(), ch);
        if let Some(r) = self.channels.get(&k) {
            return r.clone();
        }
        let r = (|| {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            // files written here are already named after their mob
            let out = if path.starts_with(&self.o.media_dir) {
                self.o.media_dir.join(format!("{stem}_ch{ch}.wav"))
            } else {
                self.out_path(&stem, mob_id, &format!("_ch{ch}"))
            };
            if !out.is_file() {
                let (info, raw) = wav::read_raw(path).map_err(|e| e.to_string())?;
                let bps = info.format.bits() as usize / 8;
                let n = info.channels as usize;
                let c = (ch as usize - 1).min(n - 1);
                let mono: Vec<u8> = raw.chunks_exact(n * bps).flat_map(|f| f[c * bps..(c + 1) * bps].to_vec()).collect();
                std::fs::create_dir_all(&self.o.media_dir).map_err(|e| e.to_string())?;
                wav::write_raw(&out, info.sample_rate, 1, info.format, &mono).map_err(|e| e.to_string())?;
            }
            self.written += 1;
            Ok(out)
        })();
        self.channels.insert(k, r.clone());
        r
    }

    /// The file at the project's sample rate (converted copy when it isn't).
    fn at_rate(&mut self, path: &Path, sr: u32) -> std::result::Result<PathBuf, String> {
        if let Some(r) = self.resampled.get(path) {
            return r.clone();
        }
        let r = (|| {
            let info = wav::read_info(path).map_err(|e| e.to_string())?;
            if info.sample_rate == sr {
                return Ok(path.to_path_buf());
            }
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let out = self.o.media_dir.join(format!("{}_{sr}.wav", safe_name(&stem)));
            if !out.is_file() {
                let (_, buf) = wav::read(path).map_err(|e| e.to_string())?;
                std::fs::create_dir_all(&self.o.media_dir).map_err(|e| e.to_string())?;
                let conv = crate::mixer::resample_linear(&buf, sr);
                wav::write(&out, &conv, SampleFormat::Int24, None).map_err(|e| e.to_string())?;
            }
            self.written += 1;
            Ok(out)
        })();
        self.resampled.insert(path.to_path_buf(), r.clone());
        r
    }
}

/// (sample rate, channels, bits) from a sound descriptor.
fn pcm_params(desc: &Node) -> Option<(u32, u16, u16)> {
    let rate = desc.rational(0x3d03).or_else(|| desc.rational(0x3001))?;
    let ch = desc.u32(0x3d07)?;
    let bits = desc.u32(0x3d01)?;
    Some(((rate.0 as f64 / rate.1 as f64).round() as u32, ch as u16, bits as u16))
}

fn int_format(bits: u16) -> std::result::Result<SampleFormat, String> {
    match bits {
        16 => Ok(SampleFormat::Int16),
        24 => Ok(SampleFormat::Int24),
        32 => Ok(SampleFormat::Int32),
        b => Err(format!("{b}-bit audio is not supported")),
    }
}

fn write_pcm(out: &Path, sr: u32, ch: u16, bits: u16, data: &[u8]) -> std::result::Result<(), String> {
    if ch == 0 || sr == 0 {
        return Err("embedded media without a channel count or sample rate".into());
    }
    let block = ch as usize * bits as usize / 8;
    wav::write_raw(out, sr, ch, int_format(bits)?, &data[..data.len() / block * block]).map_err(|e| e.to_string())
}

/// IEEE 754 80-bit extended (AIFF sample rate) → f64.
fn ext80(b: &[u8]) -> f64 {
    let exp = (((b[0] & 0x7f) as i32) << 8 | b[1] as i32) - 16383 - 63;
    let mant = u64::from_be_bytes(b[2..10].try_into().unwrap());
    mant as f64 * 2f64.powi(exp)
}

/// AIFF / AIFC (PCM, big- or little-endian) → WAV.
pub(crate) fn aiff_to_wav(b: &[u8], out: &Path) -> std::result::Result<(), String> {
    if b.len() < 12 || &b[0..4] != b"FORM" || !(&b[8..12] == b"AIFF" || &b[8..12] == b"AIFC") {
        return Err("not an AIFF file".into());
    }
    let (mut comm, mut ssnd) = (None, None);
    let mut i = 12;
    while i + 8 <= b.len() {
        let size = u32::from_be_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        let body = &b[i + 8..(i + 8 + size).min(b.len())];
        match &b[i..i + 4] {
            b"COMM" => comm = Some(body),
            b"SSND" => ssnd = Some(body),
            _ => {}
        }
        i += 8 + size + (size & 1);
    }
    let comm = comm.filter(|c| c.len() >= 18).ok_or("AIFF without a COMM chunk")?;
    let ssnd = ssnd.filter(|s| s.len() >= 8).ok_or("AIFF without sound data")?;
    let ch = u16::from_be_bytes([comm[0], comm[1]]);
    let bits = u16::from_be_bytes([comm[6], comm[7]]);
    let sr = ext80(&comm[8..18]).round() as u32;
    let compression = comm.get(18..22).unwrap_or(b"NONE");
    let little = match compression {
        b"NONE" | b"twos" => false,
        b"sowt" => true,
        c => return Err(format!("AIFC compression '{}' is not supported", String::from_utf8_lossy(c))),
    };
    let offset = u32::from_be_bytes(ssnd[0..4].try_into().unwrap()) as usize;
    let data = ssnd.get(8 + offset..).unwrap_or(&[]);
    let bps = (bits as usize).div_ceil(8);
    let pcm: Vec<u8> =
        if little { data.to_vec() } else { data.chunks_exact(bps).flat_map(|s| s.iter().rev().copied().collect::<Vec<_>>()).collect() };
    write_pcm(out, sr, ch, (bps * 8) as u16, &pcm)
}

/// Audio parameters: (sample rate, channels, bits).
type Pcm = (u32, u16, u16);

/// One sound track of an MXF file: its essence, by kind.
#[derive(Default)]
struct MxfTrack {
    /// BWF or SMPTE 382 AES3 sample bytes (interleaved little-endian).
    pcm: Vec<u8>,
    /// D-10 AES3 elements (SMPTE 331).
    d10: Vec<Vec<u8>>,
}

/// MXF sound essence → WAV. Reads:
/// - BWF elements (SMPTE 382, frame- or clip-wrapped): Avid OP-Atom and most files;
/// - AES3 elements of SMPTE 382 (frame- or clip-wrapped): the samples are stored as
///   in BWF (the AES3 channel status is descriptor metadata);
/// - D-10 / IMX AES3 elements (SMPTE 331/386): a 4-byte header, then 8 channel
///   slots per sample of 32-bit little-endian AES3 subframes, audio in bits 4–27.
///
/// A file can carry several sound tracks (OP1a: one element per track in each
/// content package, or one clip-wrapped element each). Elements are told apart
/// by their track number (the key's last 4 bytes); each track takes its
/// parameters from the descriptor linked to it (track number → track ID →
/// `LinkedTrackID`). The WAV has every track's channels, in track order, so
/// channel `k` of the AAF's file mob is the file's `k`-th channel.
pub(crate) fn mxf_to_wav(b: &[u8], aaf_params: Option<Pcm>, out: &Path) -> std::result::Result<(), String> {
    const ESSENCE: [u8; 12] = [0x06, 0x0e, 0x2b, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0d, 0x01, 0x03, 0x01];
    const SET: [u8; 14] = [0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0d, 0x01, 0x01, 0x01, 0x01, 0x01];
    let same = |k: &[u8], p: &[u8]| k.len() >= p.len() && k.iter().zip(p).enumerate().all(|(i, (a, b))| i == 7 || a == b);
    let mut tracks: BTreeMap<u32, MxfTrack> = BTreeMap::new();
    // descriptors: (linked track id, parameters); timeline tracks: track number → track id
    let mut descs: Vec<(Option<u32>, Option<Pcm>)> = vec![];
    let mut track_ids: HashMap<u32, u32> = HashMap::new();
    let mut other = false;
    let mut i = 0;
    while i + 17 <= b.len() {
        let k = &b[i..i + 16];
        let (len, hl) = match b[i + 16] {
            l if l < 0x80 => (l as usize, 1),
            l => {
                let n = (l & 0x7f) as usize;
                if n == 0 || n > 8 || i + 17 + n > b.len() {
                    return Err("damaged MXF (bad length)".into());
                }
                (b[i + 17..i + 17 + n].iter().fold(0usize, |a, &x| a << 8 | x as usize), 1 + n)
            }
        };
        let v0 = i + 16 + hl;
        let v = &b[v0.min(b.len())..(v0 + len).min(b.len())];
        let number = u32::from_be_bytes(k[12..16].try_into().unwrap());
        if same(k, &ESSENCE) && k[12] == 0x16 && matches!(k[14], 0x01..=0x04) {
            // BWF and AES3, frame- or clip-wrapped
            tracks.entry(number).or_default().pcm.extend_from_slice(v);
        } else if same(k, &ESSENCE) && k[12] == 0x06 && k[14] == 0x10 {
            tracks.entry(number).or_default().d10.push(v.to_vec());
        } else if same(k, &ESSENCE) && (k[12] == 0x16 || k[12] == 0x06) {
            other = true;
        } else if same(k, &SET) && matches!(k[14], 0x42 | 0x47 | 0x48 | 0x3b) {
            // local set: tag (2), length (2), value — big-endian
            let (mut sr, mut ch, mut bits, mut linked, mut id, mut num) = (0u32, 0u16, 0u16, None, None, None);
            let mut j = 0;
            while j + 4 <= v.len() {
                let tag = u16::from_be_bytes([v[j], v[j + 1]]);
                let l = u16::from_be_bytes([v[j + 2], v[j + 3]]) as usize;
                let d = &v[(j + 4).min(v.len())..(j + 4 + l).min(v.len())];
                let u = || u32::from_be_bytes(d.try_into().unwrap());
                match (tag, d.len()) {
                    (0x3d03, 8) => {
                        let n = i32::from_be_bytes(d[0..4].try_into().unwrap());
                        let den = i32::from_be_bytes(d[4..8].try_into().unwrap()).max(1);
                        sr = (n as f64 / den as f64).round() as u32;
                    }
                    (0x3d07, 4) => ch = u() as u16,
                    (0x3d01, 4) => bits = u() as u16,
                    (0x3006, 4) => linked = Some(u()),
                    (0x4801, 4) => id = Some(u()),
                    (0x4804, 4) => num = Some(u()),
                    _ => {}
                }
                j += 4 + l;
            }
            if k[14] == 0x3b {
                if let (Some(id), Some(num)) = (id, num) {
                    track_ids.insert(num, id);
                }
            } else {
                descs.push((linked, (sr > 0 && ch > 0 && bits > 0).then_some((sr, ch, bits))));
            }
        }
        i = v0 + len;
    }
    if tracks.is_empty() {
        return Err(if other { "MXF sound essence of an unsupported kind (not PCM or AES3)".into() } else { "no audio in the MXF".into() });
    }
    // parameters per track: its linked descriptor, else the only one there is, else the AAF's
    let known: Vec<Pcm> = descs.iter().filter_map(|d| d.1).collect();
    let single = tracks.len() == 1;
    let params = |number: u32| -> Option<Pcm> {
        let linked = track_ids.get(&number).and_then(|id| descs.iter().find(|d| d.0 == Some(*id))).and_then(|d| d.1);
        linked
            .or_else(|| (known.len() == 1 || known.windows(2).all(|w| w[0] == w[1])).then(|| known.first().copied()).flatten())
            .or(if single { aaf_params } else { None })
    };
    // track order: by track id where known, else by element key
    let mut order: Vec<u32> = tracks.keys().copied().collect();
    order.sort_by_key(|n| (track_ids.get(n).copied().unwrap_or(u32::MAX), *n));
    // every track as (channels, frames of left-justified 32-bit samples)
    let mut decoded: Vec<(usize, Vec<i32>, u16, u32)> = vec![];
    for n in order {
        let t = &tracks[&n];
        let (sr, ch, bits, data) = if !t.pcm.is_empty() {
            let (sr, ch, bits) = params(n).ok_or("MXF without audio parameters")?;
            if ch == 0 || !(8..=32).contains(&bits) {
                return Err(format!("MXF audio of {ch} channels at {bits} bits"));
            }
            let bps = (bits as usize).div_ceil(8);
            let s: Vec<i32> = t
                .pcm
                .chunks_exact(bps)
                .map(|c| c.iter().rev().fold(0u32, |a, &x| a << 8 | x as u32) as i32)
                .map(|v| v << (32 - 8 * bps))
                .collect();
            (sr, ch, bits, s)
        } else {
            let (sr, ch, bits) = params(n).unwrap_or((48000, 0, 24));
            // the channels in use: as described, else those the first element flags valid
            let ch = if ch > 0 { ch.min(8) } else { (t.d10[0].get(3).copied().unwrap_or(0xff).count_ones() as u16).max(1) };
            (sr, ch, if bits <= 16 { 16 } else { 24 }, d10_samples(&t.d10, ch as usize))
        };
        decoded.push((ch as usize, data, bits, sr));
    }
    let sr = decoded[0].3;
    if decoded.iter().any(|d| d.3 != sr) {
        return Err("MXF sound tracks at different sample rates".into());
    }
    // interleave all tracks' channels; a shorter track is padded with silence
    let channels: usize = decoded.iter().map(|d| d.0).sum();
    let frames = decoded.iter().map(|d| d.1.len() / d.0).max().unwrap_or(0);
    let bits = decoded.iter().map(|d| d.2).max().unwrap_or(24);
    let bps = (bits as usize).div_ceil(8).clamp(2, 4);
    let mut pcm = Vec::with_capacity(frames * channels * bps);
    for f in 0..frames {
        for (ch, data, ..) in &decoded {
            for c in 0..*ch {
                let v = data.get(f * ch + c).copied().unwrap_or(0);
                pcm.extend_from_slice(&v.to_le_bytes()[4 - bps..]);
            }
        }
    }
    if channels > u16::MAX as usize {
        return Err("too many channels".into());
    }
    write_pcm(out, sr, channels as u16, (bps * 8) as u16, &pcm)
}

/// D-10 AES3 elements (SMPTE 331) → interleaved left-justified 32-bit samples of
/// `ch` channels (24 bits of audio).
fn d10_samples(elements: &[Vec<u8>], ch: usize) -> Vec<i32> {
    let mut out = vec![];
    for e in elements {
        let Some(data) = e.get(4..) else { continue };
        // the header's sample count; the data holds at most what fits
        let n = (u16::from_le_bytes([e[1], e[2]]) as usize).min(data.len() / 32);
        for frame in data.chunks_exact(32).take(n) {
            for w in frame.chunks_exact(4).take(ch) {
                let w = u32::from_le_bytes(w.try_into().unwrap());
                out.push((((w >> 4) & 0xff_ffff) << 8) as i32);
            }
        }
    }
    out
}

// ---------------------------------------------------------------- import

/// A project track to make: one AAF track, or a stereo pair.
struct Planned {
    name: String,
    left: Lane,
    right: Option<Lane>,
}

pub(crate) fn amp_db(a: f64) -> f64 {
    if a <= 1e-5 {
        -96.0
    } else {
        (20.0 * a.log10()).clamp(-96.0, 12.0)
    }
}

pub(crate) fn strip_side(name: &str) -> Option<&str> {
    [" L", " R", ".L", ".R", "_L", "_R"].iter().find_map(|s| name.strip_suffix(s)).map(str::trim_end)
}

fn same_clips(a: &Lane, b: &Lane) -> bool {
    a.clips.len() == b.clips.len()
        && !a.clips.is_empty()
        && a.clips.iter().zip(&b.clips).all(|(x, y)| match (&x.src, &y.src) {
            (Ok(sx), Ok(sy)) => {
                x.at == y.at
                    && x.len == y.len
                    && x.fade_in == y.fade_in
                    && x.fade_out == y.fade_out
                    && (x.gain - y.gain).abs() < 1e-6
                    && x.muted == y.muted
                    && sx.file_mob == sy.file_mob
                    && sx.offset == sy.offset
                    && sx.channel == 1
                    && sy.channel == 2
            }
            _ => false,
        })
}

/// Import the AAF's composition into the project as new tracks.
pub fn import(s: &mut ProjectState, path: &Path, o: &AafImportOptions) -> Result<AafImportReport> {
    let content = Content::read(path)?;
    let a = &content.aaf;
    let comp = content.pick_composition(o.composition.as_deref())?;
    let comp_name = comp.string(0x4402).filter(|n| !n.is_empty()).unwrap_or_else(|| "AAF".into());
    let psr = s.project.sample_rate;
    let sr: Rate = (psr as i64, 1);
    let mut notes = Notes::default();

    // ---- placement: the composition's timecode
    let slots = a.children(&comp, 0x4403);
    let mut tc_start = None;
    for sl in &slots {
        let Some(seg) = a.child(sl, 0x4803) else { continue };
        let tc = if a.is(&seg, c::TIMECODE) {
            Some(seg)
        } else if a.is(&seg, c::SEQUENCE) {
            a.children(&seg, 0x1001).into_iter().find(|x| a.is(x, c::TIMECODE))
        } else {
            None
        };
        if let (Some(tc), Some(rate)) = (tc, Content::slot_rate(sl)) {
            let start = tc.i64(0x1501).unwrap_or(0);
            tc_start = Some(conv(start, rate, sr) - s.tb().origin_since_midnight());
            break;
        }
    }
    let base = match (o.at, tc_start) {
        (Some(at), _) => at,
        (None, Some(t)) => t,
        (None, None) => {
            notes.add("no timecode in the composition: placed at the project start", 1);
            0
        }
    };

    // ---- sound tracks → lanes
    let mut lanes = vec![];
    let mut events = vec![];
    for sl in &slots {
        let Some(seg) = a.child(sl, 0x4803) else { continue };
        if a.is(sl, c::EVENT_SLOT) {
            events.push((sl.clone(), seg));
            continue;
        }
        if !content.is_sound(&seg) {
            continue;
        }
        let Some(rate) = Content::slot_rate(sl) else { continue };
        let name = sl.string(0x4802).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("A{}", lanes.len() + 1));
        let slot = sl.u32(0x4801).unwrap_or(0);
        let mut w = Walk {
            lane: Lane { name, slot, ..Default::default() },
            rate,
            sr,
            pending_in: 0,
            zero_clip: false,
            inactive: false,
            notes: &mut notes,
        };
        let origin = sl.i64(0x4b02).unwrap_or(0);
        content.walk(&seg, -origin, 1.0, &mut w, 0);
        if !w.lane.clips.is_empty() {
            lanes.push(w.lane);
        }
    }

    // VC Sound Design's own description (the composition's "VCSD" comment)
    let vcsd: serde_json::Value = a
        .children(&comp, 0x4406)
        .iter()
        .find(|t| t.string(0x5001).as_deref() == Some("VCSD"))
        .and_then(|t| t.data(0x5003).map(<[u8]>::to_vec))
        .filter(|d| d.len() > 17 && hexs(&d[1..17]) == key::TYPE_STRING)
        .and_then(|d| serde_json::from_str(&utf16(&d[17..])).ok())
        .unwrap_or(serde_json::Value::Null);
    let meta_sr = vcsd["sample_rate"].as_f64().unwrap_or(psr as f64);
    let from_meta = |v: &serde_json::Value| v.as_f64().map(|x| (x * psr as f64 / meta_sr).round() as i64);
    let slot_meta = |slot: u32| vcsd["slots"][slot.to_string()].clone();
    // what our export silenced (kept muted on request) comes back muted, its levels intact;
    // other zero gains are silence
    // In other apps' AAFs (AAF has no mute), a zero gain is how a mute travels: the
    // clip or track comes in muted, its other gains kept as its levels.
    for lane in &mut lanes {
        let m = slot_meta(lane.slot);
        let described = m.is_object();
        if m["silenced"].as_bool() == Some(true) {
            lane.zero_spans.clear();
        } else if !described && !lane.zero_spans.is_empty() {
            lane.zero_spans.clear();
            lane.muted = true;
            notes.add("zero gains read as mutes (other gains kept as the levels)", 1);
        } else {
            let z = std::mem::take(&mut lane.zero_spans);
            lane.gains.extend(z);
        }
        let muted: Vec<(i64, i64)> =
            m["muted_clips"].as_array().into_iter().flatten().filter_map(|c| Some((from_meta(&c[0])?, from_meta(&c[1])?))).collect();
        for c in lane.clips.iter_mut().filter(|c| c.zeroed) {
            if muted.iter().any(|&(at, l)| (at - c.at).abs() <= 1 && (l - c.len).abs() <= 1) {
                c.muted = true;
            } else if !described {
                c.muted = true;
                notes.add("zero gains read as mutes (other gains kept as the levels)", 1);
            } else {
                c.gain = 0.0;
            }
        }
    }

    // ---- media for every clip
    let mut media = Media {
        content: &content,
        o,
        aaf_dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
        index: None,
        files: HashMap::new(),
        channels: HashMap::new(),
        resampled: HashMap::new(),
        written: 0,
    };
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    let mut file_channels: HashMap<Vec<u8>, u16> = HashMap::new();
    for l in &mut lanes {
        for clip in &mut l.clips {
            let Ok(src) = &clip.src else { continue };
            match media.file(&src.file_mob) {
                Ok((p, _)) => {
                    let ch = wav::read_info(&p).map(|i| i.channels).unwrap_or(1);
                    file_channels.insert(src.file_mob.clone(), ch);
                }
                Err(e) => clip.src = Err(e),
            }
        }
    }

    // ---- stereo pairs: channels 1 and 2 of the same stereo files, hard L/R or named L/R
    let mut planned = vec![];
    let mut i = 0;
    while i < lanes.len() {
        if i + 1 < lanes.len() {
            let (l, r) = (&lanes[i], &lanes[i + 1]);
            let stereo_files = l.clips.iter().all(|c| c.src.as_ref().is_ok_and(|s| file_channels.get(&s.file_mob) == Some(&2)));
            let (lo, hi) = l.extent();
            let keys: Vec<i64> =
                l.keys().into_iter().chain(r.keys()).chain(l.clips.iter().map(|c| c.at)).filter(|&t| t >= lo && t < hi).collect();
            let hard = !l.pans.is_empty() && keys.iter().all(|&t| l.pan(t) < 0.01 && r.pan(t) > 0.99);
            let named =
                l.pans.is_empty() && r.pans.is_empty() && strip_side(&l.name).is_some() && strip_side(&l.name) == strip_side(&r.name);
            if stereo_files && same_clips(l, r) && (hard || named) {
                let name = strip_side(&l.name).unwrap_or(&l.name).to_string();
                planned.push(Planned { name, left: lanes[i].clone(), right: Some(lanes[i + 1].clone()) });
                i += 2;
                continue;
            }
        }
        planned.push(Planned { name: lanes[i].name.clone(), left: lanes[i].clone(), right: None });
        i += 1;
    }

    // ---- tracks, clips and mix
    let folder = o.folder.clone().unwrap_or_else(|| format!("AAF/{}", comp_name.replace('/', " & ")));
    let mut report = AafImportReport { composition: comp_name.clone(), start_tc: s.tb().tc_at(base), ..Default::default() };
    let mut order = s.tracks.values().map(|t| t.order + 1).max().unwrap_or(0);
    let mut assets: HashMap<PathBuf, String> = HashMap::new();
    let license = License { name: "Editorial media (AAF)".into(), redistributable: true, source: "editorial".into(), ..Default::default() };
    // ---- buses: VC Sound Design's own description (the composition's "VCSD"
    // comment), restored per `o.buses`; otherwise they stay folded into the tracks
    let exported: Vec<serde_json::Value> = vcsd["buses"].as_array().cloned().unwrap_or_default();
    let track_bus_ids: Vec<String> = planned.iter().filter_map(|p| slot_meta(p.left.slot)["bus_id"].as_str().map(String::from)).collect();
    let track_buses: Vec<&str> = track_bus_ids.iter().map(String::as_str).collect();
    let comp_len = planned.iter().map(|p| p.left.extent().1).max().unwrap_or(0);
    let restore = crate::otio_import::restore_buses(s, &exported, &track_buses, o.buses, base, comp_len, &from_meta);
    if let Some(n) = &restore.note {
        notes.add(n.clone(), 1);
    }
    let bus_map = restore.map.unwrap_or_default();
    report.bus_levels = if bus_map.is_empty() { "fold".into() } else { "buses".into() };
    (report.buses_restored, report.buses_created, report.buses_matched) = (restore.restored, restore.created, restore.matched);

    for p in planned {
        let stereo = p.right.is_some();
        let lane = &p.left;
        let category = guess_category(&p.name);
        // restored buses: this track's own levels, routed to its bus
        let meta = slot_meta(lane.slot);
        let routed = meta["bus_id"].as_str().and_then(|b| bus_map.get(b)).cloned();
        // its fader, folded into clip gain on the way out (unless printed)
        let fader_in_clips =
            if routed.is_some() && meta["printed"].as_bool() != Some(true) { meta["gain_db"].as_f64().unwrap_or(0.0) } else { 0.0 };
        let bus_id = match (&routed, &category) {
            (Some(b), _) => b.clone(),
            (None, Category::Custom(_)) => {
                s.buses.values().find(|b| b.name == crate::mixer::MASTER_BUS).map(|b| b.id.clone()).unwrap_or_default()
            }
            (None, cat) => crate::builder::bus_for_category(s, cat),
        };
        let tid = new_id("trk");
        let mut clips = vec![];
        for rc in &lane.clips {
            let src = match &rc.src {
                Ok(x) => x,
                Err(e) => {
                    *missing.entry(e.clone()).or_default() += 1;
                    continue;
                }
            };
            let resolved = (|| -> std::result::Result<(String, i64), String> {
                let (file, _) = media.file(&src.file_mob)?;
                let fch = file_channels.get(&src.file_mob).copied().unwrap_or(1);
                let file = if !stereo && fch > 1 { media.channel(&file, &src.file_mob, src.channel)? } else { file };
                let fsr = wav::read_info(&file).map_err(|e| e.to_string())?.sample_rate;
                let file = media.at_rate(&file, psr)?;
                let id = match assets.get(&file) {
                    Some(id) => id.clone(),
                    None => {
                        let meta = AssetMeta { kind: AssetKind::Imported, license: Some(license.clone()), ..Default::default() };
                        let id = import_wav(s, &file, meta).map_err(|e| e.to_string())?;
                        assets.insert(file.clone(), id.clone());
                        id
                    }
                };
                Ok((id, conv(src.offset, (fsr as i64, 1), sr)))
            })();
            let (asset_id, offset) = match resolved {
                Ok(x) => x,
                Err(e) => {
                    *missing.entry(e).or_default() += 1;
                    continue;
                }
            };
            clips.push(ClipInstance {
                id: new_id("clp"),
                track_id: tid.clone(),
                cue_id: None,
                asset_id,
                timeline_start: base + rc.at,
                source_offset: offset,
                length: rc.len,
                gain_db: (amp_db(rc.gain) - fader_in_clips) as f32,
                fade_in: rc.fade_in.min(rc.len),
                fade_out: rc.fade_out.min(rc.len),
                fill: FillMode::OneShot,
                muted: rc.muted,
                take_id: None,
                performance_event_id: None,
                sub_index: None,
                is_placeholder: false,
                history: vec![],
            });
        }
        if clips.is_empty() {
            continue;
        }
        let mut track = Track {
            id: tid.clone(),
            name: p.name.clone(),
            scene_id: None,
            identity_id: None,
            category,
            bus_id,
            channels: if stereo { 2 } else { 1 },
            order,
            // our own export: its folder rebuilt under the import's (slot order kept)
            folder: match meta["folder"].as_str().map(crate::editing::normalize_folder).filter(|f| !f.is_empty()) {
                Some(sub) => crate::editing::normalize_folder(&format!("{folder}/{sub}")),
                None => folder.clone(),
            },
            gain_db: 0.0,
            pan: 0.0,
            // silenced on export: its own mute when its buses come back (a muted bus is
            // restored muted), else muted as it sounded
            mute: lane.muted || meta["silenced"].as_bool() == Some(true) && (routed.is_none() || meta["muted"].as_bool() == Some(true)),
            // the description's solo flag (monitoring state; other apps' AAFs have none)
            solo: meta["solo"].as_bool().unwrap_or(false),
            armed: false,
            frozen_asset_id: None,
            inserts: vec![],
            automation_mode: AutoMode::Read,
        };
        order += 1;
        // its inserts and their plugin automation (our description; AAF can't hold them).
        // A printed track's clip already went through them, so they stay off it.
        if meta["printed"].as_bool() == Some(true) {
            let n = meta["inserts"].as_array().map(|a| a.len()).unwrap_or(0);
            notes.add("inserts of printed tracks not rebuilt (their clip is the render through them)", n);
        } else {
            for i in meta["inserts"].as_array().into_iter().flatten() {
                let Ok(ins) = serde_json::from_value::<Insert>(i.clone()) else {
                    notes.add("insert effects that couldn't be read left out", 1);
                    continue;
                };
                let ins = match ins {
                    Insert::Plugin(mut p) => {
                        let old = std::mem::replace(&mut p.id, new_id("ins"));
                        for (param, pts) in meta["plugin_automation"][old.as_str()].as_object().into_iter().flatten() {
                            let Ok(param) = param.parse::<u32>() else { continue };
                            let mut any = false;
                            for pt in pts.as_array().into_iter().flatten() {
                                if let (Some(t), Some(v)) = (from_meta(&pt[0]), pt[1].as_f64()) {
                                    add_point(s, &p.id, AutomationParam::Plugin(param), base + t, (v as f32).clamp(0.0, 1.0));
                                    any = true;
                                }
                            }
                            report.automated_parameters += any as usize;
                        }
                        report.plugins += 1;
                        Insert::Plugin(p)
                    }
                    other => other,
                };
                report.inserts += 1;
                track.inserts.push(ins);
            }
        }

        if routed.is_some() {
            // the track's own fader, pan and lanes (the AAF's track gain and pan include the buses)
            track.gain_db = meta["gain_db"].as_f64().unwrap_or(0.0) as f32;
            track.pan = meta["pan"].as_f64().unwrap_or(0.0).clamp(-1.0, 1.0) as f32;
            let mut automated = false;
            for (key, param) in [("volume_db", AutomationParam::VolumeDb), ("pan_automation", AutomationParam::Pan)] {
                for pt in meta[key].as_array().into_iter().flatten() {
                    if let (Some(t), Some(v)) = (from_meta(&pt[0]), pt[1].as_f64()) {
                        add_point(s, &tid, param, base + t, crate::editing::clamp_automation(param, v as f32));
                        automated |= param == AutomationParam::VolumeDb;
                    }
                }
            }
            report.automated_tracks += automated as usize;
            report.panned_tracks += (track.pan.abs() > 0.001 || meta["pan_automation"].as_array().is_some_and(|a| !a.is_empty())) as usize;
            report.stereo_tracks += stereo as usize;
            report.clips += clips.len();
            s.tracks.insert(tid.clone(), track);
            for c in clips {
                s.clips.insert(c.id.clone(), c);
            }
            report.tracks.push(tid);
            continue;
        }
        // gain and pan as functions of time (samples from the composition start)
        let right = p.right.as_ref();
        let gain = |t: i64| match right {
            Some(r) => lane.gain(t).max(r.gain(t)),
            None => lane.gain(t),
        };
        let pan = |t: i64| match right {
            // the mixer's balance: centre = unity both sides, the far side dips
            Some(r) => {
                let (gl, gr) = (lane.gain(t), r.gain(t));
                if gl <= 1e-9 && gr <= 1e-9 {
                    0.0
                } else if gl >= gr {
                    gr / gl - 1.0
                } else {
                    1.0 - gl / gr
                }
            }
            None => lane.pan(t) * 2.0 - 1.0,
        };
        let mut keys = lane.keys();
        if let Some(r) = right {
            keys.extend(r.keys());
        }
        // the lanes only matter where the clips play
        let (start, end) = lane.extent();
        let vol = simplify(&fit(&keys, start, end, &|t| amp_db(gain(t)), &|lin, v| (lin - v).abs() > 0.1), 0.02);
        let pans = simplify(&fit(&keys, start, end, &|t| pan(t).clamp(-1.0, 1.0), &|lin, v| (lin - v).abs() > 0.002), 0.0005);
        let flat = |p: &[(i64, f64)], tol: f64| p.iter().all(|x| (x.1 - p[0].1).abs() < tol);
        let mut automated = false;
        if flat(&vol, 0.01) {
            track.gain_db = vol[0].1 as f32;
        } else {
            automated = true;
            for (t, v) in &vol {
                let v = crate::editing::clamp_automation(AutomationParam::VolumeDb, *v as f32);
                add_point(s, &tid, AutomationParam::VolumeDb, base + t, v);
            }
        }
        if flat(&pans, 0.002) {
            track.pan = pans[0].1 as f32;
        } else {
            for (t, v) in &pans {
                add_point(s, &tid, AutomationParam::Pan, base + t, *v as f32);
            }
        }
        report.automated_tracks += automated as usize;
        report.panned_tracks += (!flat(&pans, 0.002) || track.pan.abs() > 0.001) as usize;
        report.stereo_tracks += stereo as usize;
        report.clips += clips.len();
        s.tracks.insert(tid.clone(), track);
        for c in clips {
            s.clips.insert(c.id.clone(), c);
        }
        report.tracks.push(tid);
    }
    report.media_linked = media.files.values().filter(|r| matches!(r, Ok((_, false)))).count();
    report.media_written = media.written;
    report.missing_media = missing.into_iter().map(|(k, n)| format!("{k} — {n} clip{}", if n == 1 { "" } else { "s" })).collect();

    // ---- markers → spotting markers (inside scenes)
    for (sl, seg) in events {
        let rate = Content::slot_rate(&sl).unwrap_or(sr);
        let items = if a.is(&seg, c::SEQUENCE) { a.children(&seg, 0x1001) } else { vec![seg] };
        for m in items.iter().filter(|m| a.is(m, c::COMMENT_MARKER)) {
            let at = base + conv(m.i64(0x0601).unwrap_or(0), rate, sr);
            let len = conv(m.len(), rate, sr);
            let label = m.string(0x0602).unwrap_or_default();
            if s.scene_at(at).is_none() {
                notes.add("markers outside every scene left out (add scenes, then import again)", 1);
                continue;
            }
            let lane = marker_lane(s, "AAF markers", "aaf");
            let nm = crate::spotting::NewMarker {
                lane_id: lane,
                scene_id: None,
                kind: if len > 0 { EventType::Range } else { EventType::Point },
                start: at,
                end: at + len,
                label: if label.trim().is_empty() { "AAF marker".into() } else { label },
                identity_id: None,
                sub_events: vec![],
                origin: TimingOrigin::Imported,
            };
            if crate::spotting::add_marker(s, nm).is_ok() {
                report.markers += 1;
            }
        }
    }
    report.soloed_tracks = report.tracks.iter().filter(|t| s.tracks[t.as_str()].solo).count();
    let others: Vec<&Track> = s.tracks.values().filter(|t| !report.tracks.contains(&t.id)).collect();
    if report.soloed_tracks > 0 && !others.is_empty() && !others.iter().any(|t| t.solo) {
        notes.add(
            "solo restored on imported tracks: while it's on, the project's other tracks are silent (unsolo to hear them)",
            report.soloed_tracks,
        );
    }
    report.notes = notes.list();
    if report.tracks.is_empty() && report.markers == 0 {
        let why = if report.missing_media.is_empty() { String::new() } else { format!(": {}", report.missing_media.join("; ")) };
        return Err(invalid(format!("nothing to import from '{comp_name}'{why}")));
    }
    Ok(report)
}

/// Drop breakpoints that lie (within `tol`) on the line between their
/// neighbours (Ramer–Douglas–Peucker on the value).
pub(crate) fn simplify(p: &[(i64, f64)], tol: f64) -> Vec<(i64, f64)> {
    fn keep(p: &[(i64, f64)], a: usize, b: usize, tol: f64, out: &mut Vec<bool>) {
        if b <= a + 1 {
            return;
        }
        let (pa, pb) = (p[a], p[b]);
        let line = |t: i64| pa.1 + (pb.1 - pa.1) * (t - pa.0) as f64 / (pb.0 - pa.0).max(1) as f64;
        let (i, d) = (a + 1..b).map(|i| (i, (p[i].1 - line(p[i].0)).abs())).fold((a, 0.0), |m, x| if x.1 > m.1 { x } else { m });
        if d > tol {
            out[i] = true;
            keep(p, a, i, tol, out);
            keep(p, i, b, tol, out);
        }
    }
    if p.len() < 3 {
        return p.to_vec();
    }
    let mut k = vec![false; p.len()];
    k[0] = true;
    k[p.len() - 1] = true;
    keep(p, 0, p.len() - 1, tol, &mut k);
    p.iter().zip(k).filter(|x| x.1).map(|x| *x.0).collect()
}

pub(crate) fn add_point(s: &mut ProjectState, owner: &str, param: AutomationParam, sample: i64, value: f32) {
    let id = new_id("aut");
    s.automation.insert(id.clone(), AutomationPoint { id, owner_id: owner.into(), param, sample, value });
}

/// The marker lane named `name` (made when missing) for imported markers.
pub(crate) fn marker_lane(s: &mut ProjectState, name: &str, tag: &str) -> String {
    if let Some(l) = s.lanes.values().find(|l| l.name == name) {
        return l.id.clone();
    }
    let order = s.lanes.values().map(|l| l.order + 1).max().unwrap_or(0);
    crate::spotting::add_lane(s, name, Category::Custom(tag.into()), "#8a8f98", order)
}

/// A category from a track name's first word ("FX 1", "DIA • Greg", "BG L"…).
pub(crate) fn guess_category(name: &str) -> Category {
    for word in name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).take(2) {
        let w = word.trim_end_matches(|c: char| c.is_ascii_digit());
        let c = Category::parse(w);
        if !matches!(c, Category::Custom(_)) {
            return c;
        }
    }
    Category::Custom("editorial".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indirect_values_read_rationals_and_integers() {
        let ind = |ty: &str, v: &[u8]| {
            let mut d = vec![0x4c];
            d.extend(hex_bytes(ty));
            d.extend(v);
            indirect(&d)
        };
        assert_eq!(ind(key::TYPE_RATIONAL, &[3, 0, 0, 0, 4, 0, 0, 0]), Some(0.75));
        assert_eq!(ind("0007010100000000060e2b3401040101", &(-5i32).to_le_bytes()), Some(-5.0), "aafInt32");
        assert_eq!(ind("0001010100000000060e2b3401040101", &[200]), Some(200.0), "aafUInt8");
        assert_eq!(ind("0006010100000000060e2b3401040101", &(-2i16).to_le_bytes()), Some(-2.0), "aafInt16");
        assert_eq!(ind("0002100100000000060e2b3401040101", &[0, 0]), None, "a string isn't a number");
    }

    fn hex_bytes(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn rate_conversion_rounds_to_nearest() {
        assert_eq!(conv(25, (25, 1), (48000, 1)), 48000);
        assert_eq!(conv(1, (24000, 1001), (48000, 1)), 2002);
        assert_eq!(conv(1, (48000, 1), (25, 1)), 0);
        assert_eq!(conv(1000, (48000, 1), (25, 1)), 1);
        assert_eq!(conv(-1920, (48000, 1), (25, 1)), -1);
        assert_eq!(conv(-1000, (48000, 1), (25, 1)), -1);
        assert_eq!(conv(-900, (48000, 1), (25, 1)), 0);
    }

    #[test]
    fn file_urls_decode() {
        assert_eq!(decode_url("file:///Volumes/Media/Audio%20Files/a.wav"), "/Volumes/Media/Audio Files/a.wav");
        assert_eq!(decode_url("file:///C:/Avid%20MediaFiles/MXF/1/x.mxf"), "C:/Avid MediaFiles/MXF/1/x.mxf");
        assert_eq!(decode_url("file://localhost/tmp/a.wav"), "/tmp/a.wav");
        assert_eq!(decode_url("file:///tmp/100%"), "/tmp/100%");
    }

    #[test]
    fn categories_from_track_names() {
        assert_eq!(guess_category("FX • Glass smash"), Category::HardFx);
        assert_eq!(guess_category("DIA 1"), Category::Dialogue);
        assert_eq!(guess_category("BG L"), Category::Background);
        assert_eq!(guess_category("A1"), Category::Custom("editorial".into()));
    }

    #[test]
    fn aiff_big_endian_becomes_wav() {
        let dir = std::env::temp_dir().join(format!("vcsd_aiff_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 2 frames, mono, 16-bit, 48 kHz
        let mut comm = vec![0, 1, 0, 0, 0, 2, 0, 16];
        comm.extend([0x40, 0x0e, 0xbb, 0x80, 0, 0, 0, 0, 0, 0]); // 48000 as 80-bit extended
        let ssnd = [0u8, 0, 0, 0, 0, 0, 0, 0, 0x12, 0x34, 0xff, 0xfe];
        let mut body = b"AIFF".to_vec();
        for (id, d) in [(b"COMM", &comm[..]), (b"SSND", &ssnd[..])] {
            body.extend(id);
            body.extend((d.len() as u32).to_be_bytes());
            body.extend(d);
        }
        let mut f = b"FORM".to_vec();
        f.extend((body.len() as u32).to_be_bytes());
        f.extend(body);
        let out = dir.join("a.wav");
        aiff_to_wav(&f, &out).unwrap();
        let (info, raw) = wav::read_raw(&out).unwrap();
        assert_eq!((info.sample_rate, info.channels, info.frames), (48000, 1, 2));
        assert_eq!(raw, vec![0x34, 0x12, 0xfe, 0xff]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mxf_pcm_essence_becomes_wav() {
        let dir = std::env::temp_dir().join(format!("vcsd_mxf_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let klv = |k: &[u8], v: &[u8]| {
            let mut o = k.to_vec();
            o.extend([0x83, (v.len() >> 16) as u8, (v.len() >> 8) as u8, v.len() as u8]);
            o.extend(v);
            o
        };
        // WaveAudioDescriptor: 48 kHz, 1 channel, 24-bit (version byte 7 differs on purpose)
        let mut desc = vec![];
        for (tag, val) in [
            (0x3d03u16, [&48000i32.to_be_bytes()[..], &1i32.to_be_bytes()[..]].concat()),
            (0x3d07, 1u32.to_be_bytes().to_vec()),
            (0x3d01, 24u32.to_be_bytes().to_vec()),
        ] {
            desc.extend(tag.to_be_bytes());
            desc.extend((val.len() as u16).to_be_bytes());
            desc.extend(val);
        }
        let mut f = klv(&[0x06, 0x0e, 0x2b, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0d, 0x01, 0x02, 0x01, 0x01, 0x02, 0x04, 0x00], &[0; 20]);
        f.extend(klv(&[0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x05, 0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, 0x48, 0x00], &desc));
        let pcm: Vec<u8> = (0..30u8).collect();
        f.extend(klv(&[0x06, 0x0e, 0x2b, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0d, 0x01, 0x03, 0x01, 0x16, 0x01, 0x02, 0x01], &pcm));
        let out = dir.join("m.wav");
        mxf_to_wav(&f, None, &out).unwrap();
        let (info, raw) = wav::read_raw(&out).unwrap();
        assert_eq!((info.sample_rate, info.channels, info.frames), (48000, 1, 10));
        assert_eq!(raw, pcm);

        // SMPTE 382 AES3, clip-wrapped (element type 04): samples stored as in BWF
        let mut g = klv(&[0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, 0x47, 0x00], &desc);
        g.extend(klv(&[0x06, 0x0e, 0x2b, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0d, 0x01, 0x03, 0x01, 0x16, 0x01, 0x04, 0x01], &pcm));
        mxf_to_wav(&g, None, &out).unwrap();
        assert_eq!(wav::read_raw(&out).unwrap().1, pcm);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// One D-10 AES3 element: header, then per sample 8 subframes (audio in bits 4–27).
    fn d10_element(samples: &[[i32; 8]]) -> Vec<u8> {
        let mut e = vec![0x80];
        e.extend((samples.len() as u16).to_le_bytes());
        e.push(0x0f); // channels 1–4 valid
        for s in samples {
            for (c, v) in s.iter().enumerate() {
                // V/U/C/P bits and the channel number around the sample, as a recorder writes them
                let w = (((*v as u32) & 0xff_ffff) << 4) | (c as u32 & 0x7) | 0xf000_0000;
                e.extend(w.to_le_bytes());
            }
        }
        e
    }

    #[test]
    fn mxf_d10_aes3_essence_becomes_wav() {
        let dir = std::env::temp_dir().join(format!("vcsd_d10_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let klv = |k: &[u8], v: &[u8]| {
            let mut o = k.to_vec();
            o.extend([0x83, (v.len() >> 16) as u8, (v.len() >> 8) as u8, v.len() as u8]);
            o.extend(v);
            o
        };
        let key = [0x06, 0x0e, 0x2b, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0d, 0x01, 0x03, 0x01, 0x06, 0x01, 0x10, 0x00];
        let pic = [0x06, 0x0e, 0x2b, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0d, 0x01, 0x03, 0x01, 0x05, 0x01, 0x01, 0x00];
        let s = |k: i32| [k, -k, 0x7f_ffff - k, -0x80_0000 + k, 9, 9, 9, 9];
        // two frames (content packages), each a picture element and a sound element
        let mut f = vec![];
        for frame in [vec![s(1), s(2)], vec![s(3)]] {
            f.extend(klv(&pic, &[0; 64]));
            f.extend(klv(&key, &d10_element(&frame)));
        }
        // a generic sound descriptor: 48 kHz, 4 channels, 24-bit
        let mut desc = vec![];
        for (tag, val) in [
            (0x3d03u16, [&48000i32.to_be_bytes()[..], &1i32.to_be_bytes()[..]].concat()),
            (0x3d07, 4u32.to_be_bytes().to_vec()),
            (0x3d01, 24u32.to_be_bytes().to_vec()),
        ] {
            desc.extend(tag.to_be_bytes());
            desc.extend((val.len() as u16).to_be_bytes());
            desc.extend(val);
        }
        let mut file = klv(&[0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, 0x42, 0x00], &desc);
        file.extend(f);
        let out = dir.join("d10.wav");
        mxf_to_wav(&file, None, &out).unwrap();
        let (info, buf) = wav::read(&out).unwrap();
        assert_eq!((info.sample_rate, info.channels, info.frames), (48000, 4, 3));
        let (_, raw) = wav::read_raw(&out).unwrap();
        let got: Vec<i32> = raw.chunks_exact(3).map(|c| ((c[2] as i32) << 24 | (c[1] as i32) << 16 | (c[0] as i32) << 8) >> 8).collect();
        let want: Vec<i32> = [s(1), s(2), s(3)].iter().flat_map(|x| x[0..4].to_vec()).collect();
        assert_eq!(got, want, "24-bit samples exact, channel bits and V/U/C/P dropped");
        assert!(buf.peak() > 0.99);

        // 16-bit: the top 16 bits of each 24-bit slot; no descriptor → channels from the valid flags
        let e = d10_element(&[[0x1234 << 8, -(0x10 << 8), 0, 0, 0, 0, 0, 0]]);
        mxf_to_wav(&klv(&key, &e), Some((48000, 2, 16)), &out).unwrap();
        let (info, raw) = wav::read_raw(&out).unwrap();
        assert_eq!((info.channels, info.format.bits()), (2, 16));
        assert_eq!(raw, [0x34, 0x12, 0xf0, 0xff]);
        mxf_to_wav(&klv(&key, &e), None, &out).unwrap();
        assert_eq!(wav::read_info(&out).unwrap().channels, 4, "four valid flags");
        std::fs::remove_dir_all(&dir).ok();
    }
    fn local_set(items: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut v = vec![];
        for (tag, val) in items {
            v.extend(tag.to_be_bytes());
            v.extend((val.len() as u16).to_be_bytes());
            v.extend(val);
        }
        v
    }

    fn klv(k: &[u8], v: &[u8]) -> Vec<u8> {
        let mut o = k.to_vec();
        o.extend([0x83, (v.len() >> 16) as u8, (v.len() >> 8) as u8, v.len() as u8]);
        o.extend(v);
        o
    }

    fn set_key(kind: u8) -> [u8; 16] {
        [0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, kind, 0x00]
    }

    fn sound_key(number: u32) -> [u8; 16] {
        let mut k = [0x06, 0x0e, 0x2b, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0d, 0x01, 0x03, 0x01, 0, 0, 0, 0];
        k[12..16].copy_from_slice(&number.to_be_bytes());
        k
    }

    fn sound_desc(linked: Option<u32>, ch: u32, bits: u32) -> Vec<u8> {
        let mut items = vec![
            (0x3d03u16, [&48000i32.to_be_bytes()[..], &1i32.to_be_bytes()[..]].concat()),
            (0x3d07, ch.to_be_bytes().to_vec()),
            (0x3d01, bits.to_be_bytes().to_vec()),
        ];
        if let Some(l) = linked {
            items.push((0x3006, l.to_be_bytes().to_vec()));
        }
        klv(&set_key(0x48), &local_set(&items))
    }

    #[test]
    fn multitrack_mxf_becomes_one_multichannel_wav() {
        let dir = std::env::temp_dir().join(format!("vcsd_mxfmt_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // OP1a, frame-wrapped: track 3 (mono 24-bit, element 2) and track 2 (stereo 16-bit,
        // element 1); the descriptors come in the opposite order to the elements
        let (mono, stereo) = (0x1602_0102u32, 0x1602_0101u32);
        let track = |id: u32, num: u32| {
            klv(&set_key(0x3b), &local_set(&[(0x4801, id.to_be_bytes().to_vec()), (0x4804, num.to_be_bytes().to_vec())]))
        };
        let mut f = track(2, stereo);
        f.extend(track(3, mono));
        f.extend(sound_desc(Some(3), 1, 24));
        f.extend(sound_desc(Some(2), 2, 16));
        let pic = [0x06, 0x0e, 0x2b, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0d, 0x01, 0x03, 0x01, 0x15, 0x01, 0x05, 0x01];
        // two content packages of 2 frames: mono samples m, stereo (l, r)
        let m = |k: i32| k * 1000;
        for p in 0..2 {
            f.extend(klv(&pic, &[0; 16]));
            let mut mb = vec![];
            let mut sb = vec![];
            for j in 0..2 {
                let k = p * 2 + j;
                mb.extend(&m(k).to_le_bytes()[0..3]);
                sb.extend((k as i16 * 10).to_le_bytes());
                sb.extend((-(k as i16) * 10).to_le_bytes());
            }
            f.extend(klv(&sound_key(mono), &mb));
            f.extend(klv(&sound_key(stereo), &sb));
        }
        let out = dir.join("op1a.wav");
        mxf_to_wav(&f, None, &out).unwrap();
        let (info, raw) = wav::read_raw(&out).unwrap();
        assert_eq!((info.channels, info.frames, info.format.bits()), (3, 4, 24), "2 + 1 channels at the widest depth");
        let got: Vec<i32> = raw.chunks_exact(3).map(|c| ((c[2] as i32) << 24 | (c[1] as i32) << 16 | (c[0] as i32) << 8) >> 8).collect();
        // track 2 (stereo) first, by track id; 16-bit samples shifted up to 24
        let want: Vec<i32> = (0..4).flat_map(|k| [(k * 10) << 8, (-k * 10) << 8, m(k)]).collect();
        assert_eq!(got, want);

        // clip-wrapped, one unlinked descriptor for both mono tracks, no track sets: element order
        let mut g = sound_desc(None, 1, 16);
        g.extend(klv(&sound_key(0x1602_0202), &[3, 0, 4, 0]));
        g.extend(klv(&sound_key(0x1602_0201), &[1, 0, 2, 0, 9, 9]));
        mxf_to_wav(&g, None, &out).unwrap();
        let (info, raw) = wav::read_raw(&out).unwrap();
        assert_eq!((info.channels, info.frames), (2, 3), "the shorter track padded");
        assert_eq!(raw, [1, 0, 3, 0, 2, 0, 4, 0, 9, 9, 0, 0]);

        // tracks with different parameters and nothing linking them can't be guessed
        let mut h = sound_desc(None, 1, 16);
        h.extend(sound_desc(None, 2, 24));
        h.extend(klv(&sound_key(0x1602_0201), &[0; 4]));
        h.extend(klv(&sound_key(0x1602_0202), &[0; 6]));
        assert!(mxf_to_wav(&h, None, &out).unwrap_err().contains("parameters"));
        std::fs::remove_dir_all(&dir).ok();
    }
}

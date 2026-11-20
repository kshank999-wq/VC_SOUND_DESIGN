//! Audio Unit hosting (macOS), through the AudioUnit v2 API — which also
//! loads most v3 (app extension) effects.
//!
//! Audio is pulled: `AudioUnitRender` calls our render callback for input,
//! which hands over the block we are processing. State is the unit's
//! `ClassInfo` property list, stored as binary plist data.
#![allow(non_upper_case_globals, non_snake_case, clippy::missing_safety_doc)]

use crate::{join_out, split_in, Instance, ParamInfo, PluginInfo, Setup};
use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering::*};
use std::sync::Mutex;

type OSStatus = i32;
type AudioComponent = *mut c_void;
type AudioUnit = *mut c_void;
type CFTypeRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AudioComponentDescription {
    componentType: u32,
    componentSubType: u32,
    componentManufacturer: u32,
    componentFlags: u32,
    componentFlagsMask: u32,
}

#[repr(C)]
struct AudioStreamBasicDescription {
    mSampleRate: f64,
    mFormatID: u32,
    mFormatFlags: u32,
    mBytesPerPacket: u32,
    mFramesPerPacket: u32,
    mBytesPerFrame: u32,
    mChannelsPerFrame: u32,
    mBitsPerChannel: u32,
    mReserved: u32,
}

#[repr(C)]
#[derive(Default)]
struct SMPTETime {
    mSubframes: i16,
    mSubframeDivisor: i16,
    mCounter: u32,
    mType: u32,
    mFlags: u32,
    mHours: i16,
    mMinutes: i16,
    mSeconds: i16,
    mFrames: i16,
}

#[repr(C)]
#[derive(Default)]
struct AudioTimeStamp {
    mSampleTime: f64,
    mHostTime: u64,
    mRateScalar: f64,
    mWordClockTime: u64,
    mSMPTETime: SMPTETime,
    mFlags: u32,
    mReserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioBuffer {
    mNumberChannels: u32,
    mDataByteSize: u32,
    mData: *mut c_void,
}

const MAX_CH: usize = 8;

#[repr(C)]
struct AudioBufferList {
    mNumberBuffers: u32,
    mBuffers: [AudioBuffer; MAX_CH],
}

type RenderCallback = unsafe extern "C" fn(*mut c_void, *mut u32, *const AudioTimeStamp, u32, u32, *mut AudioBufferList) -> OSStatus;

#[repr(C)]
struct AURenderCallbackStruct {
    inputProc: RenderCallback,
    inputProcRefCon: *mut c_void,
}

#[repr(C)]
struct AudioUnitParameterInfo {
    name: [c_char; 52],
    unitName: CFTypeRef,
    clumpID: u32,
    cfNameString: CFTypeRef,
    unit: u32,
    minValue: f32,
    maxValue: f32,
    defaultValue: f32,
    flags: u32,
}

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioComponentFindNext(c: AudioComponent, d: *const AudioComponentDescription) -> AudioComponent;
    fn AudioComponentGetDescription(c: AudioComponent, d: *mut AudioComponentDescription) -> OSStatus;
    fn AudioComponentCopyName(c: AudioComponent, name: *mut CFTypeRef) -> OSStatus;
    fn AudioComponentGetVersion(c: AudioComponent, v: *mut u32) -> OSStatus;
    fn AudioComponentInstanceNew(c: AudioComponent, u: *mut AudioUnit) -> OSStatus;
    fn AudioComponentInstanceDispose(u: AudioUnit) -> OSStatus;
    fn AudioUnitInitialize(u: AudioUnit) -> OSStatus;
    fn AudioUnitUninitialize(u: AudioUnit) -> OSStatus;
    fn AudioUnitSetProperty(u: AudioUnit, id: u32, scope: u32, el: u32, data: *const c_void, size: u32) -> OSStatus;
    fn AudioUnitGetProperty(u: AudioUnit, id: u32, scope: u32, el: u32, data: *mut c_void, size: *mut u32) -> OSStatus;
    fn AudioUnitGetPropertyInfo(u: AudioUnit, id: u32, scope: u32, el: u32, size: *mut u32, writable: *mut u8) -> OSStatus;
    fn AudioUnitRender(
        u: AudioUnit,
        flags: *mut u32,
        ts: *const AudioTimeStamp,
        bus: u32,
        frames: u32,
        data: *mut AudioBufferList,
    ) -> OSStatus;
    fn AudioUnitGetParameter(u: AudioUnit, id: u32, scope: u32, el: u32, v: *mut f32) -> OSStatus;
    fn AudioUnitSetParameter(u: AudioUnit, id: u32, scope: u32, el: u32, v: f32, offset: u32) -> OSStatus;
    fn AudioUnitReset(u: AudioUnit, scope: u32, el: u32) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(p: CFTypeRef);
    fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, size: isize, enc: u32) -> u8;
    fn CFPropertyListCreateData(alloc: CFTypeRef, plist: CFTypeRef, format: isize, opts: usize, err: *mut CFTypeRef) -> CFTypeRef;
    fn CFPropertyListCreateWithData(alloc: CFTypeRef, data: CFTypeRef, opts: usize, format: *mut isize, err: *mut CFTypeRef) -> CFTypeRef;
    fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, len: isize) -> CFTypeRef;
    fn CFDataGetLength(d: CFTypeRef) -> isize;
    fn CFDataGetBytePtr(d: CFTypeRef) -> *const u8;
}

const kCFStringEncodingUTF8: u32 = 0x0800_0100;
const kCFPropertyListBinaryFormat_v1_0: isize = 200;

const PROP_CLASS_INFO: u32 = 0;
const PROP_PARAMETER_LIST: u32 = 3;
const PROP_PARAMETER_INFO: u32 = 4;
const PROP_STREAM_FORMAT: u32 = 8;
const PROP_LATENCY: u32 = 12;
const PROP_MAX_FRAMES: u32 = 14;
const PROP_SET_RENDER_CALLBACK: u32 = 23;
const PROP_OFFLINE_RENDER: u32 = 37;
const SCOPE_GLOBAL: u32 = 0;
const SCOPE_INPUT: u32 = 1;
const SCOPE_OUTPUT: u32 = 2;
const FLAG_HAS_CFNAME: u32 = 1 << 27;
const FLAG_WRITABLE: u32 = 1 << 31;

fn fourcc(s: &str) -> u32 {
    s.bytes().take(4).fold(0, |a, b| (a << 8) | b as u32)
}
fn fourcc_str(v: u32) -> String {
    v.to_be_bytes().iter().map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' }).collect()
}

unsafe fn cfstring(s: CFTypeRef) -> String {
    if s.is_null() {
        return String::new();
    }
    let mut buf = [0 as c_char; 512];
    if CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, kCFStringEncodingUTF8) == 0 {
        return String::new();
    }
    std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
}

fn desc_of(id: &str) -> Result<AudioComponentDescription, String> {
    let p: Vec<&str> = id.strip_prefix("au:").unwrap_or(id).split(':').collect();
    if p.len() != 3 {
        return Err(format!("bad Audio Unit id '{id}'"));
    }
    Ok(AudioComponentDescription {
        componentType: fourcc(p[0]),
        componentSubType: fourcc(p[1]),
        componentManufacturer: fourcc(p[2]),
        ..Default::default()
    })
}

/// Installed effect Audio Units (no plugin code is loaded).
pub fn list() -> Vec<PluginInfo> {
    let mut out = vec![];
    for ty in ["aufx", "aumf"] {
        let want = AudioComponentDescription { componentType: fourcc(ty), ..Default::default() };
        let mut c: AudioComponent = ptr::null_mut();
        unsafe {
            loop {
                c = AudioComponentFindNext(c, &want);
                if c.is_null() {
                    break;
                }
                let mut d = AudioComponentDescription::default();
                if AudioComponentGetDescription(c, &mut d) != 0 {
                    continue;
                }
                let mut name: CFTypeRef = ptr::null();
                AudioComponentCopyName(c, &mut name);
                let full = cfstring(name);
                if !name.is_null() {
                    CFRelease(name);
                }
                let (vendor, title) = match full.split_once(": ") {
                    Some((v, n)) => (v.to_string(), n.to_string()),
                    None => (String::new(), full.clone()),
                };
                let mut ver = 0u32;
                AudioComponentGetVersion(c, &mut ver);
                out.push(PluginInfo {
                    id: format!(
                        "au:{}:{}:{}",
                        fourcc_str(d.componentType),
                        fourcc_str(d.componentSubType),
                        fourcc_str(d.componentManufacturer)
                    ),
                    format: "au".into(),
                    name: title,
                    vendor,
                    version: format!("{}.{}.{}", ver >> 16, (ver >> 8) & 0xff, ver & 0xff),
                    category: ty.into(),
                    path: String::new(),
                });
            }
        }
    }
    out
}

/// What the render callback reads: the input block for the current render.
struct Feed {
    ins: Vec<Vec<f32>>,
    frames: usize,
}

struct Proc {
    next_pos: Option<i64>,
    sample_time: f64,
    outs: Vec<Vec<f32>>,
}

pub struct AuInstance {
    info: PluginInfo,
    setup: Setup,
    unit: AudioUnit,
    in_ch: usize,
    out_ch: usize,
    /// Boxed so its address (the callback's refcon) never moves.
    feed: Box<Mutex<Feed>>,
    proc: Mutex<Proc>,
    pending: Mutex<Vec<(u32, f32)>>,
    dirty: AtomicBool,
}

unsafe impl Send for AuInstance {}
unsafe impl Sync for AuInstance {}

unsafe extern "C" fn input_cb(
    refcon: *mut c_void,
    _f: *mut u32,
    _ts: *const AudioTimeStamp,
    _bus: u32,
    frames: u32,
    data: *mut AudioBufferList,
) -> OSStatus {
    let feed = &*(refcon as *const Mutex<Feed>);
    let Ok(f) = feed.try_lock() else { return -1 };
    let abl = &mut *data;
    for b in 0..(abl.mNumberBuffers as usize).min(MAX_CH) {
        let dst = abl.mBuffers[b].mData as *mut f32;
        if dst.is_null() {
            continue;
        }
        let n = (frames as usize).min(f.frames);
        match f.ins.get(b).or_else(|| f.ins.last()) {
            Some(src) => ptr::copy_nonoverlapping(src.as_ptr(), dst, n),
            None => ptr::write_bytes(dst, 0, n),
        }
    }
    0
}

fn asbd(rate: f64, ch: usize) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: rate,
        mFormatID: fourcc("lpcm"),
        mFormatFlags: 1 | 8 | 32, // float, packed, non-interleaved
        mBytesPerPacket: 4,
        mFramesPerPacket: 1,
        mBytesPerFrame: 4,
        mChannelsPerFrame: ch as u32,
        mBitsPerChannel: 32,
        mReserved: 0,
    }
}

impl AuInstance {
    pub fn new(info: &PluginInfo, setup: Setup) -> Result<AuInstance, String> {
        let want = desc_of(&info.id)?;
        unsafe {
            let c = AudioComponentFindNext(ptr::null_mut(), &want);
            if c.is_null() {
                return Err(format!("{} is not installed", info.name));
            }
            let mut unit: AudioUnit = ptr::null_mut();
            if AudioComponentInstanceNew(c, &mut unit) != 0 || unit.is_null() {
                return Err(format!("{} could not be created", info.name));
            }
            let fail = |m: &str| {
                AudioComponentInstanceDispose(unit);
                Err(format!("{}: {m}", info.name))
            };
            // try our channel count; fall back to stereo, then to what the unit has
            let mut ch = setup.channels.clamp(1, 2);
            let set = |c: usize| {
                let f = asbd(setup.sample_rate, c);
                let sz = std::mem::size_of::<AudioStreamBasicDescription>() as u32;
                AudioUnitSetProperty(unit, PROP_STREAM_FORMAT, SCOPE_INPUT, 0, &f as *const _ as *const c_void, sz) == 0
                    && AudioUnitSetProperty(unit, PROP_STREAM_FORMAT, SCOPE_OUTPUT, 0, &f as *const _ as *const c_void, sz) == 0
            };
            if !set(ch) {
                ch = if ch == 1 { 2 } else { 1 };
                if !set(ch) {
                    return fail("unsupported channel layout");
                }
            }
            let maxf = setup.max_block as u32;
            AudioUnitSetProperty(unit, PROP_MAX_FRAMES, SCOPE_GLOBAL, 0, &maxf as *const _ as *const c_void, 4);
            let offline: u32 = setup.offline as u32;
            AudioUnitSetProperty(unit, PROP_OFFLINE_RENDER, SCOPE_GLOBAL, 0, &offline as *const _ as *const c_void, 4);
            let feed = Box::new(Mutex::new(Feed { ins: vec![vec![0.0; setup.max_block]; ch], frames: 0 }));
            let cb = AURenderCallbackStruct { inputProc: input_cb, inputProcRefCon: &*feed as *const Mutex<Feed> as *mut c_void };
            if AudioUnitSetProperty(
                unit,
                PROP_SET_RENDER_CALLBACK,
                SCOPE_INPUT,
                0,
                &cb as *const _ as *const c_void,
                std::mem::size_of::<AURenderCallbackStruct>() as u32,
            ) != 0
            {
                return fail("does not accept audio input");
            }
            if AudioUnitInitialize(unit) != 0 {
                return fail("failed to initialise");
            }
            Ok(AuInstance {
                info: info.clone(),
                setup,
                unit,
                in_ch: ch,
                out_ch: ch,
                feed,
                proc: Mutex::new(Proc { next_pos: None, sample_time: 0.0, outs: vec![vec![0.0; setup.max_block]; ch] }),
                pending: Mutex::new(vec![]),
                dirty: AtomicBool::new(false),
            })
        }
    }

    fn param_ids(&self) -> Vec<u32> {
        unsafe {
            let mut size = 0u32;
            if AudioUnitGetPropertyInfo(self.unit, PROP_PARAMETER_LIST, SCOPE_GLOBAL, 0, &mut size, ptr::null_mut()) != 0 || size == 0 {
                return vec![];
            }
            let mut ids = vec![0u32; size as usize / 4];
            if AudioUnitGetProperty(self.unit, PROP_PARAMETER_LIST, SCOPE_GLOBAL, 0, ids.as_mut_ptr() as *mut c_void, &mut size) != 0 {
                return vec![];
            }
            ids.truncate(size as usize / 4);
            ids
        }
    }

    fn param_info(&self, id: u32) -> Option<AudioUnitParameterInfo> {
        unsafe {
            let mut pi: AudioUnitParameterInfo = std::mem::zeroed();
            let mut size = std::mem::size_of::<AudioUnitParameterInfo>() as u32;
            (AudioUnitGetProperty(self.unit, PROP_PARAMETER_INFO, SCOPE_GLOBAL, id, &mut pi as *mut _ as *mut c_void, &mut size) == 0)
                .then_some(pi)
        }
    }
}

impl Instance for AuInstance {
    fn info(&self) -> &PluginInfo {
        &self.info
    }

    fn latency(&self) -> u32 {
        let mut secs = 0f64;
        let mut size = 8u32;
        unsafe { AudioUnitGetProperty(self.unit, PROP_LATENCY, SCOPE_GLOBAL, 0, &mut secs as *mut f64 as *mut c_void, &mut size) };
        (secs * self.setup.sample_rate).round().max(0.0) as u32
    }

    fn params(&self) -> Vec<ParamInfo> {
        self.param_ids()
            .into_iter()
            .filter_map(|id| {
                let pi = self.param_info(id)?;
                let title = unsafe {
                    if pi.flags & FLAG_HAS_CFNAME != 0 && !pi.cfNameString.is_null() {
                        cfstring(pi.cfNameString)
                    } else {
                        std::ffi::CStr::from_ptr(pi.name.as_ptr()).to_string_lossy().into_owned()
                    }
                };
                let mut v = 0f32;
                unsafe { AudioUnitGetParameter(self.unit, id, SCOPE_GLOBAL, 0, &mut v) };
                let span = (pi.maxValue - pi.minValue).max(1e-9);
                let norm = |x: f32| ((x - pi.minValue) / span).clamp(0.0, 1.0) as f64;
                Some(ParamInfo {
                    id,
                    title,
                    units: String::new(),
                    step_count: 0,
                    default: norm(pi.defaultValue),
                    value: norm(v),
                    display: format!("{v:.2}"),
                    automatable: true,
                    read_only: pi.flags & FLAG_WRITABLE == 0,
                    is_bypass: false,
                })
            })
            .collect()
    }

    fn set_param(&self, id: u32, value: f64) -> Result<(), String> {
        let pi = self.param_info(id).ok_or_else(|| format!("{} has no parameter {id}", self.info.name))?;
        let v = pi.minValue + (pi.maxValue - pi.minValue) * value.clamp(0.0, 1.0) as f32;
        self.pending.lock().unwrap().push((id, v));
        Ok(())
    }

    fn get_state(&self) -> Result<Vec<u8>, String> {
        unsafe {
            let mut plist: CFTypeRef = ptr::null();
            let mut size = std::mem::size_of::<CFTypeRef>() as u32;
            if AudioUnitGetProperty(self.unit, PROP_CLASS_INFO, SCOPE_GLOBAL, 0, &mut plist as *mut CFTypeRef as *mut c_void, &mut size)
                != 0
                || plist.is_null()
            {
                return Err(format!("{} could not save its state", self.info.name));
            }
            let data = CFPropertyListCreateData(ptr::null(), plist, kCFPropertyListBinaryFormat_v1_0, 0, ptr::null_mut());
            CFRelease(plist);
            if data.is_null() {
                return Err(format!("{} state could not be serialised", self.info.name));
            }
            let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize).to_vec();
            CFRelease(data);
            Ok(bytes)
        }
    }

    fn set_state(&self, bytes: &[u8]) -> Result<(), String> {
        let _p = self.proc.lock().unwrap();
        unsafe {
            let data = CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize);
            let plist = CFPropertyListCreateWithData(ptr::null(), data, 0, ptr::null_mut(), ptr::null_mut());
            CFRelease(data);
            if plist.is_null() {
                return Err(format!("{}: not an Audio Unit state", self.info.name));
            }
            let r = AudioUnitSetProperty(
                self.unit,
                PROP_CLASS_INFO,
                SCOPE_GLOBAL,
                0,
                &plist as *const CFTypeRef as *const c_void,
                std::mem::size_of::<CFTypeRef>() as u32,
            );
            CFRelease(plist);
            if r != 0 {
                return Err(format!("{} rejected the saved state", self.info.name));
            }
        }
        Ok(())
    }

    fn process(&self, buf: &mut [f32], channels: usize, pos: i64) {
        let channels = channels.max(1);
        let frames = buf.len() / channels;
        let mut p = self.proc.lock().unwrap();
        if p.next_pos.is_some_and(|n| n != pos) {
            unsafe { AudioUnitReset(self.unit, SCOPE_GLOBAL, 0) };
        }
        for (id, v) in std::mem::take(&mut *self.pending.lock().unwrap()) {
            unsafe { AudioUnitSetParameter(self.unit, id, SCOPE_GLOBAL, 0, v, 0) };
        }
        let mut off = 0;
        while off < frames {
            let n = (frames - off).min(self.setup.max_block);
            let chunk = &mut buf[off * channels..(off + n) * channels];
            {
                let mut f = self.feed.lock().unwrap();
                f.frames = n;
                split_in(chunk, channels, n, &mut f.ins[..self.in_ch]);
            }
            let mut abl = AudioBufferList {
                mNumberBuffers: self.out_ch as u32,
                mBuffers: [AudioBuffer { mNumberChannels: 1, mDataByteSize: 0, mData: ptr::null_mut() }; MAX_CH],
            };
            for (c, o) in p.outs.iter_mut().enumerate().take(self.out_ch) {
                abl.mBuffers[c] = AudioBuffer { mNumberChannels: 1, mDataByteSize: (n * 4) as u32, mData: o.as_mut_ptr() as *mut c_void };
            }
            let ts = AudioTimeStamp { mSampleTime: p.sample_time, mFlags: 1, ..Default::default() };
            let mut flags = 0u32;
            let r = unsafe { AudioUnitRender(self.unit, &mut flags, &ts, 0, n as u32, &mut abl) };
            if r == 0 {
                // the unit may hand back its own buffers
                for c in 0..self.out_ch {
                    let src = abl.mBuffers[c].mData as *const f32;
                    if !src.is_null() && src != p.outs[c].as_ptr() {
                        unsafe { ptr::copy_nonoverlapping(src, p.outs[c].as_mut_ptr(), n) };
                    }
                }
                join_out(&p.outs[..self.out_ch], chunk, channels, n);
            }
            p.sample_time += n as f64;
            off += n;
        }
        p.next_pos = Some(pos + frames as i64);
    }

    fn reset(&self) {
        let mut p = self.proc.lock().unwrap();
        unsafe { AudioUnitReset(self.unit, SCOPE_GLOBAL, 0) };
        p.next_pos = None;
    }

    fn next_pos(&self) -> Option<i64> {
        self.proc.lock().unwrap().next_pos
    }

    fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Relaxed)
    }
}

impl Drop for AuInstance {
    fn drop(&mut self) {
        let _p = self.proc.lock().unwrap();
        unsafe {
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
        }
    }
}

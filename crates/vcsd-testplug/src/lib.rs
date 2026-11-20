//! A minimal VST3 plugin used only to test the host (not shipped).
//!
//! One factory, two effects sharing a controller:
//! - **VCSD Test Gain**: parameter 0 "Gain" (normalised 0.5 = unity, linear ×2 at 1.0);
//!   its value is the plugin state.
//! - **VCSD Test Delay**: the same gain plus a pure 480-sample delay that it reports
//!   as latency, so delay compensation can be checked to the sample.
//!
//! Mono and stereo layouts are both accepted (input and output must match).
//! Structure follows the `vst3` crate's gain example.
// Binding types differ by platform, so some casts are no-ops on one OS and needed on another.
#![allow(non_snake_case, non_upper_case_globals, clippy::missing_safety_doc, clippy::unnecessary_cast)]

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::{ptr, slice};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{uid, Class, ComRef, ComWrapper};

pub const GAIN_CID: TUID = uid(0x5643_5344, 0x4741_494E, 0x8E1A_0001, 0x0000_0001);
pub const DELAY_CID: TUID = uid(0x5643_5344, 0x4445_4C41, 0x8E1A_0001, 0x0000_0002);
const CONTROLLER_CID: TUID = uid(0x5643_5344, 0x4354_524C, 0x8E1A_0001, 0x0000_0003);
pub const DELAY_SAMPLES: usize = 480;

fn cstr(src: &str, dst: &mut [c_char]) {
    for (d, b) in dst.iter_mut().zip(src.bytes().chain(std::iter::once(0))) {
        *d = b as c_char;
    }
    if let Some(l) = dst.last_mut() {
        *l = 0;
    }
}
fn wstr(src: &str, dst: &mut [TChar]) {
    let mut n = 0;
    for (d, u) in dst.iter_mut().zip(src.encode_utf16()) {
        *d = u as TChar;
        n += 1;
    }
    dst[n.min(dst.len() - 1)] = 0;
}

fn gain_of(n: f64) -> f32 {
    (n.clamp(0.0, 1.0) * 2.0) as f32
}

struct Processor {
    delay: usize,
    gain: AtomicU64,
    channels: AtomicI32,
    line: Mutex<(Vec<f32>, usize)>,
}

impl Class for Processor {
    type Interfaces = (IComponent, IAudioProcessor);
}

impl Processor {
    fn new(delay: usize) -> Self {
        Processor { delay, gain: AtomicU64::new(0.5f64.to_bits()), channels: AtomicI32::new(2), line: Mutex::new((vec![], 0)) }
    }
    fn arr(&self) -> SpeakerArrangement {
        if self.channels.load(Relaxed) == 1 {
            SpeakerArr::kMono
        } else {
            SpeakerArr::kStereo
        }
    }
}

impl IPluginBaseTrait for Processor {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for Processor {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        *class_id = CONTROLLER_CID;
        kResultOk
    }
    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }
    unsafe fn getBusCount(&self, media: MediaType, _dir: BusDirection) -> i32 {
        (media == MediaTypes_::kAudio as MediaType) as i32
    }
    unsafe fn getBusInfo(&self, media: MediaType, dir: BusDirection, index: i32, bus: *mut BusInfo) -> tresult {
        if media != MediaTypes_::kAudio as MediaType || index != 0 {
            return kInvalidArgument;
        }
        let b = &mut *bus;
        b.mediaType = media;
        b.direction = dir;
        b.channelCount = self.channels.load(Relaxed);
        wstr(if dir == BusDirections_::kInput as BusDirection { "Input" } else { "Output" }, &mut b.name);
        b.busType = BusTypes_::kMain as BusType;
        b.flags = BusInfo_::BusFlags_::kDefaultActive as u32;
        kResultOk
    }
    unsafe fn getRoutingInfo(&self, _i: *mut RoutingInfo, _o: *mut RoutingInfo) -> tresult {
        kNotImplemented
    }
    unsafe fn activateBus(&self, _m: MediaType, _d: BusDirection, _i: i32, _s: TBool) -> tresult {
        kResultOk
    }
    unsafe fn setActive(&self, state: TBool) -> tresult {
        let ch = self.channels.load(Relaxed).max(1) as usize;
        *self.line.lock().unwrap() = (if state != 0 { vec![0.0; self.delay * ch] } else { vec![] }, 0);
        kResultOk
    }
    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let Some(s) = ComRef::from_raw(state) else { return kInvalidArgument };
        let mut b = [0u8; 8];
        let mut n = 0;
        if s.read(b.as_mut_ptr() as *mut c_void, 8, &mut n) != kResultOk || n != 8 {
            return kResultFalse;
        }
        self.gain.store(f64::from_le_bytes(b).to_bits(), Relaxed);
        kResultOk
    }
    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let Some(s) = ComRef::from_raw(state) else { return kInvalidArgument };
        let mut b = f64::from_bits(self.gain.load(Relaxed)).to_le_bytes();
        let mut n = 0;
        s.write(b.as_mut_ptr() as *mut c_void, 8, &mut n)
    }
}

impl IAudioProcessorTrait for Processor {
    unsafe fn setBusArrangements(&self, ins: *mut SpeakerArrangement, ni: i32, outs: *mut SpeakerArrangement, no: i32) -> tresult {
        if ni != 1 || no != 1 || *ins != *outs {
            return kResultFalse;
        }
        match *ins {
            SpeakerArr::kMono => self.channels.store(1, Relaxed),
            SpeakerArr::kStereo => self.channels.store(2, Relaxed),
            _ => return kResultFalse,
        }
        kResultTrue
    }
    unsafe fn getBusArrangement(&self, _dir: BusDirection, index: i32, arr: *mut SpeakerArrangement) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        *arr = self.arr();
        kResultOk
    }
    unsafe fn canProcessSampleSize(&self, size: i32) -> tresult {
        if size == SymbolicSampleSizes_::kSample32 as i32 {
            kResultOk
        } else {
            kNotImplemented
        }
    }
    unsafe fn getLatencySamples(&self) -> u32 {
        self.delay as u32
    }
    unsafe fn setupProcessing(&self, _setup: *mut ProcessSetup) -> tresult {
        kResultOk
    }
    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let d = &*data;
        if let Some(changes) = ComRef::from_raw(d.inputParameterChanges) {
            for i in 0..changes.getParameterCount() {
                if let Some(q) = ComRef::from_raw(changes.getParameterData(i)) {
                    let (mut off, mut v) = (0, 0.0);
                    if q.getParameterId() == 0 && q.getPoint(q.getPointCount() - 1, &mut off, &mut v) == kResultOk {
                        self.gain.store(v.to_bits(), Relaxed);
                    }
                }
            }
        }
        if d.numInputs < 1 || d.numOutputs < 1 || d.numSamples <= 0 {
            return kResultOk;
        }
        let n = d.numSamples as usize;
        let (ib, ob) = (&*d.inputs, &*d.outputs);
        let ch = ib.numChannels.min(ob.numChannels) as usize;
        let ins = slice::from_raw_parts(ib.__field0.channelBuffers32, ch);
        let outs = slice::from_raw_parts(ob.__field0.channelBuffers32, ch);
        let g = gain_of(f64::from_bits(self.gain.load(Relaxed)));
        let mut line = self.line.lock().unwrap();
        let (buf, w) = &mut *line;
        if buf.len() != self.delay * ch {
            *buf = vec![0.0; self.delay * ch];
            *w = 0;
        }
        for f in 0..n {
            for c in 0..ch {
                let x = *ins[c].add(f);
                let y = if self.delay == 0 {
                    x
                } else {
                    let i = *w * ch + c;
                    std::mem::replace(&mut buf[i], x)
                };
                *outs[c].add(f) = y * g;
            }
            if self.delay > 0 {
                *w = (*w + 1) % self.delay;
            }
        }
        kResultOk
    }
    unsafe fn getTailSamples(&self) -> u32 {
        self.delay as u32
    }
}

struct Controller {
    gain: Mutex<f64>,
}

impl Class for Controller {
    type Interfaces = (IEditController,);
}

impl IPluginBaseTrait for Controller {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }
    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IEditControllerTrait for Controller {
    unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
        let Some(s) = ComRef::from_raw(state) else { return kInvalidArgument };
        let mut b = [0u8; 8];
        let mut n = 0;
        if s.read(b.as_mut_ptr() as *mut c_void, 8, &mut n) == kResultOk && n == 8 {
            *self.gain.lock().unwrap() = f64::from_le_bytes(b);
        }
        kResultOk
    }
    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }
    unsafe fn getParameterCount(&self) -> i32 {
        1
    }
    unsafe fn getParameterInfo(&self, index: i32, info: *mut ParameterInfo) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        let i = &mut *info;
        i.id = 0;
        wstr("Gain", &mut i.title);
        wstr("Gain", &mut i.shortTitle);
        wstr("dB", &mut i.units);
        i.stepCount = 0;
        i.defaultNormalizedValue = 0.5;
        i.unitId = 0;
        i.flags = ParameterInfo_::ParameterFlags_::kCanAutomate as i32;
        kResultOk
    }
    unsafe fn getParamStringByValue(&self, id: u32, v: f64, out: *mut String128) -> tresult {
        if id != 0 {
            return kInvalidArgument;
        }
        let g = gain_of(v);
        let s = if g <= 0.0 { "-inf".to_string() } else { format!("{:.1}", 20.0 * g.log10()) };
        wstr(&s, &mut *out);
        kResultOk
    }
    unsafe fn getParamValueByString(&self, _id: u32, _s: *mut TChar, _v: *mut f64) -> tresult {
        kNotImplemented
    }
    unsafe fn normalizedParamToPlain(&self, _id: u32, v: f64) -> f64 {
        v
    }
    unsafe fn plainParamToNormalized(&self, _id: u32, v: f64) -> f64 {
        v
    }
    unsafe fn getParamNormalized(&self, id: u32) -> f64 {
        if id == 0 {
            *self.gain.lock().unwrap()
        } else {
            0.0
        }
    }
    unsafe fn setParamNormalized(&self, id: u32, v: f64) -> tresult {
        if id != 0 {
            return kInvalidArgument;
        }
        *self.gain.lock().unwrap() = v;
        kResultOk
    }
    unsafe fn setComponentHandler(&self, _h: *mut IComponentHandler) -> tresult {
        kResultOk
    }
    unsafe fn createView(&self, _name: *const c_char) -> *mut IPlugView {
        ptr::null_mut()
    }
}

struct Factory;

impl Class for Factory {
    type Interfaces = (IPluginFactory, IPluginFactory2);
}

const CLASSES: [(TUID, &str, &str, &str); 3] = [
    (GAIN_CID, "Audio Module Class", "VCSD Test Gain", "Fx"),
    (DELAY_CID, "Audio Module Class", "VCSD Test Delay", "Fx|Delay"),
    (CONTROLLER_CID, "Component Controller Class", "VCSD Test Controller", ""),
];

impl IPluginFactoryTrait for Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        let i = &mut *info;
        cstr("VC Studio (test)", &mut i.vendor);
        cstr("", &mut i.url);
        cstr("", &mut i.email);
        i.flags = PFactoryInfo_::FactoryFlags_::kUnicode as int32;
        kResultOk
    }
    unsafe fn countClasses(&self) -> i32 {
        CLASSES.len() as i32
    }
    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        let Some((cid, cat, name, _)) = CLASSES.get(index as usize) else { return kInvalidArgument };
        let i = &mut *info;
        i.cid = *cid;
        i.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
        cstr(cat, &mut i.category);
        cstr(name, &mut i.name);
        kResultOk
    }
    unsafe fn createInstance(&self, cid: FIDString, iid: FIDString, obj: *mut *mut c_void) -> tresult {
        let unknown = match *(cid as *const TUID) {
            GAIN_CID => ComWrapper::new(Processor::new(0)).to_com_ptr::<FUnknown>(),
            DELAY_CID => ComWrapper::new(Processor::new(DELAY_SAMPLES)).to_com_ptr::<FUnknown>(),
            CONTROLLER_CID => ComWrapper::new(Controller { gain: Mutex::new(0.5) }).to_com_ptr::<FUnknown>(),
            _ => None,
        };
        match unknown {
            Some(u) => {
                let p = u.as_ptr();
                ((*(*p).vtbl).queryInterface)(p, iid as *const TUID, obj)
            }
            None => kInvalidArgument,
        }
    }
}

impl IPluginFactory2Trait for Factory {
    unsafe fn getClassInfo2(&self, index: i32, info: *mut PClassInfo2) -> tresult {
        let Some((cid, cat, name, sub)) = CLASSES.get(index as usize) else { return kInvalidArgument };
        let i = &mut *info;
        i.cid = *cid;
        i.cardinality = PClassInfo_::ClassCardinality_::kManyInstances as int32;
        cstr(cat, &mut i.category);
        cstr(name, &mut i.name);
        i.classFlags = 0;
        cstr(sub, &mut i.subCategories);
        cstr("VC Studio (test)", &mut i.vendor);
        cstr("1.0.0", &mut i.version);
        cstr("VST 3.7", &mut i.sdkVersion);
        kResultOk
    }
}

#[cfg(target_os = "windows")]
#[no_mangle]
extern "system" fn InitDll() -> bool {
    true
}
#[cfg(target_os = "windows")]
#[no_mangle]
extern "system" fn ExitDll() -> bool {
    true
}
#[cfg(target_os = "macos")]
#[no_mangle]
extern "system" fn bundleEntry(_bundle: *mut c_void) -> bool {
    true
}
#[cfg(target_os = "macos")]
#[no_mangle]
extern "system" fn bundleExit() -> bool {
    true
}
#[cfg(target_os = "linux")]
#[no_mangle]
extern "system" fn ModuleEntry(_handle: *mut c_void) -> bool {
    true
}
#[cfg(target_os = "linux")]
#[no_mangle]
extern "system" fn ModuleExit() -> bool {
    true
}

/// The VST3 entry point.
#[no_mangle]
pub extern "system" fn GetPluginFactory() -> *mut IPluginFactory {
    ComWrapper::new(Factory).to_com_ptr::<IPluginFactory>().map(|p| p.into_raw()).unwrap_or(ptr::null_mut())
}

/// For tests: put this plugin, as built by cargo, into a `.vst3` bundle under
/// `dir` (`dir/VCSD Test.vst3`) and return the bundle path. Builds the library
/// first if it isn't there yet.
pub fn make_bundle(dir: &std::path::Path) -> std::path::PathBuf {
    use std::path::Path;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let (file, sub) = if cfg!(windows) {
        ("vcsd_testplug.dll", format!("{}-win", std::env::consts::ARCH))
    } else if cfg!(target_os = "macos") {
        ("libvcsd_testplug.dylib", "MacOS".to_string())
    } else {
        ("libvcsd_testplug.so", format!("{}-linux", std::env::consts::ARCH))
    };
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let lib = target.join(profile).join(file);
    if !lib.exists() {
        let mut cmd = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cmd.args(["build", "-p", "vcsd-testplug"]);
        if profile == "release" {
            cmd.arg("--release");
        }
        assert!(cmd.status().map(|s| s.success()).unwrap_or(false), "building vcsd-testplug failed");
    }
    let bundle = dir.join("VCSD Test.vst3");
    let bin = bundle.join("Contents").join(&sub);
    std::fs::create_dir_all(&bin).unwrap();
    let name = if cfg!(windows) {
        "VCSD Test.vst3"
    } else if cfg!(target_os = "macos") {
        "VCSD Test"
    } else {
        "VCSD Test.so"
    };
    std::fs::copy(&lib, bin.join(name)).unwrap();
    bundle
}

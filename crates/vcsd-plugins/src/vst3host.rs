//! VST3 hosting (all platforms).
//!
//! Modules are loaded once and never unloaded (many plugins misbehave when
//! their library goes away). The host side implements the objects plugins ask
//! for: the host application context (which creates messages and attribute
//! lists for processor ↔ controller traffic), memory streams for state,
//! parameter-change queues and a component handler that hears edits made in
//! the plugin's own editor.
// VST3 binding types differ by platform (e.g. TChar, flag enums), so a cast that is
// a no-op on one platform is needed on another.
#![allow(non_snake_case, clippy::missing_safety_doc, clippy::unnecessary_cast)]

use crate::{join_out, split_in, Instance, ParamInfo, PluginInfo, Setup};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::*};
use std::sync::{Arc, Mutex, OnceLock};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComWrapper, Interface};

const AUDIO: MediaType = MediaTypes_::kAudio as MediaType;
const INPUT: BusDirection = BusDirections_::kInput as BusDirection;
const OUTPUT: BusDirection = BusDirections_::kOutput as BusDirection;

fn ok(r: tresult) -> bool {
    r == kResultOk
}

fn utf16(s: &[TChar]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n].iter().map(|&c| c as u16).collect::<Vec<_>>())
}

fn cstr(s: &[c_char]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..n].iter().map(|&c| c as u8).collect::<Vec<_>>()).into_owned()
}

/// Class id as 32 hex digits (the form used in plugin ids).
pub fn cid_hex(cid: &TUID) -> String {
    cid.iter().map(|b| format!("{:02X}", *b as u8)).collect()
}

fn parse_cid(id: &str) -> Result<TUID, String> {
    let h = id.strip_prefix("vst3:").unwrap_or(id);
    if h.len() != 32 {
        return Err(format!("bad VST3 class id '{id}'"));
    }
    let mut t: TUID = [0; 16];
    for (i, b) in t.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).map_err(|_| format!("bad VST3 class id '{id}'"))? as c_char;
    }
    Ok(t)
}

// ---------------------------------------------------------------- modules

pub struct Module {
    _lib: libloading::Library,
    factory: ComPtr<IPluginFactory>,
}
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

/// The binary inside a `.vst3` bundle for this platform (or the path itself
/// for a single-file module).
pub fn binary_path(bundle: &Path) -> Result<PathBuf, String> {
    if bundle.is_file() {
        return Ok(bundle.to_path_buf());
    }
    let stem = bundle.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let contents = bundle.join("Contents");
    let candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        let mut v = vec![contents.join("MacOS").join(&stem)];
        if let Ok(rd) = std::fs::read_dir(contents.join("MacOS")) {
            v.extend(rd.flatten().map(|e| e.path()));
        }
        v
    } else if cfg!(target_os = "windows") {
        let archs: &[&str] = if cfg!(target_arch = "aarch64") { &["arm64-win", "arm64x-win", "arm64ec-win"] } else { &["x86_64-win"] };
        archs.iter().map(|a| contents.join(a).join(format!("{stem}.vst3"))).collect()
    } else {
        vec![contents.join(format!("{}-linux", std::env::consts::ARCH)).join(format!("{stem}.so"))]
    };
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| format!("{} has no binary for this platform", bundle.display()))
}

#[cfg(target_os = "macos")]
mod cf {
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFURLCreateFromFileSystemRepresentation(alloc: *const c_void, buf: *const u8, len: isize, dir: u8) -> *const c_void;
        pub fn CFBundleCreate(alloc: *const c_void, url: *const c_void) -> *mut c_void;
        pub fn CFRelease(p: *const c_void);
    }
}

unsafe fn load(bundle: &Path) -> Result<Module, String> {
    let bin = binary_path(bundle)?;
    let err = |e: libloading::Error| e.to_string();
    #[cfg(unix)]
    let (lib, handle) = {
        use libloading::os::unix::{Library as U, RTLD_LAZY, RTLD_LOCAL};
        let u = U::open(Some(&bin), RTLD_LAZY | RTLD_LOCAL).map_err(err)?;
        let h = u.into_raw();
        (libloading::Library::from(U::from_raw(h)), h)
    };
    #[cfg(not(unix))]
    let lib = libloading::Library::new(&bin).map_err(err)?;

    #[cfg(target_os = "linux")]
    if let Ok(entry) = lib.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"ModuleEntry\0") {
        if !entry(handle) {
            return Err("ModuleEntry failed".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = handle;
        let p = bundle.as_os_str().as_encoded_bytes();
        let url = cf::CFURLCreateFromFileSystemRepresentation(ptr::null(), p.as_ptr(), p.len() as isize, 1);
        let b = if url.is_null() { ptr::null_mut() } else { cf::CFBundleCreate(ptr::null(), url) };
        if !url.is_null() {
            cf::CFRelease(url);
        }
        // the bundle ref stays alive for the life of the process, like the module
        for sym in [&b"bundleEntry\0"[..], &b"BundleEntry\0"[..]] {
            if let Ok(entry) = lib.get::<unsafe extern "C" fn(*mut c_void) -> bool>(sym) {
                if !entry(b) {
                    return Err("bundleEntry failed".into());
                }
                break;
            }
        }
    }
    #[cfg(target_os = "windows")]
    if let Ok(init) = lib.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0") {
        if !init() {
            return Err("InitDll failed".into());
        }
    }
    let get = lib.get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0").map_err(err)?;
    let factory = ComPtr::from_raw(get()).ok_or("GetPluginFactory returned nothing")?;
    Ok(Module { _lib: lib, factory })
}

/// Load (once) and keep a module.
pub fn module(bundle: &Path) -> Result<Arc<Module>, String> {
    static MODULES: OnceLock<Mutex<HashMap<PathBuf, Arc<Module>>>> = OnceLock::new();
    let mut m = MODULES.get_or_init(Default::default).lock().unwrap();
    if let Some(x) = m.get(bundle) {
        return Ok(x.clone());
    }
    let x = Arc::new(unsafe { load(bundle)? });
    m.insert(bundle.to_path_buf(), x.clone());
    Ok(x)
}

/// Every effect/instrument class in a module (loads it: run in a child process).
pub fn scan_module(bundle: &Path) -> Result<Vec<PluginInfo>, String> {
    let m = module(bundle)?;
    scan_factory(&m.factory, &bundle.display().to_string())
}

/// Classes of an already-obtained factory (also used by tests).
pub fn scan_factory(f: &ComPtr<IPluginFactory>, path: &str) -> Result<Vec<PluginInfo>, String> {
    unsafe {
        let mut fi: PFactoryInfo = std::mem::zeroed();
        let vendor = if ok(f.getFactoryInfo(&mut fi)) { cstr(&fi.vendor) } else { String::new() };
        let f2 = f.cast::<IPluginFactory2>();
        let mut out = vec![];
        for i in 0..f.countClasses() {
            let mut ci: PClassInfo = std::mem::zeroed();
            if !ok(f.getClassInfo(i, &mut ci)) || cstr(&ci.category) != "Audio Module Class" {
                continue;
            }
            let mut p = PluginInfo {
                id: format!("vst3:{}", cid_hex(&ci.cid)),
                format: "vst3".into(),
                name: cstr(&ci.name),
                vendor: vendor.clone(),
                version: String::new(),
                category: "Fx".into(),
                path: path.into(),
            };
            if let Some(f2) = &f2 {
                let mut c2: PClassInfo2 = std::mem::zeroed();
                if ok(f2.getClassInfo2(i, &mut c2)) {
                    p.category = cstr(&c2.subCategories);
                    if !cstr(&c2.vendor).is_empty() {
                        p.vendor = cstr(&c2.vendor);
                    }
                    p.version = cstr(&c2.version);
                }
            }
            out.push(p);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- host objects

fn query<C: Class>(obj: &ComWrapper<C>, iid: *const TUID, out: *mut *mut c_void) -> tresult {
    match obj.to_com_ptr::<FUnknown>() {
        Some(u) => unsafe { ((*(*u.as_ptr()).vtbl).queryInterface)(u.as_ptr(), iid, out) },
        None => kNoInterface,
    }
}

fn iid_eq(p: *const TUID, iid: &[u8; 16]) -> bool {
    !p.is_null() && unsafe { std::slice::from_raw_parts(p as *const u8, 16) } == iid
}

struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        let n = &mut *name;
        let mut i = 0;
        for u in "VC Sound Design".encode_utf16() {
            n[i] = u as TChar;
            i += 1;
        }
        n[i] = 0;
        kResultOk
    }
    unsafe fn createInstance(&self, cid: *mut TUID, iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        if iid_eq(cid, &IMessage::IID) {
            return query(&ComWrapper::new(Message::new()), iid, obj);
        }
        if iid_eq(cid, &IAttributeList::IID) {
            return query(&ComWrapper::new(Attrs::default()), iid, obj);
        }
        *obj = ptr::null_mut();
        kNoInterface
    }
}

struct Message {
    id: Mutex<Option<CString>>,
    attrs: ComPtr<IAttributeList>,
}
unsafe impl Send for Message {}
unsafe impl Sync for Message {}

impl Message {
    fn new() -> Self {
        Message { id: Mutex::new(None), attrs: ComWrapper::new(Attrs::default()).to_com_ptr().unwrap() }
    }
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id.lock().unwrap().as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null())
    }
    unsafe fn setMessageID(&self, id: FIDString) {
        *self.id.lock().unwrap() = (!id.is_null()).then(|| CStr::from_ptr(id).to_owned());
    }
    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        // like the SDK's host message: no extra reference
        self.attrs.as_ptr()
    }
}

enum Attr {
    Int(i64),
    Float(f64),
    Str(Vec<TChar>),
    Bin(Vec<u8>),
}

#[derive(Default)]
struct Attrs(Mutex<HashMap<String, Attr>>);

impl Class for Attrs {
    type Interfaces = (IAttributeList,);
}

unsafe fn key(id: IAttributeList_::AttrID) -> Option<String> {
    (!id.is_null()).then(|| CStr::from_ptr(id).to_string_lossy().into_owned())
}

impl IAttributeListTrait for Attrs {
    unsafe fn setInt(&self, id: IAttributeList_::AttrID, v: int64) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        self.0.lock().unwrap().insert(k, Attr::Int(v));
        kResultOk
    }
    unsafe fn getInt(&self, id: IAttributeList_::AttrID, v: *mut int64) -> tresult {
        match key(id).and_then(|k| match self.0.lock().unwrap().get(&k) {
            Some(Attr::Int(x)) => Some(*x),
            _ => None,
        }) {
            Some(x) => {
                *v = x;
                kResultOk
            }
            None => kResultFalse,
        }
    }
    unsafe fn setFloat(&self, id: IAttributeList_::AttrID, v: f64) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        self.0.lock().unwrap().insert(k, Attr::Float(v));
        kResultOk
    }
    unsafe fn getFloat(&self, id: IAttributeList_::AttrID, v: *mut f64) -> tresult {
        match key(id).and_then(|k| match self.0.lock().unwrap().get(&k) {
            Some(Attr::Float(x)) => Some(*x),
            _ => None,
        }) {
            Some(x) => {
                *v = x;
                kResultOk
            }
            None => kResultFalse,
        }
    }
    unsafe fn setString(&self, id: IAttributeList_::AttrID, s: *const TChar) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        let mut v = vec![];
        if !s.is_null() {
            let mut i = 0;
            while *s.add(i) != 0 {
                v.push(*s.add(i));
                i += 1;
            }
        }
        self.0.lock().unwrap().insert(k, Attr::Str(v));
        kResultOk
    }
    unsafe fn getString(&self, id: IAttributeList_::AttrID, out: *mut TChar, size: uint32) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        let g = self.0.lock().unwrap();
        let Some(Attr::Str(v)) = g.get(&k) else { return kResultFalse };
        let cap = (size as usize / std::mem::size_of::<TChar>()).saturating_sub(1);
        let n = v.len().min(cap);
        ptr::copy_nonoverlapping(v.as_ptr(), out, n);
        *out.add(n) = 0;
        kResultOk
    }
    unsafe fn setBinary(&self, id: IAttributeList_::AttrID, data: *const c_void, size: uint32) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        let v = if data.is_null() { vec![] } else { std::slice::from_raw_parts(data as *const u8, size as usize).to_vec() };
        self.0.lock().unwrap().insert(k, Attr::Bin(v));
        kResultOk
    }
    unsafe fn getBinary(&self, id: IAttributeList_::AttrID, data: *mut *const c_void, size: *mut uint32) -> tresult {
        let Some(k) = key(id) else { return kInvalidArgument };
        let g = self.0.lock().unwrap();
        let Some(Attr::Bin(v)) = g.get(&k) else { return kResultFalse };
        // valid until the attribute is replaced (as in the SDK)
        *data = v.as_ptr() as *const c_void;
        *size = v.len() as uint32;
        kResultOk
    }
}

/// In-memory IBStream for plugin state.
#[derive(Default)]
pub struct MemStream(Mutex<(Vec<u8>, usize)>);

impl Class for MemStream {
    type Interfaces = (IBStream,);
}

impl MemStream {
    fn with(data: &[u8]) -> ComWrapper<MemStream> {
        ComWrapper::new(MemStream(Mutex::new((data.to_vec(), 0))))
    }
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().0.clone()
    }
    fn rewind(&self) {
        self.0.lock().unwrap().1 = 0;
    }
}

impl IBStreamTrait for MemStream {
    unsafe fn read(&self, buf: *mut c_void, n: int32, got: *mut int32) -> tresult {
        let mut g = self.0.lock().unwrap();
        let (data, pos) = &mut *g;
        let k = (n.max(0) as usize).min(data.len().saturating_sub(*pos));
        ptr::copy_nonoverlapping(data.as_ptr().add(*pos), buf as *mut u8, k);
        *pos += k;
        if !got.is_null() {
            *got = k as int32;
        }
        kResultOk
    }
    unsafe fn write(&self, buf: *mut c_void, n: int32, wrote: *mut int32) -> tresult {
        let mut g = self.0.lock().unwrap();
        let (data, pos) = &mut *g;
        let n = n.max(0) as usize;
        if data.len() < *pos + n {
            data.resize(*pos + n, 0);
        }
        ptr::copy_nonoverlapping(buf as *const u8, data.as_mut_ptr().add(*pos), n);
        *pos += n;
        if !wrote.is_null() {
            *wrote = n as int32;
        }
        kResultOk
    }
    unsafe fn seek(&self, to: int64, mode: int32, result: *mut int64) -> tresult {
        let mut g = self.0.lock().unwrap();
        let (data, pos) = &mut *g;
        let base = match mode {
            0 => 0,
            1 => *pos as i64,
            2 => data.len() as i64,
            _ => return kInvalidArgument,
        };
        let p = (base + to).max(0) as usize;
        *pos = p.min(data.len());
        if !result.is_null() {
            *result = *pos as int64;
        }
        kResultOk
    }
    unsafe fn tell(&self, at: *mut int64) -> tresult {
        if !at.is_null() {
            *at = self.0.lock().unwrap().1 as int64;
        }
        kResultOk
    }
}

struct Queue {
    id: ParamID,
    value: f64,
}

impl Class for Queue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for Queue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id
    }
    unsafe fn getPointCount(&self) -> int32 {
        1
    }
    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        if index != 0 {
            return kInvalidArgument;
        }
        *offset = 0;
        *value = self.value;
        kResultOk
    }
    unsafe fn addPoint(&self, _o: int32, _v: ParamValue, _i: *mut int32) -> tresult {
        kResultFalse
    }
}

struct Changes(Vec<ComPtr<IParamValueQueue>>);

impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> int32 {
        self.0.len() as int32
    }
    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        self.0.get(index as usize).map(|q| q.as_ptr()).unwrap_or(ptr::null_mut())
    }
    unsafe fn addParameterData(&self, _id: *const ParamID, _index: *mut int32) -> *mut IParamValueQueue {
        ptr::null_mut()
    }
}

struct NoEvents;

impl Class for NoEvents {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for NoEvents {
    unsafe fn getEventCount(&self) -> int32 {
        0
    }
    unsafe fn getEvent(&self, _i: int32, _e: *mut Event) -> tresult {
        kInvalidArgument
    }
    unsafe fn addEvent(&self, _e: *mut Event) -> tresult {
        kResultFalse
    }
}

#[derive(Default)]
struct Edits {
    /// Parameter changes waiting to reach the processor.
    pending: Mutex<Vec<(ParamID, f64)>>,
    /// The plugin's own editor changed something since the last save.
    dirty: AtomicBool,
    latency_changed: AtomicBool,
}

struct Handler(Arc<Edits>);

impl Class for Handler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, id: ParamID, v: ParamValue) -> tresult {
        self.0.pending.lock().unwrap().push((id, v));
        self.0.dirty.store(true, Relaxed);
        kResultOk
    }
    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        if flags & RestartFlags_::kLatencyChanged as int32 != 0 {
            self.0.latency_changed.store(true, Relaxed);
        }
        if flags & (RestartFlags_::kParamValuesChanged as int32 | RestartFlags_::kReloadComponent as int32) != 0 {
            self.0.dirty.store(true, Relaxed);
        }
        kResultOk
    }
}

// ---------------------------------------------------------------- instance

struct Proc {
    next_pos: Option<i64>,
    continuous: i64,
    /// Per bus, per channel scratch buffers (max_block long).
    ins: Vec<Vec<Vec<f32>>>,
    outs: Vec<Vec<Vec<f32>>>,
}

pub struct Vst3Instance {
    info: PluginInfo,
    setup: Setup,
    _module: Arc<Module>,
    _host: ComWrapper<HostApp>,
    edits: Arc<Edits>,
    handler: ComWrapper<Handler>,
    no_events: ComWrapper<NoEvents>,
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    separate: bool,
    links: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    /// Serialises controller calls (they may come from several API threads).
    ctrl: Mutex<()>,
    proc: Mutex<Proc>,
    latency: AtomicU32,
}

// The plugin objects are used from the control and render threads, as every
// VST3 host does; the processor is only touched with `proc` held.
unsafe impl Send for Vst3Instance {}
unsafe impl Sync for Vst3Instance {}

unsafe fn create<I: Interface>(f: &ComPtr<IPluginFactory>, cid: &TUID) -> Option<ComPtr<I>> {
    let mut obj = ptr::null_mut();
    if ok(f.createInstance(cid.as_ptr(), I::IID.as_ptr() as FIDString, &mut obj)) {
        ComPtr::from_raw(obj as *mut I)
    } else {
        None
    }
}

impl Vst3Instance {
    pub fn new(info: &PluginInfo, setup: Setup) -> Result<Vst3Instance, String> {
        let module = module(Path::new(&info.path))?;
        Self::with_factory(info, setup, module)
    }

    /// For tests: a factory obtained without loading a bundle.
    ///
    /// # Safety
    /// `factory` must be a valid `IPluginFactory` (ownership of one reference is taken).
    pub unsafe fn from_factory(info: &PluginInfo, setup: Setup, factory: *mut IPluginFactory) -> Result<Vst3Instance, String> {
        let factory = unsafe { ComPtr::from_raw(factory) }.ok_or("null factory")?;
        // a module with no library (the factory lives in this process)
        #[cfg(unix)]
        let lib = libloading::Library::from(libloading::os::unix::Library::this());
        #[cfg(windows)]
        let lib = libloading::Library::from(libloading::os::windows::Library::this().map_err(|e| e.to_string())?);
        Self::with_factory(info, setup, Arc::new(Module { _lib: lib, factory }))
    }

    fn with_factory(info: &PluginInfo, setup: Setup, module: Arc<Module>) -> Result<Vst3Instance, String> {
        let cid = parse_cid(&info.id)?;
        unsafe {
            let host = ComWrapper::new(HostApp);
            let host_ptr = host.to_com_ptr::<FUnknown>().unwrap();
            let component: ComPtr<IComponent> =
                create(&module.factory, &cid).ok_or_else(|| format!("{} could not be created", info.name))?;
            if !ok(component.initialize(host_ptr.as_ptr())) {
                return Err(format!("{} failed to initialise", info.name));
            }
            let processor = component.cast::<IAudioProcessor>().ok_or_else(|| format!("{} is not an audio processor", info.name))?;
            let (controller, separate) = match component.cast::<IEditController>() {
                Some(c) => (Some(c), false),
                None => {
                    let mut ccid: TUID = [0; 16];
                    let c = if ok(component.getControllerClassId(&mut ccid)) {
                        create::<IEditController>(&module.factory, &ccid)
                    } else {
                        None
                    };
                    if let Some(c) = &c {
                        if !ok(c.initialize(host_ptr.as_ptr())) {
                            return Err(format!("{} controller failed to initialise", info.name));
                        }
                    }
                    (c, true)
                }
            };
            let mut links = None;
            if let (true, Some(c)) = (separate, &controller) {
                if let (Some(a), Some(b)) = (component.cast::<IConnectionPoint>(), c.cast::<IConnectionPoint>()) {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                    links = Some((a, b));
                }
            }
            let edits = Arc::new(Edits::default());
            let handler = ComWrapper::new(Handler(edits.clone()));
            if let Some(c) = &controller {
                c.setComponentHandler(handler.to_com_ptr::<IComponentHandler>().unwrap().as_ptr());
                // the controller learns the processor's (default) state
                let s = MemStream::with(&[]);
                let sp = s.to_com_ptr::<IBStream>().unwrap();
                if ok(component.getState(sp.as_ptr())) {
                    s.rewind();
                    c.setComponentState(sp.as_ptr());
                }
            }

            // buses: main in/out at our channel count, others as the plugin likes
            let want = if setup.channels == 1 { SpeakerArr::kMono } else { SpeakerArr::kStereo };
            let arrs = |dir: BusDirection| -> Vec<SpeakerArrangement> {
                (0..component.getBusCount(AUDIO, dir))
                    .map(|i| {
                        let mut a: SpeakerArrangement = 0;
                        processor.getBusArrangement(dir, i, &mut a);
                        if i == 0 {
                            want
                        } else {
                            a
                        }
                    })
                    .collect()
            };
            let (mut ia, mut oa) = (arrs(INPUT), arrs(OUTPUT));
            processor.setBusArrangements(ia.as_mut_ptr(), ia.len() as i32, oa.as_mut_ptr(), oa.len() as i32);
            let counts = |dir: BusDirection, n: usize| -> Vec<usize> {
                (0..n as i32)
                    .map(|i| {
                        let mut a: SpeakerArrangement = 0;
                        processor.getBusArrangement(dir, i, &mut a);
                        let mut bi: BusInfo = std::mem::zeroed();
                        let fallback = if ok(component.getBusInfo(AUDIO, dir, i, &mut bi)) { bi.channelCount.max(0) as usize } else { 0 };
                        if a != 0 {
                            a.count_ones() as usize
                        } else {
                            fallback
                        }
                    })
                    .collect()
            };
            let (ic, oc) = (counts(INPUT, ia.len()), counts(OUTPUT, oa.len()));
            if oc.first().copied().unwrap_or(0) == 0 {
                return Err(format!("{} has no audio output", info.name));
            }
            for i in 0..ic.len() {
                component.activateBus(AUDIO, INPUT, i as i32, 1);
            }
            for i in 0..oc.len() {
                component.activateBus(AUDIO, OUTPUT, i as i32, 1);
            }
            let mut ps = ProcessSetup {
                processMode: if setup.offline { ProcessModes_::kOffline } else { ProcessModes_::kRealtime } as int32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
                maxSamplesPerBlock: setup.max_block as int32,
                sampleRate: setup.sample_rate,
            };
            if !ok(processor.setupProcessing(&mut ps)) {
                return Err(format!("{} rejected {} Hz / {} frames", info.name, setup.sample_rate, setup.max_block));
            }
            component.setActive(1);
            processor.setProcessing(1);
            let latency = processor.getLatencySamples();
            let bufs = |v: &[usize]| v.iter().map(|&c| vec![vec![0.0f32; setup.max_block]; c]).collect::<Vec<_>>();
            Ok(Vst3Instance {
                info: info.clone(),
                setup,
                _module: module,
                _host: host,
                edits,
                handler,
                no_events: ComWrapper::new(NoEvents),
                component,
                processor,
                controller,
                separate,
                links,
                ctrl: Mutex::new(()),
                proc: Mutex::new(Proc { next_pos: None, continuous: 0, ins: bufs(&ic), outs: bufs(&oc) }),
                latency: AtomicU32::new(latency),
            })
        }
    }

    fn restart(&self) {
        unsafe {
            self.processor.setProcessing(0);
            self.component.setActive(0);
            self.component.setActive(1);
            self.processor.setProcessing(1);
        }
    }

    fn block(&self, p: &mut Proc, buf: &mut [f32], channels: usize, frames: usize, pos: i64, changes: Option<&ComWrapper<Changes>>) {
        unsafe {
            if let Some(bus) = p.ins.first_mut() {
                split_in(buf, channels, frames, bus);
            }
            for bus in p.ins.iter_mut().skip(1) {
                bus.iter_mut().for_each(|c| c[..frames].fill(0.0));
            }
            for bus in p.outs.iter_mut() {
                bus.iter_mut().for_each(|c| c[..frames].fill(0.0));
            }
            let mut ip: Vec<Vec<*mut f32>> = p.ins.iter_mut().map(|b| b.iter_mut().map(|c| c.as_mut_ptr()).collect()).collect();
            let mut op: Vec<Vec<*mut f32>> = p.outs.iter_mut().map(|b| b.iter_mut().map(|c| c.as_mut_ptr()).collect()).collect();
            let bus = |v: &mut Vec<*mut f32>| AudioBusBuffers {
                numChannels: v.len() as int32,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 { channelBuffers32: v.as_mut_ptr() },
            };
            let mut ib: Vec<AudioBusBuffers> = ip.iter_mut().map(bus).collect();
            let mut ob: Vec<AudioBusBuffers> = op.iter_mut().map(bus).collect();
            let mut ctx: ProcessContext = std::mem::zeroed();
            // playing | tempo valid | time signature valid
            ctx.state = (1 << 1) | (1 << 10) | (1 << 13);
            ctx.sampleRate = self.setup.sample_rate;
            ctx.projectTimeSamples = pos;
            ctx.continousTimeSamples = p.continuous;
            ctx.tempo = 120.0;
            ctx.timeSigNumerator = 4;
            ctx.timeSigDenominator = 4;
            let changes_ptr = changes.and_then(|c| c.to_com_ptr::<IParameterChanges>());
            let events = self.no_events.to_com_ptr::<IEventList>().unwrap();
            let mut data = ProcessData {
                processMode: if self.setup.offline { ProcessModes_::kOffline } else { ProcessModes_::kRealtime } as int32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
                numSamples: frames as int32,
                numInputs: ib.len() as int32,
                numOutputs: ob.len() as int32,
                inputs: if ib.is_empty() { ptr::null_mut() } else { ib.as_mut_ptr() },
                outputs: ob.as_mut_ptr(),
                inputParameterChanges: changes_ptr.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null_mut()),
                outputParameterChanges: ptr::null_mut(),
                inputEvents: events.as_ptr(),
                outputEvents: ptr::null_mut(),
                processContext: &mut ctx,
            };
            self.processor.process(&mut data);
            p.continuous += frames as i64;
            if let Some(out) = p.outs.first() {
                join_out(out, buf, channels, frames);
            }
        }
    }
}

impl Instance for Vst3Instance {
    fn info(&self) -> &PluginInfo {
        &self.info
    }

    fn latency(&self) -> u32 {
        self.latency.load(Relaxed)
    }

    fn params(&self) -> Vec<ParamInfo> {
        let Some(c) = &self.controller else { return vec![] };
        let _g = self.ctrl.lock().unwrap();
        unsafe {
            (0..c.getParameterCount())
                .filter_map(|i| {
                    let mut pi: ParameterInfo = std::mem::zeroed();
                    if !ok(c.getParameterInfo(i, &mut pi)) {
                        return None;
                    }
                    let flags = pi.flags;
                    let f = |x: i64| (flags as i64) & x != 0;
                    use ParameterInfo_::ParameterFlags_ as PF;
                    if f(PF::kIsHidden as i64) {
                        return None;
                    }
                    let value = c.getParamNormalized(pi.id);
                    let mut s: String128 = [0; 128];
                    let display = if ok(c.getParamStringByValue(pi.id, value, &mut s)) { utf16(&s) } else { format!("{value:.3}") };
                    Some(ParamInfo {
                        id: pi.id,
                        title: utf16(&pi.title),
                        units: utf16(&pi.units),
                        step_count: pi.stepCount,
                        default: pi.defaultNormalizedValue,
                        value,
                        display,
                        automatable: f(PF::kCanAutomate as i64),
                        read_only: f(PF::kIsReadOnly as i64),
                        is_bypass: f(PF::kIsBypass as i64),
                    })
                })
                .collect()
        }
    }

    fn set_param(&self, id: u32, value: f64) -> Result<(), String> {
        let v = value.clamp(0.0, 1.0);
        if let Some(c) = &self.controller {
            let _g = self.ctrl.lock().unwrap();
            if !ok(unsafe { c.setParamNormalized(id, v) }) {
                return Err(format!("{} has no parameter {id}", self.info.name));
            }
        }
        self.edits.pending.lock().unwrap().push((id, v));
        Ok(())
    }

    fn get_state(&self) -> Result<Vec<u8>, String> {
        let _g = self.ctrl.lock().unwrap();
        unsafe {
            let a = MemStream::with(&[]);
            let r = self.component.getState(a.to_com_ptr::<IBStream>().unwrap().as_ptr());
            if !ok(r) && r != kNotImplemented {
                return Err(format!("{} could not save its state", self.info.name));
            }
            let b = MemStream::with(&[]);
            if let Some(c) = &self.controller {
                c.getState(b.to_com_ptr::<IBStream>().unwrap().as_ptr());
            }
            let (a, b) = (a.bytes(), b.bytes());
            let mut out = b"VCS3".to_vec();
            out.extend((a.len() as u32).to_le_bytes());
            out.extend(&a);
            out.extend((b.len() as u32).to_le_bytes());
            out.extend(&b);
            Ok(out)
        }
    }

    fn set_state(&self, data: &[u8]) -> Result<(), String> {
        let bad = || format!("{}: not a VST3 state", self.info.name);
        if data.len() < 12 || &data[..4] != b"VCS3" {
            return Err(bad());
        }
        let n = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        let comp = data.get(8..8 + n).ok_or_else(bad)?;
        let m = u32::from_le_bytes(data.get(8 + n..12 + n).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let ctrl = data.get(12 + n..12 + n + m).ok_or_else(bad)?;
        let _p = self.proc.lock().unwrap();
        let _g = self.ctrl.lock().unwrap();
        unsafe {
            let a = MemStream::with(comp);
            let ap = a.to_com_ptr::<IBStream>().unwrap();
            if !ok(self.component.setState(ap.as_ptr())) {
                return Err(format!("{} rejected the saved state", self.info.name));
            }
            if let Some(c) = &self.controller {
                a.rewind();
                c.setComponentState(ap.as_ptr());
                if !ctrl.is_empty() {
                    let b = MemStream::with(ctrl);
                    c.setState(b.to_com_ptr::<IBStream>().unwrap().as_ptr());
                }
            }
        }
        self.edits.dirty.store(false, Relaxed);
        Ok(())
    }

    fn process(&self, buf: &mut [f32], channels: usize, pos: i64) {
        let channels = channels.max(1);
        let frames = buf.len() / channels;
        let mut p = self.proc.lock().unwrap();
        if p.next_pos.is_some_and(|n| n != pos) {
            self.restart();
        }
        if self.edits.latency_changed.swap(false, Relaxed) {
            self.latency.store(unsafe { self.processor.getLatencySamples() }, Relaxed);
        }
        let pending = std::mem::take(&mut *self.edits.pending.lock().unwrap());
        let mut changes = None;
        if !pending.is_empty() {
            // last value per parameter wins
            let mut last: Vec<(ParamID, f64)> = vec![];
            for (id, v) in pending {
                match last.iter_mut().find(|x| x.0 == id) {
                    Some(x) => x.1 = v,
                    None => last.push((id, v)),
                }
            }
            changes = Some(ComWrapper::new(Changes(
                last.into_iter().map(|(id, value)| ComWrapper::new(Queue { id, value }).to_com_ptr().unwrap()).collect(),
            )));
        }
        let mut off = 0;
        while off < frames {
            let n = (frames - off).min(self.setup.max_block);
            let ch = if off == 0 { changes.as_ref() } else { None };
            self.block(&mut p, &mut buf[off * channels..(off + n) * channels], channels, n, pos + off as i64, ch);
            off += n;
        }
        p.next_pos = Some(pos + frames as i64);
    }

    fn reset(&self) {
        let mut p = self.proc.lock().unwrap();
        self.restart();
        p.next_pos = None;
    }

    fn next_pos(&self) -> Option<i64> {
        self.proc.lock().unwrap().next_pos
    }

    fn take_dirty(&self) -> bool {
        self.edits.dirty.swap(false, Relaxed)
    }
}

impl Vst3Instance {
    /// The plugin's own editor (if it has one), for embedding in a native window.
    pub fn create_view(&self) -> Option<ComPtr<IPlugView>> {
        let c = self.controller.as_ref()?;
        let _g = self.ctrl.lock().unwrap();
        unsafe { ComPtr::from_raw(c.createView(c"editor".as_ptr())) }
    }
}

impl Drop for Vst3Instance {
    fn drop(&mut self) {
        let _p = self.proc.lock().unwrap();
        unsafe {
            self.processor.setProcessing(0);
            self.component.setActive(0);
            if let Some((a, b)) = &self.links {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(c) = &self.controller {
                c.setComponentHandler(ptr::null_mut());
                if self.separate {
                    c.terminate();
                }
            }
            self.component.terminate();
        }
        let _ = &self.handler;
    }
}

//! Realtime playback.
//!
//! ```text
//!  UI / API ──atomics──▶ render thread ──rtrb (lock-free SPSC)──▶ device callback
//!                         (vcsd-core Mixer,                       (CoreAudio/WASAPI/ALSA
//!                          resample to device rate)                via cpal, or a clocked
//!                                                                  null device)
//! ```
//! The callback never allocates or locks the project. Each block carries the
//! seek *epoch* and its timeline position, so stale audio is discarded after
//! a locate and the heard position is known exactly (minus output latency).

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering::*};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vcsd_core::mixer::{Mixer, RenderOptions, SourceCache};
use vcsd_core::wav::AudioBuffer;
use vcsd_core::ProjectState;

pub const BLOCK: usize = 256;
const CHUNK: usize = 1024;
const RING_BLOCKS: usize = 24;

#[derive(Clone, Copy)]
struct Block {
    epoch: u64,
    /// Timeline position (project samples) of each frame's source.
    pos: [i64; BLOCK],
    advance: bool,
    data: [f32; BLOCK * 2],
}

impl Default for Block {
    fn default() -> Self {
        Block { epoch: 0, pos: [0; BLOCK], advance: false, data: [0.0; BLOCK * 2] }
    }
}

fn f32_store(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Relaxed);
}
fn f32_load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Relaxed))
}

/// State shared between control, render and output threads.
pub struct Shared {
    pub playing: AtomicBool,
    /// Heard position (timeline samples, latency-compensated).
    pub position: AtomicI64,
    epoch: AtomicU64,
    seek_target: AtomicI64,
    loop_a: AtomicI64,
    loop_b: AtomicI64,
    pub dropouts: AtomicU64,
    load: AtomicU32,
    master_peak: AtomicU32,
    pub meters: Mutex<HashMap<String, f32>>,
    snapshot: Mutex<(u64, Option<Arc<ProjectState>>)>,
    snap_version: AtomicU64,
    preview: Mutex<Option<(Arc<AudioBuffer>, usize)>>,
    preview_active: AtomicBool,
    pub device_name: Mutex<String>,
    pub device_rate: AtomicU32,
    pub project_rate: AtomicU32,
    pub output_latency: AtomicI64,
    shutdown: AtomicBool,
    pub cache: SourceCache,
    /// Monotonic ms of the last locate/play (ring refill grace period).
    seek_ms: AtomicU64,
    /// Requested output device (None = system default).
    pub output_pref: Mutex<Option<String>>,
    /// Why the requested device is not the one in use, if it isn't.
    pub device_warning: Mutex<Option<String>>,
    /// Live automation values while a fader is being written (monitoring).
    overrides: Mutex<HashMap<(String, vcsd_core::AutomationParam), f32>>,
    overrides_version: AtomicU64,
}

fn now_ms() -> u64 {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis() as u64
}

impl Shared {
    /// An underrun counts as a dropout only once playback has had time to buffer.
    fn count_dropout(&self) {
        if self.playing.load(Relaxed) && now_ms().saturating_sub(self.seek_ms.load(Relaxed)) > 150 {
            self.dropouts.fetch_add(1, Relaxed);
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TransportState {
    pub playing: bool,
    pub position: i64,
    pub dropouts: u64,
    pub load: f32,
    pub master_peak: f32,
    pub meters: HashMap<String, f32>,
    pub device: String,
    pub device_rate: u32,
    pub output_latency: i64,
    pub loop_range: Option<(i64, i64)>,
    pub previewing: bool,
    pub device_warning: Option<String>,
}

enum OutputCmd {
    Open(u32),
    Shutdown,
}

pub struct Player {
    pub shared: Arc<Shared>,
    out_tx: mpsc::Sender<OutputCmd>,
}

impl Player {
    pub fn start() -> Player {
        let shared = Arc::new(Shared {
            playing: AtomicBool::new(false),
            position: AtomicI64::new(0),
            epoch: AtomicU64::new(1),
            seek_target: AtomicI64::new(0),
            loop_a: AtomicI64::new(0),
            loop_b: AtomicI64::new(0),
            dropouts: AtomicU64::new(0),
            load: AtomicU32::new(0),
            master_peak: AtomicU32::new(0),
            meters: Mutex::new(HashMap::new()),
            snapshot: Mutex::new((0, None)),
            snap_version: AtomicU64::new(0),
            preview: Mutex::new(None),
            preview_active: AtomicBool::new(false),
            device_name: Mutex::new("none".into()),
            device_rate: AtomicU32::new(48_000),
            project_rate: AtomicU32::new(48_000),
            output_latency: AtomicI64::new(0),
            shutdown: AtomicBool::new(false),
            cache: Arc::new(Mutex::new(HashMap::new())),
            seek_ms: AtomicU64::new(0),
            output_pref: Mutex::new(None),
            device_warning: Mutex::new(None),
            overrides: Mutex::new(HashMap::new()),
            overrides_version: AtomicU64::new(1),
        });
        let (prod, cons) = rtrb::RingBuffer::<Block>::new(RING_BLOCKS);
        let consumer = Arc::new(Mutex::new(Consumer { ring: cons, cur: None, idx: 0 }));
        {
            let sh = shared.clone();
            thread::Builder::new().name("vcsd-render".into()).spawn(move || render_thread(sh, prod)).unwrap();
        }
        let (out_tx, out_rx) = mpsc::channel();
        {
            let sh = shared.clone();
            thread::Builder::new().name("vcsd-output".into()).spawn(move || output_thread(sh, consumer, out_rx)).unwrap();
        }
        Player { shared, out_tx }
    }

    /// Install a new project snapshot (after every edit).
    pub fn set_project(&self, state: Option<Arc<ProjectState>>, version: u64) {
        let new_rate = state.as_ref().map(|s| s.project.sample_rate);
        *self.shared.snapshot.lock().unwrap() = (version, state);
        self.shared.snap_version.store(version, SeqCst);
        if let Some(r) = new_rate {
            if r != self.shared.project_rate.swap(r, SeqCst) || self.shared.device_name.lock().unwrap().as_str() == "none" {
                let _ = self.out_tx.send(OutputCmd::Open(r));
            }
        }
    }

    /// Switch output device (None = system default); takes effect immediately,
    /// also while playing.
    pub fn set_output_device(&self, name: Option<String>) {
        *self.shared.output_pref.lock().unwrap() = name;
        let _ = self.out_tx.send(OutputCmd::Open(self.shared.project_rate.load(SeqCst)));
    }

    /// Set (Some) or release (None) the live value of an automation lane.
    pub fn set_override(&self, owner_id: &str, param: vcsd_core::AutomationParam, value: Option<f32>) {
        let mut o = self.shared.overrides.lock().unwrap();
        match value {
            Some(v) => o.insert((owner_id.to_string(), param), v),
            None => o.remove(&(owner_id.to_string(), param)),
        };
        self.shared.overrides_version.fetch_add(1, SeqCst);
    }

    pub fn clear_overrides(&self) {
        self.shared.overrides.lock().unwrap().clear();
        self.shared.overrides_version.fetch_add(1, SeqCst);
    }

    pub fn clear_cache(&self) {
        self.shared.cache.lock().unwrap().clear();
    }

    pub fn play(&self, from: Option<i64>) {
        let at = from.unwrap_or_else(|| self.shared.position.load(SeqCst));
        self.locate(at);
        self.shared.playing.store(true, SeqCst);
    }

    pub fn stop(&self) {
        self.shared.playing.store(false, SeqCst);
        let at = self.shared.position.load(SeqCst);
        self.locate(at);
    }

    pub fn locate(&self, at: i64) {
        self.shared.seek_target.store(at, SeqCst);
        self.shared.position.store(at, SeqCst);
        self.shared.seek_ms.store(now_ms(), SeqCst);
        self.shared.epoch.fetch_add(1, SeqCst);
    }

    pub fn set_loop(&self, r: Option<(i64, i64)>) {
        let (a, b) = r.unwrap_or((0, 0));
        self.shared.loop_a.store(a, SeqCst);
        self.shared.loop_b.store(b, SeqCst);
    }

    pub fn position(&self) -> i64 {
        self.shared.position.load(SeqCst)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(SeqCst)
    }

    /// Audition a buffer (already at project rate) on top of the transport.
    pub fn preview(&self, buf: Option<Arc<AudioBuffer>>) {
        let active = buf.is_some();
        *self.shared.preview.lock().unwrap() = buf.map(|b| (b, 0));
        self.shared.preview_active.store(active, SeqCst);
    }

    pub fn state(&self) -> TransportState {
        let sh = &self.shared;
        let (a, b) = (sh.loop_a.load(Relaxed), sh.loop_b.load(Relaxed));
        let idle = !sh.playing.load(Relaxed) && !sh.preview_active.load(Relaxed);
        if idle {
            // meters fall to silence when nothing is rendering
            f32_store(&sh.master_peak, 0.0);
            if let Ok(mut m) = sh.meters.try_lock() {
                m.clear();
            }
        }
        TransportState {
            playing: sh.playing.load(Relaxed),
            position: sh.position.load(Relaxed),
            dropouts: sh.dropouts.load(Relaxed),
            load: f32_load(&sh.load),
            master_peak: f32_load(&sh.master_peak),
            meters: sh.meters.lock().map(|m| m.clone()).unwrap_or_default(),
            device: sh.device_name.lock().unwrap().clone(),
            device_rate: sh.device_rate.load(Relaxed),
            output_latency: sh.output_latency.load(Relaxed),
            loop_range: (b > a).then_some((a, b)),
            previewing: sh.preview_active.load(Relaxed),
            device_warning: sh.device_warning.lock().unwrap().clone(),
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, SeqCst);
        let _ = self.out_tx.send(OutputCmd::Shutdown);
    }
}

// ---------------------------------------------------------------- render thread

fn render_thread(sh: Arc<Shared>, mut prod: rtrb::Producer<Block>) {
    let mut local_epoch = 0u64;
    let mut src: VecDeque<([f32; 2], i64)> = VecDeque::new();
    let mut phase = 0.0f64;
    let mut rpos = 0i64;
    'outer: while !sh.shutdown.load(Relaxed) {
        let (ver, snap) = sh.snapshot.lock().unwrap().clone();
        let Some(snap) = snap else {
            thread::sleep(Duration::from_millis(10));
            continue;
        };
        let mixer = Mixer::with_cache(&snap, RenderOptions { exclude_temp_refs: false, honor_solo: true }, sh.cache.clone());
        let mut ov_seen = 0u64; // force an override sync for the new mixer
        loop {
            if sh.shutdown.load(Relaxed) {
                return;
            }
            if sh.snap_version.load(SeqCst) != ver {
                // keep position continuity, rebuild mixer on new snapshot
                continue 'outer;
            }
            let ep = sh.epoch.load(SeqCst);
            if ep != local_epoch {
                local_epoch = ep;
                rpos = sh.seek_target.load(SeqCst);
                src.clear();
                phase = 0.0;
                mixer.reset();
            }
            let ov = sh.overrides_version.load(Relaxed);
            if ov != ov_seen {
                if let Ok(o) = sh.overrides.try_lock() {
                    *mixer.overrides.borrow_mut() = o.clone();
                    ov_seen = ov;
                }
            }
            let playing = sh.playing.load(Relaxed);
            let previewing = sh.preview_active.load(Relaxed);
            if (!playing && !previewing) || prod.slots() == 0 {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            let ratio = snap.project.sample_rate as f64 / sh.device_rate.load(Relaxed).max(1) as f64;
            let mut blk = Block { epoch: local_epoch, advance: playing, ..Default::default() };
            let t0 = Instant::now();
            for i in 0..BLOCK {
                while src.len() < phase as usize + 2 {
                    // render a chunk of project-rate audio
                    let (la, lb) = (sh.loop_a.load(Relaxed), sh.loop_b.load(Relaxed));
                    let mut n = CHUNK;
                    if playing && lb > la {
                        if rpos >= lb {
                            rpos = la;
                            mixer.reset();
                        }
                        if rpos < lb {
                            n = n.min((lb - rpos) as usize);
                        }
                    }
                    let mut buf = if playing {
                        mixer.render_master(rpos, n).unwrap_or_else(|_| AudioBuffer::silent(snap.project.sample_rate, 2, n))
                    } else {
                        AudioBuffer::silent(snap.project.sample_rate, 2, n)
                    };
                    if previewing {
                        if let Ok(mut pv) = sh.preview.lock() {
                            if let Some((pb, off)) = pv.as_mut() {
                                let ch = pb.channels as usize;
                                for f in 0..n {
                                    if *off >= pb.frames() {
                                        break;
                                    }
                                    let l = pb.data[*off * ch];
                                    let r = pb.data[*off * ch + (ch > 1) as usize];
                                    buf.data[f * 2] += l;
                                    buf.data[f * 2 + 1] += r;
                                    *off += 1;
                                }
                                if *off >= pb.frames() {
                                    *pv = None;
                                    sh.preview_active.store(false, Relaxed);
                                }
                            }
                        }
                    }
                    for f in 0..n {
                        src.push_back(([buf.data[f * 2], buf.data[f * 2 + 1]], rpos + if playing { f as i64 } else { 0 }));
                    }
                    if playing {
                        rpos += n as i64;
                    }
                }
                let i0 = phase as usize;
                let fr = (phase - i0 as f64) as f32;
                let (a, pa) = src[i0];
                let (b, _) = src[i0 + 1];
                blk.data[i * 2] = a[0] + (b[0] - a[0]) * fr;
                blk.data[i * 2 + 1] = a[1] + (b[1] - a[1]) * fr;
                blk.pos[i] = pa;
                phase += ratio;
                while phase >= 1.0 {
                    src.pop_front();
                    phase -= 1.0;
                }
            }
            let dt = t0.elapsed().as_secs_f32();
            let budget = BLOCK as f32 / sh.device_rate.load(Relaxed).max(1) as f32;
            f32_store(&sh.load, f32_load(&sh.load) * 0.9 + 0.1 * (dt / budget));
            let peak = blk.data.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            f32_store(&sh.master_peak, peak.max(f32_load(&sh.master_peak) * 0.93));
            if let Ok(mut m) = sh.meters.try_lock() {
                for (k, v) in mixer.meters.borrow().iter() {
                    let e = m.entry(k.clone()).or_insert(0.0);
                    *e = v.max(*e * 0.93);
                }
            }
            if sh.epoch.load(SeqCst) == local_epoch {
                let _ = prod.push(blk);
            }
        }
    }
}

// ---------------------------------------------------------------- output

struct Consumer {
    ring: rtrb::Consumer<Block>,
    cur: Option<Block>,
    idx: usize,
}

impl Consumer {
    /// Fill interleaved f32 `out` with `ch` channels; returns false on underrun.
    fn fill(&mut self, sh: &Shared, out: &mut [f32], ch: usize, latency: i64) -> bool {
        let ep = sh.epoch.load(Acquire);
        let mut ok = true;
        let mut last: Option<i64> = None;
        for frame in out.chunks_mut(ch.max(1)) {
            if self.cur.as_ref().is_none_or(|b| self.idx >= BLOCK || b.epoch != ep) {
                self.cur = None;
                while let Ok(b) = self.ring.pop() {
                    if b.epoch == ep {
                        self.cur = Some(b);
                        self.idx = 0;
                        break;
                    }
                }
            }
            match &self.cur {
                Some(b) => {
                    let (l, r) = (b.data[self.idx * 2], b.data[self.idx * 2 + 1]);
                    match frame.len() {
                        1 => frame[0] = (l + r) * 0.5,
                        _ => {
                            frame[0] = l;
                            frame[1] = r;
                            for x in frame.iter_mut().skip(2) {
                                *x = 0.0;
                            }
                        }
                    }
                    if b.advance {
                        last = Some(b.pos[self.idx]);
                    }
                    self.idx += 1;
                }
                None => {
                    frame.iter_mut().for_each(|x| *x = 0.0);
                    ok = false;
                }
            }
        }
        if let Some(p) = last {
            if sh.playing.load(Relaxed) && sh.epoch.load(Relaxed) == ep {
                sh.position.store(p - latency, Release);
            }
        }
        ok
    }
}

fn output_thread(sh: Arc<Shared>, consumer: Arc<Mutex<Consumer>>, rx: mpsc::Receiver<OutputCmd>) {
    let mut _stream: Option<cpal::Stream> = None;
    let null_running = Arc::new(AtomicBool::new(false));
    while let Ok(cmd) = rx.recv() {
        match cmd {
            OutputCmd::Shutdown => break,
            OutputCmd::Open(rate) => {
                _stream = None;
                null_running.store(false, SeqCst);
                thread::sleep(Duration::from_millis(15));
                match open_device(&sh, &consumer, rate) {
                    Ok(s) => _stream = Some(s),
                    Err(e) => {
                        *sh.device_name.lock().unwrap() = format!("null (no audio device: {e})");
                        sh.device_rate.store(rate, SeqCst);
                        sh.output_latency.store(0, SeqCst);
                        null_running.store(true, SeqCst);
                        let (sh2, c2, run) = (sh.clone(), consumer.clone(), null_running.clone());
                        thread::spawn(move || null_device(sh2, c2, run));
                    }
                }
            }
        }
    }
    null_running.store(false, SeqCst);
}

/// Software clock that consumes audio in real time when no device exists
/// (headless servers, CI): the transport still runs at the right speed.
fn null_device(sh: Arc<Shared>, consumer: Arc<Mutex<Consumer>>, run: Arc<AtomicBool>) {
    let rate = sh.device_rate.load(SeqCst) as f64;
    let start = Instant::now();
    let mut done = 0u64;
    let mut buf = vec![0.0f32; 2 * 4096];
    while run.load(Relaxed) && !sh.shutdown.load(Relaxed) {
        thread::sleep(Duration::from_millis(5));
        let due = (start.elapsed().as_secs_f64() * rate) as u64;
        let n = ((due - done) as usize).min(4096);
        done += n as u64;
        let ok = consumer.lock().unwrap().fill(&sh, &mut buf[..n * 2], 2, 0);
        if !ok {
            sh.count_dropout();
        }
    }
}

fn open_device(sh: &Arc<Shared>, consumer: &Arc<Mutex<Consumer>>, rate: u32) -> Result<cpal::Stream, String> {
    let pref = sh.output_pref.lock().unwrap().clone();
    let (dev, warning) = crate::devices::find_output(pref.as_deref()).inspect_err(|e| {
        *sh.device_warning.lock().unwrap() = pref.as_ref().map(|p| format!("'{p}' unavailable: {e}"));
    })?;
    *sh.device_warning.lock().unwrap() = warning;
    let name = crate::devices::device_name(&dev);
    let mut chosen = None;
    if let Ok(cfgs) = dev.supported_output_configs() {
        let mut cands: Vec<_> = cfgs.collect();
        cands.sort_by_key(|c| (c.sample_format() != cpal::SampleFormat::F32, c.channels() < 2, c.channels()));
        for c in cands {
            if matches!(c.sample_format(), cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::I32) {
                if let Some(sc) = c.try_with_sample_rate(rate) {
                    chosen = Some(sc);
                    break;
                }
            }
        }
    }
    let cfg = match chosen {
        Some(c) => c,
        None => dev.default_output_config().map_err(|e| e.to_string())?,
    };
    if cfg.sample_rate() != rate {
        let w = format!("device runs at {} Hz; project is {rate} Hz (resampled for monitoring only)", cfg.sample_rate());
        let mut dw = sh.device_warning.lock().unwrap();
        *dw = Some(match dw.take() {
            Some(x) => format!("{x}; {w}"),
            None => w,
        });
    }
    sh.device_rate.store(cfg.sample_rate(), SeqCst);
    *sh.device_name.lock().unwrap() = format!("{name} ({} Hz, {} ch)", cfg.sample_rate(), cfg.channels());
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&dev, cfg.config(), sh.clone(), consumer.clone()),
        cpal::SampleFormat::I16 => build::<i16>(&dev, cfg.config(), sh.clone(), consumer.clone()),
        cpal::SampleFormat::I32 => build::<i32>(&dev, cfg.config(), sh.clone(), consumer.clone()),
        f => Err(format!("unsupported device sample format {f:?}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build<T: SizedSample + FromSample<f32> + Send + 'static>(
    dev: &cpal::Device,
    cfg: cpal::StreamConfig,
    sh: Arc<Shared>,
    consumer: Arc<Mutex<Consumer>>,
) -> Result<cpal::Stream, String> {
    let ch = cfg.channels as usize;
    let mut scratch: Vec<f32> = vec![0.0; 8192 * ch.max(1)];
    let dev_rate = cfg.sample_rate as f64;
    dev.build_output_stream::<T, _, _>(
        cfg,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            let ts = info.timestamp();
            let lat_dev = ts.playback.duration_since(ts.callback).as_secs_f64();
            let proj_rate = sh.project_rate.load(Relaxed) as f64;
            let latency = (lat_dev * proj_rate) as i64;
            sh.output_latency.store(latency, Relaxed);
            let _ = dev_rate;
            if scratch.len() < data.len() {
                scratch.resize(data.len(), 0.0); // only if the host grows the buffer
            }
            let out = &mut scratch[..data.len()];
            let ok = match consumer.try_lock() {
                Ok(mut c) => c.fill(&sh, out, ch, latency),
                Err(_) => {
                    out.iter_mut().for_each(|x| *x = 0.0);
                    false
                }
            };
            if !ok {
                sh.count_dropout();
            }
            for (d, s) in data.iter_mut().zip(out.iter()) {
                *d = T::from_sample(*s);
            }
        },
        |e| eprintln!("audio stream error: {e}"),
        None,
    )
    .map_err(|e| e.to_string())
}

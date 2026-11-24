//! Live input capture for Foley takes (spec §11.2). The input stream lives on
//! its own thread; samples cross a lock-free ring to a collector. The take is
//! stamped with the timeline position the performer was *hearing* when the
//! first sample was captured, minus measured input latency.

use crate::player::Shared;
use cpal::traits::{DeviceTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering::*};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

pub struct Captured {
    pub data: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    /// Timeline sample of the first captured sample (latency-compensated).
    pub start_pos: i64,
    pub device: String,
}

pub struct Recording {
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<Result<Captured, String>>,
    pub device: String,
    pub cue_id: Option<String>,
    pub track_id: Option<String>,
}

impl Recording {
    pub fn finish(self) -> Result<Captured, String> {
        self.stop.store(true, SeqCst);
        self.handle.join().map_err(|_| "recording thread panicked".to_string())?
    }
}

pub fn start(
    sh: Arc<Shared>,
    rate: u32,
    device: Option<String>,
    cue_id: Option<String>,
    track_id: Option<String>,
) -> Result<Recording, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel::<Result<String, String>>();
    let stop2 = stop.clone();
    let handle = thread::Builder::new()
        .name("vcsd-record".into())
        .spawn(move || -> Result<Captured, String> {
            let dev = match crate::devices::find_input(device.as_deref()) {
                Ok(d) => d,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.clone()));
                    return Err(e);
                }
            };
            let name = crate::devices::device_name(&dev);
            let cfg = dev
                .supported_input_configs()
                .ok()
                .and_then(|c| {
                    let mut v: Vec<_> = c.filter(|c| c.sample_format() == cpal::SampleFormat::F32).collect();
                    v.sort_by_key(|c| c.channels());
                    v.into_iter().find_map(|c| c.try_with_sample_rate(rate))
                })
                .ok_or_else(|| format!("input device cannot capture 32-bit float at {rate} Hz"));
            let cfg = match cfg {
                Ok(c) => c,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.clone()));
                    return Err(e);
                }
            };
            let ch = cfg.channels();
            let (mut prod, mut cons) = rtrb::RingBuffer::<f32>::new(rate as usize * ch as usize * 4);
            let start_pos = Arc::new(AtomicI64::new(i64::MIN));
            let (sp, shc) = (start_pos.clone(), sh.clone());
            let stream = dev.build_input_stream::<f32, _, _>(
                cfg.config(),
                move |data: &[f32], info: &cpal::InputCallbackInfo| {
                    if sp.load(Relaxed) == i64::MIN {
                        let ts = info.timestamp();
                        let lat = ts.callback.duration_since(ts.capture).as_secs_f64();
                        sp.store(shc.position.load(Relaxed) - (lat * rate as f64) as i64, Relaxed);
                    }
                    for &v in data {
                        let _ = prod.push(v);
                    }
                },
                |e| eprintln!("input stream error: {e}"),
                None,
            );
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return Err(e.to_string());
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(e.to_string()));
                return Err(e.to_string());
            }
            let _ = ready_tx.send(Ok(name.clone()));
            let mut data = Vec::with_capacity(rate as usize * ch as usize * 30);
            // A take holds at most real time's worth of audio (plus a second): a device
            // with no clock of its own (ALSA's `null`, a broken driver) can deliver as
            // fast as the CPU allows. Only what is waiting is drained each pass, so a
            // stop is seen even then.
            let began = Instant::now();
            let per_sec = rate as f64 * ch as f64;
            let take = |cons: &mut rtrb::Consumer<f32>, data: &mut Vec<f32>| {
                // whole frames only
                let room = ((began.elapsed().as_secs_f64() + 1.0) * per_sec) as usize / ch as usize * ch as usize;
                for _ in 0..cons.slots() {
                    let Ok(v) = cons.pop() else { break };
                    if data.len() < room {
                        data.push(v);
                    }
                }
            };
            loop {
                take(&mut cons, &mut data);
                if stop2.load(SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            drop(stream);
            take(&mut cons, &mut data);
            let sp = start_pos.load(SeqCst);
            Ok(Captured {
                data,
                channels: ch,
                sample_rate: rate,
                start_pos: if sp == i64::MIN { sh.position.load(SeqCst) } else { sp },
                device: name,
            })
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(device)) => Ok(Recording { stop, handle, device, cue_id, track_id }),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => {
            stop.store(true, SeqCst);
            Err("input device did not start".into())
        }
    }
}

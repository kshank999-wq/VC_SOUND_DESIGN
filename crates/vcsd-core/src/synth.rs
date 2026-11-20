#![allow(clippy::needless_range_loop)]
//! Procedural starter sound pack. Every sound is generated deterministically
//! by this code, so the pack is owned outright and redistributable — a safe
//! default placeholder library (spec §9, §22 "placeholder library licensing").

use crate::error::Result;
use crate::wav::{AudioBuffer, SampleFormat};
use std::f32::consts::PI;
use std::path::Path;

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407))
    }
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    }
}

fn mono(sr: u32, secs: f32) -> AudioBuffer {
    AudioBuffer::silent(sr, 1, (sr as f32 * secs) as usize)
}

/// One-pole lowpass in place.
fn lowpass(x: &mut [f32], sr: u32, hz: f32) {
    let a = (-2.0 * PI * hz / sr as f32).exp();
    let mut y = 0.0;
    for v in x.iter_mut() {
        y = (1.0 - a) * *v + a * y;
        *v = y;
    }
}

fn normalize(b: &mut AudioBuffer, peak: f32) {
    let p = b.peak();
    if p > 0.0 {
        b.data.iter_mut().for_each(|v| *v *= peak / p);
    }
}

fn burst(out: &mut [f32], sr: u32, at: f32, decay_s: f32, amp: f32, rng: &mut Rng, lp_hz: f32) {
    let start = (at * sr as f32) as usize;
    let len = ((decay_s * 6.0) * sr as f32) as usize;
    let mut tmp: Vec<f32> = (0..len).map(|i| rng.next() * (-(i as f32) / (decay_s * sr as f32)).exp() * amp).collect();
    lowpass(&mut tmp, sr, lp_hz);
    for (i, v) in tmp.into_iter().enumerate() {
        if let Some(o) = out.get_mut(start + i) {
            *o += v;
        }
    }
}

/// Footstep: heel then toe transient. `surface` 0 = gravel, 1 = wood, 2 = concrete.
pub fn footstep(sr: u32, surface: u32, variant: u32) -> AudioBuffer {
    let mut b = mono(sr, 0.35);
    let mut r = Rng::new(1000 + surface as u64 * 97 + variant as u64);
    let (lp, dec, grit) = match surface {
        0 => (5200.0, 0.018, 0.5),
        1 => (1800.0, 0.012, 0.0),
        _ => (3500.0, 0.008, 0.1),
    };
    let jitter = 0.004 * variant as f32;
    burst(&mut b.data, sr, 0.0, dec, 1.0, &mut r, lp);
    burst(&mut b.data, sr, 0.07 + jitter, dec * 0.8, 0.6, &mut r, lp * 1.2);
    if grit > 0.0 {
        for k in 0..12 {
            burst(&mut b.data, sr, 0.01 + 0.012 * k as f32 + r.next().abs() * 0.01, 0.003, grit * 0.3, &mut r, 7000.0);
        }
    }
    if surface == 1 {
        let n = b.frames();
        for i in 0..n.min(sr as usize / 8) {
            let t = i as f32 / sr as f32;
            b.data[i] += (2.0 * PI * 140.0 * t).sin() * (-t * 30.0).exp() * 0.5;
        }
    }
    normalize(&mut b, 0.7);
    b
}

/// Metallic door-handle rattle: several damped partial clusters.
pub fn handle_rattle(sr: u32, variant: u32) -> AudioBuffer {
    let mut b = mono(sr, 0.7);
    let mut r = Rng::new(2000 + variant as u64);
    let hits = 5 + variant as usize;
    for h in 0..hits {
        let at = 0.02 + h as f32 * 0.11 + r.next().abs() * 0.03;
        let s0 = (at * sr as f32) as usize;
        for (k, f) in [1830.0f32, 2710.0, 4120.0, 5390.0].iter().enumerate() {
            let f = f * (1.0 + 0.02 * r.next());
            for i in 0..(sr as usize / 10) {
                if let Some(o) = b.data.get_mut(s0 + i) {
                    let t = i as f32 / sr as f32;
                    *o += (2.0 * PI * f * t).sin() * (-t * (40.0 + k as f32 * 15.0)).exp() * 0.25;
                }
            }
        }
        burst(&mut b.data, sr, at, 0.003, 0.4, &mut r, 6000.0);
    }
    normalize(&mut b, 0.7);
    b
}

pub fn glass_smash(sr: u32, variant: u32) -> AudioBuffer {
    let mut b = mono(sr, 1.8);
    let mut r = Rng::new(3000 + variant as u64);
    burst(&mut b.data, sr, 0.0, 0.05, 1.0, &mut r, 9000.0);
    for _ in 0..60 {
        let at = r.next().abs() * 1.2;
        let f = 2500.0 + r.next().abs() * 6000.0;
        let s0 = (at * sr as f32) as usize;
        let amp = 0.15 * (1.0 - at / 1.4).max(0.05);
        for i in 0..(sr as usize / 12) {
            if let Some(o) = b.data.get_mut(s0 + i) {
                let t = i as f32 / sr as f32;
                *o += (2.0 * PI * f * t).sin() * (-t * 60.0).exp() * amp;
            }
        }
    }
    normalize(&mut b, 0.8);
    b
}

/// Seamless-ish stereo bird ambience loop.
pub fn birds_loop(sr: u32, secs: f32) -> AudioBuffer {
    let n = (sr as f32 * secs) as usize;
    let mut b = AudioBuffer::silent(sr, 2, n);
    let mut r = Rng::new(4000);
    let mut bed: Vec<f32> = (0..n).map(|_| r.next() * 0.02).collect();
    lowpass(&mut bed, sr, 900.0);
    for i in 0..n {
        b.data[i * 2] = bed[i];
        b.data[i * 2 + 1] = bed[(i + 977) % n];
    }
    let chirps = (secs * 3.0) as usize;
    for _ in 0..chirps {
        let at = r.next().abs() * (secs - 0.3);
        let f0 = 2800.0 + r.next().abs() * 2200.0;
        let pan = r.next();
        let s0 = (at * sr as f32) as usize;
        let len = (0.08 + r.next().abs() * 0.12) * sr as f32;
        let mut ph = 0.0f32;
        for i in 0..len as usize {
            let t = i as f32 / len;
            let f = f0 * (1.0 + 0.35 * (PI * t).sin());
            ph += 2.0 * PI * f / sr as f32;
            let v = ph.sin() * (PI * t).sin().powi(2) * 0.25;
            if s0 + i < n {
                b.data[(s0 + i) * 2] += v * (1.0 - pan) * 0.5;
                b.data[(s0 + i) * 2 + 1] += v * (1.0 + pan) * 0.5;
            }
        }
    }
    normalize(&mut b, 0.5);
    b
}

pub fn wind_loop(sr: u32, secs: f32) -> AudioBuffer {
    let n = (sr as f32 * secs) as usize;
    let mut b = AudioBuffer::silent(sr, 2, n);
    for ch in 0..2 {
        let mut r = Rng::new(5000 + ch as u64);
        let mut x: Vec<f32> = (0..n).map(|_| r.next()).collect();
        lowpass(&mut x, sr, 400.0);
        lowpass(&mut x, sr, 600.0);
        for i in 0..n {
            let t = i as f32 / sr as f32;
            let gust = 0.6 + 0.4 * (2.0 * PI * t / secs * 2.0 + ch as f32).sin();
            b.data[i * 2 + ch] = x[i] * gust;
        }
    }
    normalize(&mut b, 0.4);
    b
}

pub fn room_tone(sr: u32, secs: f32) -> AudioBuffer {
    let n = (sr as f32 * secs) as usize;
    let mut b = AudioBuffer::silent(sr, 2, n);
    for ch in 0..2 {
        let mut r = Rng::new(6000 + ch as u64);
        let mut x: Vec<f32> = (0..n).map(|_| r.next()).collect();
        lowpass(&mut x, sr, 250.0);
        for i in 0..n {
            let t = i as f32 / sr as f32;
            b.data[i * 2 + ch] = x[i] + (2.0 * PI * 60.0 * t).sin() * 0.02;
        }
    }
    normalize(&mut b, 0.1);
    b
}

/// Stereo car pass-by: engine harmonics + tyre noise with Doppler-ish pitch and pan sweep.
pub fn car_pass(sr: u32, secs: f32) -> AudioBuffer {
    let n = (sr as f32 * secs) as usize;
    let mut b = AudioBuffer::silent(sr, 2, n);
    let mut r = Rng::new(7000);
    let mut noise: Vec<f32> = (0..n).map(|_| r.next()).collect();
    lowpass(&mut noise, sr, 1200.0);
    let mut ph = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let x = (t - 0.5) * 4.0;
        let amp = 1.0 / (1.0 + x * x);
        let f = 95.0 * (1.0 - 0.12 * x.atan() / (PI / 2.0));
        ph += 2.0 * PI * f / sr as f32;
        let v = (ph.sin() + 0.5 * (2.0 * ph).sin() + 0.3 * (3.0 * ph).sin()) * 0.3 + noise[i] * 0.8;
        let pan = (x / 2.0).tanh();
        b.data[i * 2] = v * amp * (1.0 - pan) * 0.5;
        b.data[i * 2 + 1] = v * amp * (1.0 + pan) * 0.5;
    }
    normalize(&mut b, 0.7);
    b
}

/// Write the starter pack (folders per category + `pack.json`).
pub fn write_starter_pack(dir: &Path, sr: u32) -> Result<()> {
    let w = |rel: &str, b: &AudioBuffer| -> Result<()> {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap())?;
        crate::wav::write(&p, b, SampleFormat::Int24, None)
    };
    for v in 0..4 {
        w(&format!("footsteps/gravel/boot_gravel_{:02}.wav", v + 1), &footstep(sr, 0, v))?;
        w(&format!("footsteps/wood/shoe_wood_{:02}.wav", v + 1), &footstep(sr, 1, v))?;
        w(&format!("footsteps/concrete/heel_concrete_{:02}.wav", v + 1), &footstep(sr, 2, v))?;
    }
    for v in 0..3 {
        w(&format!("doors/handle/door_handle_rattle_{:02}.wav", v + 1), &handle_rattle(sr, v))?;
    }
    w("glass/glass_smash_01.wav", &glass_smash(sr, 0))?;
    w("glass/glass_smash_02.wav", &glass_smash(sr, 1))?;
    w("ambience/birds_morning_loop.wav", &birds_loop(sr, 8.0))?;
    w("ambience/wind_exterior_loop.wav", &wind_loop(sr, 8.0))?;
    w("ambience/room_tone_interior_loop.wav", &room_tone(sr, 6.0))?;
    w("vehicles/car_pass_by.wav", &car_pass(sr, 4.0))?;
    std::fs::write(
        dir.join("pack.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "name": "VC Starter Pack (procedural)",
            "version": "1.0",
            "license": {"name": "VC Sound Design procedural starter pack", "redistributable": true, "source": "generated by vcsd-core synth", "attribution": ""}
        }))?,
    )?;
    Ok(())
}

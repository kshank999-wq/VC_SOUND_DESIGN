//! Deterministic transport and block-based playback (spec §12, §19 WP13).
//!
//! The transport is a sample-clock state machine; audio output is pulled in
//! fixed blocks through [`AudioDevice`]. Device back-ends (CoreAudio, WASAPI,
//! ASIO via a native host) implement the trait on a realtime thread; this
//! module stays allocation-light and testable without hardware.

use crate::error::Result;
use crate::mixer::{Mixer, RenderOptions};
use crate::state::ProjectState;
use crate::wav::AudioBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Stopped,
    Playing,
    /// Playing with input armed tracks capturing.
    Recording,
}

#[derive(Debug, Clone)]
pub struct Transport {
    pub position: i64,
    pub state: PlayState,
    pub loop_range: Option<(i64, i64)>,
    pub preroll: i64,
    /// Rehearsal speed (1.0 = normal). Varispeed playback is for rehearsal only;
    /// captured positions are always in timeline samples.
    pub speed: f32,
    pub block_size: usize,
    /// Output latency reported by the device, used to align the picture.
    pub output_latency: i64,
    /// Input latency, used to compensate recordings/performances.
    pub input_latency: i64,
    pub dropouts: u64,
}

impl Transport {
    pub fn new(block_size: usize) -> Self {
        Self {
            position: 0,
            state: PlayState::Stopped,
            loop_range: None,
            preroll: 0,
            speed: 1.0,
            block_size,
            output_latency: 0,
            input_latency: 0,
            dropouts: 0,
        }
    }

    pub fn play_from(&mut self, at: i64) {
        self.position = at - self.preroll;
        self.state = PlayState::Playing;
    }
    pub fn stop(&mut self) {
        self.state = PlayState::Stopped;
    }
    pub fn locate(&mut self, at: i64) {
        self.position = at;
    }

    /// Step whole frames (negative = backwards), landing on frame boundaries.
    pub fn step_frames(&mut self, s: &ProjectState, n: i64) {
        let tb = s.tb();
        self.position = tb.sample_at_frame(tb.frame_at(self.position) + n);
    }

    /// Position the picture should display for the audio currently heard.
    pub fn picture_position(&self) -> i64 {
        self.position - self.output_latency
    }

    /// Timeline sample at which an input event captured *now* actually happened.
    pub fn compensate_input(&self, captured: i64) -> i64 {
        captured - self.input_latency
    }

    /// Render the next block and advance. Loops wrap sample-accurately.
    pub fn next_block(&mut self, mixer: &Mixer) -> Result<AudioBuffer> {
        let n = self.block_size;
        if self.state == PlayState::Stopped {
            return Ok(AudioBuffer::silent(mixer.s.project.sample_rate, 2, n));
        }
        let mut out = AudioBuffer::silent(mixer.s.project.sample_rate, 2, 0);
        let mut remaining = n as i64;
        while remaining > 0 {
            let take = match self.loop_range {
                Some((_, z)) if self.position < z => remaining.min(z - self.position),
                _ => remaining,
            };
            let b = mixer.render_master(self.position, take as usize)?;
            out.data.extend_from_slice(&b.data);
            self.position += take;
            remaining -= take;
            if let Some((a, z)) = self.loop_range {
                if self.position >= z {
                    self.position = a;
                    mixer.reset();
                }
            }
        }
        Ok(out)
    }
}

/// Realtime device back-end contract.
pub trait AudioDevice {
    fn name(&self) -> String;
    fn sample_rate(&self) -> u32;
    fn output_latency(&self) -> i64;
    fn input_latency(&self) -> i64;
    /// Deliver one block; returns false on underrun (counted as a dropout).
    fn write_block(&mut self, block: &AudioBuffer) -> bool;
}

/// Offline device that collects output — used for tests and bounce.
pub struct CaptureDevice {
    pub sr: u32,
    pub captured: AudioBuffer,
}

impl CaptureDevice {
    pub fn new(sr: u32) -> Self {
        Self { sr, captured: AudioBuffer::silent(sr, 2, 0) }
    }
}

impl AudioDevice for CaptureDevice {
    fn name(&self) -> String {
        "capture".into()
    }
    fn sample_rate(&self) -> u32 {
        self.sr
    }
    fn output_latency(&self) -> i64 {
        0
    }
    fn input_latency(&self) -> i64 {
        0
    }
    fn write_block(&mut self, block: &AudioBuffer) -> bool {
        self.captured.data.extend_from_slice(&block.data);
        true
    }
}

/// Play `[from, to)` through a device; returns blocks delivered.
pub fn play_range(s: &ProjectState, dev: &mut dyn AudioDevice, from: i64, to: i64, block: usize) -> Result<usize> {
    let mixer = Mixer::new(s, RenderOptions::default());
    let mut t = Transport::new(block);
    t.output_latency = dev.output_latency();
    t.play_from(from);
    let mut blocks = 0;
    while t.position < to {
        t.block_size = block.min((to - t.position) as usize);
        let b = t.next_block(&mixer)?;
        if !dev.write_block(&b) {
            t.dropouts += 1;
        }
        blocks += 1;
    }
    Ok(blocks)
}

/// Rough engine load figures for the performance meters.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PerfStats {
    pub tracks: usize,
    pub clips: usize,
    pub assets: usize,
    pub decoded_mb_estimate: f64,
    /// Seconds of audio rendered per wall-clock second (>1 is faster than realtime).
    pub realtime_factor: f64,
}

pub fn perf_probe(s: &ProjectState, seconds: f64) -> Result<PerfStats> {
    let mixer = Mixer::new(s, RenderOptions::default());
    let (a, z) = crate::mixer::timeline_extent(s);
    let n = ((seconds * s.project.sample_rate as f64) as i64).min((z - a).max(1)) as usize;
    let t0 = std::time::Instant::now();
    mixer.render_master(a, n)?;
    let dt = t0.elapsed().as_secs_f64().max(1e-6);
    let bytes: f64 = s.assets.values().map(|x| x.length_samples as f64 * x.channels as f64 * 4.0).sum();
    Ok(PerfStats {
        tracks: s.tracks.len(),
        clips: s.clips.len(),
        assets: s.assets.len(),
        decoded_mb_estimate: bytes / 1_048_576.0,
        realtime_factor: (n as f64 / s.project.sample_rate as f64) / dt,
    })
}

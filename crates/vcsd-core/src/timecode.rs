//! Rational frame rates, SMPTE timecode (DF/NDF) and the sample-clock timebase.
//!
//! Timeline truth is an integer sample position. Frames are mapped with exact
//! integer arithmetic on the rational rate (`num/den`) so there is no
//! floating-point drift at any session length.
//!
//! Frame `f` covers samples `[ceil(f*k), ceil((f+1)*k))` with
//! `k = sample_rate*den/num`, so `frame_at_sample(sample_at_frame(f)) == f`
//! for every frame, even when `k` is fractional (29.97 @ 48 kHz).

use crate::error::{invalid, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
    pub drop_frame: bool,
}

impl FrameRate {
    pub const FPS_23_976: Self = Self { num: 24_000, den: 1001, drop_frame: false };
    pub const FPS_24: Self = Self { num: 24, den: 1, drop_frame: false };
    pub const FPS_25: Self = Self { num: 25, den: 1, drop_frame: false };
    pub const FPS_29_97_NDF: Self = Self { num: 30_000, den: 1001, drop_frame: false };
    pub const FPS_29_97_DF: Self = Self { num: 30_000, den: 1001, drop_frame: true };
    pub const FPS_30: Self = Self { num: 30, den: 1, drop_frame: false };
    pub const FPS_50: Self = Self { num: 50, den: 1, drop_frame: false };
    pub const FPS_59_94_NDF: Self = Self { num: 60_000, den: 1001, drop_frame: false };
    pub const FPS_59_94_DF: Self = Self { num: 60_000, den: 1001, drop_frame: true };
    pub const FPS_60: Self = Self { num: 60, den: 1, drop_frame: false };

    pub fn new(num: u32, den: u32, drop_frame: bool) -> Result<Self> {
        if num == 0 || den == 0 {
            return Err(invalid("frame rate numerator/denominator must be > 0"));
        }
        let r = Self { num, den, drop_frame };
        if drop_frame && !(den == 1001 && r.nominal().is_multiple_of(30)) {
            return Err(invalid(format!("drop-frame is only defined for 29.97/59.94, not {}", r.label())));
        }
        Ok(r)
    }

    /// Integer frame count per timecode second (24 for 23.976, 30 for 29.97).
    pub fn nominal(&self) -> u32 {
        (self.num + self.den / 2) / self.den
    }

    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Frames dropped per minute in DF counting (2 for 29.97, 4 for 59.94).
    fn drop_count(&self) -> i64 {
        if self.drop_frame {
            self.nominal() as i64 / 15
        } else {
            0
        }
    }

    pub fn label(&self) -> String {
        let base = if self.den == 1 {
            format!("{}", self.num)
        } else {
            let v = self.as_f64();
            let s = format!("{v:.3}");
            let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
            s
        };
        if self.den == 1001 && self.nominal().is_multiple_of(30) {
            format!("{base} {}", if self.drop_frame { "DF" } else { "NDF" })
        } else {
            base
        }
    }

    /// Parse labels such as `23.976`, `24`, `25`, `29.97df`, `29.97 NDF`,
    /// `30`, `59.94DF`, or an explicit rational `24000/1001`.
    pub fn parse(s: &str) -> Result<Self> {
        let t = s.trim().to_ascii_lowercase().replace([' ', '_', '-'], "");
        let (body, df) = if let Some(b) = t.strip_suffix("ndf") {
            (b.to_string(), Some(false))
        } else if let Some(b) = t.strip_suffix("df") {
            (b.to_string(), Some(true))
        } else {
            (t.clone(), None)
        };
        let df = df.unwrap_or(false);
        if let Some((n, d)) = body.split_once('/') {
            let n: u32 = n.parse().map_err(|_| invalid(format!("bad frame rate '{s}'")))?;
            let d: u32 = d.parse().map_err(|_| invalid(format!("bad frame rate '{s}'")))?;
            return Self::new(n, d, df);
        }
        let r = match body.as_str() {
            "23.976" | "23.98" => Self::FPS_23_976,
            "24" => Self::FPS_24,
            "25" => Self::FPS_25,
            "29.97" => Self::FPS_29_97_NDF,
            "30" => Self::FPS_30,
            "50" => Self::FPS_50,
            "59.94" => Self::FPS_59_94_NDF,
            "60" => Self::FPS_60,
            _ => return Err(invalid(format!("unsupported frame rate '{s}'"))),
        };
        Self::new(r.num, r.den, df)
    }

    /// First sample of frame `frame` (frames relative to some origin).
    pub fn sample_at_frame(&self, frame: i64, sample_rate: u32) -> i64 {
        let n = frame as i128 * sample_rate as i128 * self.den as i128;
        let d = self.num as i128;
        // ceiling division that is correct for negative numerators
        (-((-n).div_euclid(d))) as i64
    }

    /// Frame containing `sample`.
    pub fn frame_at_sample(&self, sample: i64, sample_rate: u32) -> i64 {
        let n = sample as i128 * self.num as i128;
        let d = sample_rate as i128 * self.den as i128;
        n.div_euclid(d) as i64
    }

    /// Convert a frame count (since 00:00:00:00) into a timecode label.
    pub fn frames_to_tc(&self, frames: i64) -> String {
        let neg = frames < 0;
        let mut f = frames.abs();
        let n = self.nominal() as i64;
        if self.drop_frame {
            let d = self.drop_count();
            let fp10 = n * 600 - d * 9;
            let fpm = n * 60 - d;
            let tens = f / fp10;
            let m = f % fp10;
            f += if m > d { d * 9 * tens + d * ((m - d) / fpm) } else { d * 9 * tens };
        }
        let ff = f % n;
        let ss = (f / n) % 60;
        let mm = (f / (n * 60)) % 60;
        let hh = f / (n * 3600);
        let sep = if self.drop_frame { ';' } else { ':' };
        format!("{}{hh:02}:{mm:02}:{ss:02}{sep}{ff:02}", if neg { "-" } else { "" })
    }

    /// Parse `HH:MM:SS:FF` (or `;`/`.` separators) into frames since midnight.
    pub fn tc_to_frames(&self, tc: &str) -> Result<i64> {
        let t = tc.trim();
        let (neg, body) = match t.strip_prefix('-') {
            Some(b) => (true, b),
            None => (false, t),
        };
        let parts: Vec<&str> = body.split([':', ';', '.']).collect();
        if parts.len() != 4 {
            return Err(invalid(format!("timecode '{tc}' must be HH:MM:SS:FF")));
        }
        let mut v = [0i64; 4];
        for (i, p) in parts.iter().enumerate() {
            v[i] = p.parse().map_err(|_| invalid(format!("timecode '{tc}' has a non-numeric field")))?;
        }
        let [h, m, s, f] = v;
        let n = self.nominal() as i64;
        if m >= 60 || s >= 60 || f >= n || h < 0 || m < 0 || s < 0 || f < 0 {
            return Err(invalid(format!("timecode '{tc}' out of range for {}", self.label())));
        }
        let mut frames = (h * 3600 + m * 60 + s) * n + f;
        if self.drop_frame {
            let d = self.drop_count();
            if s == 0 && m % 10 != 0 && f < d {
                return Err(invalid(format!("timecode '{tc}' does not exist in drop-frame counting")));
            }
            let total_min = h * 60 + m;
            frames -= d * (total_min - total_min / 10);
        }
        Ok(if neg { -frames } else { frames })
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// Maps between timeline samples and absolute timecode. Timeline sample 0 is
/// the project/sequence start timecode (`start_frame`, frames since midnight).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeBase {
    pub rate: FrameRate,
    pub sample_rate: u32,
    pub start_frame: i64,
}

impl TimeBase {
    pub fn new(rate: FrameRate, sample_rate: u32, start_frame: i64) -> Self {
        Self { rate, sample_rate, start_frame }
    }

    /// Absolute frame (since midnight) containing timeline sample `s`.
    pub fn frame_at(&self, s: i64) -> i64 {
        self.start_frame + self.rate.frame_at_sample(s, self.sample_rate)
    }

    /// First timeline sample of absolute frame `frame`.
    pub fn sample_at_frame(&self, frame: i64) -> i64 {
        self.rate.sample_at_frame(frame - self.start_frame, self.sample_rate)
    }

    pub fn tc_at(&self, s: i64) -> String {
        self.rate.frames_to_tc(self.frame_at(s))
    }

    pub fn sample_at_tc(&self, tc: &str) -> Result<i64> {
        Ok(self.sample_at_frame(self.rate.tc_to_frames(tc)?))
    }

    /// Snap a sample to the first sample of the frame that contains it.
    pub fn snap(&self, s: i64) -> i64 {
        self.sample_at_frame(self.frame_at(s))
    }

    /// Samples per frame (rounded) — for display and heuristics only.
    pub fn samples_per_frame(&self) -> f64 {
        self.sample_rate as f64 / self.rate.as_f64()
    }

    /// Absolute sample since midnight of timeline sample 0 (BWF time reference origin).
    pub fn origin_since_midnight(&self) -> i64 {
        self.rate.sample_at_frame(self.start_frame, self.sample_rate)
    }

    /// Duration label in timecode form (length counted from 00:00:00:00).
    pub fn duration_tc(&self, samples: i64) -> String {
        let frames = self.rate.frame_at_sample(samples.max(0), self.sample_rate);
        // durations are always shown NDF-style so they read as elapsed frames
        FrameRate { drop_frame: false, ..self.rate }.frames_to_tc(frames)
    }

    pub fn seconds(&self, samples: i64) -> f64 {
        samples as f64 / self.sample_rate as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rates() {
        assert_eq!(FrameRate::parse("23.976").unwrap(), FrameRate::FPS_23_976);
        assert_eq!(FrameRate::parse("29.97 DF").unwrap(), FrameRate::FPS_29_97_DF);
        assert_eq!(FrameRate::parse("29.97ndf").unwrap(), FrameRate::FPS_29_97_NDF);
        assert_eq!(FrameRate::parse("24000/1001").unwrap(), FrameRate::FPS_23_976);
        assert!(FrameRate::parse("24df").is_err());
        assert_eq!(FrameRate::FPS_29_97_DF.label(), "29.97 DF");
        assert_eq!(FrameRate::FPS_23_976.label(), "23.976");
        assert_eq!(FrameRate::FPS_25.label(), "25");
    }

    #[test]
    fn ndf_roundtrip() {
        for r in [FrameRate::FPS_23_976, FrameRate::FPS_24, FrameRate::FPS_25, FrameRate::FPS_29_97_NDF, FrameRate::FPS_30] {
            for f in [0i64, 1, 23, 24, 86_399, 1_000_000, 2_159_999] {
                assert_eq!(r.tc_to_frames(&r.frames_to_tc(f)).unwrap(), f, "{r} {f}");
            }
        }
        assert_eq!(FrameRate::FPS_24.tc_to_frames("01:00:00:00").unwrap(), 86_400);
    }

    #[test]
    fn drop_frame_known_values() {
        let r = FrameRate::FPS_29_97_DF;
        assert_eq!(r.frames_to_tc(1799), "00:00:59;29");
        assert_eq!(r.frames_to_tc(1800), "00:01:00;02");
        assert_eq!(r.frames_to_tc(17_982), "00:10:00;00");
        assert_eq!(r.frames_to_tc(107_892), "01:00:00;00");
        assert_eq!(r.tc_to_frames("01:00:00;00").unwrap(), 107_892);
        assert!(r.tc_to_frames("00:01:00;00").is_err());
        assert!(r.tc_to_frames("00:10:00;00").is_ok());
        for f in (0..200_000).step_by(7) {
            assert_eq!(r.tc_to_frames(&r.frames_to_tc(f)).unwrap(), f);
        }
        let r60 = FrameRate::FPS_59_94_DF;
        for f in (0..400_000).step_by(13) {
            assert_eq!(r60.tc_to_frames(&r60.frames_to_tc(f)).unwrap(), f);
        }
    }

    #[test]
    fn sample_frame_mapping_exact() {
        // 23.976 @ 48k = exactly 2002 samples/frame
        let r = FrameRate::FPS_23_976;
        assert_eq!(r.sample_at_frame(1, 48_000), 2002);
        assert_eq!(r.sample_at_frame(24_000, 48_000), 48_048_000);
        // 29.97 @ 48k = 1601.6 samples/frame — must round-trip every frame
        for r in [FrameRate::FPS_29_97_DF, FrameRate::FPS_23_976, FrameRate::FPS_25, FrameRate::FPS_59_94_NDF] {
            for sr in [44_100u32, 48_000, 96_000] {
                for f in -50i64..5000 {
                    let s = r.sample_at_frame(f, sr);
                    assert_eq!(r.frame_at_sample(s, sr), f);
                    assert_eq!(r.frame_at_sample(s - 1, sr), f - 1);
                }
            }
        }
        // no drift after 10 hours at 23.976
        let ten_h = 10 * 3600 * 24;
        assert_eq!(r.frame_at_sample(r.sample_at_frame(ten_h, 48_000), 48_000), ten_h);
    }

    #[test]
    fn timebase_nonzero_start() {
        let tb = TimeBase::new(FrameRate::FPS_24, 48_000, FrameRate::FPS_24.tc_to_frames("01:00:00:00").unwrap());
        assert_eq!(tb.sample_at_tc("01:00:00:00").unwrap(), 0);
        assert_eq!(tb.sample_at_tc("01:00:01:00").unwrap(), 48_000);
        assert_eq!(tb.tc_at(48_000 + 1999), "01:00:01:00");
        assert_eq!(tb.tc_at(48_000 + 2000), "01:00:01:01");
        assert_eq!(tb.tc_at(48_000 + 2000 * 3 + 5), "01:00:01:03");
        assert_eq!(tb.tc_at(-1), "00:59:59:23");
        assert_eq!(tb.snap(48_000 + 2100), 48_000 + 2000);
        assert_eq!(tb.duration_tc(48_000 * 61), "00:01:01:00");
        assert_eq!(tb.origin_since_midnight(), 3600 * 48_000);
    }
}

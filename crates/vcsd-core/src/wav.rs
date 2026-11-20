//! WAV / Broadcast WAV (BWF `bext`) reader and writer.
//! Supports PCM 16/24/32-bit integer and 32-bit float, any channel count,
//! WAVE_FORMAT_EXTENSIBLE input, and the BWF time reference.

use crate::error::{Error, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    Int16,
    Int24,
    Int32,
    Float32,
}

impl SampleFormat {
    pub fn bits(&self) -> u16 {
        match self {
            SampleFormat::Int16 => 16,
            SampleFormat::Int24 => 24,
            SampleFormat::Int32 | SampleFormat::Float32 => 32,
        }
    }
    pub fn from_bits(bits: u16) -> SampleFormat {
        match bits {
            16 => SampleFormat::Int16,
            32 => SampleFormat::Int32,
            _ => SampleFormat::Int24,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bext {
    pub description: String,
    pub originator: String,
    pub originator_reference: String,
    pub origination_date: String,
    pub origination_time: String,
    /// Samples since midnight of the first sample.
    pub time_reference: u64,
    pub coding_history: String,
}

#[derive(Debug, Clone)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub format: SampleFormat,
    pub frames: u64,
    pub bext: Option<Bext>,
}

/// Interleaved float audio.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u16,
    pub data: Vec<f32>,
}

impl AudioBuffer {
    pub fn silent(sample_rate: u32, channels: u16, frames: usize) -> Self {
        Self { sample_rate, channels, data: vec![0.0; frames * channels as usize] }
    }
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.data.len() / self.channels as usize
        }
    }
    #[inline]
    pub fn get(&self, frame: usize, ch: usize) -> f32 {
        self.data[frame * self.channels as usize + ch]
    }
    pub fn peak(&self) -> f32 {
        self.data.iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }
    pub fn peak_dbfs(&self) -> f32 {
        let p = self.peak();
        if p <= 0.0 {
            -144.0
        } else {
            20.0 * p.log10()
        }
    }
    pub fn add(&mut self, other: &AudioBuffer) {
        assert_eq!(self.channels, other.channels);
        for (a, b) in self.data.iter_mut().zip(other.data.iter()) {
            *a += *b;
        }
    }
}

fn wav_err(m: impl Into<String>) -> Error {
    Error::Wav(m.into())
}

fn rd_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn fixed_str(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

struct Parsed {
    info: WavInfo,
    float: bool,
    data_offset: u64,
    data_len: u64,
}

fn parse_header<R: Read + Seek>(r: &mut R) -> Result<Parsed> {
    let mut hdr = [0u8; 12];
    r.read_exact(&mut hdr).map_err(|_| wav_err("file too short"))?;
    if &hdr[0..4] != b"RIFF" || &hdr[8..12] != b"WAVE" {
        return Err(wav_err("not a RIFF/WAVE file"));
    }
    let (mut fmt, mut bext, mut data) = (None, None, None);
    loop {
        let mut ch = [0u8; 8];
        if r.read_exact(&mut ch).is_err() {
            break;
        }
        let id = [ch[0], ch[1], ch[2], ch[3]];
        let len = rd_u32(&ch, 4) as u64;
        let pos = r.stream_position()?;
        match &id {
            b"fmt " => {
                let mut b = vec![0u8; len as usize];
                r.read_exact(&mut b)?;
                fmt = Some(b);
            }
            b"bext" => {
                let mut b = vec![0u8; len as usize];
                r.read_exact(&mut b)?;
                if b.len() >= 602 {
                    let lo = rd_u32(&b, 338) as u64;
                    let hi = rd_u32(&b, 342) as u64;
                    bext = Some(Bext {
                        description: fixed_str(&b[0..256]),
                        originator: fixed_str(&b[256..288]),
                        originator_reference: fixed_str(&b[288..320]),
                        origination_date: fixed_str(&b[320..330]),
                        origination_time: fixed_str(&b[330..338]),
                        time_reference: (hi << 32) | lo,
                        coding_history: fixed_str(&b[602..]),
                    });
                }
            }
            b"data" => {
                data = Some((pos, len));
            }
            _ => {}
        }
        let next = pos + len + (len & 1);
        if r.seek(SeekFrom::Start(next)).is_err() {
            break;
        }
        if data.is_some() && fmt.is_some() && id == *b"data" {
            // keep scanning for a trailing bext chunk, but stop at EOF
        }
    }
    let fmt = fmt.ok_or_else(|| wav_err("missing fmt chunk"))?;
    if fmt.len() < 16 {
        return Err(wav_err("fmt chunk too short"));
    }
    let mut tag = rd_u16(&fmt, 0);
    let channels = rd_u16(&fmt, 2);
    let sample_rate = rd_u32(&fmt, 4);
    let bits = rd_u16(&fmt, 14);
    if tag == 0xFFFE && fmt.len() >= 26 {
        tag = rd_u16(&fmt, 24);
    }
    let (format, float) = match (tag, bits) {
        (1, 16) => (SampleFormat::Int16, false),
        (1, 24) => (SampleFormat::Int24, false),
        (1, 32) => (SampleFormat::Int32, false),
        (3, 32) => (SampleFormat::Float32, true),
        _ => return Err(wav_err(format!("unsupported WAV encoding (format {tag}, {bits}-bit)"))),
    };
    if channels == 0 || sample_rate == 0 {
        return Err(wav_err("invalid channel count or sample rate"));
    }
    let (data_offset, data_len) = data.ok_or_else(|| wav_err("missing data chunk"))?;
    let frame_bytes = channels as u64 * (bits as u64 / 8);
    Ok(Parsed { info: WavInfo { sample_rate, channels, format, frames: data_len / frame_bytes, bext }, float, data_offset, data_len })
}

pub fn read_info(path: &Path) -> Result<WavInfo> {
    let mut f = std::fs::File::open(path)?;
    Ok(parse_header(&mut f)?.info)
}

pub fn read(path: &Path) -> Result<(WavInfo, AudioBuffer)> {
    let mut f = std::io::BufReader::new(std::fs::File::open(path)?);
    let p = parse_header(&mut f)?;
    f.seek(SeekFrom::Start(p.data_offset))?;
    let mut raw = vec![0u8; p.data_len as usize];
    let n = f.read(&mut raw)?;
    let mut got = n;
    while got < raw.len() {
        let k = f.read(&mut raw[got..])?;
        if k == 0 {
            break;
        }
        got += k;
    }
    raw.truncate(got);
    let data: Vec<f32> = match (p.info.format, p.float) {
        (SampleFormat::Int16, _) => raw.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32_768.0).collect(),
        (SampleFormat::Int24, _) => raw
            .chunks_exact(3)
            .map(|c| (((c[2] as i32) << 24 | (c[1] as i32) << 16 | (c[0] as i32) << 8) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (SampleFormat::Int32, _) => {
            raw.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0).collect()
        }
        (SampleFormat::Float32, _) => raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
    };
    let ch = p.info.channels as usize;
    let mut buf = AudioBuffer { sample_rate: p.info.sample_rate, channels: p.info.channels, data };
    buf.data.truncate(buf.data.len() / ch * ch);
    Ok((p.info, buf))
}

fn put_fixed(out: &mut Vec<u8>, s: &str, n: usize) {
    let b = s.as_bytes();
    let k = b.len().min(n);
    out.extend_from_slice(&b[..k]);
    out.extend(std::iter::repeat_n(0u8, n - k));
}

/// Write `buf` as a PCM/float WAV, with a BWF `bext` chunk when given.
/// The file is written to a temp path and renamed (no partial files on failure).
pub fn write(path: &Path, buf: &AudioBuffer, format: SampleFormat, bext: Option<&Bext>) -> Result<()> {
    let ch = buf.channels;
    let bits = format.bits();
    let block = ch as u32 * bits as u32 / 8;
    let mut pcm: Vec<u8> = Vec::with_capacity(buf.data.len() * bits as usize / 8);
    for &s in &buf.data {
        let s = if s.is_finite() { s } else { 0.0 };
        match format {
            SampleFormat::Int16 => pcm.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32_767.0).round() as i16).to_le_bytes()),
            SampleFormat::Int24 => {
                let v = (s.clamp(-1.0, 1.0) as f64 * 8_388_607.0).round() as i32;
                pcm.extend_from_slice(&v.to_le_bytes()[0..3]);
            }
            SampleFormat::Int32 => pcm.extend_from_slice(&((s.clamp(-1.0, 1.0) as f64 * 2_147_483_647.0).round() as i32).to_le_bytes()),
            SampleFormat::Float32 => pcm.extend_from_slice(&s.to_le_bytes()),
        }
    }
    let mut chunks: Vec<u8> = Vec::new();
    // fmt
    chunks.extend_from_slice(b"fmt ");
    chunks.extend_from_slice(&16u32.to_le_bytes());
    chunks.extend_from_slice(&(if format == SampleFormat::Float32 { 3u16 } else { 1u16 }).to_le_bytes());
    chunks.extend_from_slice(&ch.to_le_bytes());
    chunks.extend_from_slice(&buf.sample_rate.to_le_bytes());
    chunks.extend_from_slice(&(buf.sample_rate * block).to_le_bytes());
    chunks.extend_from_slice(&(block as u16).to_le_bytes());
    chunks.extend_from_slice(&bits.to_le_bytes());
    if let Some(b) = bext {
        let mut body = Vec::with_capacity(602 + b.coding_history.len());
        put_fixed(&mut body, &b.description, 256);
        put_fixed(&mut body, &b.originator, 32);
        put_fixed(&mut body, &b.originator_reference, 32);
        put_fixed(&mut body, &b.origination_date, 10);
        put_fixed(&mut body, &b.origination_time, 8);
        body.extend_from_slice(&((b.time_reference & 0xFFFF_FFFF) as u32).to_le_bytes());
        body.extend_from_slice(&((b.time_reference >> 32) as u32).to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes()); // version
        body.extend(std::iter::repeat_n(0u8, 64)); // UMID
        body.extend(std::iter::repeat_n(0u8, 10)); // loudness fields (unset)
        body.extend(std::iter::repeat_n(0u8, 180)); // reserved
        body.extend_from_slice(b.coding_history.as_bytes());
        if body.len() % 2 == 1 {
            body.push(0);
        }
        chunks.extend_from_slice(b"bext");
        chunks.extend_from_slice(&(body.len() as u32).to_le_bytes());
        chunks.extend_from_slice(&body);
    }
    chunks.extend_from_slice(b"data");
    chunks.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    let riff_len = 4 + chunks.len() + pcm.len() + (pcm.len() & 1);
    if riff_len > u32::MAX as usize {
        return Err(wav_err("file exceeds 4 GiB WAV limit (RF64 not yet supported)"));
    }
    let tmp = path.with_extension("wav.partial");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        f.write_all(b"RIFF")?;
        f.write_all(&(riff_len as u32).to_le_bytes())?;
        f.write_all(b"WAVE")?;
        f.write_all(&chunks)?;
        f.write_all(&pcm)?;
        if pcm.len() % 2 == 1 {
            f.write_all(&[0])?;
        }
        f.flush()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_formats_and_bext() {
        let dir = std::env::temp_dir().join(format!("vcsd_wav_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut buf = AudioBuffer::silent(48_000, 2, 1001);
        for (i, v) in buf.data.iter_mut().enumerate() {
            *v = ((i as f32) * 0.013).sin() * 0.8;
        }
        for fmt in [SampleFormat::Int16, SampleFormat::Int24, SampleFormat::Int32, SampleFormat::Float32] {
            let p = dir.join(format!("t{}.wav", fmt.bits()));
            let bext = Bext { description: "test".into(), time_reference: 172_800_000 + 7, ..Default::default() };
            write(&p, &buf, fmt, Some(&bext)).unwrap();
            let (info, back) = read(&p).unwrap();
            assert_eq!(info.frames, 1001);
            assert_eq!(info.channels, 2);
            assert_eq!(info.bext.as_ref().unwrap().time_reference, 172_800_007);
            assert_eq!(info.bext.unwrap().description, "test");
            let tol = match fmt {
                SampleFormat::Int16 => 1e-4,
                _ => 1e-6,
            };
            for (a, b) in buf.data.iter().zip(back.data.iter()) {
                assert!((a - b).abs() < tol, "{fmt:?}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

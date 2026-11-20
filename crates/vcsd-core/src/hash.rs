//! Content hashing for media identity, relinking and delivery checksums.

use crate::error::Result;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

pub fn sha256_bytes(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

/// Fast fingerprint for large picture files: size plus SHA-256 of up to
/// 16 evenly spaced 1 MiB windows. Stable per file content; used as the AI
/// suggestion cache key per picture version.
pub fn fingerprint_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let mut h = Sha256::new();
    h.update(size.to_le_bytes());
    let win = 1u64 << 20;
    let mut buf = vec![0u8; win as usize];
    let windows = if size <= win * 16 { 1 } else { 16 };
    for i in 0..windows {
        let off = if windows == 1 { 0 } else { (size - win) * i / (windows - 1) };
        f.seek(SeekFrom::Start(off))?;
        let n = f.read(&mut buf)?;
        h.update(&buf[..n]);
        if windows == 1 && n as u64 == win {
            // small-ish file read fully
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
        }
    }
    Ok(format!("fp1:{}", hex(&h.finalize())))
}

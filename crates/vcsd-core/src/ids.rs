//! Stable, immutable identifiers. IDs are UUIDv7 (time-ordered) with a short
//! entity prefix so that exported manifests are self-describing, e.g.
//! `cue_01936c4f8a2b7c3d9e0f112233445566`.

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::now_v7().simple())
}

/// Seconds since the Unix epoch, used for audit timestamps.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// UTC date `YYYY-MM-DD` for a unix timestamp (civil-from-days algorithm).
pub fn date_from_unix(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// UTC time `HH:MM:SS` for a unix timestamp.
pub fn time_from_unix(secs: i64) -> String {
    let s = secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(date_from_unix(0), "1970-01-01");
        assert_eq!(date_from_unix(1_700_000_000), "2023-11-14");
        assert_eq!(time_from_unix(1_700_000_000), "22:13:20");
        let a = new_id("cue");
        assert!(a.starts_with("cue_") && a.len() == 36);
        assert_ne!(a, new_id("cue"));
    }
}

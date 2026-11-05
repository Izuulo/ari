//! 5-hour rate-limit burn rate: sample `used_percentage`, fit a line, warn before the limit.

use crate::usage::Usage;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const WINDOW_MS: u64 = 15 * 60_000;
const MIN_SPAN_MS: u64 = 2 * 60_000;
const MIN_GAP_MS: u64 = 30_000;
const MAX_BYTES: u64 = 1 << 20;
const KEEP_MS: u64 = 2 * 3_600_000;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub t: u64,
    pub pct: f64,
    pub resets_at: u64,
}

fn tail(path: &Path, bytes: u64) -> String {
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = f.metadata().map_or(0, |m| m.len());
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(len.saturating_sub(bytes))).is_ok() {
        let _ = f.read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn parse(text: &str) -> Vec<Sample> {
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Append a sample when the 5h percentage moved, or every 30 s otherwise.
pub fn record(dir: &Path, u: &Usage, now: u64) {
    let Some(l) = u.five_h else { return };
    let path = dir.join("limits.jsonl");
    if let Some(last) = parse(&tail(&path, 256)).last()
        && last.pct == l.pct
        && last.resets_at == l.resets_at
        && now.saturating_sub(last.t) < MIN_GAP_MS
    {
        return;
    }
    let s = Sample {
        t: now,
        pct: l.pct,
        resets_at: l.resets_at,
    };
    let Ok(line) = serde_json::to_string(&s) else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let keep: String = parse(&std::fs::read_to_string(&path).unwrap_or_default())
            .into_iter()
            .filter(|s| now.saturating_sub(s.t) < KEEP_MS)
            .filter_map(|s| serde_json::to_string(&s).ok())
            .map(|l| l + "\n")
            .collect();
        let _ = crate::paths::write_atomic(&path, keep.as_bytes());
    }
}

/// Samples from the last 15 minutes.
pub fn load(dir: &Path, now: u64) -> Vec<Sample> {
    parse(&tail(&dir.join("limits.jsonl"), 64 * 1024))
        .into_iter()
        .filter(|s| now.saturating_sub(s.t) <= WINDOW_MS)
        .collect()
}

/// Milliseconds until the 5h limit hits 100% at the current pace, only if that's before it resets.
pub fn projection(samples: &[Sample], now: u64) -> Option<u64> {
    let last = samples.last()?;
    let window: Vec<&Sample> = samples
        .iter()
        .filter(|s| s.resets_at == last.resets_at && now.saturating_sub(s.t) <= WINDOW_MS)
        .collect();
    if window.len() < 3 || last.resets_at == 0 || last.t - window[0].t < MIN_SPAN_MS {
        return None;
    }
    // Least squares, x in minutes since the first sample.
    let t0 = window[0].t;
    let xs: Vec<f64> = window
        .iter()
        .map(|s| (s.t - t0) as f64 / 60_000.0)
        .collect();
    let n = xs.len() as f64;
    let (mx, my) = (
        xs.iter().sum::<f64>() / n,
        window.iter().map(|s| s.pct).sum::<f64>() / n,
    );
    let var: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    let cov: f64 = xs
        .iter()
        .zip(&window)
        .map(|(x, s)| (x - mx) * (s.pct - my))
        .sum();
    let slope = cov / var; // % per minute
    if !(slope > 0.0) || last.pct >= 100.0 {
        return None;
    }
    let eta = ((100.0 - last.pct) / slope * 60_000.0) as u64;
    (now + eta < last.resets_at * 1000).then_some(eta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::Limit;

    const NOW: u64 = 1_800_000_000_000;

    fn series(start_pct: f64, per_min: f64, minutes: u64, resets_in_s: u64) -> Vec<Sample> {
        (0..=minutes)
            .map(|m| Sample {
                t: NOW - (minutes - m) * 60_000,
                pct: start_pct + per_min * m as f64,
                resets_at: NOW / 1000 + resets_in_s,
            })
            .collect()
    }

    #[test]
    fn rising_pace_warns_before_reset() {
        // 40% → 50% over 10 min at 1%/min: 50 min left.
        let eta = projection(&series(40.0, 1.0, 10, 3 * 3600), NOW).unwrap();
        assert!((eta as i64 - 3_000_000).abs() < 1_000, "{eta}");
        assert_eq!(
            projection(&series(40.0, 1.0, 10, 20 * 60), NOW),
            None,
            "resets first"
        );
    }

    #[test]
    fn no_warning_without_a_trend() {
        assert_eq!(projection(&series(40.0, 0.0, 10, 3600), NOW), None, "flat");
        assert_eq!(
            projection(&series(40.0, -1.0, 10, 3600), NOW),
            None,
            "falling"
        );
        assert_eq!(
            projection(&series(40.0, 1.0, 1, 3600), NOW),
            None,
            "too short"
        );
        assert_eq!(projection(&[], NOW), None);
    }

    #[test]
    fn ignores_previous_window() {
        let mut s = series(90.0, 1.0, 5, 10);
        s.extend(
            series(1.0, 0.0, 5, 5 * 3600)
                .into_iter()
                .map(|x| Sample { t: x.t + 1, ..x }),
        );
        assert_eq!(
            projection(&s, NOW + 1),
            None,
            "new window is flat; old samples don't count"
        );
    }

    #[test]
    fn record_dedupes_and_loads() {
        let dir = std::env::temp_dir().join(format!("ari-burn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let u = |pct| Usage {
            five_h: Some(Limit {
                pct,
                resets_at: NOW / 1000 + 3600,
            }),
            ..Default::default()
        };
        record(&dir, &u(10.0), NOW);
        record(&dir, &u(10.0), NOW + 1_000);
        record(&dir, &u(11.0), NOW + 2_000);
        record(&dir, &u(11.0), NOW + 40_000);
        record(&dir, &Usage::default(), NOW + 50_000);
        let got = load(&dir, NOW + 60_000);
        assert_eq!(
            got.iter().map(|s| s.pct).collect::<Vec<_>>(),
            [10.0, 11.0, 11.0]
        );
        assert!(load(&dir, NOW + WINDOW_MS + 100_000).is_empty());
    }
}

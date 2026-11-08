//! 5-hour rate-limit burn rate: sample `used_percentage`, fit a line, warn before the limit.

use crate::usage::{Limit, Usage};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const WINDOW_MS: u64 = 15 * 60_000;
const MIN_SPAN_MS: u64 = 2 * 60_000;
const MIN_GAP_MS: u64 = 30_000;
const MAX_BYTES: u64 = 1 << 20;
const KEEP_MS: u64 = 2 * 3_600_000;
const WEEK_WINDOW_MS: u64 = 6 * 3_600_000;
const WEEK_MIN_SPAN_MS: u64 = 3_600_000;
const WEEK_GAP_MS: u64 = 10 * 60_000;
const WEEK_KEEP_MS: u64 = 24 * 3_600_000;
const WEEK_MAX_BYTES: u64 = 256 * 1024;
const WEEK_FILE: &str = "week.jsonl";

#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Sample {
    pub t: u64,
    /// 5h used %, 0 with `resets_at` 0 when the payload has no rate limits.
    pub pct: f64,
    pub resets_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// The session's token total when sampled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tok: Option<u64>,
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

/// Append a sample when anything moved, or every 30 s otherwise.
pub fn record(dir: &Path, u: &Usage, tok: Option<u64>, now: u64) {
    if u.five_h.is_none() && u.ctx_pct.is_none() && u.cost_usd.is_none() {
        return;
    }
    let l = u.five_h.unwrap_or_default();
    let s = Sample {
        t: now,
        pct: l.pct,
        resets_at: l.resets_at,
        ctx: u.ctx_pct,
        cost: u.cost_usd,
        tok,
    };
    let path = dir.join("limits.jsonl");
    if let Some(last) = parse(&tail(&path, 512)).last()
        && (Sample { t: now, ..*last }) == s
        && now.saturating_sub(last.t) < MIN_GAP_MS
    {
        return;
    }
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

/// The file's tail, unfiltered by time: enough for the last several turns.
pub fn recent(dir: &Path) -> Vec<Sample> {
    parse(&tail(&dir.join("limits.jsonl"), 64 * 1024))
}

/// Tokens spent since `t`: the newest total minus the total at or before `t`
/// (else the oldest known). `None` when no sample has a total.
// shortcut: stuck longer than the sample tail (~64 KB) undercounts; keep a longer tail if that matters.
pub fn tokens_since(samples: &[Sample], t: u64) -> Option<u64> {
    let known = || samples.iter().filter_map(|s| Some((s.t, s.tok?)));
    let newest = known().next_back()?.1;
    let base = known()
        .rfind(|&(st, _)| st <= t)
        .or_else(|| known().next())?
        .1;
    Some(newest.saturating_sub(base))
}

/// Turns left before context reaches `at_pct`, from the median growth over the last turns.
/// `prompts` are turn start times; a turn containing `compacted` is skipped.
pub fn compacts_in(
    samples: &[Sample],
    prompts: &[u64],
    compacted: Option<u64>,
    at_pct: f64,
) -> Option<u32> {
    let ctx_before = |t: u64| samples.iter().rev().find(|s| s.t <= t)?.ctx;
    let points: Vec<(u64, f64)> = prompts
        .iter()
        .filter_map(|&t| Some((t, ctx_before(t)?)))
        .collect();
    if points.len() < 3 {
        return None;
    }
    let mut deltas: Vec<f64> = points
        .windows(2)
        .filter(|w| !compacted.is_some_and(|c| c > w[0].0 && c <= w[1].0))
        .map(|w| w[1].1 - w[0].1)
        .collect();
    deltas.sort_by(f64::total_cmp);
    let growth = *deltas.get(deltas.len() / 2)?;
    let now = samples.iter().rev().find_map(|s| s.ctx)?;
    (growth > 0.0).then(|| ((at_pct - now).max(0.0) / growth).round() as u32)
}

/// `compacts_in` for a session dir.
pub fn forecast(dir: &Path, st: &crate::state::State, cfg: &crate::config::Config) -> Option<u32> {
    let prompts: Vec<u64> = st.prompts.iter().copied().collect();
    compacts_in(
        &recent(dir),
        &prompts,
        st.compacted_at,
        cfg.thresholds.compact_pct as f64,
    )
}

/// Append the 7-day sample unless it repeats the last one within 10 minutes.
pub fn record_week(root: &Path, l: Option<Limit>, now: u64) {
    let Some(l) = l else { return };
    let s = Sample {
        t: now,
        pct: l.pct,
        resets_at: l.resets_at,
        ..Default::default()
    };
    let path = root.join(WEEK_FILE);
    if let Some(last) = parse(&tail(&path, 512)).last()
        && (Sample { t: now, ..*last }) == s
        && now.saturating_sub(last.t) < WEEK_GAP_MS
    {
        return;
    }
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
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > WEEK_MAX_BYTES) {
        let keep: String = parse(&tail(&path, WEEK_MAX_BYTES / 2))
            .into_iter()
            .filter(|s| now.saturating_sub(s.t) < WEEK_KEEP_MS)
            .filter_map(|s| serde_json::to_string(&s).ok())
            .map(|l| l + "\n")
            .collect();
        let _ = crate::paths::write_atomic(&path, keep.as_bytes());
    }
}

/// 7-day samples from the last 6 hours, oldest first.
pub fn load_week(root: &Path, now: u64) -> Vec<Sample> {
    let mut v: Vec<Sample> = parse(&tail(&root.join(WEEK_FILE), 64 * 1024))
        .into_iter()
        .filter(|s| now.saturating_sub(s.t) <= WEEK_WINDOW_MS)
        .collect();
    v.sort_by_key(|s| s.t);
    v
}

/// Milliseconds until the 5h limit hits 100% at the current pace, only if that's before it resets.
pub fn projection(samples: &[Sample], now: u64) -> Option<u64> {
    eta(samples, now, WINDOW_MS, MIN_SPAN_MS)
}

/// `projection` for the session's own 7-day limit: a 6 h fit that needs an hour of samples.
/// No limit in the payload, no forecast.
pub fn week_projection(samples: &[Sample], limit: Option<Limit>, now: u64) -> Option<u64> {
    let l = limit?;
    let own: Vec<Sample> = samples
        .iter()
        .filter(|s| s.resets_at == l.resets_at)
        .copied()
        .collect();
    eta(&own, now, WEEK_WINDOW_MS, WEEK_MIN_SPAN_MS)
}

fn eta(samples: &[Sample], now: u64, window_ms: u64, min_span_ms: u64) -> Option<u64> {
    let resets_at = samples.last()?.resets_at;
    let mut window: Vec<&Sample> = samples
        .iter()
        .filter(|s| s.resets_at == resets_at && now.saturating_sub(s.t) <= window_ms)
        .collect();
    // Two statuslines can append out of order.
    window.sort_by_key(|s| s.t);
    if window.len() < 3 || resets_at == 0 {
        return None;
    }
    let last = window[window.len() - 1];
    if last.t.saturating_sub(window[0].t) < min_span_ms {
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
    if slope.is_nan() || slope <= 0.0 || last.pct >= 100.0 {
        return None;
    }
    let ms = ((100.0 - last.pct) / slope * 60_000.0) as u64;
    (now + ms < last.resets_at * 1000).then_some(ms)
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
                ..Default::default()
            })
            .collect()
    }

    fn ctx_at(t: u64, ctx: f64) -> Sample {
        Sample {
            t,
            ctx: Some(ctx),
            ..Default::default()
        }
    }

    #[test]
    fn compaction_forecast() {
        let prompts = [1_000, 2_000, 3_000, 4_000];
        let s: Vec<Sample> = [40.0, 46.0, 52.0, 58.0]
            .iter()
            .zip(prompts)
            .map(|(c, t)| ctx_at(t - 10, *c))
            .collect();
        assert_eq!(compacts_in(&s, &prompts, None, 90.0), Some(5));
        assert_eq!(
            compacts_in(&s[..2], &prompts[..2], None, 90.0),
            None,
            "too few turns"
        );
        // A compaction between the 2nd and 3rd prompt: that drop is skipped.
        let s2: Vec<Sample> = [40.0, 46.0, 10.0, 16.0, 22.0]
            .iter()
            .zip([1_000, 2_000, 3_000, 4_000, 5_000])
            .map(|(c, t)| ctx_at(t - 10, *c))
            .collect();
        assert_eq!(
            compacts_in(&s2, &[1_000, 2_000, 3_000, 4_000, 5_000], Some(2_500), 90.0),
            Some(11)
        );
        let flat: Vec<Sample> = prompts.iter().map(|t| ctx_at(t - 10, 40.0)).collect();
        assert_eq!(
            compacts_in(&flat, &prompts, None, 90.0),
            None,
            "not growing"
        );
    }

    #[test]
    fn old_lines_and_ctx_only_samples() {
        let old = r#"{"t":5,"pct":40.0,"resets_at":9}"#;
        assert_eq!(parse(old)[0].ctx, None);
        let dir = std::env::temp_dir().join(format!("ari-burn-ctx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let u = Usage {
            ctx_pct: Some(33.0),
            cost_usd: Some(1.5),
            ..Default::default()
        };
        record(&dir, &u, None, NOW);
        let got = recent(&dir);
        assert_eq!(
            (got[0].ctx, got[0].cost),
            (Some(33.0), Some(1.5)),
            "no 5h limit needed"
        );
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
        record(&dir, &u(10.0), Some(42), NOW);
        record(&dir, &u(10.0), Some(42), NOW + 1_000);
        record(&dir, &u(11.0), Some(42), NOW + 2_000);
        record(&dir, &u(11.0), Some(42), NOW + 40_000);
        record(&dir, &Usage::default(), Some(42), NOW + 50_000);
        let got = load(&dir, NOW + 60_000);
        assert_eq!(got[0].tok, Some(42));
        assert_eq!(
            got.iter().map(|s| s.pct).collect::<Vec<_>>(),
            [10.0, 11.0, 11.0]
        );
        assert!(load(&dir, NOW + WINDOW_MS + 100_000).is_empty());
    }

    #[test]
    fn tokens_since_uses_the_sample_before() {
        let s: Vec<Sample> = [(10, 100), (20, 150), (30, 400)]
            .iter()
            .map(|&(t, tok)| Sample {
                t,
                tok: Some(tok),
                ..Default::default()
            })
            .collect();
        assert_eq!(tokens_since(&s, 20), Some(250));
        assert_eq!(tokens_since(&s, 25), Some(250));
        assert_eq!(tokens_since(&s, 5), Some(300));
        let bare = vec![Sample::default(); 3];
        assert_eq!(tokens_since(&bare, 5), None);
    }

    fn series_h(start_pct: f64, per_hour: f64, hours: u64, resets_in_s: u64) -> Vec<Sample> {
        let n = hours * 6;
        (0..=n)
            .map(|i| Sample {
                t: NOW - (n - i) * 600_000,
                pct: start_pct + per_hour * i as f64 / 6.0,
                resets_at: NOW / 1000 + resets_in_s,
                ..Default::default()
            })
            .collect()
    }

    fn lim(resets_in_s: u64) -> Option<Limit> {
        Some(Limit {
            pct: 50.0,
            resets_at: NOW / 1000 + resets_in_s,
        })
    }

    #[test]
    fn week_needs_the_sessions_own_limit() {
        let s = series_h(40.0, 2.0, 3, 3 * 86400);
        assert!(week_projection(&s, lim(3 * 86400), NOW).is_some());
        assert_eq!(week_projection(&s, None, NOW), None);
        assert_eq!(week_projection(&s, lim(4 * 86400), NOW), None);
    }

    #[test]
    fn week_pace_warns_before_reset() {
        // 40% -> 46% over 3 h at 2%/h: 54% left = 27 h.
        let eta = week_projection(&series_h(40.0, 2.0, 3, 3 * 86400), lim(3 * 86400), NOW).unwrap();
        assert!((eta as i64 - 27 * 3_600_000).abs() < 60_000, "{eta}");
        assert_eq!(
            week_projection(&series_h(40.0, 2.0, 3, 20 * 3600), lim(20 * 3600), NOW),
            None,
            "resets first"
        );
    }

    #[test]
    fn week_needs_an_hour_of_samples() {
        let short: Vec<Sample> = series(40.0, 0.1, 50, 86400);
        assert_eq!(week_projection(&short, lim(86400), NOW), None);
        assert_eq!(
            week_projection(
                &series_h(40.0, 2.0, 3, 86400),
                lim(86400),
                NOW + 7 * 3_600_000
            ),
            None,
            "too old"
        );
    }

    #[test]
    fn week_ignores_previous_window() {
        // Old window rises fast from 90%; the new one is 2% -> 4% over 2 h.
        let mut s = series_h(90.0, 2.0, 2, 10);
        let new = series_h(2.0, 1.0, 2, 5 * 86400);
        s.extend(new.iter().map(|x| Sample { t: x.t + 1, ..*x }));
        let alone: Vec<Sample> = new.iter().map(|x| Sample { t: x.t + 1, ..*x }).collect();
        let got = week_projection(&s, lim(5 * 86400), NOW + 1);
        assert!(got.is_some());
        assert_eq!(got, week_projection(&alone, lim(5 * 86400), NOW + 1));
    }

    #[test]
    fn week_fit_survives_out_of_order_lines() {
        let sorted = series_h(40.0, 2.0, 3, 3 * 86400);
        let mut swapped = sorted.clone();
        swapped.swap(5, 6);
        swapped.swap(0, 1);
        assert_eq!(
            week_projection(&swapped, lim(3 * 86400), NOW),
            week_projection(&sorted, lim(3 * 86400), NOW)
        );
        assert!(week_projection(&swapped, lim(3 * 86400), NOW).is_some());
        // The newest line landing earlier in the file must not panic either.
        let mut tail_first = sorted.clone();
        tail_first.rotate_right(1);
        assert!(week_projection(&tail_first, lim(3 * 86400), NOW).is_some());
    }

    #[test]
    fn record_week_dedupes_and_loads() {
        let dir = std::env::temp_dir().join(format!("ari-burn-week-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let l = |pct| {
            Some(Limit {
                pct,
                resets_at: NOW / 1000 + 86400,
            })
        };
        record_week(&dir, l(10.0), NOW);
        record_week(&dir, l(10.0), NOW + 60_000);
        assert_eq!(load_week(&dir, NOW + 60_000).len(), 1);
        record_week(&dir, l(10.0), NOW + WEEK_GAP_MS);
        assert_eq!(load_week(&dir, NOW + WEEK_GAP_MS).len(), 2);
        record_week(&dir, l(11.0), NOW + WEEK_GAP_MS + 1_000);
        assert_eq!(load_week(&dir, NOW + WEEK_GAP_MS + 1_000).len(), 3);
        assert_eq!(
            load_week(&dir, NOW + WEEK_WINDOW_MS + 1).len(),
            2,
            "first dropped"
        );
    }

    #[test]
    fn record_week_skips_missing_limit() {
        let dir = std::env::temp_dir().join(format!("ari-burn-weeknone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        record_week(&dir, None, NOW);
        assert!(!dir.join(WEEK_FILE).exists());
    }
}

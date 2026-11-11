//! Quality baseline: one summary line per session in `<state root>/quality.jsonl`
//! (counts only: no prompts, paths or output), compared against the user's own usual.

use crate::{app::App, hook, paths, state, state::State, usage};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FILE: &str = "quality.jsonl";
pub const MAX_SESSIONS: usize = 200;
pub const MAX_BYTES: u64 = 256 * 1024;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Summary {
    pub id: String,
    pub t: u64,
    pub model: String,
    pub turns: u32,
    pub reads: u32,
    pub edits: u32,
    pub failed: u32,
    pub retries: u32,
}

impl Summary {
    pub fn of(id: &str, t: u64, model: &str, st: &State) -> Summary {
        let q = st.quality;
        Summary {
            id: id.into(),
            t,
            model: model.into(),
            turns: st.turns,
            reads: q.reads,
            edits: q.edits,
            failed: q.failed,
            retries: q.retries,
        }
    }
}

/// Append one summary; compact once the file passes `MAX_BYTES`. Errors are ignored.
pub fn record(root: &Path, s: &Summary) {
    let path = root.join(FILE);
    let Ok(line) = serde_json::to_string(s) else {
        return;
    };
    let _ = hook::append(&path, (line + "\n").as_bytes());
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        compact(root, u64::MAX, s.t);
    }
}

/// The hook's entry point at main's Stop and at SessionEnd: fold the session, append its summary.
pub fn on_stop(root: &Path, dir: &Path, id: &str, t: u64, app: App) {
    let st = state::load_session(dir);
    if st.turns == 0 {
        return;
    }
    let model = usage::load(dir)
        .model
        .unwrap_or_else(|| app.as_str().into());
    record(root, &Summary::of(id, t, &model, &st));
}

/// Every session's last summary, oldest first; broken lines are skipped.
pub fn load(root: &Path) -> Vec<Summary> {
    let Ok(text) = std::fs::read_to_string(root.join(FILE)) else {
        return Vec::new();
    };
    let mut out: Vec<Summary> = Vec::new();
    for s in text
        .lines()
        .filter_map(|l| serde_json::from_str::<Summary>(l).ok())
    {
        out.retain(|o| o.id != s.id);
        out.push(s);
    }
    out.sort_by_key(|s| s.t);
    out
}

/// Rewrite the file: last line per session, none older than `max_age_ms`, the newest `MAX_SESSIONS`.
pub fn compact(root: &Path, max_age_ms: u64, now: u64) {
    if !root.join(FILE).exists() {
        return;
    }
    let mut all = load(root);
    all.retain(|s| now.saturating_sub(s.t) <= max_age_ms);
    let skip = all.len().saturating_sub(MAX_SESSIONS);
    let text: String = all[skip..]
        .iter()
        .filter_map(|s| serde_json::to_string(s).ok())
        .map(|l| l + "\n")
        .collect();
    // shortcut: a Stop line appended during the rewrite is lost; the session's next Stop writes it again.
    let _ = paths::write_atomic(&root.join(FILE), text.as_bytes());
}

pub const MIN_SESSIONS: usize = 5;
pub const MIN_EDITS: u32 = 5;
pub const BASELINE_SESSIONS: usize = 30;
pub const READS_LOW: f64 = 0.6;
pub const RATE_HIGH: f64 = 1.5;
pub const RATE_FLOOR: f64 = 1.0;

/// A rate worse than the user's usual on this model.
#[derive(Clone, Debug, PartialEq)]
pub struct Deviation {
    pub what: &'static str,
    pub now: f64,
    pub usual: f64,
}

impl Deviation {
    pub fn text(&self) -> String {
        format!(
            "{} {:.1} · your usual {:.1}",
            self.what, self.now, self.usual
        )
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Compare `cur` with the median of its past sessions on the same model; only worse shows.
pub fn check(cur: &Summary, history: &[Summary]) -> Vec<Deviation> {
    let past: Vec<&Summary> = history
        .iter()
        .rev()
        .filter(|s| s.id != cur.id && s.model == cur.model && s.edits >= MIN_EDITS)
        .take(BASELINE_SESSIONS)
        .collect();
    if past.len() < MIN_SESSIONS || cur.edits < MIN_EDITS {
        return Vec::new();
    }
    type Rate = fn(&Summary) -> f64;
    let rates: [(&'static str, Rate); 3] = [
        ("reads/edit", |s| s.reads as f64 / s.edits as f64),
        ("errors/prompt", |s| s.failed as f64 / s.turns.max(1) as f64),
        ("retries/prompt", |s| {
            s.retries as f64 / s.turns.max(1) as f64
        }),
    ];
    rates
        .into_iter()
        .filter_map(|(what, rate)| {
            let now = rate(cur);
            let usual = median(past.iter().map(|s| rate(s)).collect());
            let worse = if what == "reads/edit" {
                now <= usual * READS_LOW && now <= usual - RATE_FLOOR
            } else {
                now >= usual * RATE_HIGH && now >= usual + RATE_FLOOR
            };
            worse.then_some(Deviation { what, now, usual })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Quality;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ari-quality-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn sum(id: &str, t: u64, reads: u32) -> Summary {
        Summary {
            id: id.into(),
            t,
            reads,
            ..Default::default()
        }
    }
    fn line(s: &Summary) -> String {
        serde_json::to_string(s).unwrap() + "\n"
    }

    fn s(
        id: &str,
        model: &str,
        turns: u32,
        reads: u32,
        edits: u32,
        failed: u32,
        retries: u32,
    ) -> Summary {
        Summary {
            id: id.into(),
            model: model.into(),
            turns,
            reads,
            edits,
            failed,
            retries,
            ..Default::default()
        }
    }
    fn past(model: &str, reads: &[u32]) -> Vec<Summary> {
        reads
            .iter()
            .enumerate()
            .map(|(i, &r)| s(&format!("p{i}"), model, 2, r, 10, 0, 0))
            .collect()
    }

    #[test]
    fn low_reads_per_edit_shows() {
        let h = past("Opus", &[60, 60, 62, 61, 60]);
        let d = check(&s("cur", "Opus", 2, 20, 10, 0, 0), &h);
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].what, d[0].now, d[0].usual), ("reads/edit", 2.0, 6.0));
        assert_eq!(d[0].text(), "reads/edit 2.0 · your usual 6.0");
    }

    #[test]
    fn needs_min_sessions_and_min_edits() {
        let cur = s("cur", "Opus", 2, 20, 10, 0, 0);
        assert!(check(&cur, &past("Opus", &[60; 4])).is_empty());
        let few = s("cur", "Opus", 2, 8, 4, 0, 0);
        assert!(check(&few, &past("Opus", &[60; 5])).is_empty());
        let mut h = past("Opus", &[60; 4]);
        h.push(s("small", "Opus", 2, 24, 4, 0, 0));
        assert!(check(&cur, &h).is_empty());
    }

    #[test]
    fn excludes_itself_and_other_models() {
        let cur = s("cur", "Opus", 2, 20, 10, 0, 0);
        let mine: Vec<Summary> = (0..5).map(|_| s("cur", "Opus", 2, 60, 10, 0, 0)).collect();
        assert!(check(&cur, &mine).is_empty());
        assert!(check(&cur, &past("Haiku", &[60; 5])).is_empty());
    }

    #[test]
    fn zero_usual_errors_needs_floor() {
        let h: Vec<Summary> = (0..5)
            .map(|i| s(&format!("p{i}"), "Opus", 5, 60, 10, 0, 0))
            .collect();
        assert!(check(&s("cur", "Opus", 4, 60, 10, 2, 0), &h).is_empty());
        let d = check(&s("cur", "Opus", 4, 60, 10, 6, 0), &h);
        assert_eq!(d.len(), 1);
        assert_eq!(
            (d[0].what, d[0].now, d[0].usual),
            ("errors/prompt", 1.5, 0.0)
        );
    }

    #[test]
    fn better_than_usual_is_silent() {
        let h: Vec<Summary> = (0..5)
            .map(|i| s(&format!("p{i}"), "Opus", 2, 60, 10, 4, 2))
            .collect();
        assert!(check(&s("cur", "Opus", 2, 90, 10, 0, 0), &h).is_empty());
    }

    #[test]
    fn low_usual_reads_need_floor() {
        // Reads through Bash don't count, so a usual near 1 read/edit: a drop to 0.1 is noise.
        let h = past("Opus", &[9; 5]);
        assert!(check(&s("cur", "Opus", 2, 1, 10, 0, 0), &h).is_empty());
    }

    #[test]
    fn median_not_mean() {
        let h = past("Opus", &[10, 60, 60, 60, 600]);
        let d = check(&s("cur", "Opus", 2, 20, 10, 0, 0), &h);
        assert_eq!(d[0].usual, 6.0);
    }

    #[test]
    fn summary_of_state() {
        let st = State {
            turns: 3,
            quality: Quality {
                reads: 12,
                edits: 4,
                failed: 2,
                retries: 1,
            },
            ..Default::default()
        };
        let s = Summary::of("s1", 9, "Opus", &st);
        assert_eq!(
            s,
            Summary {
                id: "s1".into(),
                t: 9,
                model: "Opus".into(),
                turns: 3,
                reads: 12,
                edits: 4,
                failed: 2,
                retries: 1
            }
        );
    }

    #[test]
    fn load_keeps_last_line_per_session_and_skips_garbage() {
        let d = dir("load");
        let text = line(&sum("s1", 1, 1))
            + "not json\n"
            + "{\"id\":\"s2\",\"t\":2}\n"
            + "{\"id\":\"s1\",\"t\":\n"
            + &line(&sum("s1", 3, 7));
        std::fs::write(d.join(FILE), text).unwrap();
        let all = load(&d);
        let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s2", "s1"]);
        assert_eq!(all[1].reads, 7);
    }

    #[test]
    fn compact_drops_old_and_caps_count() {
        let d = dir("compact");
        let text: String = (1..=205)
            .map(|t| line(&sum(&format!("s{t}"), t, 0)))
            .collect();
        std::fs::write(d.join(FILE), text).unwrap();
        compact(&d, u64::MAX, 205);
        let all = load(&d);
        assert_eq!((all.len(), all[0].t, all[199].t), (200, 6, 205));
        compact(&d, 100, 300);
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert_eq!(text.lines().count(), 6);
        assert!(
            text.lines()
                .all(|l| serde_json::from_str::<Summary>(l).is_ok_and(|s| s.t >= 200))
        );
    }

    #[test]
    fn record_appends_and_compacts_past_max_bytes() {
        let d = dir("record");
        let one = line(&sum("big", 1, 1));
        let n = MAX_BYTES as usize / one.len();
        std::fs::write(d.join(FILE), one.repeat(n)).unwrap();
        record(&d, &sum("new", 2, 0));
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn missing_file_is_empty_and_not_created() {
        let d = dir("missing");
        assert!(load(&d).is_empty());
        compact(&d, 1, 1);
        assert!(!d.join(FILE).exists());
    }
}

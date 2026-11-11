//! Quality baseline: one summary line per session in `<state root>/quality.jsonl`
//! (counts only: no prompts, paths or output), compared against the user's own usual.

use crate::{hook, paths, state::State};
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

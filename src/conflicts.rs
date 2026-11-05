//! Files another live session edited recently, that this session edited too.

use crate::sessions::{self, SessionInfo};
use crate::state::{self, State};
use serde::{Deserialize, Serialize};
use std::path::Path;

const WINDOW_MS: u64 = 30 * 60_000;
const DEBOUNCE_MS: u64 = 5_000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conflict {
    pub path: String,
    /// The other session: folder name and short id (`tankobon #bbbb`).
    pub other: String,
}

fn recent(st: &State, now: u64) -> impl Iterator<Item = &String> {
    st.edits
        .iter()
        .filter(move |(_, t)| now.saturating_sub(**t) < WINDOW_MS)
        .map(|(p, _)| p)
}

fn label(s: &SessionInfo) -> String {
    let name = s
        .cwd
        .as_deref()
        .and_then(|c| c.trim_end_matches('/').rsplit('/').next())
        .filter(|n| !n.is_empty())
        .unwrap_or("session");
    format!("{name} #{}", s.id.chars().take(4).collect::<String>())
}

/// Files the session in `me` edited in the last 30 min that another live session under `root`
/// also edited in that time.
pub fn scan(root: &Path, me: &Path, now: u64) -> Vec<Conflict> {
    let mine = state::load_session(me);
    let my: Vec<&String> = recent(&mine, now).collect();
    if my.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for s in sessions::list(root)
        .iter()
        .filter(|s| s.dir != me && sessions::is_live(s, now))
    {
        let other = state::load_session(&s.dir);
        for p in recent(&other, now).filter(|p| my.contains(p)) {
            out.push(Conflict {
                path: p.clone(),
                other: label(s),
            });
        }
    }
    out
}

/// `scan`, at most every 5 s per session; the result is cached in `conflicts.json`.
pub fn cached(root: &Path, me: &Path, now: u64) -> Vec<Conflict> {
    let path = me.join("conflicts.json");
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_millis() < DEBOUNCE_MS as u128);
    if fresh
        && let Some(c) = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    {
        return c;
    }
    let c = scan(root, me, now);
    if let Ok(bytes) = serde_json::to_vec(&c) {
        let _ = crate::paths::write_atomic(&path, &bytes);
    }
    c
}

/// `⚠ a.rs also in tankobon #bbbb +1`.
pub fn summary(c: &[Conflict]) -> Option<String> {
    let first = c.first()?;
    let file = first.path.rsplit('/').next().unwrap_or(&first.path);
    let more = if c.len() > 1 {
        format!(" +{}", c.len() - 1)
    } else {
        String::new()
    };
    Some(format!("⚠ {file} also in {}{more}", first.other))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, to_line};
    use std::path::{Path, PathBuf};

    const MIN: u64 = 60_000;

    fn now() -> u64 {
        crate::hook::now_ms()
    }

    fn root(name: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("ari-conflicts-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&r);
        r
    }

    fn edit(t: u64, path: &str) -> Event {
        Event {
            t,
            e: "PostToolUse".into(),
            tool: Some("Edit".into()),
            sig: Some(format!("Edit:{path}")),
            ok: Some(true),
            ..Default::default()
        }
    }

    fn session(root: &Path, id: &str, cwd: &str, events: &[Event]) -> PathBuf {
        let dir = root.join("sessions").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.json"), format!(r#"{{"cwd":"{cwd}"}}"#)).unwrap();
        let text: String = events.iter().map(to_line).collect();
        std::fs::write(dir.join("events.jsonl"), text).unwrap();
        dir
    }

    #[test]
    fn both_sessions_see_a_shared_file() {
        let r = root("shared");
        let t = now();
        let a = session(&r, "aaaa1111", "/x/ari", &[edit(t - MIN, "/x/a.rs")]);
        let b = session(
            &r,
            "bbbb2222",
            "/x/tankobon",
            &[edit(t - 2 * MIN, "/x/a.rs")],
        );
        let from_a = scan(&r, &a, t);
        assert_eq!(
            from_a,
            vec![Conflict {
                path: "/x/a.rs".into(),
                other: "tankobon #bbbb".into()
            }]
        );
        assert_eq!(scan(&r, &b, t)[0].other, "ari #aaaa");
    }

    #[test]
    fn old_edits_ended_sessions_and_self_dont_count() {
        let r = root("old");
        let t = now();
        let a = session(&r, "aaaa1111", "/x/ari", &[edit(t - MIN, "/x/a.rs")]);
        session(&r, "bbbb2222", "/x/b", &[edit(t - 31 * MIN, "/x/a.rs")]);
        let ended = Event {
            t: t - MIN,
            e: "SessionEnd".into(),
            ..Default::default()
        };
        session(&r, "cccc3333", "/x/c", &[edit(t - MIN, "/x/a.rs"), ended]);
        assert_eq!(scan(&r, &a, t), vec![]);
    }

    #[test]
    fn short_labels() {
        let c = |p: &str| Conflict {
            path: p.into(),
            other: "tankobon #bbbb".into(),
        };
        assert_eq!(summary(&[]), None);
        assert_eq!(
            summary(&[c("/x/a.rs")]).unwrap(),
            "⚠ a.rs also in tankobon #bbbb"
        );
        assert_eq!(
            summary(&[c("/x/a.rs"), c("/x/b.rs")]).unwrap(),
            "⚠ a.rs also in tankobon #bbbb +1"
        );
    }
}

//! Find recorded sessions and pick the one to watch.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const LIVE_MS: u64 = 30 * 60_000;

#[derive(Clone, Debug, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub dir: PathBuf,
    pub cwd: Option<String>,
    pub last_ms: u64,
    pub ended: bool,
}

fn mtime_ms(p: &Path) -> Option<u64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

fn json(p: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn cwd_of(dir: &Path) -> Option<String> {
    let from = |v: Value, ptrs: &[&str]| {
        ptrs.iter()
            .find_map(|p| v.pointer(p)?.as_str().map(String::from))
    };
    json(&dir.join("meta.json"))
        .and_then(|m| from(m, &["/cwd"]))
        .or_else(|| {
            json(&dir.join("usage.json")).and_then(|u| from(u, &["/workspace/current_dir", "/cwd"]))
        })
}

/// Reads only the file's tail: logs can be large and `list` runs every couple of seconds.
fn last_line_is_end(p: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(p) else {
        return false;
    };
    let len = f.metadata().map_or(0, |m| m.len());
    let mut tail = Vec::new();
    if f.seek(SeekFrom::Start(len.saturating_sub(512))).is_err()
        || f.read_to_end(&mut tail).is_err()
    {
        return false;
    }
    let text = String::from_utf8_lossy(&tail);
    text.trim_end()
        .lines()
        .last()
        .is_some_and(|l| l.contains(r#""e":"SessionEnd""#))
}

/// Every session under `root`, most recently active first.
pub fn list(root: &Path) -> Vec<SessionInfo> {
    let Ok(entries) = std::fs::read_dir(root.join("sessions")) else {
        return Vec::new();
    };
    let mut out: Vec<SessionInfo> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let dir = e.path();
            let last_ms = [dir.join("events.jsonl"), dir.join("usage.json")]
                .iter()
                .filter_map(|p| mtime_ms(p))
                .max()
                .or_else(|| mtime_ms(&dir))
                .unwrap_or(0);
            let ended = last_line_is_end(&dir.join("events.jsonl"));
            SessionInfo {
                id: e.file_name().to_string_lossy().into_owned(),
                cwd: cwd_of(&dir),
                dir,
                last_ms,
                ended,
            }
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.last_ms));
    out
}

pub fn is_live(s: &SessionInfo, now_ms: u64) -> bool {
    !s.ended && now_ms.saturating_sub(s.last_ms) < LIVE_MS
}

/// Most recently active session started in `cwd`.
pub fn pick<'a>(list: &'a [SessionInfo], cwd: &str) -> Option<&'a SessionInfo> {
    list.iter().find(|s| s.cwd.as_deref() == Some(cwd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn session(
        root: &Path,
        id: &str,
        meta: Option<&str>,
        usage: Option<&str>,
        events: &str,
        age_s: u64,
    ) {
        let d = root.join("sessions").join(id);
        std::fs::create_dir_all(&d).unwrap();
        if let Some(m) = meta {
            std::fs::write(d.join("meta.json"), m).unwrap();
        }
        if let Some(u) = usage {
            std::fs::write(d.join("usage.json"), u).unwrap();
        }
        let f = std::fs::File::create(d.join("events.jsonl")).unwrap();
        std::io::Write::write_all(&mut &f, events.as_bytes()).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age_s))
            .unwrap();
        if usage.is_some() {
            let f = std::fs::File::options()
                .append(true)
                .open(d.join("usage.json"))
                .unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(age_s))
                .unwrap();
        }
    }

    #[test]
    fn lists_and_picks() {
        let root = std::env::temp_dir().join(format!("ari-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        session(&root, "old", Some(r#"{"cwd":"/p"}"#), None, "{}\n", 600);
        session(
            &root,
            "new",
            None,
            Some(r#"{"workspace":{"current_dir":"/p"}}"#),
            "{}\n",
            10,
        );
        session(&root, "other", Some(r#"{"cwd":"/q"}"#), None, "{}\n", 5);
        session(
            &root,
            "gone",
            Some(r#"{"cwd":"/p"}"#),
            None,
            "{\"t\":1,\"e\":\"SessionEnd\"}\n",
            1,
        );
        let l = list(&root);
        let ids: Vec<&str> = l.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["gone", "other", "new", "old"]);
        assert_eq!(l[2].cwd.as_deref(), Some("/p"));
        assert!(l[0].ended);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(!is_live(&l[0], now) && is_live(&l[1], now));
        assert!(!is_live(
            &SessionInfo {
                last_ms: now - LIVE_MS - 1,
                ..l[1].clone()
            },
            now
        ));
        // `gone` is the newest in /p, but ended sessions still win `pick`; watch skips them itself.
        assert_eq!(pick(&l, "/p").unwrap().id, "gone");
        assert_eq!(pick(&l, "/q").unwrap().id, "other");
        assert!(pick(&l, "/nowhere").is_none());
        assert!(list(&root.join("missing")).is_empty());
    }
}

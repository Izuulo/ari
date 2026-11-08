//! Desktop and ntfy notifications. Text only ever travels as its own argv element, never via a shell or script string.
use crate::config::{Config, Notify, ntfy_ok};
use crate::state::{State, Verdict};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

pub const NOTE_MAX_CHARS: usize = 140;

pub fn title(dir: &Path) -> String {
    match crate::sessions::cwd_of(dir)
        .as_deref()
        .and_then(|c| Path::new(c).file_name()?.to_str().map(String::from))
    {
        Some(name) => format!("ARI: {name}"),
        None => "ARI".into(),
    }
}

fn clean(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    crate::event::truncate(&flat, NOTE_MAX_CHARS)
}

pub fn commands(note: &Note, n: &Notify, os: &str) -> Vec<Vec<String>> {
    let (t, b) = (clean(&note.title), clean(&note.body));
    let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut out = vec![];
    if n.desktop && os == "macos" {
        let mut c = argv(&[
            "osascript",
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
        ]);
        c.extend([t.clone(), b.clone()]);
        out.push(c);
    } else if n.desktop && os == "linux" {
        let mut c = argv(&["notify-send", "--app-name=ARI", "--"]);
        c.extend([t.clone(), b.clone()]);
        out.push(c);
    }
    if ntfy_ok(&n.ntfy) {
        let mut c = argv(&["curl", "-fsS", "-m", "10", "-H"]);
        c.extend([
            format!("Title: {t}"),
            "--data-raw".into(),
            b,
            "--".into(),
            n.ntfy.clone(),
        ]);
        out.push(c);
    }
    out
}

/// Fire and forget: own process group, no stdio, never waited on.
pub fn spawn_all(cmds: &[Vec<String>]) {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    for c in cmds {
        let Some((prog, args)) = c.split_first() else {
            continue;
        };
        let _ = Command::new(prog)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
    }
}

pub fn send(note: &Note, n: &Notify) {
    spawn_all(&commands(note, n, std::env::consts::OS))
}

/// Last verdict seen and last verdict notified, kept in `notify.json` in the session dir.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Seen {
    pub v: String,
    pub fired: String,
    pub fired_t: u64,
    pub fired_turn: u32,
}

/// The same verdict doesn't notify twice for one session within this.
pub const REPEAT_MS: u64 = 60_000;

/// New `Seen` and whether this verdict flip should notify.
pub fn decide(
    prev: &Seen,
    v: Verdict,
    turn_active: bool,
    turns: u32,
    now: u64,
    on: &[String],
) -> (Seen, bool) {
    let id = v.id();
    let turn_end_reported = v == Verdict::NeedsYou
        && !turn_active
        && matches!(prev.fired.as_str(), "done" | "idle" | "blocked")
        && prev.fired_turn == turns;
    let fire = on.iter().any(|o| o == id)
        && prev.v != id
        && !(prev.fired == id && now.saturating_sub(prev.fired_t) < REPEAT_MS)
        && !turn_end_reported;
    let mut next = Seen {
        v: id.into(),
        ..prev.clone()
    };
    if fire {
        next.fired = id.into();
        next.fired_t = now;
        next.fired_turn = turns;
    }
    (next, fire)
}

pub fn note(dir: &Path, st: &State, v: Verdict, repeats: u8) -> Note {
    let detail = match v {
        Verdict::NeedsYou => st
            .needs_you
            .clone()
            .or_else(|| {
                // PermissionRequest lands before the Notification, so needs_you may still be unset.
                st.waiting().map(|a| match a.waiting.as_ref() {
                    Some((_, req)) if !req.is_empty() => req.clone(),
                    _ => a.label(),
                })
            }),
        Verdict::Stuck => st.stuck_reason(repeats),
        _ => None,
    };
    let mut body = format!("{} {}", v.glyph(), v.label());
    if let Some(d) = detail {
        body += &format!(" \u{b7} {d}");
    }
    Note {
        title: title(dir),
        body,
    }
}

/// Under an exclusive lock on `notify.json`, decide whether the session just flipped into a
/// verdict worth a notification. Concurrent hooks of one session therefore fire once. Does not send.
pub fn check(dir: &Path, cfg: &Config, now: u64) -> Option<Note> {
    if !cfg.notify.desktop && cfg.notify.ntfy.is_empty() {
        return None;
    }
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("notify.json"))
        .ok()?;
    // The lock is released when `f` drops.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return None;
    }
    let mut raw = String::new();
    let _ = f.read_to_string(&mut raw);
    let prev: Seen = serde_json::from_str(&raw).unwrap_or_default();
    let st = crate::state::load_session(dir);
    let repeats = cfg.thresholds.stuck_repeats;
    let v = st.verdict_with(repeats);
    let (next, fire) = decide(&prev, v, st.turn_active, st.turns, now, &cfg.notify.on);
    // In place: write_atomic would swap the inode out from under the lock.
    if next != prev {
        let bytes = serde_json::to_vec(&next).ok()?;
        f.set_len(0).ok()?;
        f.rewind().ok()?;
        f.write_all(&bytes).ok()?;
    }
    fire.then(|| note(dir, &st, v, repeats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> Note {
        Note {
            title: "ARI: proj".into(),
            body: "◉ NEEDS YOU · Bash".into(),
        }
    }
    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn macos_and_linux_commands() {
        let n = Notify::default();
        assert_eq!(
            commands(&note(), &n, "macos"),
            vec![strs(&[
                "osascript",
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run",
                "ARI: proj",
                "◉ NEEDS YOU · Bash"
            ])]
        );
        assert_eq!(
            commands(&note(), &n, "linux"),
            vec![strs(&[
                "notify-send",
                "--app-name=ARI",
                "--",
                "ARI: proj",
                "◉ NEEDS YOU · Bash"
            ])]
        );
        assert!(commands(&note(), &n, "windows").is_empty());
        let off = Notify {
            desktop: false,
            ..Notify::default()
        };
        assert!(commands(&note(), &off, "macos").is_empty());
    }

    #[test]
    fn ntfy_only_when_set_and_valid() {
        let mut n = Notify {
            desktop: false,
            ..Notify::default()
        };
        n.ntfy = "https://ntfy.sh/t".into();
        assert_eq!(
            commands(&note(), &n, "macos"),
            vec![strs(&[
                "curl",
                "-fsS",
                "-m",
                "10",
                "-H",
                "Title: ARI: proj",
                "--data-raw",
                "◉ NEEDS YOU · Bash",
                "--",
                "https://ntfy.sh/t"
            ])]
        );
        for bad in ["ftp://x", "-K /etc/passwd", ""] {
            n.ntfy = bad.into();
            assert!(commands(&note(), &n, "macos").is_empty());
        }
    }

    #[test]
    fn hostile_text_stays_one_argv_element() {
        let body = "\"; do shell script \"rm -rf ~\" --\n@/etc/passwd $(id)";
        let want = body.replace('\n', " ");
        let n = Notify {
            ntfy: "https://ntfy.sh/t".into(),
            ..Notify::default()
        };
        let note = Note {
            title: "ARI: proj".into(),
            body: body.into(),
        };
        for os in ["macos", "linux"] {
            let cmds = commands(&note, &n, os);
            assert_eq!(cmds.len(), 2);
            for c in &cmds {
                assert_eq!(c.iter().filter(|a| **a == want).count(), 1);
            }
        }
        let mac = &commands(&note, &n, "macos")[0];
        assert_eq!(
            mac[..7],
            strs(&[
                "osascript",
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run"
            ])[..]
        );
    }

    #[test]
    fn long_text_is_cut() {
        let note = Note {
            title: "t".into(),
            body: "x".repeat(500),
        };
        let c = &commands(&note, &Notify::default(), "linux")[0];
        assert!(c[4].chars().count() <= NOTE_MAX_CHARS);
    }

    #[test]
    fn title_from_cwd() {
        let d = std::env::temp_dir().join(format!("ari-notify-title-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(title(&d), "ARI");
        std::fs::write(d.join("meta.json"), r#"{"cwd":"/x/tankobon"}"#).unwrap();
        assert_eq!(title(&d), "ARI: tankobon");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn spawn_ignores_missing_binary() {
        spawn_all(&[vec!["/nonexistent/ari-notify".into()]]);
    }

    fn on() -> Vec<String> {
        Notify::default().on
    }
    const T: u64 = 1_000_000;

    #[test]
    fn fires_once_per_flip() {
        let o = on();
        let (s, f) = decide(&Seen::default(), Verdict::OnTrack, true, 1, T, &o);
        assert!(!f);
        let (s, f) = decide(&s, Verdict::Stuck, true, 1, T + 1, &o);
        assert!(f);
        let (s, f) = decide(&s, Verdict::Stuck, true, 1, T + 2, &o);
        assert!(!f);
        let (s, _) = decide(&s, Verdict::OnTrack, true, 1, T + 3, &o);
        let (_, f) = decide(&s, Verdict::Stuck, true, 1, T + 120_000, &o);
        assert!(f);
    }

    #[test]
    fn not_in_on_list_never_fires() {
        let o = vec!["done".to_string()];
        let (s, f) = decide(&Seen::default(), Verdict::NeedsYou, true, 1, T, &o);
        assert!(!f);
        assert_eq!(s.v, "needs_you");
    }

    #[test]
    fn same_verdict_within_repeat_ms_is_quiet() {
        let o = on();
        let (s, f) = decide(&Seen::default(), Verdict::Stuck, true, 1, T, &o);
        assert!(f);
        let (s, _) = decide(&s, Verdict::Retrying, true, 1, T + 10_000, &o);
        let (s2, f) = decide(&s, Verdict::Stuck, true, 1, T + 20_000, &o);
        assert!(!f);
        let (_, f) = decide(&s2, Verdict::Stuck, true, 1, T + REPEAT_MS + 1, &o);
        assert!(!f, "still the same verdict, not a flip");
        let (s3, _) = decide(&s2, Verdict::Retrying, true, 1, T + REPEAT_MS + 1, &o);
        let (_, f) = decide(&s3, Verdict::Stuck, true, 1, T + REPEAT_MS + 2, &o);
        assert!(f);
    }

    #[test]
    fn idle_prompt_after_done_does_not_double_notify() {
        let o = on();
        let (s, f) = decide(&Seen::default(), Verdict::Done, false, 3, T, &o);
        assert!(f);
        let (s, f) = decide(&s, Verdict::NeedsYou, false, 3, T + 60_000, &o);
        assert!(!f);
        let (s, _) = decide(&s, Verdict::OnTrack, true, 4, T + 70_000, &o);
        let (_, f) = decide(&s, Verdict::NeedsYou, true, 4, T + 80_000, &o);
        assert!(f);
    }

    #[test]
    fn idle_prompt_after_unnotified_idle_fires() {
        let o = on();
        let (s, f) = decide(&Seen::default(), Verdict::Idle, false, 3, T, &o);
        assert!(!f);
        let (_, f) = decide(&s, Verdict::NeedsYou, false, 3, T + 60_000, &o);
        assert!(f);
    }

    fn stuck_dir(name: &str) -> std::path::PathBuf {
        use crate::event::{Event, to_line};
        let d = std::env::temp_dir().join(format!("ari-notify-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ev = |e: &str| Event {
            t: 1_000,
            e: e.into(),
            ..Default::default()
        };
        let mut lines = to_line(&Event {
            msg: Some("go".into()),
            ..ev("UserPromptSubmit")
        });
        let fail = Event {
            tool: Some("Bash".into()),
            sig: Some("Bash:cargo build".into()),
            ok: Some(false),
            act: Some("$ cargo build".into()),
            ..ev("PostToolUseFailure")
        };
        for _ in 0..3 {
            lines += &to_line(&fail);
        }
        std::fs::write(d.join("events.jsonl"), lines).unwrap();
        d
    }

    fn cfg(desktop: bool, ntfy: &str) -> Config {
        let mut c = Config::default();
        c.notify.desktop = desktop;
        c.notify.ntfy = ntfy.into();
        c
    }

    #[test]
    fn off_does_nothing() {
        let d = stuck_dir("off");
        assert_eq!(check(&d, &cfg(false, ""), T), None);
        assert!(!d.join("notify.json").exists() && !d.join("state.json").exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn needs_you_names_the_request() {
        use crate::event::{Event, to_line};
        let d = std::env::temp_dir().join(format!("ari-notify-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ev = |e: &str| Event {
            t: 1_000,
            e: e.into(),
            ..Default::default()
        };
        let lines = to_line(&Event {
            msg: Some("go".into()),
            ..ev("UserPromptSubmit")
        }) + &to_line(&Event {
            tool: Some("Bash".into()),
            act: Some("$ rm -rf build".into()),
            ..ev("PermissionRequest")
        });
        std::fs::write(d.join("events.jsonl"), lines).unwrap();
        let n = check(&d, &cfg(true, ""), T).expect("fires");
        assert!(n.body.contains("NEEDS YOU"), "{}", n.body);
        assert!(n.body.contains("rm -rf build"), "{}", n.body);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn check_reports_stuck_once() {
        let d = stuck_dir("once");
        let c = cfg(true, "");
        let n = check(&d, &c, T).expect("fires");
        assert!(n.body.starts_with("\u{21bb} STUCK \u{b7} "), "{}", n.body);
        assert_eq!(check(&d, &c, T + 1), None);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn concurrent_checks_fire_once() {
        let d = stuck_dir("conc");
        let c = cfg(true, "");
        let fired: usize = (0..8)
            .map(|_| {
                let (d, c) = (d.clone(), c.clone());
                std::thread::spawn(move || check(&d, &c, T).is_some())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap() as usize)
            .sum();
        assert_eq!(fired, 1);
        std::fs::remove_dir_all(&d).ok();
    }
}

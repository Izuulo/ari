//! Desktop and ntfy notifications. Text only ever travels as its own argv element, never via a shell or script string.
use crate::config::{Notify, ntfy_ok};
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
}

//! The panel's git section: branch, recent commits and GitHub PRs for the session's repo.
//! Fetched off the draw thread (`gh` talks to the network), at most every `EVERY`.

use serde::Deserialize;
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const EVERY: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Git {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    /// Changed or untracked files.
    pub dirty: u32,
    /// Newest first.
    pub commits: Vec<Commit>,
    /// Most recently updated first; empty without `gh` or a GitHub remote.
    pub prs: Vec<Pr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    /// Unix ms.
    pub t: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    /// `OPEN`, `MERGED` or `CLOSED`.
    pub state: String,
    #[serde(rename = "isDraft", default)]
    pub draft: bool,
    #[serde(rename = "updatedAt")]
    pub updated: String,
}

fn run(cmd: &mut Command) -> Option<String> {
    let out = cmd
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// None when `cwd` isn't a git repo.
pub fn fetch(cwd: &str) -> Option<Git> {
    let git = |args: &[&str]| run(Command::new("git").arg("-C").arg(cwd).args(args));
    let mut g = parse_status(&git(&["status", "--porcelain=v2", "--branch"])?);
    g.commits =
        parse_log(&git(&["log", "-n", "3", "--format=%h%x09%ct%x09%s"]).unwrap_or_default());
    g.prs = run(Command::new("gh").current_dir(cwd).args([
        "pr",
        "list",
        "--state",
        "all",
        "--limit",
        "3",
        "--json",
        "number,title,state,isDraft,updatedAt",
    ]))
    .and_then(|s| serde_json::from_str(&s).ok())
    .unwrap_or_default();
    Some(g)
}

fn parse_status(s: &str) -> Git {
    let mut g = Git::default();
    for l in s.lines() {
        if let Some(b) = l.strip_prefix("# branch.head ") {
            g.branch = b.to_string();
        } else if let Some(ab) = l.strip_prefix("# branch.ab ") {
            let mut it = ab.split(' ');
            g.ahead = it
                .next()
                .and_then(|a| a.trim_start_matches('+').parse().ok())
                .unwrap_or(0);
            g.behind = it
                .next()
                .and_then(|b| b.trim_start_matches('-').parse().ok())
                .unwrap_or(0);
        } else if !l.starts_with('#') && !l.is_empty() {
            g.dirty += 1;
        }
    }
    g
}

fn parse_log(s: &str) -> Vec<Commit> {
    s.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            Some(Commit {
                hash: it.next()?.to_string(),
                t: it.next()?.parse::<u64>().ok()? * 1000,
                subject: it.next()?.to_string(),
            })
        })
        .collect()
}

/// Unix ms of an RFC 3339 UTC time as `gh` prints it (`2026-11-07T10:20:30Z`).
pub fn parse_utc(s: &str) -> Option<u64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(((days * 24 + hh) * 60 + mm) * 60 + ss)
        .ok()
        .map(|s| s * 1000)
}

/// Keeps one repo's `Git` fresh in the background; `poll` every frame.
#[derive(Default)]
pub struct Watcher {
    cwd: String,
    job: Option<JoinHandle<Option<Git>>>,
    at: Option<Instant>,
    pub git: Option<Git>,
}

impl Watcher {
    pub fn poll(&mut self, cwd: Option<&str>) {
        let Some(cwd) = cwd else {
            self.git = None;
            return;
        };
        if cwd != self.cwd {
            *self = Self {
                cwd: cwd.to_string(),
                ..Default::default()
            };
        }
        if self.job.as_ref().is_some_and(|j| j.is_finished()) {
            self.git = self.job.take().and_then(|j| j.join().ok()).flatten();
        }
        if self.job.is_none() && self.at.is_none_or(|t| t.elapsed() >= EVERY) {
            self.at = Some(Instant::now());
            let cwd = self.cwd.clone();
            self.job = Some(std::thread::spawn(move || fetch(&cwd)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_branch_ahead_dirty() {
        let g = parse_status(
            "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b src/x.rs\n? new.txt\n",
        );
        assert_eq!(
            (g.branch.as_str(), g.ahead, g.behind, g.dirty),
            ("main", 2, 1, 2)
        );
    }

    #[test]
    fn log_lines() {
        let c = parse_log("1ec1c52\t1762500000\tMerge pull request #38\tx\nbad\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].subject, "Merge pull request #38\tx");
        assert_eq!(c[0].t, 1_762_500_000_000);
    }

    #[test]
    fn utc() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2026-11-07T10:20:30Z"), Some(1_794_046_830_000));
        assert_eq!(parse_utc("nope"), None);
    }

    #[test]
    fn prs_from_gh_json() {
        let p: Vec<Pr> = serde_json::from_str(
            r#"[{"number":38,"title":"t","state":"MERGED","isDraft":false,"updatedAt":"2026-11-07T10:20:30Z"}]"#,
        )
        .unwrap();
        assert_eq!(p[0].state, "MERGED");
    }
}

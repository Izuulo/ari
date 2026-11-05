//! `ari recap`: what a session got done, assembled from its events. No LLM.

use crate::event::Event;
use crate::state::{State, TestRun};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Since {
    Start,
    /// The last real prompt: what Claude did with your latest message.
    Prompt,
    /// Milliseconds before now.
    Ago(u64),
}

impl Since {
    pub fn parse(s: &str) -> Option<Since> {
        match s {
            "start" => return Some(Since::Start),
            "prompt" => return Some(Since::Prompt),
            _ => {}
        }
        let unit = match s.chars().last()? {
            's' => 1_000,
            'm' => 60_000,
            'h' => 3_600_000,
            _ => return None,
        };
        let n: u64 = s[..s.len() - 1].parse().ok()?;
        Some(Since::Ago(n * unit))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recap {
    pub dir: String,
    pub title: Option<String>,
    pub since: String,
    pub from: u64,
    pub events: usize,
    pub verdict: String,
    pub done: Vec<String>,
    pub open: Vec<String>,
    pub errors_hit: usize,
    /// (what failed, who fixed it)
    pub fixed: Vec<(String, String)>,
    pub still_open: Vec<String>,
    pub tests: Option<(Option<TestRun>, TestRun)>,
    /// (file relative to the session dir, edits), most edited first.
    pub files: Vec<(String, u32)>,
    pub commits: Vec<String>,
    pub wall_ms: u64,
    pub split: Option<crate::state::Split>,
    pub cost: Option<f64>,
    pub lines: Option<(u64, u64)>,
    pub models: Vec<String>,
    pub failed_calls: u32,
    pub failed_ms: u64,
    pub reread: u32,
}

fn fold(events: &[Event]) -> State {
    let mut s = State::default();
    for e in events {
        s.apply(e);
    }
    s
}

/// Replay the session up to the window start and to now, and report the difference.
pub fn build(dir: &Path, since: Since, now: u64) -> Recap {
    let events = crate::replay::load(dir).unwrap_or_default();
    let usage = std::fs::read(dir.join("usage.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .map(|v| crate::usage::from_value(&v))
        .unwrap_or_default();
    let cwd = crate::sessions::cwd_of(dir).unwrap_or_default();
    let first = events.first().map_or(now, |e| e.t);
    let from = match since {
        Since::Start => first,
        Since::Prompt => events
            .iter()
            .rev()
            .find(|e| {
                e.e == "UserPromptSubmit"
                    && !crate::event::is_synthetic(e.msg.as_deref().unwrap_or(""))
            })
            .map_or(first, |e| e.t),
        Since::Ago(ms) => now.saturating_sub(ms),
    };
    let split_at = events.partition_point(|e| e.t < from);
    let (before, after) = (fold(&events[..split_at]), fold(&events));
    let window = &events[split_at..];
    let was_done = |id: &str| {
        before
            .tasks
            .iter()
            .any(|t| t.id == id && t.status == "completed")
    };
    let mut r = Recap {
        dir: cwd.clone(),
        title: usage.session_name.clone().or(after.label.clone()),
        since: match since {
            Since::Start => "start".into(),
            Since::Prompt => "your last prompt".into(),
            Since::Ago(ms) => format!("{} ago", crate::panel::dur(ms)),
        },
        from,
        events: window.len(),
        verdict: {
            let v = after.verdict(now);
            format!("{} {}", v.glyph(), v.label())
        },
        done: after
            .tasks
            .iter()
            .filter(|t| t.status == "completed" && !was_done(&t.id))
            .map(|t| t.subject.clone())
            .collect(),
        open: after
            .tasks
            .iter()
            .filter(|t| t.status != "completed")
            .map(|t| t.subject.clone())
            .collect(),
        ..Default::default()
    };
    for e in &after.errors {
        if e.first_t >= from {
            r.errors_hit += 1;
        }
        match (e.resolved_t, &e.resolved_by) {
            (Some(t), Some(by)) if t >= from => {
                r.fixed.push((crate::panel::sig_words(e), by.clone()));
            }
            (None, _) => r.still_open.push(crate::panel::sig_words(e)),
            _ => {}
        }
        if e.last_t >= from {
            // Only the failures inside the window: subtract what this error had before it.
            let prior = before
                .errors
                .iter()
                .find(|b| b.sig == e.sig && b.first_t == e.first_t)
                .map_or((0, 0), |b| (b.retries + 1, b.time_ms));
            r.failed_calls += (e.retries + 1).saturating_sub(prior.0);
            r.failed_ms += e.time_ms.saturating_sub(prior.1);
        }
    }
    r.reread = after.reread - before.reread;
    if let Some(last) = after.test_runs.back().filter(|t| t.t >= from) {
        r.tests = Some((before.test_runs.back().copied(), *last));
    }
    let mut files: BTreeMap<String, u32> = BTreeMap::new();
    for e in window.iter().filter(|e| e.e == "PostToolUse") {
        if let Some((tool, path)) = e.sig.as_deref().and_then(|s| s.split_once(':'))
            && matches!(tool, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
        {
            let rel = path
                .strip_prefix(&cwd)
                .map(|p| p.trim_start_matches('/'))
                .filter(|p| !cwd.is_empty() && !p.is_empty())
                .map(String::from)
                .unwrap_or_else(|| crate::panel::tilde(path));
            *files.entry(rel).or_default() += 1;
        }
    }
    r.files = files.into_iter().collect();
    r.files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if !cwd.is_empty() {
        r.commits = commits(&cwd, from);
    }
    r.wall_ms = after.last_t.saturating_sub(from);
    if r.wall_ms > 0 {
        let (a, b) = (&after.time, &before.time);
        let tools = a.tools_ms - b.tools_ms;
        // A stretch already open at the window start only counts from `from` on.
        let open = |since: Option<u64>| since.map_or(0, |s| from.saturating_sub(s));
        let waiting = (a.waiting_ms - b.waiting_ms).saturating_sub(open(b.waiting_since));
        let idle = (a.idle_ms - b.idle_ms).saturating_sub(open(b.idle_since));
        let api = (since == Since::Start)
            .then_some(usage.api_duration_ms)
            .flatten();
        r.split = Some(crate::state::Split {
            claude: api.unwrap_or(r.wall_ms.saturating_sub(tools + waiting + idle)),
            tools,
            waiting,
            idle,
            other: 0,
        });
    }
    if since == Since::Start {
        r.cost = usage.cost_usd;
        r.lines = usage.lines_added.zip(usage.lines_removed);
    } else {
        let samples = crate::burn::recent(dir);
        let cost_at = |pick: &dyn Fn(&&crate::burn::Sample) -> bool| {
            samples.iter().filter(pick).find_map(|s| s.cost)
        };
        let start = samples
            .iter()
            .rev()
            .filter(|s| s.t <= from)
            .find_map(|s| s.cost)
            .or_else(|| cost_at(&|s| s.t >= from));
        let end = samples.iter().rev().find_map(|s| s.cost);
        r.cost = start.zip(end).map(|(a, b)| b - a).filter(|c| *c > 0.0);
    }
    let tokens = crate::tokens::cached(dir);
    let mut models: Vec<(String, &str)> = Vec::new();
    if let Some(m) = &tokens.main.last_model {
        models.push((crate::tokens::short_model(m), "main"));
    }
    for l in tokens.agents.values() {
        if let Some(m) = &l.last_model {
            models.push((crate::tokens::short_model(m), "agent"));
        }
    }
    // "main" sorts before "agent" only if keyed so: 0 for main, 1 for agents.
    let mut seen: BTreeMap<(u8, String), u32> = BTreeMap::new();
    for (m, who) in models.into_iter().filter(|(m, _)| m != "unknown") {
        *seen.entry((u8::from(who != "main"), m)).or_default() += 1;
    }
    r.models = seen
        .into_iter()
        .map(|((agent, m), n)| match (agent, n) {
            (0, _) => format!("{m} (main)"),
            (_, 1) => format!("{m} (1 agent)"),
            (_, n) => format!("{m} ({n} agents)"),
        })
        .collect();
    r
}

fn fail(msg: &str) -> ! {
    eprintln!("ari recap: {msg}");
    std::process::exit(2)
}

pub fn run(args: &[String]) -> ! {
    let (mut session, mut since, mut md) = (None, Since::Start, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--md" => md = true,
            "--session" => {
                let id = it.next().unwrap_or_else(|| fail("--session needs an id"));
                if !crate::paths::valid_session_id(id) {
                    fail("invalid session id");
                }
                session = Some(id.clone());
            }
            "--since" => {
                since = it
                    .next()
                    .and_then(|s| Since::parse(s))
                    .unwrap_or_else(|| fail("--since takes start, prompt, or a time like 30m"));
            }
            other => fail(&format!(
                "unknown option {other} (try --session <id>, --since <start|prompt|30m>, --md)"
            )),
        }
    }
    let root = crate::paths::state_root().unwrap_or_else(|| fail("HOME is not set"));
    let dir = match session {
        Some(id) => crate::paths::session_dir(&root, &id),
        None => {
            let cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            crate::sessions::pick(&crate::sessions::list(&root), &cwd).map(|s| s.dir.clone())
        }
    }
    .filter(|d| d.is_dir())
    .unwrap_or_else(|| fail("no session found here (try --session <id>)"));
    let r = build(&dir, since, crate::hook::now_ms());
    if md {
        print!("{}", markdown(&r));
    } else {
        use std::io::IsTerminal;
        let width = ratatui::crossterm::terminal::size()
            .ok()
            .filter(|_| std::io::stdout().is_terminal())
            .map(|(w, _)| w)
            .or_else(|| std::env::var("COLUMNS").ok()?.parse().ok())
            .unwrap_or(80);
        let cfg = crate::config::load();
        let lines = crate::panel::recap_lines(&r, width, false, cfg.ascii);
        println!("{}", crate::panel::to_plain(&lines));
    }
    std::process::exit(0)
}

/// `git log` since `since_ms` in `cwd`, newest first, max 10. Empty on any problem.
pub fn commits(cwd: &str, since_ms: u64) -> Vec<String> {
    let Ok(mut child) = std::process::Command::new("git")
        .args([
            "-C",
            cwd,
            "log",
            &format!("--since=@{}", since_ms / 1000),
            "--format=%h %s",
            "-n",
            "10",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                return Vec::new();
            }
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        use std::io::Read;
        let _ = s.read_to_string(&mut out);
    }
    out.lines().map(String::from).collect()
}

fn files_line(r: &Recap) -> String {
    r.files
        .iter()
        .take(8)
        .map(|(f, n)| {
            if *n > 1 {
                format!("{f} ×{n}")
            } else {
                f.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
        + &if r.files.len() > 8 {
            format!(" and {} more", r.files.len() - 8)
        } else {
            String::new()
        }
}

pub(crate) fn split_line(r: &Recap) -> Option<String> {
    let sp = r.split?;
    let total = r.wall_ms.max(sp.claude + sp.tools + sp.waiting + sp.idle) as f64;
    let mut s = crate::panel::dur(r.wall_ms);
    for (name, ms) in [
        ("claude", sp.claude),
        ("tools", sp.tools),
        ("you", sp.waiting),
        ("idle", sp.idle),
    ] {
        let pct = (ms as f64 * 100.0 / total).round();
        if pct > 0.0 {
            s += &format!(" · {name} {pct:.0}%");
        }
    }
    Some(s)
}

pub(crate) fn tests_line(r: &Recap) -> Option<String> {
    let (before, now) = r.tests?;
    let show = |t: TestRun| {
        if t.fail > 0 {
            format!("{} passed, {} failed", t.pass, t.fail)
        } else {
            format!("{} passed", t.pass)
        }
    };
    Some(match before {
        Some(b) => format!("{} → {}", show(b), show(now)),
        None => show(now),
    })
}

pub(crate) fn mistakes_line(r: &Recap) -> Option<String> {
    if r.failed_calls == 0 && r.reread == 0 {
        return None;
    }
    let s = if r.failed_calls == 1 { "" } else { "s" };
    let mut line = format!(
        "{} failed call{s} · {}",
        r.failed_calls,
        crate::panel::dur(r.failed_ms)
    );
    if r.reread > 0 {
        line += &format!(
            " · {} re-read{}",
            r.reread,
            if r.reread == 1 { "" } else { "s" }
        );
    }
    Some(line)
}

/// Markdown bullets, for pasting into a PR.
pub fn markdown(r: &Recap) -> String {
    let mut out = String::from("### Recap\n\n");
    let mut item = |k: &str, v: String| out += &format!("- **{k}:** {v}\n");
    if !r.done.is_empty() {
        item("Done", r.done.join(", "));
    }
    if !r.open.is_empty() {
        item("Still open", r.open.join(", "));
    }
    if !r.fixed.is_empty() {
        item(
            "Fixed",
            r.fixed
                .iter()
                .map(|(w, _)| w.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if !r.still_open.is_empty() {
        item("Failing", r.still_open.join(", "));
    }
    if let Some(t) = tests_line(r) {
        item("Tests", t);
    }
    if !r.files.is_empty() {
        item("Files", files_line(r));
    }
    if !r.commits.is_empty() {
        item("Commits", r.commits.join("; "));
    }
    if let Some(s) = split_line(r) {
        item("Time", s);
    }
    if let Some(c) = r.cost {
        item("Cost", format!("${c:.2}"));
    }
    if !r.models.is_empty() {
        item("Models", r.models.join(", "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, to_line};
    use std::path::{Path, PathBuf};

    fn fixture_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ari-recap-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/multi/raw.jsonl");
        std::fs::copy(src, d.join("raw.jsonl")).unwrap();
        std::fs::write(d.join("meta.json"), r#"{"cwd":"/home/user/proj"}"#).unwrap();
        d
    }

    /// Last event of the multi fixture + 1 s.
    const END: u64 = 1_793_901_788_723 + 1_000;

    #[test]
    fn whole_session() {
        let d = fixture_dir("start");
        let r = build(&d, Since::Start, END);
        assert_eq!(r.done, vec!["scaffold", "build", "survey", "summarize"]);
        assert!(r.open.is_empty());
        assert_eq!((r.errors_hit, r.fixed.len(), r.still_open.len()), (1, 1, 0));
        assert_eq!(
            r.files,
            vec![("calc.py".to_string(), 2), ("test_calc.py".to_string(), 1)]
        );
        assert_eq!(r.dir, "/home/user/proj");
        let text = crate::panel::to_plain(&crate::panel::recap_lines(&r, 60, false, false));
        assert!(text.starts_with("╭─ ARI recap · since start "), "{text}");
        assert!(text.contains("│ ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰ 4/4 tasks"), "{text}");
        assert!(text.contains("│ ✓ scaffold"), "{text}");
        assert!(text.contains("├─ errors "), "{text}");
        assert!(text.contains("│ 1 hit · 1 fixed · 0 open"), "{text}");
        assert!(text.contains("│ calc.py ×2"), "{text}");
        assert!(text.contains("│ time     "), "{text}");
        assert!(!text.contains("├─ commits"), "not a git repo: {text}");
        assert!(text.trim_end().ends_with('╯'), "{text}");
        let md = markdown(&r);
        assert!(md.starts_with("### Recap"), "{md}");
        assert!(
            md.contains("- **Done:** scaffold, build, survey, summarize"),
            "{md}"
        );
        assert!(md.contains("- **Files:** calc.py ×2, test_calc.py"), "{md}");
    }

    #[test]
    fn last_seconds_window() {
        let d = fixture_dir("ago");
        let r = build(&d, Since::Ago(11_000), END);
        assert_eq!(r.done, vec!["build", "survey", "summarize"]);
        assert_eq!(
            (r.errors_hit, r.fixed.len()),
            (0, 1),
            "hit before the window, fixed in it"
        );
        assert_eq!(r.files, vec![("calc.py".to_string(), 1)]);
    }

    fn session_from(name: &str, events: &[Event]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ari-recap-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let text: String = events.iter().map(to_line).collect();
        std::fs::write(d.join("events.jsonl"), text).unwrap();
        d
    }

    #[test]
    fn idle_before_the_window_is_not_counted() {
        let ev = |t: u64, e: &str| Event {
            t,
            e: e.into(),
            msg: Some("go".into()),
            ..Default::default()
        };
        let edit = Event {
            tool: Some("Edit".into()),
            sig: Some("Edit:/x/a.rs".into()),
            ok: Some(true),
            dur: Some(1_000),
            ..ev(7_230_000, "PostToolUse")
        };
        let d = session_from(
            "idle",
            &[
                ev(0, "UserPromptSubmit"),
                ev(10_000, "Stop"),
                ev(7_200_000, "UserPromptSubmit"),
                edit,
            ],
        );
        let r = build(&d, Since::Prompt, 7_231_000);
        let sp = r.split.unwrap();
        assert_eq!((r.wall_ms, sp.idle, sp.tools), (30_000, 0, 1_000));
    }

    #[test]
    fn mistakes_only_inside_the_window() {
        let fail = |t: u64| Event {
            t,
            e: "PostToolUseFailure".into(),
            tool: Some("Bash".into()),
            sig: Some("Bash:make".into()),
            ok: Some(false),
            dur: Some(5_000),
            ..Default::default()
        };
        let d = session_from("mistakes", &[fail(1_000), fail(60_000)]);
        let r = build(&d, Since::Ago(30_000), 70_000);
        assert_eq!((r.failed_calls, r.failed_ms), (1, 5_000));
    }

    #[test]
    fn since_last_prompt() {
        let d = std::env::temp_dir().join(format!("ari-recap-{}-prompt", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ev = |t: u64, e: &str| Event {
            t,
            e: e.into(),
            ..Default::default()
        };
        let prompt = |t: u64| Event {
            msg: Some("go".into()),
            ..ev(t, "UserPromptSubmit")
        };
        let done = |t: u64, id: &str| Event {
            task: Some(crate::event::TaskRef {
                id: id.into(),
                subject: Some(format!("task {id}")),
                status: Some("completed".into()),
            }),
            ..ev(t, "TaskCompleted")
        };
        let events = [
            prompt(1_000),
            done(2_000, "1"),
            ev(3_000, "Stop"),
            prompt(4_000),
            done(5_000, "2"),
            Event {
                msg: Some("<agent-message from=\"w\">hi".into()),
                ..ev(5_200, "UserPromptSubmit")
            },
        ];
        let text: String = events.iter().map(to_line).collect();
        std::fs::write(d.join("events.jsonl"), text).unwrap();
        assert_eq!(build(&d, Since::Prompt, 6_000).done, vec!["task 2"]);
        assert_eq!(
            build(&d, Since::Start, 6_000).done,
            vec!["task 1", "task 2"]
        );
        let quiet = crate::panel::to_plain(&crate::panel::recap_lines(
            &build(&d, Since::Ago(500), 6_000),
            50,
            false,
            false,
        ));
        assert!(quiet.contains("│ nothing happened since "), "{quiet}");
    }

    #[test]
    fn parse_since() {
        assert_eq!(Since::parse("start"), Some(Since::Start));
        assert_eq!(Since::parse("prompt"), Some(Since::Prompt));
        assert_eq!(Since::parse("30m"), Some(Since::Ago(30 * 60_000)));
        assert_eq!(Since::parse("2h"), Some(Since::Ago(2 * 3_600_000)));
        assert_eq!(Since::parse("45s"), Some(Since::Ago(45_000)));
        assert_eq!(Since::parse("soon"), None);
    }

    #[test]
    fn commits_from_git() {
        let d = std::env::temp_dir().join(format!("ari-recap-{}-git", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&d)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !git(&["init", "-q"]) {
            return; // no git on this machine
        }
        std::fs::write(d.join("a"), "a").unwrap();
        git(&["add", "a"]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "first change",
        ]);
        let c = commits(d.to_str().unwrap(), crate::hook::now_ms() - 60_000);
        assert_eq!(c.len(), 1);
        assert!(c[0].ends_with(" first change"), "{c:?}");
        assert!(commits("/nonexistent-ari-dir", 0).is_empty());
    }
}

//! `ari find <text>`: search every Claude Code transcript, newest first,
//! and print how to resume each match.

use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub const FIND_LIMIT: usize = 10;
pub const MAX_LIMIT: usize = 500;
const SNIPPET: usize = 100;
const SNIPPET_BEFORE: usize = 30;
const PROMPT_CHARS: usize = 80;

#[derive(Debug, PartialEq)]
pub struct Hit {
    pub id: String,
    pub cwd: String,
    pub mtime_ms: u64,
    pub title: Option<String>,
    pub prompt: Option<String>,
    pub snippet: String,
}

#[derive(Debug, PartialEq)]
pub struct Opts {
    pub query: String,
    pub here: bool,
    pub limit: usize,
}

/// `Err` is the message after `ari find: `.
pub fn parse(args: &[String]) -> Result<Opts, String> {
    let (mut words, mut here, mut limit) = (Vec::new(), false, FIND_LIMIT);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--here" => here = true,
            "--limit" => {
                limit = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| (1..=MAX_LIMIT).contains(n))
                    .ok_or(format!("--limit takes a number from 1 to {MAX_LIMIT}"))?;
            }
            f if f.starts_with("--") => {
                return Err(format!("unknown option {f} (try --here, --limit <n>)"));
            }
            w => words.push(w),
        }
    }
    // One space between words, so the snippet's collapsed text can match.
    let query = words
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if query.is_empty() {
        return Err("what to look for? (ari find <text> [--here] [--limit <n>])".into());
    }
    Ok(Opts { query, here, limit })
}

/// About `SNIPPET` chars of `text` around the first `needle` (already
/// ASCII-lowercased), whitespace collapsed, `…` where it was cut.
pub fn snippet(text: &str, needle: &str) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // ASCII lowercasing keeps byte offsets, so `at` indexes `text` too.
    let at = text.to_ascii_lowercase().find(needle)?;
    let start = text[..at]
        .char_indices()
        .rev()
        .nth(SNIPPET_BEFORE - 1)
        .map_or(0, |(i, _)| i);
    let mut rest = text[start..].chars();
    let s: String = rest.by_ref().take(SNIPPET).collect();
    let (pre, post) = (start > 0, rest.next().is_some());
    Some(format!(
        "{}{s}{}",
        if pre { "…" } else { "" },
        if post { "…" } else { "" }
    ))
}

/// `cd <cwd> && claude --resume <id>`, the cwd shell-quoted when it needs it.
pub fn resume_cmd(cwd: &str, id: &str) -> String {
    if cwd.is_empty() {
        return format!("claude --resume {id}");
    }
    let plain = cwd
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._~+-".contains(c));
    if plain {
        format!("cd {cwd} && claude --resume {id}")
    } else {
        format!(
            "cd '{}' && claude --resume {id}",
            cwd.replace('\'', "'\\''")
        )
    }
}

/// The text of one transcript line that you or Claude wrote: prompts,
/// replies and titles. Tool output and JSON keys never count.
fn texts(v: &Value) -> Vec<&str> {
    let s = |k: &str| v.get(k).and_then(Value::as_str);
    let content = v.pointer("/message/content");
    let text_items = || {
        content
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|i| i.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|i| i.get("text").and_then(Value::as_str))
    };
    match s("type") {
        Some("user") => match content.and_then(Value::as_str) {
            Some(t) => vec![t],
            None => text_items().collect(),
        },
        Some("assistant") => text_items().collect(),
        Some("custom-title") => s("customTitle").into_iter().collect(),
        Some("ai-title") => s("aiTitle").into_iter().collect(),
        Some("summary") => s("summary").into_iter().collect(),
        _ => vec![],
    }
}

fn has(hay: &[u8], key: &[u8]) -> bool {
    hay.windows(key.len()).any(|w| w == key)
}

/// One transcript, streamed line by line. A line is parsed only when it can
/// hold the match, the first prompt, the title or the cwd; reading stops once
/// the match and the first prompt are known.
fn scan(path: &Path, mtime_ms: u64, needle: &str, here: Option<&str>) -> Option<Hit> {
    let id = path.file_stem()?.to_str()?;
    if !crate::paths::valid_session_id(id) {
        return None; // the id goes into a command the user pastes
    }
    // Matched against raw JSON, so `"` and `\` must be in their escaped form.
    let raw = serde_json::to_string(needle).ok()?;
    let raw = &raw[1..raw.len() - 1];
    let mut r = BufReader::with_capacity(1 << 16, File::open(path).ok()?);
    let (mut line, mut lower) = (Vec::new(), Vec::new());
    let (mut cwd, mut title, mut custom) = (None::<String>, None::<String>, false);
    let (mut prompt, mut snip) = (None::<String>, None::<String>);
    loop {
        line.clear();
        match r.read_until(b'\n', &mut line) {
            // A last line without '\n' is still being written: skip it.
            Ok(_) if line.ends_with(b"\n") => {}
            _ => break,
        }
        let hit = snip.is_none() && {
            lower.clear();
            lower.extend(line.iter().map(u8::to_ascii_lowercase));
            std::str::from_utf8(&lower).is_ok_and(|l| l.contains(raw))
        };
        let want = hit
            || (cwd.is_none() && has(&line, b"\"cwd\":\""))
            || (prompt.is_none() && has(&line, b"\"type\":\"user\""))
            || (!custom && has(&line, b"\"type\":\"custom-title\""))
            || (title.is_none()
                && (has(&line, b"\"type\":\"ai-title\"") || has(&line, b"\"type\":\"summary\"")));
        if !want {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if cwd.is_none()
            && let Some(c) = v.get("cwd").and_then(Value::as_str)
        {
            if here.is_some_and(|h| h != c) {
                return None;
            }
            cwd = Some(c.to_string());
        }
        let ty = v.get("type").and_then(Value::as_str);
        let first = texts(&v).first().map(|t| t.to_string());
        match ty {
            Some("custom-title") if first.is_some() => (title, custom) = (first.clone(), true),
            Some("ai-title" | "summary") if title.is_none() => title = first.clone(),
            _ => {}
        }
        let flag = |k: &str| v.get(k).and_then(Value::as_bool) == Some(true);
        if prompt.is_none() && ty == Some("user") && !flag("isMeta") && !flag("isSidechain") {
            prompt = first
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|t| !t.is_empty() && !t.starts_with('<'))
                .map(|t| crate::event::truncate(&t, PROMPT_CHARS));
        }
        if hit {
            snip = texts(&v).into_iter().find_map(|t| snippet(t, needle));
        }
        if snip.is_some() && prompt.is_some() {
            break;
        }
    }
    if here.is_some_and(|h| cwd.as_deref() != Some(h)) {
        return None;
    }
    Some(Hit {
        id: id.to_string(),
        cwd: cwd.unwrap_or_default(),
        mtime_ms,
        title,
        prompt,
        snippet: snip?,
    })
}

/// Main transcripts (`projects/*/*.jsonl`, not `<id>/subagents/`), newest
/// first, scanned in that order until `limit` match.
pub fn search(projects: &Path, query: &str, here: Option<&str>, limit: usize) -> Vec<Hit> {
    let ms = |m: &std::fs::Metadata| {
        m.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64)
    };
    let mut files: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(projects)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((ms(&e.metadata().ok().filter(|m| m.is_file())?), e.path())))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let needle = query.to_ascii_lowercase();
    files
        .iter()
        .filter_map(|(t, p)| scan(p, *t, &needle, here))
        .take(limit)
        .collect()
}

pub fn run(args: &[String]) -> ! {
    let fail = |code: i32, msg: &str| -> ! {
        eprintln!("{msg}");
        std::process::exit(code)
    };
    let o = parse(args).unwrap_or_else(|e| fail(2, &format!("ari find: {e}")));
    let projects = crate::setup::settings_path()
        .and_then(|p| Some(p.parent()?.join("projects")))
        .unwrap_or_else(|| fail(2, "ari find: HOME is not set"));
    if !projects.is_dir() {
        let p = crate::panel::tilde(&projects.to_string_lossy());
        fail(1, &format!("no Claude Code transcripts in {p}"));
    }
    let here = o.here.then(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let hits = search(&projects, &o.query, here.as_deref(), o.limit);
    if hits.is_empty() {
        fail(1, &format!("no session mentions \"{}\"", o.query));
    }
    let out: Vec<String> = hits.iter().map(format).collect();
    println!("{}", out.join("\n\n"));
    if hits.len() == o.limit {
        eprintln!("newest {} shown; --limit for more", o.limit);
    }
    std::process::exit(0)
}

fn format(h: &Hit) -> String {
    let title = h.title.as_ref().map(|t| format!("  · {t}"));
    format!(
        "{}  {}{}\n  you    {}\n  match  {}\n  {}",
        crate::panel::local_date(h.mtime_ms),
        crate::panel::tilde(&h.cwd),
        title.unwrap_or_default(),
        h.prompt.as_deref().unwrap_or("(no prompt)"),
        h.snippet,
        resume_cmd(&h.cwd, &h.id)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ari-find-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn line(ty: &str, cwd: &str, content: Value, meta: bool) -> Value {
        json!({"parentUuid":null,"isSidechain":false,"isMeta":meta,"type":ty,
            "message":{"role":ty,"content":content},"cwd":cwd,"sessionId":"s",
            "timestamp":"2026-11-07T10:00:00Z"})
    }
    fn user(cwd: &str, text: &str) -> Value {
        line("user", cwd, json!(text), false)
    }
    fn meta_user(cwd: &str, text: &str) -> Value {
        line("user", cwd, json!(text), true)
    }
    fn asst(text: &str) -> Value {
        line(
            "assistant",
            "/p",
            json!([{"type":"text","text":text}]),
            false,
        )
    }
    fn tool_result(text: &str) -> Value {
        line(
            "user",
            "/p",
            json!([{"type":"tool_result","content":text}]),
            false,
        )
    }
    fn title(t: &str) -> Value {
        json!({"type":"custom-title","customTitle":t,"sessionId":"s"})
    }

    /// Writes `root/dir/id.jsonl` with mtime `mtime_s`.
    fn project(root: &Path, dir: &str, id: &str, lines: &[Value], mtime_s: u64) -> PathBuf {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("{id}.jsonl"));
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&p, body).unwrap();
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime_s);
        File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(t)
            .unwrap();
        p
    }

    fn scan_one(name: &str, lines: &[Value], query: &str) -> Option<Hit> {
        let p = project(&tmp(name), "d", "abc", lines, 1);
        scan(&p, 0, &query.to_ascii_lowercase(), None)
    }

    #[test]
    fn skips_meta_and_command_prompts() {
        let h = scan_one(
            "meta",
            &[
                meta_user("/p", "<local-command-caveat>caching</local-command-caveat>"),
                user("/p", "<command-name>/clear</command-name>"),
                user("/p", "real question about   caching"),
            ],
            "caching",
        )
        .unwrap();
        assert_eq!(h.prompt.as_deref(), Some("real question about caching"));
        assert_eq!(h.cwd, "/p");
    }

    #[test]
    fn quotes_backslashes_and_case() {
        let l = [user("/p", r#"say "hi" from C:\temp"#)];
        for q in [r#"say "hi""#, r"C:\temp", "SAY"] {
            assert!(scan_one("quotes", &l, q).is_some(), "{q}");
        }
    }

    #[test]
    fn only_prompts_replies_and_titles() {
        assert_eq!(
            scan_one("tool", &[user("/p", "hi"), tool_result("zebra")], "zebra"),
            None
        );
        assert_eq!(scan_one("keys", &[user("/p", "hi")], "parentuuid"), None);
        let h = scan_one("title", &[user("/p", "hi"), title("zebra notes")], "zebra").unwrap();
        assert_eq!(h.title.as_deref(), Some("zebra notes"));
        let h = scan_one("reply", &[user("/p", "hi"), asst("a zebra")], "zebra").unwrap();
        assert_eq!(
            (h.prompt.as_deref(), h.snippet.as_str()),
            (Some("hi"), "a zebra")
        );
    }

    #[test]
    fn partial_last_line_is_skipped() {
        let d = tmp("partial");
        let p = d.join("abc.jsonl");
        let cut = r#"{"type":"user","message":{"content":"zeb"#;
        std::fs::write(&p, format!("{}\n{cut}", user("/p", "zebra"))).unwrap();
        assert!(scan(&p, 0, "zeb", None).is_some());
        std::fs::write(&p, cut).unwrap();
        assert_eq!(scan(&p, 0, "zeb", None), None);
    }

    #[test]
    fn parses_flags_and_joins_words() {
        let a = |s: &[&str]| parse(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(
            a(&["token", "refresh"]),
            Ok(Opts {
                query: "token refresh".into(),
                here: false,
                limit: FIND_LIMIT
            })
        );
        assert_eq!(
            a(&["--here", "--limit", "3", "x"]),
            Ok(Opts {
                query: "x".into(),
                here: true,
                limit: 3
            })
        );
        assert!(a(&[]).is_err());
        assert!(a(&["--here"]).is_err());
        assert!(a(&["  "]).is_err());
        assert_eq!(a(&["a  b", "c"]).unwrap().query, "a b c");
        assert!(a(&["x", "--limit", "0"]).is_err());
        assert!(a(&["x", "--limit", "501"]).is_err());
        assert!(a(&["x", "--limit"]).is_err());
        assert!(a(&["x", "--nope"]).is_err());
    }

    #[test]
    fn snippet_cuts_around_the_match() {
        let text = format!(
            "{} the Token refresh failed {}",
            "a ".repeat(100),
            "b ".repeat(100)
        );
        let s = snippet(&text, "token refresh").unwrap();
        assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
        assert!(s.contains("Token refresh"), "{s}");
        assert!(s.chars().count() <= SNIPPET + 2, "{s}");
        assert_eq!(
            snippet("short\n\n  text here", "text").unwrap(),
            "short text here"
        );
        assert_eq!(snippet("🦀🦀 crab 🦀", "crab").unwrap(), "🦀🦀 crab 🦀");
        assert_eq!(snippet("nothing", "zzz"), None);
    }

    #[test]
    fn resume_command_is_quoted() {
        assert_eq!(
            resume_cmd("/p/proj", "abc"),
            "cd /p/proj && claude --resume abc"
        );
        assert_eq!(
            resume_cmd("/p/My Project", "abc"),
            "cd '/p/My Project' && claude --resume abc"
        );
        assert_eq!(
            resume_cmd("/tmp/it's", "abc"),
            "cd '/tmp/it'\\''s' && claude --resume abc"
        );
        assert_eq!(resume_cmd("", "abc"), "claude --resume abc");
    }

    #[test]
    fn texts_are_prompts_replies_and_titles() {
        let t = |v: Value| texts(&v).into_iter().map(String::from).collect::<Vec<_>>();
        assert_eq!(
            t(serde_json::json!({"type":"user","message":{"content":"hi"}})),
            ["hi"]
        );
        assert_eq!(
            t(serde_json::json!({"type":"user","message":{"content":[
                {"type":"text","text":"a"},
                {"type":"tool_result","content":"secret"}]}})),
            ["a"]
        );
        assert_eq!(
            t(serde_json::json!({"type":"assistant","message":{"content":[
                {"type":"thinking","thinking":"x"},{"type":"text","text":"b"},
                {"type":"tool_use","input":{"cmd":"y"}}]}})),
            ["b"]
        );
        assert_eq!(
            t(serde_json::json!({"type":"custom-title","customTitle":"c"})),
            ["c"]
        );
        assert_eq!(
            t(serde_json::json!({"type":"ai-title","aiTitle":"d"})),
            ["d"]
        );
        assert!(t(serde_json::json!({"type":"attachment","content":"e"})).is_empty());
    }

    fn ids(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.id.as_str()).collect()
    }

    #[test]
    fn finds_newest_first() {
        let r = tmp("newest");
        project(
            &r,
            "-p-a",
            "a",
            &[user("/p/a", "fix the token refresh")],
            100,
        );
        let b = [user("/p/b", "hello"), asst("the token refresh works now")];
        project(&r, "-p-b", "b", &b, 200);
        project(&r, "-p-c", "c", &[user("/p/c", "unrelated")], 300);
        let hits = search(&r, "token refresh", None, 10);
        assert_eq!(ids(&hits), ["b", "a"]);
        assert_eq!(hits[0].prompt.as_deref(), Some("hello"));
        assert!(hits[0].snippet.contains("token refresh"));
        assert_eq!((hits[0].cwd.as_str(), hits[0].mtime_ms), ("/p/b", 200_000));
    }

    #[test]
    fn here_filters_by_cwd() {
        let r = tmp("here");
        project(&r, "-p-a", "a", &[user("/p/a", "zebra")], 1);
        project(&r, "-p-b", "b", &[user("/p/b", "zebra")], 2);
        assert_eq!(ids(&search(&r, "zebra", Some("/p/a"), 10)), ["a"]);
    }

    #[test]
    fn limit_caps_results() {
        let r = tmp("limit");
        for (id, t) in [("x1", 1), ("x2", 2), ("x3", 3)] {
            project(&r, "d", id, &[user("/p", "zebra")], t);
        }
        assert_eq!(ids(&search(&r, "zebra", None, 2)), ["x3", "x2"]);
    }

    #[test]
    fn subagent_transcripts_skipped() {
        let r = tmp("subagents");
        project(&r, "d/abc/subagents", "agent-x", &[user("/p", "zebra")], 1);
        project(&r, "d", "abc", &[user("/p", "hi")], 1);
        assert!(search(&r, "zebra", None, 10).is_empty());
    }

    #[test]
    fn missing_projects_dir() {
        assert!(search(Path::new("/nonexistent-ari"), "x", None, 10).is_empty());
    }

    #[test]
    fn format_matches_the_spec() {
        let mut h = Hit {
            id: "abc".into(),
            cwd: "/p/My Project".into(),
            mtime_ms: 1_793_901_768_682,
            title: Some("auth".into()),
            prompt: Some("fix it".into()),
            snippet: "…the token…".into(),
        };
        let out = format(&h);
        let l: Vec<&str> = out.lines().collect();
        assert_eq!(l.len(), 4);
        assert!(
            l[0].starts_with(&crate::panel::local_date(h.mtime_ms)),
            "{}",
            l[0]
        );
        assert!(l[0].ends_with("/p/My Project  · auth"), "{}", l[0]);
        assert_eq!(l[1], "  you    fix it");
        assert_eq!(l[2], "  match  …the token…");
        assert_eq!(l[3], "  cd '/p/My Project' && claude --resume abc");
        h.prompt = None;
        assert_eq!(format(&h).lines().nth(1), Some("  you    (no prompt)"));
    }
}

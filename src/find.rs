//! `ari find <text>`: search every Claude Code transcript, newest first,
//! and print how to resume each match.

use serde_json::Value;

pub const FIND_LIMIT: usize = 10;
pub const MAX_LIMIT: usize = 500;
const SNIPPET: usize = 100;
const SNIPPET_BEFORE: usize = 30;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}

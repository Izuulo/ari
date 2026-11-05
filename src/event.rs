//! One hook payload → one compact event line. Pure; no I/O.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_LINE: usize = 4096;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Event {
    pub t: u64,
    pub e: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<Vec<Todo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg: Option<u32>,
    /// Running background tasks as `type: description`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bgs: Option<Vec<String>>,
    /// Session crons as `schedule · prompt`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crons: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dur: Option<u64>,
    /// Resume only: the prompt cache likely expired while away.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cold: Option<bool>,
    /// Resume only: estimated cache rebuild cost in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,
    /// (passed, failed), when this was a test run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tests: Option<(u32, u32)>,
    /// The test run was filtered (`N filtered out` / `N deselected`), not the full suite.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
    /// Permission mode (`plan`, `auto`, ...) the hook payload carried.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pm: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TaskRef {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Todo {
    pub c: String,
    pub s: String,
}

pub fn normalize(raw: &Value, t: u64) -> Option<Event> {
    let s = |k: &str| raw.get(k).and_then(Value::as_str);
    let owned = |k: &str| s(k).filter(|v| !v.is_empty()).map(String::from);
    let mut ev = Event {
        t,
        e: s("hook_event_name")?.to_string(),
        a: owned("agent_id"),
        at: owned("agent_type"),
        pm: owned("permission_mode"),
        dur: raw.get("duration_ms").and_then(Value::as_u64),
        ..Default::default()
    };
    let null = Value::Null;
    let input = raw.get("tool_input").unwrap_or(&null);
    if let Some(tool) = s("tool_name") {
        let cwd = s("cwd").unwrap_or("");
        ev.tool = Some(tool.into());
        ev.id = owned("tool_use_id");
        ev.act = Some(truncate(&action(tool, input, cwd), 80));
        ev.sig = Some(truncate(&signature(tool, input), 200));
        ev.h = Some(fnv1a(&canonical(input)));
    }
    let line = |k: &str| s(k).map(|v| truncate(first_line(v), 160));
    match ev.e.as_str() {
        "PostToolUse" => {
            ev.ok = Some(true);
            let resp = raw.get("tool_response").unwrap_or(&null);
            task_tool(&mut ev, input, resp);
            let out = |k: &str| resp.get(k).and_then(Value::as_str).unwrap_or("");
            (ev.tests, ev.partial) = split_run(test_run(
                input,
                &format!("{}\n{}", out("stdout"), out("stderr")),
            ));
        }
        "PostToolUseFailure" => {
            ev.ok = Some(false);
            let err = s("error").unwrap_or("");
            ev.kind = if raw.get("is_interrupt") == Some(&Value::Bool(true)) {
                Some("interrupt".into())
            } else {
                err.lines()
                    .next()
                    .and_then(|l| l.strip_prefix("Exit code "))
                    .map(|n| format!("exit {}", n.trim()))
            };
            ev.msg = Some(truncate(failure_line(err), 160));
            (ev.tests, ev.partial) = split_run(test_run(input, err));
        }
        "UserPromptSubmit" => {
            ev.msg = s("prompt")
                .map(|p| truncate(&p.split_whitespace().collect::<Vec<_>>().join(" "), 200));
        }
        "SessionStart" => {
            ev.kind = owned("source");
            ev.path = owned("transcript_path");
            ev.dur = raw
                .get("seconds_since_last_response")
                .and_then(Value::as_f64)
                .filter(|v| *v >= 0.0)
                .map(|v| (v * 1000.0) as u64);
            ev.cold = raw
                .get("prompt_cache_likely_expired")
                .and_then(Value::as_bool);
            ev.usd = raw.get("estimated_cache_write_usd").and_then(Value::as_f64);
        }
        "SessionEnd" => ev.kind = owned("reason"),
        "PreCompact" | "PostCompact" => ev.kind = owned("trigger"),
        "Notification" => {
            ev.kind = owned("notification_type");
            ev.msg = line("message");
        }
        "Stop" => {
            let running = raw
                .get("background_tasks")
                .and_then(Value::as_array)
                .map_or(0, |a| {
                    a.iter()
                        .filter(|t| t.get("status").and_then(Value::as_str) == Some("running"))
                        .count()
                });
            ev.bg = Some(running as u32);
            ev.msg = line("last_assistant_message");
            lists(raw, &mut ev);
        }
        "StopFailure" => {
            ev.kind = owned("error");
            ev.msg = line("last_assistant_message").or_else(|| line("error_details"));
        }
        "SubagentStop" => {
            ev.path = owned("agent_transcript_path");
            ev.msg = line("last_assistant_message");
            lists(raw, &mut ev);
        }
        "TaskCreated" | "TaskCompleted" => {
            ev.task = owned("task_id").map(|id| TaskRef {
                id,
                subject: s("task_subject").map(|v| truncate(v, 120)),
                status: (ev.e == "TaskCompleted").then(|| "completed".into()),
            });
        }
        _ => {}
    }
    Some(ev)
}

/// Running background tasks and session crons by name, when the payload carries them.
fn lists(raw: &Value, ev: &mut Event) {
    fn str_of<'a>(t: &'a Value, k: &str) -> Option<&'a str> {
        t.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
    }
    let one_line = |s: String| truncate(&s.split_whitespace().collect::<Vec<_>>().join(" "), 60);
    ev.bgs = raw
        .get("background_tasks")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|t| str_of(t, "status") == Some("running"))
                .filter_map(|t| {
                    let name = str_of(t, "description").or_else(|| str_of(t, "command"))?;
                    Some(one_line(format!(
                        "{}: {name}",
                        str_of(t, "type").unwrap_or("task")
                    )))
                })
                .take(5)
                .collect()
        });
    // shortcut: cron entry shape is unverified; reads schedule/prompt, skips entries with neither.
    ev.crons = raw.get("session_crons").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(|t| match (str_of(t, "schedule"), str_of(t, "prompt")) {
                (Some(s), Some(p)) => Some(one_line(format!("{s} · {p}"))),
                (Some(x), None) | (None, Some(x)) => Some(one_line(x.into())),
                _ => None,
            })
            .take(5)
            .collect()
    });
}

/// (passed, failed) in a test command's output: cargo, pytest, jest and vitest all print
/// `N passed` and `N failed`.
// shortcut: sums every count (cargo prints one line per test binary) and skips jest/vitest
// suite and file totals; teach it other runners' summaries when they show up.
fn test_run(input: &Value, out: &str) -> Option<(u32, u32, bool)> {
    let cmd = input.get("command").and_then(Value::as_str)?;
    // A runner or a `test` subcommand, not any command mentioning tests (`grep … tests/`).
    let runs_tests = cmd.split_whitespace().any(|w| {
        let w = w.rsplit('/').next().unwrap_or(w);
        w == "test"
            || w.starts_with("test:")
            || ["pytest", "jest", "vitest", "nextest", "mocha"].contains(&w)
    });
    if !runs_tests {
        return None;
    }
    let mut sum = None;
    let mut partial = false;
    for line in out
        .lines()
        .filter(|l| !l.contains("Suites") && !l.contains("Files"))
    {
        let w: Vec<&str> = line.split_whitespace().collect();
        for p in w.windows(2) {
            let Ok(n) = p[0].parse::<u32>() else { continue };
            match p[1].trim_end_matches([';', ',']) {
                "passed" => sum.get_or_insert((0, 0)).0 += n,
                "failed" => sum.get_or_insert((0, 0)).1 += n,
                "filtered" | "deselected" => partial |= n > 0,
                _ => {}
            }
        }
    }
    sum.map(|(p, f)| (p, f, partial))
}

fn split_run(r: Option<(u32, u32, bool)>) -> (Option<(u32, u32)>, bool) {
    (r.map(|(p, f, _)| (p, f)), r.is_some_and(|r| r.2))
}

fn task_tool(ev: &mut Event, input: &Value, resp: &Value) {
    let str_at = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    match ev.tool.as_deref() {
        Some("TaskCreate") => {
            ev.task = resp
                .pointer("/task/id")
                .and_then(Value::as_str)
                .map(|id| TaskRef {
                    id: id.into(),
                    subject: str_at(input, "subject").map(|v| truncate(&v, 120)),
                    status: Some("pending".into()),
                });
        }
        Some("TaskUpdate") => {
            ev.task = str_at(input, "taskId").map(|id| TaskRef {
                id,
                subject: str_at(input, "subject").map(|v| truncate(&v, 120)),
                status: str_at(input, "status"),
            });
        }
        Some("TodoWrite") => {
            ev.todos = input.get("todos").and_then(Value::as_array).map(|a| {
                a.iter()
                    .take(40)
                    .map(|t| Todo {
                        c: truncate(&str_at(t, "content").unwrap_or_default(), 80),
                        s: str_at(t, "status").unwrap_or_default(),
                    })
                    .collect()
            });
        }
        _ => {}
    }
}

fn action(tool: &str, input: &Value, cwd: &str) -> String {
    let f = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    match tool {
        "Edit" | "Write" | "MultiEdit" => format!("editing {}", rel(f("file_path"), cwd)),
        "NotebookEdit" => format!("editing {}", rel(f("notebook_path"), cwd)),
        "Read" => format!("reading {}", rel(f("file_path"), cwd)),
        "Grep" => format!("grep {}", f("pattern")),
        "Glob" => format!("glob {}", f("pattern")),
        "Bash" | "PowerShell" => format!("$ {}", first_line(f("command"))),
        "Agent" => {
            let ty = Some(f("subagent_type")).filter(|v| !v.is_empty());
            format!(
                "agent {}: {}",
                ty.unwrap_or("general-purpose"),
                f("description")
            )
        }
        "WebFetch" => format!("fetch {}", f("url")),
        "WebSearch" => format!("search {}", f("query")),
        "TaskCreate" | "TaskUpdate" | "TodoWrite" => "updating plan".into(),
        _ => match tool.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
            Some((srv, t)) => format!("{srv}: {t}"),
            None => tool.into(),
        },
    }
}

/// Error signature: Bash → first two command tokens; file tools → path; else the tool.
fn signature(tool: &str, input: &Value) -> String {
    let f = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    match tool {
        "Bash" | "PowerShell" => {
            let toks: Vec<&str> = first_line(f("command"))
                .split_whitespace()
                .take(2)
                .collect();
            format!("{tool}:{}", toks.join(" "))
        }
        "Edit" | "Write" | "MultiEdit" | "Read" => format!("{tool}:{}", f("file_path")),
        "NotebookEdit" => format!("{tool}:{}", f("notebook_path")),
        _ => tool.into(),
    }
}

fn rel<'a>(path: &'a str, cwd: &str) -> &'a str {
    if cwd.is_empty() {
        return path;
    }
    path.strip_prefix(cwd)
        .and_then(|p| p.strip_prefix('/'))
        .filter(|p| !p.is_empty())
        .unwrap_or(path)
}

fn first_line(s: &str) -> &str {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// The first line that names the problem, skipping the `Exit code N` header.
fn failure_line(err: &str) -> &str {
    let lines: Vec<&str> = err
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Exit code "))
        .collect();
    lines
        .iter()
        .find(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("error") || l.contains("failed") || l.contains("panicked")
        })
        .or(lines.first())
        .copied()
        .unwrap_or("")
}

/// Prompts the harness injects (task results, agent-team messages), not typed by you.
pub fn is_synthetic(msg: &str) -> bool {
    msg.starts_with("<task-notification>") || msg.starts_with("<agent-message")
}

pub fn truncate(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars.saturating_sub(1)) {
        Some((i, _)) if s[i..].chars().nth(1).is_some() => format!("{}…", &s[..i]),
        _ => s.to_string(),
    }
}

/// JSON bytes with object keys sorted recursively, so the hash ignores key order.
fn canonical(v: &Value) -> Vec<u8> {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<_> = m.keys().collect();
                keys.sort();
                Value::Object(keys.into_iter().map(|k| (k.clone(), sort(&m[k]))).collect())
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_vec(&sort(v)).unwrap_or_default()
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Serialized event plus newline, always under `MAX_LINE` bytes so one `write` stays atomic.
pub fn to_line(ev: &Event) -> String {
    let mut ev = ev.clone();
    for step in 0..2 {
        let s = serde_json::to_string(&ev).unwrap_or_default();
        if s.len() < MAX_LINE {
            return s + "\n";
        }
        match step {
            0 => ev.todos = None,
            _ => ev.msg = None,
        }
    }
    let s = serde_json::to_string(&ev).unwrap_or_default();
    if s.len() < MAX_LINE {
        return s + "\n";
    }
    let min = Event {
        t: ev.t,
        e: truncate(&ev.e, 64),
        ..Default::default()
    };
    serde_json::to_string(&min).unwrap_or_default() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(v: Value) -> Event {
        normalize(&v, 1).unwrap()
    }

    #[test]
    fn resume_fields_on_session_start() {
        let ev = n(
            json!({"hook_event_name": "SessionStart", "source": "resume",
            "seconds_since_last_response": 10800, "prompt_cache_likely_expired": true,
            "estimated_cache_write_usd": 0.4}),
        );
        assert_eq!(
            (ev.dur, ev.cold, ev.usd),
            (Some(10_800_000), Some(true), Some(0.4))
        );
        let ev = n(json!({"hook_event_name": "SessionStart", "source": "resume"}));
        assert_eq!((ev.dur, ev.cold, ev.usd), (None, None, None));
        let j = serde_json::to_string(&ev).unwrap();
        assert!(!j.contains("cold") && !j.contains("usd"));
    }

    #[test]
    fn permission_mode_rides_along() {
        let ev = n(json!({"hook_event_name": "Stop", "permission_mode": "plan"}));
        assert_eq!(ev.pm.as_deref(), Some("plan"));
        assert!(
            !serde_json::to_string(&n(json!({"hook_event_name": "Stop"})))
                .unwrap()
                .contains("pm")
        );
        let old: Event = serde_json::from_str(r#"{"t":1,"e":"Stop"}"#).unwrap();
        assert_eq!(old.pm, None);
    }

    #[test]
    fn test_runs_flag_partial() {
        let cmd = |c: &str| json!({ "command": c });
        let t = |c: &str, o: &str| test_run(&cmd(c), o);
        let part = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 104 filtered out";
        assert_eq!(t("cargo test foo", part), Some((1, 0, true)));
        assert_eq!(
            t("cargo test", &part.replace("104", "0")),
            Some((1, 0, false))
        );
        assert_eq!(
            t("pytest -k x", "== 2 passed, 3 deselected in 0.1s =="),
            Some((2, 0, true))
        );
    }

    #[test]
    fn test_runs_count_passes() {
        let cmd = |c: &str| json!({ "command": c });
        let cargo = "test result: ok. 67 passed; 0 failed\ntest result: ok. 3 passed; 0 failed";
        assert_eq!(test_run(&cmd("cargo test"), cargo), Some((70, 0, false)));
        let cargo_red = "test result: FAILED. 67 passed; 2 failed; 0 ignored";
        assert_eq!(
            test_run(&cmd("cargo test"), cargo_red),
            Some((67, 2, false))
        );
        let jest =
            "Test Suites: 1 failed, 2 passed, 3 total\nTests:       1 failed, 12 passed, 13 total";
        assert_eq!(test_run(&cmd("npx jest"), jest), Some((12, 1, false)));
        assert_eq!(
            test_run(&cmd("pytest -q"), "== 1 failed, 5 passed in 0.3s =="),
            Some((5, 1, false))
        );
        assert_eq!(
            test_run(&cmd("cat notes"), "5 passed"),
            None,
            "not a test command"
        );
        assert_eq!(
            test_run(&cmd("grep -rn passed tests/"), cargo_red),
            None,
            "searching test files is not a test run"
        );
        let one = "== 3 passed in 0.1s ==";
        for c in [
            "npm run test:unit",
            "npx jest",
            "go test ./...",
            "cargo nextest run",
            "uv run pytest",
        ] {
            assert_eq!(test_run(&cmd(c), one), Some((3, 0, false)), "{c}");
        }
        assert_eq!(
            test_run(&cmd("cargo test"), "error[E0425]"),
            None,
            "no summary"
        );
        let ev = n(
            json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
            "tool_input": {"command": "cargo test"}, "tool_response": {"stdout": cargo, "stderr": ""}}),
        );
        assert_eq!(ev.tests, Some((70, 0)));
        assert!(!ev.partial);
    }

    #[test]
    fn bash_pre_and_failure() {
        let pre = n(
            json!({"hook_event_name":"PreToolUse","cwd":"/p","tool_name":"Bash",
            "tool_input":{"command":"python3 hello.py","description":"Run"},"tool_use_id":"toolu_1"}),
        );
        assert_eq!(pre.act.as_deref(), Some("$ python3 hello.py"));
        assert_eq!(pre.sig.as_deref(), Some("Bash:python3 hello.py"));
        assert_eq!(pre.id.as_deref(), Some("toolu_1"));
        assert!(pre.h.is_some() && pre.ok.is_none() && pre.a.is_none());

        let fail = n(
            json!({"hook_event_name":"PostToolUseFailure","tool_name":"Bash",
            "tool_input":{"command":"python3 hello.py"},
            "error":"Exit code 1\n  File \"/p/hello.py\", line 2\n    \n    ^\nSyntaxError: unexpected EOF while parsing",
            "is_interrupt":false,"duration_ms":892}),
        );
        assert_eq!(fail.ok, Some(false));
        assert_eq!(fail.kind.as_deref(), Some("exit 1"));
        assert_eq!(
            fail.msg.as_deref(),
            Some("SyntaxError: unexpected EOF while parsing")
        );
        assert_eq!(fail.dur, Some(892));
        assert!(fail.h.is_some());
    }

    #[test]
    fn same_input_same_hash() {
        let a = n(
            json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls","description":"x"}}),
        );
        let b = n(
            json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"description":"x","command":"ls"}}),
        );
        assert_eq!(a.h, b.h);
    }

    #[test]
    fn subagent_tool_and_relative_paths() {
        let ev = n(
            json!({"hook_event_name":"PreToolUse","agent_id":"ab5f","agent_type":"Explore",
            "cwd":"/p","tool_name":"Read","tool_input":{"file_path":"/p/src/a.rs"}}),
        );
        assert_eq!(ev.a.as_deref(), Some("ab5f"));
        assert_eq!(ev.at.as_deref(), Some("Explore"));
        assert_eq!(ev.act.as_deref(), Some("reading src/a.rs"));
        assert_eq!(ev.sig.as_deref(), Some("Read:/p/src/a.rs"));
        let mcp = n(
            json!({"hook_event_name":"PreToolUse","tool_name":"mcp__github__create_issue","tool_input":{}}),
        );
        assert_eq!(mcp.act.as_deref(), Some("github: create_issue"));
    }

    #[test]
    fn tasks() {
        let created =
            n(json!({"hook_event_name":"TaskCreated","task_id":"1","task_subject":"write script"}));
        assert_eq!(
            created.task,
            Some(TaskRef {
                id: "1".into(),
                subject: Some("write script".into()),
                status: None
            })
        );
        let done = n(
            json!({"hook_event_name":"TaskCompleted","task_id":"1","task_subject":"write script"}),
        );
        assert_eq!(done.task.unwrap().status.as_deref(), Some("completed"));
        let upd = n(
            json!({"hook_event_name":"PostToolUse","tool_name":"TaskUpdate",
            "tool_input":{"taskId":"1","status":"in_progress"},
            "tool_response":{"success":true,"taskId":"1","statusChange":{"from":"pending","to":"in_progress"}}}),
        );
        assert_eq!(
            upd.task,
            Some(TaskRef {
                id: "1".into(),
                subject: None,
                status: Some("in_progress".into())
            })
        );
        assert_eq!(upd.act.as_deref(), Some("updating plan"));
        let mk = n(
            json!({"hook_event_name":"PostToolUse","tool_name":"TaskCreate",
            "tool_input":{"subject":"run script"},"tool_response":{"task":{"id":"2","subject":"run script"}}}),
        );
        assert_eq!(mk.task.unwrap().id, "2");
        let todo = n(
            json!({"hook_event_name":"PostToolUse","tool_name":"TodoWrite",
            "tool_input":{"todos":[{"content":"a","status":"completed","activeForm":"A"},{"content":"b","status":"in_progress"}]}}),
        );
        assert_eq!(
            todo.todos.unwrap(),
            vec![
                Todo {
                    c: "a".into(),
                    s: "completed".into()
                },
                Todo {
                    c: "b".into(),
                    s: "in_progress".into()
                }
            ]
        );
    }

    #[test]
    fn background_and_crons_by_name() {
        let stop = n(json!({"hook_event_name":"Stop",
            "background_tasks":[{"type":"subagent","description":"List files and read hello.py","status":"running"},
            {"type":"shell","command":"npm run dev","status":"running"},{"type":"shell","status":"running"},
            {"type":"shell","description":"done","status":"completed"}],
            "session_crons":[{"schedule":"*/5 * * * *","prompt":"check deploy"}]}));
        assert_eq!(
            stop.bgs,
            Some(vec![
                "subagent: List files and read hello.py".into(),
                "shell: npm run dev".into()
            ])
        );
        assert_eq!(stop.crons, Some(vec!["*/5 * * * * · check deploy".into()]));
        let sub = n(json!({"hook_event_name":"SubagentStop","background_tasks":[]}));
        assert_eq!(sub.bgs, Some(vec![]));
        assert_eq!(sub.crons, None);
        assert_eq!(n(json!({"hook_event_name":"Stop"})).bgs, None);
    }

    #[test]
    fn lifecycle_events() {
        let stop = n(
            json!({"hook_event_name":"Stop","last_assistant_message":"\nStill waiting.\nmore",
            "background_tasks":[{"id":"x","type":"subagent","status":"running"},{"id":"y","status":"completed"}]}),
        );
        assert_eq!(stop.bg, Some(1));
        assert_eq!(stop.msg.as_deref(), Some("Still waiting."));
        let sub = n(
            json!({"hook_event_name":"SubagentStop","agent_id":"ab5f","agent_type":"Explore",
            "agent_transcript_path":"/t/agent-ab5f.jsonl","last_assistant_message":"Found 3 files."}),
        );
        assert_eq!(sub.path.as_deref(), Some("/t/agent-ab5f.jsonl"));
        assert_eq!(sub.msg.as_deref(), Some("Found 3 files."));
        let note = n(
            json!({"hook_event_name":"Notification","message":"Claude needs your permission","notification_type":"permission_prompt"}),
        );
        assert_eq!(note.kind.as_deref(), Some("permission_prompt"));
        let sf = n(
            json!({"hook_event_name":"StopFailure","error":"rate_limit","error_details":"429 Too Many Requests"}),
        );
        assert_eq!(
            (sf.kind.as_deref(), sf.msg.as_deref()),
            (Some("rate_limit"), Some("429 Too Many Requests"))
        );
        let p = n(json!({"hook_event_name":"UserPromptSubmit","prompt":"fix   the\nbuild"}));
        assert_eq!(p.msg.as_deref(), Some("fix the build"));
        assert!(normalize(&json!({"tool_name":"Bash"}), 1).is_none());
        assert!(normalize(&json!("str"), 1).is_none());
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate("héllo🦀wörld", 6), "héllo…");
        assert_eq!(truncate("héllo", 5), "héllo");
        assert_eq!(truncate("", 3), "");
        assert_eq!(truncate("🦀🦀🦀🦀", 2), "🦀…");
    }

    #[test]
    fn line_always_fits() {
        let ev = Event {
            t: 1,
            e: "PostToolUse".into(),
            msg: Some("x".repeat(10_000)),
            todos: Some(vec![
                Todo {
                    c: "y".repeat(80),
                    s: "pending".into()
                };
                500
            ]),
            ..Default::default()
        };
        let l = to_line(&ev);
        assert!(l.len() < MAX_LINE && l.ends_with('\n'));
        let back: Event = serde_json::from_str(&l).unwrap();
        assert_eq!(back.e, "PostToolUse");
        let huge = Event {
            t: 1,
            e: "X".into(),
            act: Some("z".repeat(9000)),
            ..Default::default()
        };
        assert!(to_line(&huge).len() < MAX_LINE);
    }
}

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ari-hook-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hook(state: &Path, stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ari"))
        .arg("hook")
        .env("XDG_STATE_HOME", state)
        .env_remove("ARI_RAW_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_silent(out: &Output) {
    assert!(out.status.success(), "{:?}", out.status);
    assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
}

fn log(state: &Path, sid: &str) -> Vec<String> {
    let p = state.join("ari/sessions").join(sid).join("events.jsonl");
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

fn pre_tool(sid: &str, cmd: &str) -> Vec<u8> {
    serde_json::json!({"session_id": sid, "hook_event_name": "PreToolUse", "cwd": "/p",
        "tool_name": "Bash", "tool_input": {"command": cmd}, "tool_use_id": "toolu_1"})
    .to_string()
    .into_bytes()
}

#[test]
fn appends_one_line_silently() {
    let s = tmp("one");
    let out = hook(&s, &pre_tool("s1", "cargo build"));
    assert_silent(&out);
    let lines = log(&s, "s1");
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains(r#""act":"$ cargo build""#),
        "{}",
        lines[0]
    );
}

#[test]
fn garbage_input_is_ignored() {
    let s = tmp("garbage");
    for input in [&b""[..], b"not json", b"[1,2]", br#"{"session_id":"s1"}"#] {
        assert_silent(&hook(&s, input));
    }
    for sid in ["../../escape", "a/b", ""] {
        assert_silent(&hook(&s, &pre_tool(sid, "ls")));
    }
    assert!(!s.join("escape").exists());
    let sessions = s.join("ari/sessions");
    assert!(!sessions.exists() || std::fs::read_dir(&sessions).unwrap().next().is_none());
}

#[test]
fn huge_payload_stays_under_4k() {
    let s = tmp("huge");
    let raw = serde_json::json!({"session_id": "s1", "hook_event_name": "PreToolUse",
        "tool_name": "Write", "tool_input": {"file_path": "/p/a", "content": "x".repeat(5_000_000)}});
    assert_silent(&hook(&s, raw.to_string().as_bytes()));
    let lines = log(&s, "s1");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].len() < 4096);
}

#[test]
fn concurrent_writers_do_not_interleave() {
    let s = tmp("concurrent");
    let handles: Vec<_> = (0..50)
        .map(|i| {
            let s = s.clone();
            std::thread::spawn(move || {
                hook(&s, &pre_tool("s1", &format!("echo {i} {}", "y".repeat(60))))
            })
        })
        .collect();
    for h in handles {
        assert_silent(&h.join().unwrap());
    }
    let lines = log(&s, "s1");
    assert_eq!(lines.len(), 50);
    for l in lines {
        serde_json::from_str::<serde_json::Value>(&l).unwrap();
    }
}

#[test]
fn session_start_writes_meta_and_prunes() {
    let s = tmp("prune");
    let old = s.join("ari/sessions/old");
    std::fs::create_dir_all(&old).unwrap();
    let f = std::fs::File::create(old.join("events.jsonl")).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(8 * 86_400))
        .unwrap();
    let fresh = s.join("ari/sessions/fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    std::fs::write(fresh.join("events.jsonl"), "").unwrap();

    let start = serde_json::json!({"session_id": "s1", "hook_event_name": "SessionStart",
        "cwd": "/p", "source": "startup", "transcript_path": "/t.jsonl"});
    assert_silent(&hook(&s, start.to_string().as_bytes()));
    let meta: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(s.join("ari/sessions/s1/meta.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["cwd"], "/p");
    assert_eq!(meta["transcript"], "/t.jsonl");
    assert!(!old.exists(), "old session should be pruned");
    assert!(fresh.exists(), "fresh session must survive");
}

#[test]
fn unwritable_state_dir_is_silent() {
    let s = tmp("ro");
    std::fs::write(s.join("ari"), "a file, not a dir").unwrap();
    assert_silent(&hook(&s, &pre_tool("s1", "ls")));
}

#[test]
fn first_event_writes_meta_once() {
    let s = tmp("meta-first");
    assert_silent(&hook(&s, &pre_tool("s1", "ls")));
    let meta = s.join("ari/sessions/s1/meta.json");
    let first = std::fs::read_to_string(&meta).unwrap();
    assert!(first.contains(r#""cwd":"/p""#), "{first}");
    let start =
        serde_json::json!({"session_id": "s1", "hook_event_name": "SessionStart", "cwd": "/other"});
    assert_silent(&hook(&s, start.to_string().as_bytes()));
    assert_eq!(std::fs::read_to_string(&meta).unwrap(), first);
}

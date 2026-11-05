use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ari-sl-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("config/ari")).unwrap();
    d
}

fn script(dir: &Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    format!("sh '{}'", p.display())
}

fn statusline(dir: &Path, config: &str, stdin: &[u8]) -> (String, Duration) {
    std::fs::write(dir.join("config/ari/config.toml"), config).unwrap();
    let t = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ari"))
        .arg("statusline")
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("NO_COLOR", "1")
        .env("COLUMNS", "200")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    (String::from_utf8(out.stdout).unwrap(), t.elapsed())
}

const PAYLOAD: &str = r#"{"session_id":"s1","transcript_path":"/t.jsonl","model":{"display_name":"Opus"},
 "cost":{"total_cost_usd":2.69},"context_window":{"used_percentage":71,"context_window_size":200000},
 "rate_limits":{"five_hour":{"used_percentage":62,"resets_at":4102444800}}}"#;

fn chain_config(dir: &Path, position: &str) -> String {
    let cmds = [
        script(
            dir,
            "ponytail.sh",
            r"printf '\033[38;5;108m[PONYTAIL]\033[0m'",
        ),
        script(dir, "multi.sh", "echo line1; echo line2"),
        script(dir, "slow.sh", "sleep 2; echo slow"),
        script(dir, "fail.sh", "echo partial; exit 1"),
    ];
    let list: Vec<String> = cmds.iter().map(|c| format!("{c:?}")).collect();
    format!(
        "position = \"{position}\"\n[chain]\ncommands = [{}]\n",
        list.join(", ")
    )
}

#[test]
fn chains_untouched_and_isolated() {
    let d = tmp("chain");
    let (out, took) = statusline(&d, &chain_config(&d, "top"), PAYLOAD.as_bytes());
    let (row, rest) = out.split_once('\n').unwrap();
    assert!(
        row.starts_with("ARI ◌ no hooks yet │ ctx ▰▰▰▰▱ 71% │ 5h 62% ↺"),
        "{row}"
    );
    assert!(row.ends_with("│ 7d — │ $2.69"), "{row}");
    assert_eq!(rest, "\x1b[38;5;108m[PONYTAIL]\x1b[0m\nline1\nline2");
    assert!(took < Duration::from_millis(900), "{took:?}");
    let usage = std::fs::read_to_string(d.join("state/ari/sessions/s1/usage.json")).unwrap();
    assert_eq!(usage, PAYLOAD);
}

#[test]
fn bottom_position() {
    let d = tmp("bottom");
    let (out, _) = statusline(&d, &chain_config(&d, "bottom"), PAYLOAD.as_bytes());
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "\x1b[38;5;108m[PONYTAIL]\x1b[0m");
    assert!(lines[3].starts_with("ARI ◌ no hooks yet"));
}

#[test]
fn garbage_stdin_still_renders() {
    let d = tmp("garbage");
    let cfg = format!(
        "[chain]\ncommands = [{:?}]\n",
        script(&d, "p.sh", "printf '[P]'")
    );
    let (out, _) = statusline(&d, &cfg, b"not json at all");
    assert_eq!(out, "ARI ◌ no hooks yet │ ctx — │ 5h — │ 7d —\n[P]");
    assert!(!d.join("state/ari/sessions").exists());
}

#[test]
fn reads_hook_events() {
    let d = tmp("events");
    let dir = d.join("state/ari/sessions/s1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("events.jsonl"),
        concat!(
            r#"{"t":1,"e":"UserPromptSubmit","msg":"go"}"#, "\n",
            r#"{"t":2,"e":"TaskCreated","task":{"id":"1","subject":"a"}}"#, "\n",
            r#"{"t":3,"e":"PostToolUseFailure","tool":"Bash","sig":"Bash:make","ok":false,"msg":"boom"}"#, "\n",
        ),
    )
    .unwrap();
    let (out, _) = statusline(&d, "", PAYLOAD.as_bytes());
    assert!(
        out.starts_with("ARI ↻ RETRYING │ ") && out.contains(" thinking │ ░░░░░░░░░░ 0/1 │ ✗ 1 │"),
        "{out}"
    );
}

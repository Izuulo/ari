use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ari-setup-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("claude")).unwrap();
    d
}

/// `ari` with HOME, PATH and every app's home inside `d`, so the real ~/.codex, ~/.dsh and
/// ~/.claude are never found or written.
fn isolated(d: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ari"));
    c.args(args)
        .env("HOME", d)
        .env("PATH", "/usr/bin:/bin")
        .env_remove("CODEX_HOME")
        .env_remove("DSH_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("XDG_CONFIG_HOME", d.join("config"))
        .env("XDG_STATE_HOME", d.join("state"));
    c
}

fn run(mut c: Command) -> String {
    let out = c.output().unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn ari(d: &Path, args: &[&str]) -> String {
    let mut c = isolated(d, args);
    c.env("CLAUDE_CONFIG_DIR", d.join("claude"));
    run(c)
}

const ORIGINAL: &str = r#"{
  "permissions": {
    "allow": [],
    "defaultMode": "auto"
  },
  "model": "opus",
  "statusLine": {
    "type": "command",
    "command": "bash \"$HOME/.claude/ponytail-statusline.sh\"",
    "padding": 2
  },
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "echo hi"
          }
        ]
      }
    ]
  }
}
"#;

#[test]
fn setup_then_uninstall_restores_exactly() {
    let d = tmp("roundtrip");
    let settings = d.join("claude/settings.json");
    std::fs::write(&settings, ORIGINAL).unwrap();

    let dry = ari(&d, &["setup", "--dry-run"]);
    assert!(
        dry.contains("+ {\"type\":\"command\",\"command\":\"ari statusline\""),
        "{dry}"
    );
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), ORIGINAL);

    ari(&d, &["setup", "--yes"]);
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(after["statusLine"]["command"], "ari statusline");
    assert_eq!(after["statusLine"]["padding"], 2);
    assert_eq!(after["statusLine"]["refreshInterval"], 2);
    let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["permissions", "model", "statusLine", "hooks"]);
    let config = std::fs::read_to_string(d.join("config/ari/config.toml")).unwrap();
    let config = ari_cli::config::parse(&config);
    assert_eq!(
        config.chain.commands,
        [r#"bash "$HOME/.claude/ponytail-statusline.sh""#]
    );
    assert_eq!(config.hide, ["cost", "tokens"]);
    let backups = std::fs::read_dir(d.join("claude")).unwrap().filter(|e| {
        e.as_ref()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("ari-backup")
    });
    assert_eq!(backups.count(), 1);

    assert!(ari(&d, &["setup", "--yes"]).contains("already"));

    ari(&d, &["uninstall", "--yes"]);
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), ORIGINAL);
    let config =
        ari_cli::config::parse(&std::fs::read_to_string(d.join("config/ari/config.toml")).unwrap());
    assert!(config.chain.commands.is_empty() && config.hide.is_empty());
}

#[test]
fn uninstall_leaves_a_broken_config_alone() {
    let d = tmp("broken-config");
    let settings = d.join("claude/settings.json");
    std::fs::write(&settings, ORIGINAL).unwrap();
    ari(&d, &["setup", "--yes"]);
    let cfg = d.join("config/ari/config.toml");
    std::fs::write(&cfg, "face = [[[").unwrap();
    ari(&d, &["uninstall", "--yes"]);
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), ORIGINAL);
    assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "face = [[[");
}

#[test]
fn no_prior_statusline_and_missing_file() {
    let d = tmp("fresh");
    let settings = d.join("claude/settings.json");
    ari(&d, &["setup", "--yes"]);
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(after.as_object().unwrap().len(), 1);
    ari(&d, &["uninstall", "--yes"]);
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}\n");
}

#[test]
fn refuses_broken_settings() {
    let d = tmp("broken");
    let settings = d.join("claude/settings.json");
    std::fs::write(&settings, "{ not json").unwrap();
    let out = isolated(&d, &["setup", "--yes"])
        .env("CLAUDE_CONFIG_DIR", d.join("claude"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json");
}

#[test]
fn purge_removes_state_and_config() {
    let d = tmp("purge");
    ari(&d, &["setup", "--yes"]);
    std::fs::create_dir_all(d.join("state/ari/sessions/s1")).unwrap();
    ari(&d, &["uninstall", "--yes", "--purge"]);
    assert!(!d.join("state/ari").exists() && !d.join("config/ari").exists());
}

// ── Codex and DeepSeek Harness ──────────────────────────────────────────────

use ari_cli::setup::{codex_add, codex_remove};
use serde_json::{Value, json};

const CODEX_EVENTS: [&str; 11] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "PreCompact",
    "PostCompact",
    "Stop",
    "SubagentStart",
    "SubagentStop",
];

const USER_CODEX: &str = r#"{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": "~/guard.sh", "timeout": 5 }]
      }
    ]
  }
}"#;

fn v(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn entry(agent: &str) -> Value {
    json!({
        "type": "command",
        "command": format!("command -v ari >/dev/null 2>&1 && ari hook --agent {agent} || true"),
        "timeout": 1
    })
}

/// Every hook entry under `map`, by event.
fn entries(map: &Value) -> Vec<(String, Value)> {
    let mut out = vec![];
    for (event, groups) in map.as_object().unwrap() {
        for g in groups.as_array().unwrap() {
            for h in g["hooks"].as_array().unwrap() {
                out.push((event.clone(), h.clone()));
            }
        }
    }
    out
}

fn ours(map: &Value, agent: &str) -> Vec<String> {
    entries(map)
        .into_iter()
        .filter(|(_, h)| *h == entry(agent))
        .map(|(e, _)| e)
        .collect()
}

#[test]
fn codex_add_keeps_user_hooks() {
    let out = v(&codex_add(Some(USER_CODEX)).unwrap());
    let hooks = &out["hooks"];
    assert_eq!(
        hooks["PreToolUse"][0],
        v(USER_CODEX)["hooks"]["PreToolUse"][0]
    );
    let mut got = ours(hooks, "codex");
    let mut want = CODEX_EVENTS.map(String::from).to_vec();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    assert_eq!(entries(hooks).len(), 12);
    // A missing or empty file starts fresh.
    for fresh in [None, Some(""), Some(" \n")] {
        assert_eq!(
            ours(&v(&codex_add(fresh).unwrap())["hooks"], "codex").len(),
            11
        );
    }
}

#[test]
fn codex_add_is_idempotent() {
    for x in [None, Some(USER_CODEX)] {
        let once = codex_add(x).unwrap();
        assert_eq!(codex_add(Some(&once)).unwrap(), once);
    }
}

#[test]
fn codex_add_refuses_broken_json() {
    for bad in [
        "{ nope",
        "[]",
        r#"{"hooks":[]}"#,
        r#"{"hooks":{"Stop":{}}}"#,
    ] {
        assert!(codex_add(Some(bad)).is_err(), "{bad}");
    }
}

#[test]
fn codex_remove_only_ours() {
    let added = codex_add(Some(USER_CODEX)).unwrap();
    assert_eq!(v(&codex_remove(&added).unwrap()), v(USER_CODEX));
    assert_eq!(
        v(&codex_remove(&codex_add(None).unwrap()).unwrap()),
        json!({"hooks": {}})
    );
    assert_eq!(codex_remove(USER_CODEX), None);
    assert_eq!(codex_remove("{ broken"), None);
}

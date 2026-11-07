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

use ari_cli::app::App;
use ari_cli::setup::{
    add_hooks, codex_add, codex_remove, dsh_add, dsh_bridge, dsh_hooks, dsh_remove, remove_hooks,
};
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
const DSH_EVENTS: [&str; 7] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
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

const USER_PATCH: &str = "# my tweaks\n- id: system-prompt\n  config:\n    personaPrefix: hi\n";

#[test]
fn dsh_add_points_bridge_at_ari_file() {
    let path = "/h/.config/ari/dsh-hooks.json";
    for before in [
        None,
        Some(USER_PATCH),
        Some("[]\n"),
        Some("# only [] here\n[]\n"),
    ] {
        let out = dsh_add(before, path).unwrap();
        assert!(
            out.starts_with(before.unwrap_or("").trim_end_matches("[]\n")),
            "{out}"
        );
        assert!(out.contains("- insert:\n"), "{out}");
        assert!(
            out.contains("name: '@deepseek-ai/dsh-hooks-claude-code'"),
            "{out}"
        );
        assert!(
            out.contains("configPath: '/h/.config/ari/dsh-hooks.json'"),
            "{out}"
        );
        assert!(!out.contains("[]\n"), "{out}");
        assert_eq!(dsh_bridge(&out), Ok(Some(path.to_string())));
        assert_eq!(dsh_add(Some(&out), path).unwrap(), out, "idempotent");
    }
    assert_eq!(dsh_bridge(USER_PATCH), Ok(None));
    // A quote in the path stays one YAML scalar.
    let odd = dsh_add(None, "/h/it's/x.json").unwrap();
    assert_eq!(dsh_bridge(&odd), Ok(Some("/h/it's/x.json".to_string())));
    // Not a block list ARI can safely extend: refused.
    for bad in ["foo: bar\n", "[ {id: x} ]\n"] {
        assert!(dsh_add(Some(bad), path).is_err(), "{bad}");
    }

    let hooks = v(&dsh_hooks());
    let mut got = ours(&hooks["hooks"], "dsh");
    let mut want = DSH_EVENTS.map(String::from).to_vec();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    assert_eq!(entries(&hooks["hooks"]).len(), 7);
}

#[test]
fn dsh_add_merges_into_existing_bridge_config() {
    let patch = "- insert:\n    - id: my-hooks\n      name: '@deepseek-ai/dsh-hooks-claude-code'\n      config:\n        configPath: \"/u/.claude/hooks.json\" # mine\n";
    assert_eq!(
        dsh_bridge(patch),
        Ok(Some("/u/.claude/hooks.json".to_string()))
    );
    // The patch already has a bridge: it stays as it is, and ARI goes into that file.
    assert_eq!(dsh_add(Some(patch), "/h/ari.json").unwrap(), patch);

    let user = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#;
    let merged = v(&add_hooks(Some(user), App::Dsh).unwrap());
    assert_eq!(entries(&merged["hooks"]).len(), 8);
    assert_eq!(merged["hooks"]["Stop"][0], v(user)["hooks"]["Stop"][0]);
    assert_eq!(
        v(&remove_hooks(&merged.to_string(), App::Dsh).unwrap()),
        v(user)
    );

    // A bare event map (hooks.json style) gains entries at its root, not under a new `hooks`
    // key that would hide the user's own.
    let bare = r#"{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}"#;
    let merged = v(&add_hooks(Some(bare), App::Dsh).unwrap());
    assert!(merged.get("hooks").is_none(), "{merged}");
    assert_eq!(ours(&merged, "dsh").len(), 7);
    assert_eq!(
        v(&remove_hooks(&merged.to_string(), App::Dsh).unwrap()),
        v(bare)
    );

    // A configPath ARI can't follow is an error, never a guess.
    for p in ["./hooks.json", "!!js process.cwd()", "~/h.json"] {
        let patch = format!(
            "- name: '@deepseek-ai/dsh-hooks-claude-code'\n  config:\n    configPath: {p}\n"
        );
        assert!(dsh_bridge(&patch).is_err(), "{p}");
        assert!(dsh_add(Some(&patch), "/h/ari.json").is_err(), "{p}");
    }
}

#[test]
fn dsh_remove_restores() {
    let path = "/h/ari.json";
    for before in [USER_PATCH, "[]\n", "# c\n[]\n", "- id: x\n  config: {}"] {
        let added = dsh_add(Some(before), path).unwrap();
        let back = dsh_remove(&added).unwrap();
        assert_eq!(back.trim_end(), before.trim_end(), "{added}");
    }
    assert_eq!(dsh_remove(&dsh_add(None, path).unwrap()).unwrap(), "");
    assert_eq!(dsh_remove(USER_PATCH), None);
}

#[test]
fn codex_only_machine_setup_and_uninstall() {
    let d = std::env::temp_dir().join(format!("ari-setup-{}-codex", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let hooks = d.join(".codex/hooks.json");
    std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    std::fs::write(&hooks, USER_CODEX).unwrap();

    let dry = run(isolated(&d, &["setup", "--dry-run"]));
    assert!(dry.contains("ari hook --agent codex"), "{dry}");
    assert_eq!(std::fs::read_to_string(&hooks).unwrap(), USER_CODEX);

    let out = run(isolated(&d, &["setup", "--yes"]));
    assert!(
        out.contains(
            "Codex: hooks added to ~/.codex/hooks.json. Open Codex and run /hooks once to approve them."
        ),
        "{out}"
    );
    assert_eq!(
        ours(
            &v(&std::fs::read_to_string(&hooks).unwrap())["hooks"],
            "codex"
        )
        .len(),
        11
    );
    assert!(
        !d.join(".claude").exists(),
        "no Claude Code here, so no ~/.claude"
    );
    assert!(run(isolated(&d, &["setup", "--yes"])).contains("already"));

    run(isolated(&d, &["uninstall", "--yes"]));
    assert_eq!(v(&std::fs::read_to_string(&hooks).unwrap()), v(USER_CODEX));
    assert!(!d.join(".claude").exists());
}

#[test]
fn dsh_only_machine_setup_and_uninstall() {
    let d = std::env::temp_dir().join(format!("ari-setup-{}-dsh", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join(".dsh")).unwrap();
    let patch = d.join(".dsh/cordis.patch.yml");
    let ari_hooks = d.join("config/ari/dsh-hooks.json");

    run(isolated(&d, &["setup", "--yes"]));
    let text = std::fs::read_to_string(&patch).unwrap();
    assert_eq!(dsh_bridge(&text), Ok(Some(ari_hooks.display().to_string())));
    assert_eq!(std::fs::read_to_string(&ari_hooks).unwrap(), dsh_hooks());
    assert!(
        !d.join(".claude").exists(),
        "no Claude Code here, so no ~/.claude"
    );

    run(isolated(&d, &["uninstall", "--yes"]));
    assert!(!patch.exists() && !ari_hooks.exists());
    assert!(!d.join(".claude").exists());
}

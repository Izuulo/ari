use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ari-setup-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("claude")).unwrap();
    d
}

fn ari(d: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_ari"))
        .args(args)
        .env("CLAUDE_CONFIG_DIR", d.join("claude"))
        .env("XDG_CONFIG_HOME", d.join("config"))
        .env("XDG_STATE_HOME", d.join("state"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
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
    assert_eq!(config.hide, ["cost", "tokens", "time", "cache"]);
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
    let out = Command::new(env!("CARGO_BIN_EXE_ari"))
        .args(["setup", "--yes"])
        .env("CLAUDE_CONFIG_DIR", d.join("claude"))
        .env("XDG_CONFIG_HOME", d.join("config"))
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

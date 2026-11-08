use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ari-doctor-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("bin")).unwrap();
    std::fs::create_dir_all(d.join("claude")).unwrap();
    d
}

fn doctor(d: &Path, with_binary: bool) -> (Output, String) {
    // Desktop notifications default on, which is a problem on a Linux box without notify-send.
    let cfg = d.join("config/ari/config.toml");
    if !cfg.exists() {
        write(cfg, "[notify]\ndesktop = false\n");
    }
    if with_binary && !d.join("bin/ari").exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ari"), d.join("bin/ari")).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_ari"))
        .arg("doctor")
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("HOME", d)
        .env_remove("CODEX_HOME")
        .env_remove("DSH_HOME")
        .env("CLAUDE_CONFIG_DIR", d.join("claude"))
        .env("XDG_CONFIG_HOME", d.join("config"))
        .env("XDG_STATE_HOME", d.join("state"))
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout.clone()).unwrap();
    (out, text)
}

fn write(p: PathBuf, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

#[test]
fn all_good() {
    let d = tmp("good");
    write(
        d.join("claude/settings.json"),
        r#"{"statusLine":{"type":"command","command":"ari statusline"},"enabledPlugins":{"ari@ari":true}}"#,
    );
    write(
        d.join("claude/plugins/installed_plugins.json"),
        r#"{"version":2,"plugins":{"ari@ari":[{"scope":"user"}]}}"#,
    );
    write(
        d.join("config/ari/config.toml"),
        "[chain]\ncommands = [\"printf '[P]'\"]\n\n[notify]\ndesktop = false\n",
    );
    write(
        d.join("state/ari/sessions/s1/events.jsonl"),
        "{\"t\":1,\"e\":\"Stop\"}\n",
    );
    let (out, text) = doctor(&d, true);
    assert!(out.status.success(), "{text}");
    assert_eq!(text.matches('✓').count(), 7, "{text}");
    assert!(text.contains("All good."));
}

#[test]
fn empty_home_lists_fixes() {
    let d = tmp("empty");
    let (out, text) = doctor(&d, false);
    assert_eq!(out.status.code(), Some(1), "{text}");
    for name in ["binary", "plugin", "statusline", "hooks"] {
        let line = text.lines().find(|l| l.contains(name)).unwrap();
        assert!(line.contains('✗'), "{text}");
    }
    assert_eq!(text.matches('→').count(), 4, "{text}");
    assert!(text.contains("/plugin marketplace add Izuulo/ari"));
}

#[test]
fn slow_chain_times_out() {
    let d = tmp("slow");
    write(
        d.join("config/ari/config.toml"),
        "[chain]\ncommands = [\"sleep 2\"]\ntimeout_ms = 200\n",
    );
    let t = std::time::Instant::now();
    let (_, text) = doctor(&d, true);
    assert!(text.contains("slower than 200ms"), "{text}");
    assert!(t.elapsed() < std::time::Duration::from_millis(1500));
}

fn line<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find(|l| l.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("no {name} line: {text}"))
}

#[test]
fn codex_wired_or_not() {
    let d = tmp("codex");
    std::fs::create_dir_all(d.join(".codex")).unwrap();
    let (_, text) = doctor(&d, true);
    let l = line(&text, "codex");
    assert!(l.contains('✗'), "{text}");
    let fix = text.lines().skip_while(|x| *x != l).nth(1).unwrap();
    assert!(fix.contains("→ ari setup"), "{text}");

    write(
        d.join(".codex/hooks.json"),
        &ari_cli::setup::codex_add(None).unwrap(),
    );
    let (_, text) = doctor(&d, true);
    assert!(
        line(&text, "codex")
            .contains("✓ codex      wired (approve with /hooks if events don't arrive)"),
        "{text}"
    );
    let l = line(&text, "dsh");
    assert!(l.contains("not found") && !l.contains('✗'), "{text}");
}

#[test]
fn dsh_wired_or_not() {
    let d = tmp("dsh");
    std::fs::create_dir_all(d.join(".dsh")).unwrap();
    let (_, text) = doctor(&d, true);
    assert!(line(&text, "dsh").contains('✗'), "{text}");

    let ari_hooks = d.join("config/ari/dsh-hooks.json");
    write(ari_hooks.clone(), &ari_cli::setup::dsh_hooks());
    write(
        d.join(".dsh/cordis.patch.yml"),
        &ari_cli::setup::dsh_add(None, &ari_hooks.display().to_string()).unwrap(),
    );
    let (_, text) = doctor(&d, true);
    assert!(line(&text, "dsh").contains('✓'), "{text}");

    // A bridge whose file runs plain `ari hook`: setup leaves it, doctor says so.
    let theirs = d.join("theirs.json");
    write(
        theirs.clone(),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"ari hook"}]}]}}"#,
    );
    write(
        d.join(".dsh/cordis.patch.yml"),
        &format!(
            "- insert:\n    - id: mine\n      name: '@deepseek-ai/dsh-hooks-claude-code'\n      config:\n        configPath: {}\n",
            theirs.display()
        ),
    );
    let (_, text) = doctor(&d, true);
    assert!(line(&text, "dsh").contains("plain `ari hook`"), "{text}");
}

#[test]
fn dsh_only_machine_wired_is_all_good() {
    let d = tmp("dsh-only");
    std::fs::remove_dir_all(d.join("claude")).unwrap();
    std::fs::create_dir_all(d.join(".dsh")).unwrap();
    let ari_hooks = d.join("config/ari/dsh-hooks.json");
    write(ari_hooks.clone(), &ari_cli::setup::dsh_hooks());
    write(
        d.join(".dsh/cordis.patch.yml"),
        &ari_cli::setup::dsh_add(None, &ari_hooks.display().to_string()).unwrap(),
    );
    write(
        d.join("state/ari/sessions/s1/events.jsonl"),
        "{\"t\":1,\"e\":\"Stop\"}\n",
    );
    let (out, text) = doctor(&d, true);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("All good."), "{text}");
    let names: Vec<&str> = text
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .collect();
    assert!(
        !names.contains(&"plugin") && !names.contains(&"statusline"),
        "{text}"
    );
    assert!(line(&text, "claude").contains("not found"), "{text}");
    assert!(line(&text, "codex").contains("not found"), "{text}");

    // No events yet: the hint names dsh, not Claude Code.
    std::fs::remove_dir_all(d.join("state")).unwrap();
    let (_, text) = doctor(&d, true);
    assert!(!text.contains("Claude Code session"), "{text}");
    assert!(text.contains("DeepSeek"), "{text}");
}

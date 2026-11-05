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
    if with_binary && !d.join("bin/ari").exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ari"), d.join("bin/ari")).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_ari"))
        .arg("doctor")
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("HOME", d)
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
        "[chain]\ncommands = [\"printf '[P]'\"]\n",
    );
    write(
        d.join("state/ari/sessions/s1/events.jsonl"),
        "{\"t\":1,\"e\":\"Stop\"}\n",
    );
    let (out, text) = doctor(&d, true);
    assert!(out.status.success(), "{text}");
    assert_eq!(text.matches('✓').count(), 5, "{text}");
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

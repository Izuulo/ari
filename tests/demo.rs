use std::process::Command;

/// Scripts get the full panel, whatever view config.toml remembers.
#[test]
fn demo_once_ignores_a_saved_simple_view() {
    let d = std::env::temp_dir().join(format!("ari-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("ari")).unwrap();
    std::fs::write(
        d.join("ari/config.toml"),
        "[view]\nwatch = \"constellation\"\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ari"))
        .args(["demo", "--once"])
        .env("XDG_CONFIG_HOME", &d)
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let _ = std::fs::remove_dir_all(&d);
    assert!(out.status.success());
    assert!(
        text.contains("╭─ ARI") && !text.contains("( main )"),
        "{text}"
    );
}

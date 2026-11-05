use std::path::{Path, PathBuf};
use std::process::Command;

fn state_with_fixture(name: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("ari-recap-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("proj");
    let dir = root.join("state/ari/sessions/s1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    // The process sees the resolved path (/private/var on macOS), like Claude Code reports it.
    let project = project.canonicalize().unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/multi/raw.jsonl"),
        dir.join("raw.jsonl"),
    )
    .unwrap();
    std::fs::write(
        dir.join("meta.json"),
        format!(r#"{{"cwd":"{}"}}"#, project.display()),
    )
    .unwrap();
    (root, project)
}

fn recap(root: &Path, cwd: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ari"))
        .arg("recap")
        .args(args)
        .current_dir(cwd)
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn recap_follows_the_cwd_session() {
    let (root, project) = state_with_fixture("cwd");
    let (ok, out, err) = recap(&root, &project, &[]);
    assert!(ok, "{err}");
    assert!(out.contains("│ ████████████████████ 4/4 tasks"), "{out}");
    let (ok, md, _) = recap(&root, &project, &["--session", "s1", "--md"]);
    assert!(ok && md.starts_with("### Recap"), "{md}");
}

#[test]
fn recap_bad_input() {
    let (root, project) = state_with_fixture("bad");
    let (ok, _, err) = recap(&root, &project, &["--since", "soon"]);
    assert!(!ok && err.contains("--since"), "{err}");
    let (ok, _, err) = recap(&root, &root, &[]);
    assert!(!ok && err.contains("no session"), "{err}");
}

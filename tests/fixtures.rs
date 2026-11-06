use ari_cli::{event, replay};
use std::path::Path;

fn fixture(name: &str) -> Vec<event::Event> {
    replay::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name),
    )
    .unwrap()
}

#[test]
fn basic_events() {
    let lines: String = fixture("basic").iter().map(event::to_line).collect();
    insta::assert_snapshot!(lines);
}

#[test]
fn team_events() {
    let lines: String = fixture("team").iter().map(event::to_line).collect();
    insta::assert_snapshot!(lines);
}

#[test]
fn basic_timeline() {
    insta::assert_snapshot!(replay::format(&fixture("basic")));
}

#[test]
fn basic_verdicts() {
    use ari_cli::state::State;
    let events = fixture("basic");
    let t0 = events[0].t;
    let mut s = State::default();
    let mut last = None;
    let mut out = Vec::new();
    for ev in &events {
        s.apply(ev);
        let v = s.verdict();
        if last != Some(v) {
            let (done, total) = s.tasks_done();
            out.push(format!(
                "+{:>5.1}s {:<9} after {:<18} tasks {done}/{total} agents {} errors {}",
                (ev.t - t0) as f64 / 1000.0,
                v.label(),
                ev.e,
                s.running_agents(ev.t),
                s.open_errors()
            ));
            last = Some(v);
        }
    }
    insta::assert_snapshot!(out.join("\n"));
}

/// The multi-agent fixture, frozen at three moments: after the failure,
/// while both subagents run, and at the end.
#[test]
fn multi_panel() {
    use ari_cli::{config::Config, panel, state::State, usage::Usage};
    let events = fixture("multi");
    let at = |pred: &dyn Fn(&State, &ari_cli::event::Event) -> bool| {
        let mut s = State::default();
        for ev in &events {
            s.apply(ev);
            if pred(&s, ev) {
                return (s, ev.t);
            }
        }
        (s, events.last().unwrap().t)
    };
    let frames = [
        ("after failure", at(&|_, e| e.ok == Some(false))),
        (
            "two agents",
            at(&|s, _| {
                s.running_agents(u64::MAX / 2) == 0
                    && s.agents
                        .iter()
                        .filter(|a| a.id != "main" && !a.done && a.calls > 0)
                        .count()
                        == 2
            }),
        ),
        ("end", at(&|_, _| false)),
    ];
    let out: Vec<String> = frames
        .iter()
        .map(|(name, (s, t))| {
            let lines = panel::lines(
                s,
                &Usage::default(),
                &Default::default(),
                &Config::default(),
                panel::View::default(),
                *t,
                "12:00",
                56,
                60,
                false,
            );
            format!("--- {name} ---\n{}", panel::to_plain(&lines))
        })
        .collect();
    insta::assert_snapshot!(out.join("\n"));
}

#[test]
fn multi_verdicts() {
    use ari_cli::state::State;
    let mut s = State::default();
    let mut seen = Vec::new();
    for ev in &fixture("multi") {
        s.apply(ev);
        let v = s.verdict().label();
        if seen.last() != Some(&v) {
            seen.push(v);
        }
    }
    assert_eq!(seen, ["IDLE", "ON TRACK", "RETRYING", "ON TRACK", "DONE"]);
}

/// End of the multi run with tokens read from its real transcripts, plus cost.
#[test]
fn multi_panel_with_tokens() {
    use ari_cli::{bar::Extras, config::Config, panel, state::State, tokens, usage::Usage};
    let events = fixture("multi");
    let mut s = State::default();
    for ev in &events {
        s.apply(ev);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/multi");
    for a in s.agents.iter_mut().filter(|a| a.id != "main") {
        a.transcript = Some(
            dir.join(format!("subagents/agent-{}.jsonl", a.id))
                .to_string_lossy()
                .into(),
        );
    }
    let tmp = std::env::temp_dir().join(format!("ari-fixture-tokens-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let main = dir.join("transcript.jsonl");
    let x = Extras {
        tokens: Some(tokens::update(&tmp, &s, main.to_str())),
        burn_eta: Some(55 * 60_000),
        ..Default::default()
    };
    let u = Usage {
        cost_usd: Some(0.12),
        duration_ms: Some(40_000),
        api_duration_ms: Some(30_000),
        ..Default::default()
    };
    let t = events.last().unwrap().t;
    let lines = panel::lines(
        &s,
        &u,
        &x,
        &Config::default(),
        panel::View::default(),
        t,
        "12:00",
        56,
        60,
        false,
    );
    let out = panel::to_plain(&lines);
    assert!(out.contains("481k · 3.1k out · cache 90% hit"), "{out}");
    insta::assert_snapshot!(out);
}

/// Named subagents get their names and parent; their SendMessage calls land in the log.
#[test]
fn team_links() {
    use ari_cli::state::State;
    let mut s = State::default();
    for ev in &fixture("team") {
        s.apply(ev);
    }
    let id = |n: &str| {
        let a = s
            .agent_by_name(n)
            .unwrap_or_else(|| panic!("no agent named {n}"));
        assert_eq!(a.parent.as_deref(), Some("main"));
        a.id.clone()
    };
    let (w, c) = (id("writer"), id("checker"));
    let m: Vec<_> = s
        .messages
        .iter()
        .map(|m| (m.from.clone(), m.to.clone()))
        .collect();
    assert_eq!(m, [(w.clone(), c.clone()), (c, w)]);
}

/// Codex and dsh tool calls read like Claude's: failures count, edits name their file.
#[test]
fn codex_and_dsh_replay() {
    use ari_cli::state::State;
    for (name, edit) in [("codex", "editing src/a.rs"), ("dsh", "editing b.rs")] {
        let mut s = State::default();
        let mut act = None;
        for ev in &fixture(name) {
            s.apply(ev);
            if ev.e == "PreToolUse" && ev.tool.as_deref() == Some("Edit") {
                act = s.agents[0].act.clone();
            }
        }
        assert_eq!(act.as_deref(), Some(edit), "{name}");
        assert_eq!(s.open_errors(), 1, "{name}");
        let calls = &s.agents[0].recent;
        assert!(calls.iter().any(|c| c.ok == Some(true)), "{name}");
    }
}

/// A session dir for a replayed non-Claude fixture, as the panel would read it.
fn app_panel(name: &str, agent: &str, transcript: Option<&str>) -> String {
    use ari_cli::{bar::Extras, config::Config, panel, state::State, usage};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let dir = std::env::temp_dir().join(format!("ari-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = transcript.map(|t| root.join(t).to_string_lossy().into_owned());
    let meta = serde_json::json!({"agent": agent, "cwd": "/p", "transcript": transcript});
    std::fs::write(dir.join("meta.json"), meta.to_string()).unwrap();
    let events = fixture(name);
    let mut s = State::default();
    for ev in &events {
        s.apply(ev);
    }
    let x = Extras {
        app: ari_cli::app::of(&dir),
        ..Default::default()
    };
    let lines = panel::lines(
        &s,
        &usage::load(&dir),
        &x,
        &Config::default(),
        panel::View::default(),
        events.last().unwrap().t,
        "12:00",
        56,
        60,
        false,
    );
    panel::to_plain(&lines)
}

#[test]
fn codex_panel() {
    let out = app_panel("codex", "codex", Some("codex/rollout.jsonl"));
    assert!(out.contains("Codex") && out.contains("ctx"), "{out}");
    insta::assert_snapshot!(out);
}

#[test]
fn dsh_panel() {
    let out = app_panel("dsh", "dsh", None);
    assert!(out.contains("DeepSeek"), "{out}");
    for gone in ["ctx", "5h", "7d", "cost"] {
        assert!(!out.contains(gone), "{gone}: {out}");
    }
    insta::assert_snapshot!(out);
}

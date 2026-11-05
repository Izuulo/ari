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
        let v = s.verdict(ev.t);
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
        let v = s.verdict(ev.t).label();
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

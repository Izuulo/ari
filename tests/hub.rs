use ari_cli::bar::Extras;
use ari_cli::config::Config;
use ari_cli::hub::{self, HubView, Input};
use ari_cli::state::State;
use ari_cli::usage::Usage;
use ari_cli::{panel, replay, tokens};
use std::path::Path;

/// The team fixture replayed, with tokens read from its transcripts.
fn team() -> (State, Extras, u64) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/team");
    let events = replay::load(&dir).unwrap();
    let mut s = State::default();
    for ev in &events {
        s.apply(ev);
    }
    for a in s.agents.iter_mut().filter(|a| a.id != "main") {
        a.transcript = Some(
            dir.join(format!("subagents/agent-{}.jsonl", a.id))
                .to_string_lossy()
                .into(),
        );
    }
    let tmp = std::env::temp_dir().join(format!("ari-hub-tokens-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let main = dir.join("transcript.jsonl");
    let x = Extras {
        tokens: Some(tokens::update(&tmp, &s, main.to_str())),
        ..Default::default()
    };
    (s, x, events.last().unwrap().t + 1000)
}

fn show(st: &State, x: &Extras, now: u64, v: HubView, w: u16, h: u16) -> String {
    let (u, cfg) = (Usage::default(), Config::default());
    let i = Input {
        st,
        u: &u,
        x,
        cfg: &cfg,
        now,
        clock: "13:04",
    };
    panel::to_plain(&hub::lines(&i, &v, w, h, false))
}

#[test]
fn hub_frame_full() {
    let (s, x, now) = team();
    insta::assert_snapshot!(show(&s, &x, now, HubView::default(), 168, 49));
}

#[test]
fn hub_no_hooks() {
    let out = show(
        &State::default(),
        &Extras::default(),
        1,
        HubView::default(),
        168,
        49,
    );
    assert!(out.contains("ARI hub"), "{out}");
    assert!(out.contains("no agents yet"), "{out}");
}

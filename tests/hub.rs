use ari_cli::bar::Extras;
use ari_cli::config::Config;
use ari_cli::hub::all::{Part, merge};
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
    show_u(&Config::default(), st, &Usage::default(), x, now, v, w, h)
}

#[allow(clippy::too_many_arguments)]
fn show_u(
    cfg: &Config,
    st: &State,
    u: &Usage,
    x: &Extras,
    now: u64,
    v: HubView,
    w: u16,
    h: u16,
) -> String {
    let i = Input {
        st,
        u,
        x,
        cfg,
        now,
        clock: "13:04",
    };
    panel::to_plain(&hub::lines(&i, &v, w, h, false))
}

#[test]
fn hub_all_frame() {
    let (s, x, now) = team();
    let tokens = x.tokens.unwrap();
    let part = |key: &str, name: &str| Part {
        key: key.into(),
        name: name.into(),
        st: s.clone(),
        app: Default::default(),
        u: Usage::default(),
        tokens: tokens.clone(),
    };
    let (st, u, tokens) = merge(vec![part("aaaaaaaa", "api"), part("bbbbbbbb", "web")], now);
    let x = Extras {
        tokens: Some(tokens),
        ..Default::default()
    };
    insta::assert_snapshot!(show_u(
        &Config::default(),
        &st,
        &u,
        &x,
        now,
        HubView::default(),
        168,
        49
    ));
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

#[test]
fn graph_screen_full() {
    let (s, x, now) = team();
    let v = HubView {
        screen: hub::Screen::Graph,
        ..Default::default()
    };
    insta::assert_snapshot!(show(&s, &x, now, v, 168, 49));
}

#[test]
fn zoom_screen_full() {
    let (s, x, now) = team();
    let sel = hub::order(&s)
        .iter()
        .position(|&k| s.agents[k].name.as_deref() == Some("writer"))
        .unwrap();
    let v = HubView {
        screen: hub::Screen::Zoom,
        sel,
        ..Default::default()
    };
    insta::assert_snapshot!(show(&s, &x, now, v, 168, 49));
}

fn lim(pct: f64) -> Option<ari_cli::usage::Limit> {
    Some(ari_cli::usage::Limit { pct, resets_at: 0 })
}

fn bare(key: &str, app: ari_cli::app::App, u: Usage, ctx: u64) -> Part {
    let mut tokens = tokens::TokenCache::default();
    tokens.main.input = ctx;
    Part {
        key: key.into(),
        name: key.into(),
        st: State::default(),
        app,
        u,
        tokens,
    }
}

#[test]
fn merge_keeps_accounts_apart() {
    use ari_cli::app::App;
    // Most recent first: the Codex part is newer than the Claude one.
    let codex = Usage {
        five_h: lim(12.0),
        ..Default::default()
    };
    let claude = Usage {
        five_h: lim(40.0),
        ..Default::default()
    };
    let parts = vec![
        bare("cccccccc", App::Codex, codex, 1),
        bare("aaaaaaaa", App::Claude, claude, 1),
    ];
    let (_, u, _) = merge(parts, 0);
    assert_eq!(u.five_h.unwrap().pct, 40.0);
    let labels: Vec<_> = u.app_limits.iter().map(|l| l.0.as_str()).collect();
    assert_eq!(labels, ["Claude Code", "Codex"]);
}

#[test]
fn merge_cost_is_claude_only() {
    use ari_cli::app::App;
    let cost = |c| Usage {
        cost_usd: Some(c),
        ..Default::default()
    };
    let parts = vec![
        bare("aaaaaaaa", App::Claude, cost(1.0), 100),
        bare("cccccccc", App::Codex, cost(9.0), 200),
    ];
    let (_, u, t) = merge(parts, 0);
    assert_eq!(u.cost_usd, Some(1.0));
    assert_eq!(t.total().input, 300);
}

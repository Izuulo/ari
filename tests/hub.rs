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

// --- Calm hub and hub --all ---

fn calm(all: bool) -> Config {
    use ari_cli::config::{Look, Views};
    let mut view = Views::default();
    if all {
        view.hub_all = Look::Calm;
    } else {
        view.hub = Look::Calm;
    }
    Config {
        view,
        ..Default::default()
    }
}

fn calm_team(now: Option<u64>) -> (String, State) {
    let (s, x, t) = team();
    let out = show_u(
        &calm(false),
        &s,
        &Usage::default(),
        &x,
        now.unwrap_or(t),
        HubView::default(),
        168,
        49,
    );
    (out, s)
}

#[test]
fn calm_hub_team() {
    let (out, _) = calm_team(None);
    for bad in ["tok", "ctx", "$"] {
        assert!(!out.contains(bad), "{bad} in:\n{out}");
    }
    insta::assert_snapshot!(out);
}

#[test]
fn calm_hub_packet_caption() {
    let (_, s) = calm_team(None);
    let m = s.messages.iter().max_by_key(|m| m.t).unwrap();
    let head: String = m.text.chars().take(20).collect();
    let (out, _) = calm_team(Some(m.t + 1000));
    assert!(out.contains('→') && out.contains(&head), "{head}:\n{out}");
    let (out, _) = calm_team(Some(m.t + 3500));
    assert!(!out.contains(&head), "{out}");
}

fn calm_all(parts: Vec<Part>) -> String {
    let (st, u, tokens) = merge(parts, 1000);
    let x = Extras {
        tokens: Some(tokens),
        ..Default::default()
    };
    show_u(&calm(true), &st, &u, &x, 1000, HubView::default(), 168, 49)
}

#[test]
fn calm_hub_all() {
    use ari_cli::app::App;
    let out = calm_all(vec![
        bare("api", App::Claude, Usage::default(), 1),
        bare("web", App::Claude, Usage::default(), 1),
    ]);
    for name in ["api", "web"] {
        let n = out
            .lines()
            .filter(|l| l.contains(&format!(" {name} ")))
            .count();
        assert_eq!(n, 1, "{name}:\n{out}");
    }
    insta::assert_snapshot!(out);
}

#[test]
fn calm_cards_clip_wide_names() {
    use ari_cli::app::App;
    use unicode_width::UnicodeWidthStr;
    let mut p = bare("aaaaaaaa", App::Claude, Usage::default(), 1);
    p.name = "日本語プロジェクト".repeat(5);
    let out = calm_all(vec![p]);
    for l in out.lines() {
        assert!(l.width() <= 168, "{l}");
    }
    let card: Vec<&str> = out.lines().filter(|l| l.starts_with('│')).collect();
    assert!(!card.is_empty(), "{out}");
    for l in card {
        assert!(l.trim_end().ends_with('│'), "border missing: {l}");
    }
}

#[test]
fn calm_card_shows_the_ask() {
    use ari_cli::app::App;
    let mut p = bare("aaaaaaaa", App::Claude, Usage::default(), 1);
    p.st.agents.push(ari_cli::state::Agent {
        id: "main".into(),
        waiting: Some((1, "run it".into())),
        ..Default::default()
    });
    let out = calm_all(vec![p]);
    let card = |l: &&str| l.starts_with('│') && l.contains("needs you: run it");
    assert!(out.lines().any(|l| card(&l)), "{out}");
}

#[test]
fn calm_hub_empty() {
    let out = show_u(
        &calm(false),
        &State::default(),
        &Usage::default(),
        &Extras::default(),
        1,
        HubView::default(),
        168,
        49,
    );
    assert!(out.contains("no agents yet"), "{out}");
}

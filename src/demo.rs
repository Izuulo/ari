//! `ari demo`: replay a bundled real session in the panel. No Claude, no state dir, no network.

use crate::bar::Extras;
use crate::event::Event;
use crate::panel::{self, View};
use crate::state::{State, Verdict};
use crate::tokens::{self, Ledger, TokenCache};
use crate::usage::{Limit, Usage};
use crate::{config, hub, replay, watch};
use ratatui::layout::Alignment;
use std::time::Instant;

const RAW: &str = include_str!("../fixtures/stuck/raw.jsonl");
const MAIN: &str = include_str!("../fixtures/stuck/transcript.jsonl");
const AGENTS: [(&str, &str); 2] = [
    (
        "ab57dcf04590c7026",
        include_str!("../fixtures/stuck/subagents/agent-ab57dcf04590c7026.jsonl"),
    ),
    (
        "abb5727de68e884cc",
        include_str!("../fixtures/stuck/subagents/agent-abb5727de68e884cc.jsonl"),
    ),
];
/// Shown instead of the fixture's long scripted prompt.
const LABEL: &str = "calculator fix";

pub fn events() -> Vec<Event> {
    replay::parse(RAW)
}

fn scaled(l: &Ledger, f: f64) -> Ledger {
    let s = |n: u64| (n as f64 * f) as u64;
    Ledger {
        input: s(l.input),
        cache_write: s(l.cache_write),
        cache_read: s(l.cache_read),
        output: s(l.output),
        requests: (l.requests as f64 * f).ceil() as u32,
        by_model: l.by_model.clone(),
        ..Default::default()
    }
}

/// State, usage and extras `progress_ms` into the bundled session, and the virtual clock.
/// Usage, tokens and rate limits are synthetic here: the fixture was recorded with `claude -p`,
/// which runs no statusline. The 5h window barely moves for a 40-second Haiku session, so there
/// is no pace warning to show.
pub fn frame_at(events: &[Event], progress_ms: u64) -> (State, Usage, Extras, u64) {
    let t0 = events.first().map_or(0, |e| e.t);
    let span = events.last().map_or(1, |e| e.t - t0).max(1);
    let progress = progress_ms.min(span);
    let now = t0 + progress;
    let f = progress as f64 / span as f64;

    let mut st = State::default();
    for ev in events.iter().take_while(|e| e.t <= now) {
        st.apply(ev);
    }
    st.label = Some(LABEL.into());

    let u = Usage {
        ctx_pct: Some(8.0 + 26.0 * f),
        ctx_size: Some(200_000),
        ctx_used: Some(((8.0 + 26.0 * f) * 2000.0) as u64),
        five_h: Some(Limit {
            pct: 22.0 + f,
            resets_at: (t0 + 2 * 3_600_000 + 10 * 60_000) / 1000,
        }),
        seven_d: Some(Limit {
            pct: 18.0,
            resets_at: (t0 + 4 * 86_400_000) / 1000,
        }),
        cost_usd: Some(0.12 * f),
        duration_ms: Some(progress),
        api_duration_ms: Some(progress * 3 / 4),
        ..Default::default()
    };
    let mut cache = TokenCache {
        main: scaled(&tokens::ledger_from(MAIN), f),
        ..Default::default()
    };
    for (id, text) in AGENTS {
        if let Some(a) = st.agents.iter().find(|a| a.id == id) {
            let share = if a.done { 1.0 } else { 0.5 };
            cache
                .agents
                .insert(id.into(), scaled(&tokens::ledger_from(text), share));
        }
    }
    let x = Extras {
        tokens: Some(cache),
        ..Default::default()
    };
    (st, u, x, now)
}

fn span(events: &[Event]) -> u64 {
    match (events.first(), events.last()) {
        (Some(a), Some(b)) => b.t - a.t,
        _ => 0,
    }
}

pub fn run(args: &[String]) -> ! {
    let mut cfg = config::load();
    let mut speed = 1.5_f64;
    let mut once = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--ascii" => cfg.ascii = true,
            "--once" => once = true,
            "--speed" => match it
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|s| *s > 0.0)
            {
                Some(s) => speed = s,
                None => fail("--speed needs a positive number"),
            },
            other => fail(&format!(
                "unknown option {other} (try --speed <x>, --ascii, --once)"
            )),
        }
    }
    cfg.bell = false;
    let events = events();
    let total = span(&events);
    if once || !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        let (st, u, x, now) = frame_at(&events, total);
        let width = std::env::var("COLUMNS")
            .ok()
            .and_then(|c| c.parse().ok())
            .unwrap_or(80);
        let lines = panel::lines(
            &st,
            &u,
            &x,
            &cfg,
            View::default(),
            now,
            &panel::local_hhmm(now),
            width,
            u16::MAX,
            false,
        );
        println!("{}", panel::to_plain(&lines));
        std::process::exit(0)
    }
    let mut progress = 0.0_f64;
    let mut last = Instant::now();
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let result = watch::run_tui(View::default(), |view, _tab, w, h| {
        // The demo replays with defaults; it has no settings screen.
        view.settings = None;
        let (st, u, x, now) = frame_at(&events, progress as u64);
        progress += last.elapsed().as_secs_f64()
            * 1000.0
            * pace(st.verdict_with(now, cfg.thresholds.stuck_repeats), speed);
        last = Instant::now();
        let progress = progress as u64;
        let clock = panel::local_hhmm(now);
        let animating = progress < total || st.turn_active;
        if let Some(hv) = view.hub.filter(|_| hub::fit(w, h) != hub::Fit::TooSmall) {
            let i = hub::Input {
                st: &st,
                u: &u,
                x: &x,
                cfg: &cfg,
                now,
                clock: &clock,
            };
            let lines = hub::lines(&i, &hv, w, h, color);
            let lines = if cfg.ascii {
                crate::ascii::map_lines(lines)
            } else {
                lines
            };
            return (
                lines,
                animating,
                Alignment::Left,
                None,
                Some(st.agents.len().max(1)),
            );
        }
        let lines = panel::lines(&st, &u, &x, &cfg, *view, now, &clock, w, h, color);
        (lines, animating, Alignment::Left, None, None)
    });
    if let Err(e) = result {
        fail(&e.to_string());
    }
    std::process::exit(0)
}

/// Replay speed for a frame showing `v`: trouble plays at most at real time, STUCK slower
/// still, so a viewer can read what went wrong before the replay moves on.
fn pace(v: Verdict, speed: f64) -> f64 {
    match v {
        Verdict::Stuck => speed.min(0.4),
        Verdict::Retrying | Verdict::NeedsYou => speed.min(1.0),
        _ => speed,
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("ari demo: {msg}");
    std::process::exit(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dwells_on_trouble() {
        assert_eq!(pace(Verdict::OnTrack, 2.5), 2.5);
        assert_eq!(pace(Verdict::Retrying, 2.5), 1.0);
        assert_eq!(pace(Verdict::Stuck, 2.5), 0.4);
        assert_eq!(pace(Verdict::Stuck, 0.2), 0.2, "never faster than asked");
    }

    #[test]
    fn replays_the_story() {
        let evs = events();
        let (st, ..) = frame_at(&evs, 0);
        assert_eq!(st.verdict(evs[0].t), Verdict::Idle);
        let at = |p: fn(&Event) -> bool| evs.iter().filter(move |e| p(e)).map(|e| e.t - evs[0].t);
        let fails: Vec<u64> = at(|e| e.ok == Some(false)).collect();
        assert_eq!(fails.len(), 3, "the same test fails three times");
        let verdict = |ms| {
            let (st, _, _, now) = frame_at(&evs, ms);
            st.verdict(now)
        };
        assert_eq!(verdict(fails[0]), Verdict::Retrying);
        assert_eq!(verdict(fails[2]), Verdict::Stuck);
        let asked = at(|e| e.e == "PermissionRequest").next().unwrap();
        assert_eq!(verdict(asked + 1000), Verdict::NeedsYou);
        let mut passed = at(|e| e.ok == Some(true) && e.tool.as_deref() == Some("Bash"));
        assert_eq!(verdict(passed.next_back().unwrap()), Verdict::OnTrack);
        let (st, u, x, now) = frame_at(&evs, u64::MAX);
        assert_eq!(st.verdict(now), Verdict::Done);
        assert_eq!(st.tasks_done(), (4, 4));
        assert_eq!(st.agents.iter().filter(|a| a.done).count(), 2);
        assert!(u.cost_usd.unwrap() > 0.11 && x.tokens.unwrap().total().total() > 400_000);
        assert!(x.burn_eta.is_none(), "no pace warning for a short session");
    }

    #[test]
    fn tiny_terminal() {
        let evs = events();
        let (st, u, x, now) = frame_at(&evs, 20_000);
        for (w, h) in [(30, 10), (12, 4), (5, 2)] {
            let lines = panel::lines(
                &st,
                &u,
                &x,
                &config::Config::default(),
                View::default(),
                now,
                "12:00",
                w,
                h,
                false,
            );
            assert!(lines.iter().all(|l| l.width() <= w as usize));
        }
    }
}

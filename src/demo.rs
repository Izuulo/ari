//! `ari demo`: replay a bundled real session in the panel. No Claude, no state dir, no network.

use crate::bar::Extras;
use crate::burn::{self, Sample};
use crate::event::Event;
use crate::panel::{self, View};
use crate::state::State;
use crate::tokens::{self, Ledger, TokenCache};
use crate::usage::{Limit, Usage};
use crate::{config, replay, watch};
use ratatui::layout::Alignment;
use std::time::Instant;

const RAW: &str = include_str!("../fixtures/multi/raw.jsonl");
const MAIN: &str = include_str!("../fixtures/multi/transcript.jsonl");
const AGENTS: [(&str, &str); 2] = [
    (
        "a3a6a01361cc3173c",
        include_str!("../fixtures/multi/subagents/agent-a3a6a01361cc3173c.jsonl"),
    ),
    (
        "a5114050eef69c100",
        include_str!("../fixtures/multi/subagents/agent-a5114050eef69c100.jsonl"),
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
/// which runs no statusline.
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

    let resets_5h = (t0 + 2 * 3_600_000 + 10 * 60_000) / 1000;
    // The 5h window has been filling at 1%/min since 10 minutes before the session.
    let five = |t: u64| 41.0 + (t as f64 - t0 as f64) / 60_000.0;
    let u = Usage {
        ctx_pct: Some(8.0 + 26.0 * f),
        ctx_size: Some(200_000),
        ctx_used: Some(((8.0 + 26.0 * f) * 2000.0) as u64),
        five_h: Some(Limit {
            pct: five(now),
            resets_at: resets_5h,
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
    let history = t0 - 10 * 60_000;
    let samples: Vec<Sample> = (0..=(now - history) / 30_000)
        .map(|i| history + i * 30_000)
        .chain(std::iter::once(now))
        .map(|t| Sample {
            t,
            pct: five(t),
            resets_at: resets_5h,
            ..Default::default()
        })
        .collect();
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
        burn_eta: burn::projection(&samples, now),
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
    let start = Instant::now();
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let result = watch::run_tui(|view, _tab, w, h| {
        let progress = (start.elapsed().as_millis() as f64 * speed) as u64;
        let (st, u, x, now) = frame_at(&events, progress);
        let lines = panel::lines(
            &st,
            &u,
            &x,
            &cfg,
            view,
            now,
            &panel::local_hhmm(now),
            w,
            h,
            color,
        );
        (
            lines,
            progress < total || st.turn_active,
            Alignment::Left,
            None,
        )
    });
    if let Err(e) = result {
        fail(&e.to_string());
    }
    std::process::exit(0)
}

fn fail(msg: &str) -> ! {
    eprintln!("ari demo: {msg}");
    std::process::exit(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Verdict;

    #[test]
    fn replays_the_story() {
        let evs = events();
        let (st, ..) = frame_at(&evs, 0);
        assert_eq!(st.verdict(evs[0].t), Verdict::Idle);
        let fail_at = evs.iter().find(|e| e.ok == Some(false)).unwrap().t - evs[0].t;
        let (st, _, _, now) = frame_at(&evs, fail_at);
        assert_eq!(st.verdict(now), Verdict::Retrying);
        let (st, u, x, now) = frame_at(&evs, u64::MAX);
        assert_eq!(st.verdict(now), Verdict::Done);
        assert_eq!(st.tasks_done(), (4, 4));
        assert_eq!(st.agents.iter().filter(|a| a.done).count(), 2);
        assert!(u.cost_usd.unwrap() > 0.11 && x.tokens.unwrap().total().total() > 400_000);
        let eta = x.burn_eta.expect("burn warning shows in the demo");
        assert!((55 * 60_000..62 * 60_000).contains(&eta), "{eta}");
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

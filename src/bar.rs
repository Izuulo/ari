//! The compact statusline row. Pure: state + usage + config + clock → one string.

use crate::config::Config;
use crate::state::{State, Verdict};
use crate::tokens::TokenCache;
use crate::usage::{Limit, Usage};

pub const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const DEFAULT_WIDTH: usize = 120;

/// What the transcripts and rate-limit history add on top of state + usage (M3).
#[derive(Clone, Debug, Default)]
pub struct Extras {
    pub tokens: Option<TokenCache>,
    /// ms until the 5h limit at the current pace, only when that's before it resets.
    pub burn_eta: Option<u64>,
}

/// `950`, `12.4k`, `142k`, `6.2M`.
pub fn fmt_tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..10_000 => format!("{:.1}k", n as f64 / 1e3),
        10_000..1_000_000 => format!("{}k", n / 1000),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

pub fn spinner(now_ms: u64) -> char {
    SPINNER[(now_ms / 100 % 10) as usize]
}

/// `n` cells, `frac` of them filled, rounded.
pub fn cells(frac: f64, n: usize, full: char, empty: char) -> String {
    let filled = ((frac.clamp(0.0, 1.0) * n as f64).round() as usize).min(n);
    std::iter::repeat_n(full, filled)
        .chain(std::iter::repeat_n(empty, n - filled))
        .collect()
}

/// `↺1h40m`, `↺4d`, `↺12m`.
pub fn countdown(resets_at_s: u64, now_ms: u64) -> String {
    let secs = resets_at_s.saturating_sub(now_ms / 1000);
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 {
        format!("↺{d}d")
    } else if h > 0 {
        format!("↺{h}h{m:02}m")
    } else {
        format!("↺{m}m")
    }
}

/// ANSI SGR code per verdict. NEEDS YOU flips to reverse video: the loudest state.
pub fn verdict_color(v: Verdict) -> &'static str {
    match v {
        Verdict::NeedsYou => "1;7;35",
        Verdict::Blocked => "1;31",
        Verdict::Stuck | Verdict::Retrying => "1;33",
        Verdict::OnTrack => "1;32",
        Verdict::Done => "1;36",
        Verdict::Idle => "2",
    }
}

pub fn level_color(pct: f64, cfg: &Config) -> &'static str {
    if pct >= cfg.thresholds.ctx_crit as f64 {
        "31"
    } else if pct >= cfg.thresholds.ctx_warn as f64 {
        "33"
    } else {
        "32"
    }
}

pub fn paint(color: bool, code: &str, s: &str) -> String {
    if color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn visible_width(s: &str) -> usize {
    let mut n = 0;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI: ESC [ params... final byte in @..~
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
        } else {
            n += 1;
        }
    }
    n
}

fn limit(name: &str, l: Option<Limit>, now_ms: u64, cfg: &Config, color: bool) -> String {
    match l {
        Some(l) => {
            let pct = paint(color, level_color(l.pct, cfg), &format!("{:.0}%", l.pct));
            if l.resets_at > 0 {
                format!("{name} {pct} {}", countdown(l.resets_at, now_ms))
            } else {
                format!("{name} {pct}")
            }
        }
        None => format!("{name} —"),
    }
}

pub fn render(
    st: &State,
    u: &Usage,
    x: &Extras,
    cfg: &Config,
    now_ms: u64,
    width: usize,
    color: bool,
) -> String {
    let v = st.verdict_with(now_ms, cfg.thresholds.stuck_repeats);
    let mut segs: Vec<String> = vec![paint(
        color,
        verdict_color(v),
        &format!("ARI {} {}", v.glyph(), v.label()),
    )];
    let shown = |id: &str| !cfg.hide.iter().any(|h| h == id);

    if shown("tasks") {
        let (done, total) = st.tasks_done();
        segs.push(if total == 0 {
            paint(color, "2", "no plan yet")
        } else {
            format!(
                "{} {done}/{total}",
                cells(done as f64 / total as f64, 10, '█', '░')
            )
        });
    }
    let agents = st.running_agents(now_ms);
    if shown("agents") && agents > 0 {
        let s = if agents == 1 { "" } else { "s" };
        segs.push(format!("{} {agents} agent{s}", spinner(now_ms)));
    }
    if shown("errors") {
        let n = st.open_errors();
        segs.push(paint(
            color,
            if n > 0 { "31" } else { "2" },
            &format!("✗ {n}"),
        ));
    }
    if shown("ctx") {
        segs.push(match u.ctx_pct {
            Some(p) => format!(
                "ctx {}",
                paint(
                    color,
                    level_color(p, cfg),
                    &format!("{} {:.0}%", cells(p / 100.0, 5, '▰', '▱'), p)
                )
            ),
            None => "ctx —".into(),
        });
    }
    if shown("5h") {
        segs.push(limit("5h", u.five_h, now_ms, cfg, color));
    }
    if shown("7d") {
        segs.push(limit("7d", u.seven_d, now_ms, cfg, color));
    }
    if shown("cost")
        && let Some(c) = u.cost_usd
    {
        segs.push(format!("${c:.2}"));
    }
    if shown("tokens")
        && let Some(total) = x
            .tokens
            .as_ref()
            .map(|t| t.total().total())
            .filter(|n| *n > 0)
    {
        segs.push(format!("{} tok", fmt_tokens(total)));
    }

    let sep = paint(color, "2", " │ ");
    let width = match (cfg.max_width, width) {
        (m, _) if m > 0 => m,
        (_, 0) => DEFAULT_WIDTH,
        (_, w) => w,
    };
    loop {
        let line = segs.join(&sep);
        if segs.len() == 1 || visible_width(&line) <= width {
            return if cfg.ascii {
                crate::ascii::map(&line)
            } else {
                line
            };
        }
        segs.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Agent, ErrorRec, Task};

    const NOW: u64 = 1_738_419_600_000; // 1h40m before the 5h reset below

    fn sample() -> (State, Usage) {
        let mut st = State {
            turn_active: true,
            turns: 1,
            ..Default::default()
        };
        st.agents.push(Agent {
            id: "main".into(),
            kind: "main".into(),
            last_t: NOW,
            ..Default::default()
        });
        for id in ["a", "b"] {
            st.agents.push(Agent {
                id: id.into(),
                kind: "Explore".into(),
                last_t: NOW,
                ..Default::default()
            });
        }
        for (i, s) in [
            "completed",
            "completed",
            "completed",
            "completed",
            "in_progress",
            "pending",
            "pending",
        ]
        .iter()
        .enumerate()
        {
            st.tasks.push(Task {
                id: i.to_string(),
                subject: format!("t{i}"),
                status: s.to_string(),
            });
        }
        st.errors.push(ErrorRec {
            sig: "Bash:cargo build".into(),
            turn: 1,
            ..Default::default()
        });
        let u = Usage {
            ctx_pct: Some(71.0),
            five_h: Some(Limit {
                pct: 62.0,
                resets_at: NOW / 1000 + 6000,
            }),
            seven_d: Some(Limit {
                pct: 34.0,
                resets_at: NOW / 1000 + 4 * 86_400 + 60,
            }),
            cost_usd: Some(2.69),
            ..Default::default()
        };
        (st, u)
    }

    #[test]
    fn full_row() {
        let (st, u) = sample();
        assert_eq!(
            render(
                &st,
                &u,
                &Extras::default(),
                &Config::default(),
                NOW,
                200,
                false
            ),
            "ARI ● ON TRACK │ ██████░░░░ 4/7 │ ⠋ 2 agents │ ✗ 1 │ ctx ▰▰▰▰▱ 71% │ 5h 62% ↺1h40m │ 7d 34% ↺4d │ $2.69"
        );
    }

    #[test]
    fn empty_session() {
        let row = render(
            &State::default(),
            &Usage::default(),
            &Extras::default(),
            &Config::default(),
            NOW,
            200,
            false,
        );
        assert_eq!(row, "ARI ◌ IDLE │ no plan yet │ ✗ 0 │ ctx — │ 5h — │ 7d —");
    }

    #[test]
    fn drops_from_the_end_to_fit() {
        let (st, u) = sample();
        let cfg = Config::default();
        for w in [10, 30, 50, 70] {
            let row = render(&st, &u, &Extras::default(), &cfg, NOW, w, false);
            assert!(row.starts_with("ARI ● ON TRACK"));
            assert!(
                visible_width(&row) <= w || row == "ARI ● ON TRACK",
                "{w}: {row}"
            );
        }
        assert_eq!(
            render(&st, &u, &Extras::default(), &cfg, NOW, 31, false),
            "ARI ● ON TRACK │ ██████░░░░ 4/7"
        );
        let capped = Config {
            max_width: 31,
            ..Config::default()
        };
        assert_eq!(
            render(&st, &u, &Extras::default(), &capped, NOW, 500, false),
            "ARI ● ON TRACK │ ██████░░░░ 4/7"
        );
    }

    #[test]
    fn hide_segments() {
        let (st, u) = sample();
        let cfg = Config {
            hide: vec!["cost".into(), "7d".into(), "agents".into()],
            ..Config::default()
        };
        let row = render(&st, &u, &Extras::default(), &cfg, NOW, 200, false);
        assert!(
            !row.contains('$') && !row.contains("7d") && !row.contains("agents"),
            "{row}"
        );
    }

    #[test]
    fn color_does_not_change_width() {
        let (st, u) = sample();
        let plain = render(
            &st,
            &u,
            &Extras::default(),
            &Config::default(),
            NOW,
            200,
            false,
        );
        let colored = render(
            &st,
            &u,
            &Extras::default(),
            &Config::default(),
            NOW,
            200,
            true,
        );
        assert!(colored.contains("\x1b["));
        assert_eq!(visible_width(&colored), plain.chars().count());
    }

    #[test]
    fn ascii_bar() {
        let (st, u) = sample();
        let cfg = Config {
            ascii: true,
            ..Config::default()
        };
        let row = render(&st, &u, &Extras::default(), &cfg, NOW, 200, false);
        assert!(row.is_ascii(), "{row}");
        assert!(
            row.starts_with("ARI * ON TRACK | ######---- 4/7 | | 2 agents"),
            "{row}"
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(cells(0.71, 5, '▰', '▱'), "▰▰▰▰▱");
        assert_eq!(cells(1.5, 3, '#', '-'), "###");
        assert_eq!(countdown(100, 200_000), "↺0m");
        assert_eq!(countdown(NOW / 1000 + 720, NOW), "↺12m");
    }
}

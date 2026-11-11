//! Calm watch: today's panel layout in plain English, without the numbers.

use crate::bar::cells;
use crate::hub::{Input, bold, clip, ctx_of, dim, faint, frame, ok, put, to_lines};
use crate::panel::{shown_agents, verdict_style};
use crate::simple::{banner, draw_banner, glyph, say};
use crate::state::{Agent, AgentStatus};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

type Row = Vec<(String, Style)>;

/// `next`: the view `v` goes to, named in the footer.
pub fn panel(i: &Input, w: u16, h: u16, color: bool, next: &str) -> Vec<Line<'static>> {
    let (st, now) = (i.st, i.now);
    let v = st.verdict_with(i.cfg.thresholds.stuck_repeats);
    let verdict = format!("{} {}", v.glyph(), v.label().to_lowercase());
    if w < 12 || h < 4 {
        let s = Span::styled(clip(&verdict, w as usize), verdict_style(v));
        return vec![Line::from(s)];
    }
    let bar_w = 18.min((w as usize).saturating_sub(18));
    let alert = banner(st, now);
    let mut rows: Vec<Row> = Vec::new();
    let mut head = vec![(verdict, verdict_style(v))];
    if let Some(l) = &st.label {
        head.push((format!(" · {l}"), dim()));
    }
    rows.push(head);
    let (done, total) = st.tasks_done();
    let filled = (done as f64 / total.max(1) as f64 * bar_w as f64).round() as u64;
    if total > 0 {
        let bar = cells(done as f64 / total as f64, bar_w, '▰', '▱');
        rows.push(vec![
            (bar, Style::new().fg(Color::Green)),
            (format!(" {done} of {total} done"), dim()),
        ]);
    }
    rows.push(Vec::new());
    if let Some(main) = st.agents.first() {
        let list: Vec<&Agent> = std::iter::once(main)
            .chain(shown_agents(st, false, now))
            .collect();
        let name_w = list
            .iter()
            .map(|a| a.label().width())
            .max()
            .unwrap_or(4)
            .min(12);
        for a in list {
            let status = st.agent_status(a, now);
            let stale = status == AgentStatus::Stale;
            let (g, gs) = glyph(i, a);
            let name = clip(&a.label(), name_w);
            rows.push(vec![
                (format!("{g} "), gs),
                (
                    format!("{name:<name_w$} "),
                    if stale { faint() } else { bold() },
                ),
                (
                    say(st, a, now),
                    if stale {
                        faint()
                    } else if a.done {
                        ok()
                    } else {
                        gs
                    },
                ),
            ]);
        }
    } else {
        rows.push(vec![("no agents yet".into(), dim())]);
    }
    if let Some((_, pct)) = st.agents.first().and_then(|m| ctx_of(i, m)) {
        rows.push(Vec::new());
        rows.push(vec![
            ("memory ".into(), dim()),
            (
                cells(pct / 100.0, 14, '█', '░'),
                Style::new().fg(Color::Blue),
            ),
            (format!(" {pct:.0}% full"), dim()),
        ]);
    }
    rows.push(Vec::new());
    rows.push(vec![(format!("v {next}"), faint())]);

    let top = u16::from(alert.is_some());
    let hh = h.min(rows.len() as u16 + top + 2);
    let mut buf = Buffer::empty(Rect::new(0, 0, w, hh));
    let title = vec![Span::styled("ARI", bold())];
    // The folder this panel follows, so you can tell sessions apart at a glance.
    let room = (w as usize).saturating_sub(14);
    let folder = match &i.u.dir {
        Some(d) if room >= 4 => vec![Span::styled(clip(&crate::panel::tilde(d), room), bold())],
        _ => vec![],
    };
    let area = buf.area;
    let inner = frame(&mut buf, area, title, folder, faint(), false);
    let inner = Rect::new(
        inner.x + 2,
        inner.y,
        inner.width.saturating_sub(4),
        inner.height,
    );
    if let Some(b) = &alert {
        draw_banner(&mut buf, inner, b);
    }
    for (y, row) in rows.iter().enumerate() {
        let y = inner.y + top + y as u16;
        if y >= inner.bottom() {
            break;
        }
        let mut x = inner.x;
        for (text, style) in row {
            x = put(
                &mut buf,
                x,
                y,
                text,
                inner.right().saturating_sub(x),
                *style,
            );
        }
    }
    // The shimmer: one bright cell sweeping the filled part of the bar, every 140 ms.
    let (s, y) = ((now / 140) % 28, inner.y + top + 1);
    if total > 0 && s < filled && y < inner.bottom() {
        let st = bold().fg(Color::White);
        put(&mut buf, inner.x + s as u16, y, "▰", 1, st);
    }
    to_lines(&buf, color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::hub::tests::{kid, with_main};
    use crate::panel::tests::{NOW, multi};
    use crate::panel::to_plain;
    use crate::state::State;
    use crate::usage::Usage;

    fn draw(st: &State, u: &Usage, now: u64, w: u16, h: u16) -> Vec<Line<'static>> {
        let (cfg, x) = (Config::default(), crate::bar::Extras::default());
        let i = Input {
            st,
            u,
            x: &x,
            cfg: &cfg,
            now,
            clock: "12:41",
        };
        panel(&i, w, h, false, "constellation")
    }

    #[test]
    fn calm_panel() {
        let (st, u) = multi();
        insta::assert_snapshot!(to_plain(&draw(&st, &u, NOW, 60, 40)));
    }

    #[test]
    fn calm_panel_hides_numbers() {
        let (st, mut u) = multi();
        u.dir = Some("/x/proj".into());
        let t = to_plain(&draw(&st, &u, NOW, 60, 40));
        assert!(t.lines().next().unwrap().contains(" /x/proj ─╮"), "{t}");
        // Past the top border, which names the folder.
        let t = t.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(!t.contains(['$', '/']) && !t.contains("tok"), "{t}");
        assert!(t.contains("on track") && t.contains("4 of 7 done"), "{t}");
        assert!(t.contains("z Plan") && t.contains("v constellation"), "{t}");
    }

    #[test]
    fn calm_panel_banner() {
        let mut a = kid("a", "main", 0);
        a.waiting = Some((1, "run it".into()));
        let st = with_main(vec![a]);
        let t = to_plain(&draw(&st, &Usage::default(), 0, 60, 20));
        assert!(t.lines().nth(1).unwrap().starts_with("│  ! "), "{t}");
    }

    #[test]
    fn calm_panel_tiny() {
        let (st, mut u) = multi();
        u.dir = Some("/a/really/long/folder/name/for/a/project".into());
        for (w, h) in [(30, 10), (12, 4), (5, 2)] {
            for l in draw(&st, &u, NOW, w, h) {
                assert!(l.width() <= w as usize, "{w}x{h}");
            }
        }
    }

    #[test]
    fn calm_panel_empty() {
        let t = to_plain(&draw(&State::default(), &Usage::default(), NOW, 40, 20));
        assert!(t.contains("no agents yet"), "{t}");
    }
}

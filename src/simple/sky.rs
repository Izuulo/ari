//! Constellation: each session as an animated star system.

use crate::hub::{
    HubView, Input, areas, bold, chrome, clip, dim, err, faint, fit, frame, lead, msg, needs_you,
    node_style, put, rput, to_lines, top_of,
};
use crate::simple::{banner, draw_banner, glyph, phrase, say};
use crate::state::{Agent, State};
use ratatui::{buffer::Buffer, layout::Rect, style::Style, text::Line, text::Span};
use std::cmp::Reverse;
use std::f64::consts::TAU;
use unicode_width::UnicodeWidthStr;

/// One orbit, in ms.
const TURN: f64 = 20_000.0;
/// Bodies drawn around one sun.
const MAX: usize = 8;
/// A message's flight, in ms.
const COMET: u64 = 3000;
/// A name's widest, in columns.
const NAME: usize = 16;

/// Room enough to draw a sky.
pub fn fits(w: u16, h: u16) -> bool {
    w >= 30 && h >= 8
}

/// `ari watch` in Constellation: the session's sky in the panel's frame; Calm when too small.
pub fn panel(i: &Input, w: u16, h: u16, color: bool) -> Vec<Line<'static>> {
    if !fits(w, h) {
        return crate::simple::calm::panel(i, w, h, color, "details");
    }
    let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
    let title = vec![Span::styled("ARI", bold())];
    let clock = vec![Span::styled(i.clock.to_string(), dim())];
    let inner = frame(&mut buf, Rect::new(0, 0, w, h), title, clock, dim(), false);
    system(&mut buf, inner, i, false);
    to_lines(&buf, color)
}

/// The hub in Constellation: bare chrome over one system, or the galaxy for `--all`.
pub fn hub(buf: &mut Buffer, i: &Input, v: &HubView) {
    let (w, h) = (buf.area.width, buf.area.height);
    let all = if v.all {
        "this session"
    } else {
        "all sessions"
    };
    let mut keys = vec![];
    // `ari hub --all` is merged already: nothing for `a` to switch to.
    if v.all || !crate::hub::merged(i) {
        keys.push(("a", all.to_string()));
    }
    for (k, l) in [
        ("x", "back to panel"),
        ("v", "details"),
        ("s", "settings"),
        ("q", "quit"),
    ] {
        keys.push((k, l.into()));
    }
    chrome(buf, i, &areas(w, h, fit(w, h)), &keys, true);
    let body = Rect::new(0, 2, w, h.saturating_sub(3));
    if crate::hub::merged(i) {
        galaxy(buf, body, i);
    } else {
        system(buf, body, i, true);
    }
}

/// One session: main at the centre, its helpers orbiting, messages as comets. `big` adds verbs.
pub fn system(buf: &mut Buffer, area: Rect, i: &Input, big: bool) {
    let st = i.st;
    stars(buf, area, i.now);
    if st.agents.is_empty() {
        return empty(buf, area, i);
    }
    let cx = area.x + area.width / 2;
    let cy = area.y + area.height.saturating_sub(1) / 2;
    let rx = (area.width / 2).saturating_sub(6);
    // Room under the lowest name for its verb.
    let ry = (area.height / 2).saturating_sub(2 + big as u16);
    ring(buf, area, cx, cy, rx, ry);

    let mut at = vec![];
    if let Some(m) = st.agents.iter().position(|a| a.id == "main") {
        let a = &st.agents[m];
        let (g, gs) = body(i, a, alarm(st, m));
        let ms = if alarm(st, m) { err() } else { lead() };
        put(buf, cx.saturating_sub(4), cy, "( main )", 8, ms);
        put(buf, cx, cy.saturating_sub(1), &g, 1, gs);
        at.push((m, (cx, cy)));
    }
    let all: Vec<usize> = (0..st.agents.len()).filter(|&k| suns(st, k)).collect();
    let total = all.len();
    let mut helpers = all.clone();
    helpers.sort_by_key(|&k| Reverse(st.agents[k].last_t));
    helpers.truncate(MAX);
    // Slots by start, so a helper keeps its place as others get busy.
    helpers.sort_by_key(|&k| st.agents[k].started);
    let turn = i.now as f64 / TURN * TAU;
    let n = helpers.len() as f64;
    for (k, &h) in helpers.iter().enumerate() {
        let (x, y) = orbit(cx, cy, rx, ry, turn + k as f64 * TAU / n);
        let a = &st.agents[h];
        let (g, gs) = body(i, a, alarm(st, h));
        let text = format!("{g} {}", clip(&a.label(), NAME));
        // Keep the label inside the pane at the right edge.
        let lx = x
            .min(area.right().saturating_sub(text.width() as u16))
            .max(area.x);
        put(buf, lx, y, &text, area.right() - lx, gs);
        if big {
            let vx = (lx + 2).min(area.right());
            put(
                buf,
                vx,
                y + 1,
                &say(st, a, i.now),
                area.right() - vx,
                faint(),
            );
        }
        at.push((h, (x, y)));
    }
    let caption = comets(buf, area, i, &at);

    let y = area.bottom() - 1;
    blank(
        buf,
        Rect {
            y,
            height: 1,
            ..area
        },
    );
    let out = all.iter().filter(|&&k| !st.agents[k].done).count();
    let mut right = match out {
        0 if total > 0 => "all helpers back".to_string(),
        0 => "no helpers yet".into(),
        1 => "1 helper out".into(),
        n => format!("{n} helpers out"),
    };
    if total > MAX {
        right += &format!(" · +{} more", total - MAX);
    }
    let right = clip(&right, area.width.saturating_sub(2) as usize);
    let rx0 = rput(buf, area.right().saturating_sub(2), y, &right, faint());
    let room = rx0.saturating_sub(area.x + 3);
    let mut end = area.x + 1;
    if let Some(m) = st.agents.iter().find(|a| a.id == "main") {
        let left = format!("main is {}", phrase(st, m, i.now));
        end = put(buf, area.x + 1, y, &left, room, Style::default());
    }
    if let Some(c) = caption.filter(|_| big) {
        let x = (end + 3).max(cx.saturating_sub(c.width() as u16 / 2));
        put(buf, x, y, &c, rx0.saturating_sub(x + 2), msg());
    }
    alert(buf, area, i);
}

/// `hub --all`: each session a small system on a grid, its sun in its verdict's colour.
pub fn galaxy(buf: &mut Buffer, area: Rect, i: &Input) {
    let st = i.st;
    stars(buf, area, i.now);
    let leads: Vec<usize> = (0..st.agents.len()).filter(|&k| suns(st, k)).collect();
    if leads.is_empty() {
        return empty(buf, area, i);
    }
    let cols = (leads.len() as f64).sqrt().ceil() as usize;
    let rows = leads.len().div_ceil(cols);
    let (cw, ch) = (area.width / cols as u16, area.height / rows as u16);
    let turn = i.now as f64 / TURN * TAU;
    let blink = (i.now / 500).is_multiple_of(2);
    let mut at = vec![];
    for (s, &l) in leads.iter().enumerate() {
        let cx = area.x + (s % cols) as u16 * cw + cw / 2;
        let cy = area.y + (s / cols) as u16 * ch + (ch / 2).saturating_sub(1);
        ring(buf, area, cx, cy, 6, 2);
        let kids: Vec<usize> = (0..st.agents.len())
            .filter(|&k| k != l && top_of(st, k) == l)
            .take(MAX)
            .collect();
        let speed = 1.0 + 0.25 * s as f64;
        for (j, &k) in kids.iter().enumerate() {
            let a = &st.agents[k];
            let a0 = turn * speed + j as f64 * TAU / kids.len() as f64;
            let (x, y) = orbit(cx, cy, 6, 2, a0);
            let (g, gs) = match needs_you(a) {
                true => body(i, a, true),
                false => ("●".into(), node_style(i, a)),
            };
            dot(buf, area, x, y, &g, gs);
        }
        let lead_a = &st.agents[l];
        let alarm = alarm(st, l);
        let (sun, ss) = match alarm {
            true => (if blink { "!" } else { " " }, err()),
            false => ("✺", node_style(i, lead_a)),
        };
        dot(buf, area, cx, cy, sun, ss);
        let name = clip(&lead_a.label(), cw.saturating_sub(2) as usize);
        let nx = cx.saturating_sub(name.width() as u16 / 2).max(area.x);
        if cy + 3 < area.bottom() {
            let ns = if alarm { err() } else { bold() };
            put(buf, nx, cy + 3, &name, area.right() - nx, ns);
        }
        at.push((l, (cx, cy)));
    }
    comets(buf, area, i, &at);
    alert(buf, area, i);
}

/// Whether agent `k` hangs off main: a helper, or (`--all`) a session's lead.
fn suns(st: &State, k: usize) -> bool {
    let a = &st.agents[k];
    a.id != "main" && a.parent.as_deref().unwrap_or("main") == "main"
}

fn empty(buf: &mut Buffer, area: Rect, i: &Input) {
    let t = "no agents yet";
    let x = (area.x + area.width / 2).saturating_sub(t.width() as u16 / 2);
    put(
        buf,
        x,
        area.y + area.height / 2,
        t,
        area.right().saturating_sub(x),
        dim(),
    );
    alert(buf, area, i);
}

/// Whether agent `k`, or any agent under it, needs you. Uncapped, so a hidden one still counts.
fn alarm(st: &State, k: usize) -> bool {
    (0..st.agents.len()).any(|j| top_of(st, j) == k && needs_you(&st.agents[j]))
}

/// The glyph a body shows: red `◉`/`○` pulsing while it (`alarm`) needs you.
fn body(i: &Input, a: &Agent, alarm: bool) -> (String, Style) {
    if alarm {
        let g = if (i.now / 500).is_multiple_of(2) {
            "◉"
        } else {
            "○"
        };
        return (g.into(), err());
    }
    glyph(i, a)
}

/// The banner over the top row while something needs you.
fn alert(buf: &mut Buffer, area: Rect, i: &Input) {
    if let Some(b) = banner(i.st, i.now) {
        let row = Rect { height: 1, ..area };
        blank(buf, row);
        draw_banner(buf, row, &b);
    }
}

/// Clear a row of stars and dots so its words stand alone.
fn blank(buf: &mut Buffer, row: Rect) {
    buf.set_string(
        row.x,
        row.y,
        " ".repeat(row.width as usize),
        Style::default(),
    );
}

/// Messages under `COMET` ms old as a `✦` with a `·` trail from sender to receiver, each at
/// its body in `at` (or its ancestor's there). Returns the newest's caption.
fn comets(buf: &mut Buffer, area: Rect, i: &Input, at: &[(usize, (u16, u16))]) -> Option<String> {
    let st = i.st;
    let find = |s: &str| {
        let k = st
            .agents
            .iter()
            .position(|a| a.id == s || a.name.as_deref() == Some(s))?;
        let p = at.iter().find(|p| p.0 == top_of(st, k))?.1;
        Some((k, p))
    };
    let mut caption = None;
    for m in &st.messages {
        if m.t > i.now || i.now - m.t >= COMET {
            continue;
        }
        let (Some((fk, a)), Some((tk, b))) = (find(&m.from), find(&m.to)) else {
            continue;
        };
        // Within one body (a sun's own session): nothing to fly, and it'd hide the body.
        if a == b {
            continue;
        }
        let f = (i.now - m.t) as f64 / COMET as f64;
        for k in (0..=3).rev() {
            let ff = (f - k as f64 * 0.04).max(0.0);
            let x = a.0 as f64 + (b.0 as f64 - a.0 as f64) * ff;
            let y = a.1 as f64 + (b.1 as f64 - a.1 as f64) * ff;
            let (g, s) = if k == 0 {
                ("✦", bold())
            } else {
                ("·", msg())
            };
            dot(buf, area, x.round() as u16, y.round() as u16, g, s);
        }
        let (from, to) = (st.agents[fk].label(), st.agents[tk].label());
        caption = Some(crate::simple::caption(&from, &to, &m.text));
    }
    caption
}

/// One cell's glyph, if it falls inside `area`.
fn dot(buf: &mut Buffer, area: Rect, x: u16, y: u16, g: &str, s: Style) {
    if x >= area.x && x < area.right() && y >= area.y && y < area.bottom() {
        put(buf, x, y, g, 1, s);
    }
}

/// A dotted ellipse on cells still blank.
fn ring(buf: &mut Buffer, area: Rect, cx: u16, cy: u16, rx: u16, ry: u16) {
    let n = ((rx + ry) * 4).max(24);
    for k in 0..n {
        let (x, y) = orbit(cx, cy, rx, ry, k as f64 * TAU / n as f64);
        if area.contains((x, y).into()) && buf[(x, y)].symbol() == " " {
            put(buf, x, y, "·", 1, faint());
        }
    }
}

/// The point at angle `a` on the ellipse round (cx, cy).
fn orbit(cx: u16, cy: u16, rx: u16, ry: u16, a: f64) -> (u16, u16) {
    let x = cx as f64 + rx as f64 * a.cos();
    let y = cy as f64 + ry as f64 * a.sin();
    (x.round().max(0.0) as u16, y.round().max(0.0) as u16)
}

/// A faint twinkling field on the blank cells.
fn stars(buf: &mut Buffer, area: Rect, now: u64) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = star(x, y, now)
                && buf[(x, y)].symbol() == " "
            {
                buf[(x, y)].set_char(c).set_style(faint());
            }
        }
    }
}

/// About one cell in 30 holds a star, showing `*` 1/9 and `·` 4/9 of the time.
fn star(x: u16, y: u16, now: u64) -> Option<char> {
    // A murmur3 finaliser: a plain multiply spreads neighbours too evenly and draws a lattice.
    let mut h = x as u64 * 37 + y as u64 * 17;
    h = (h ^ (h >> 33)).wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h = (h ^ (h >> 33)).wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    if !h.is_multiple_of(30) {
        return None;
    }
    match (now / 400 + h / 30) % 9 {
        0 => Some('*'),
        1..=4 => Some('·'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::tests::{kid, with_main};
    use crate::state::{Msg, State};
    use unicode_width::UnicodeWidthStr;

    fn draw(st: &State, now: u64, w: u16, h: u16) -> Vec<String> {
        let (u, x, cfg) = Default::default();
        let i = Input {
            st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now,
            clock: "12:41",
        };
        crate::panel::to_plain(&panel(&i, w, h, false))
            .lines()
            .map(String::from)
            .collect()
    }
    fn two() -> State {
        with_main(vec![kid("scout", "main", 1), kid("tests", "main", 2)])
    }

    #[test]
    fn sky_panel_moves() {
        let st = two();
        assert_ne!(draw(&st, 0, 46, 14), draw(&st, 5000, 46, 14));
        assert_eq!(draw(&st, 0, 46, 14), draw(&st, 0, 46, 14));
        assert!(draw(&st, 0, 46, 14)[0].starts_with("╭─ ARI"));
    }

    #[test]
    fn sky_panel_tiny() {
        let st = two();
        let (u, x, cfg) = Default::default();
        let i = Input {
            st: &st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now: 0,
            clock: "12:41",
        };
        // Under 30×8 the panel is Calm's, and its width is Calm's to keep.
        for (w, h) in [(30, 10), (29, 10), (12, 4), (5, 2)] {
            let calm = crate::simple::calm::panel(&i, w, h, false, "details");
            assert_eq!(panel(&i, w, h, false) == calm, !fits(w, h), "{w}×{h}");
        }
        for l in draw(&st, 0, 30, 10) {
            assert!(l.width() <= 30, "{l}");
        }
    }

    #[test]
    fn sky_too_small_footer_says_details() {
        let out = draw(&two(), 0, 29, 20).join("\n");
        assert!(
            out.contains("v details") && !out.contains("constellation"),
            "{out}"
        );
    }

    #[test]
    fn sky_crowd() {
        let kids = (0..20)
            .map(|n| {
                let mut a = kid(&format!("kid{n:02}"), "main", n);
                a.last_t = n;
                a.running_since = Some(n);
                a
            })
            .collect();
        let text = draw(&with_main(kids), 0, 100, 30).join("\n");
        let shown = (0..20)
            .filter(|n| text.contains(&format!("kid{n:02}")))
            .count();
        assert!((1..=8).contains(&shown), "{text}");
        assert!(text.contains("+12 more"), "{text}");
        assert!(text.contains("kid19"), "the newest are shown: {text}");
    }

    #[test]
    fn sky_comet() {
        let mut st = two();
        st.messages.push_back(Msg {
            t: 10_000,
            from: "main".into(),
            to: "scout".into(),
            text: "hi".into(),
        });
        assert!(draw(&st, 11_500, 60, 20).join("\n").contains('✦'));
        assert!(!draw(&st, 13_500, 60, 20).join("\n").contains('✦'));
    }

    #[test]
    fn sky_needs_you() {
        let mut a = kid("scout", "main", 0);
        a.waiting = Some((1, "Bash".into()));
        let out = draw(&with_main(vec![a]), 0, 60, 20);
        assert!(out[1].starts_with("│! "), "{}", out[1]);
        assert!(out.join("\n").contains('◉'));
    }

    #[test]
    fn sky_empty() {
        let out = draw(&State::default(), 0, 46, 14).join("\n");
        assert!(out.contains("no agents yet"), "{out}");
    }

    #[test]
    fn sky_wide_names() {
        let mut a = kid("wide", "main", 0);
        a.name = Some("日本語".repeat(20));
        let st = with_main(vec![a]);
        let (u, x) = Default::default();
        let mut cfg = crate::config::Config::default();
        cfg.view.hub = crate::config::Look::Constellation;
        let i = Input {
            st: &st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now: 0,
            clock: "",
        };
        let v = HubView::default();
        let out = crate::panel::to_plain(&crate::hub::lines(&i, &v, 168, 49, false));
        assert!(out.contains("日本語") && out.contains('…'), "{out}");
        assert!(out.lines().all(|l| l.width() <= 168));
    }

    #[test]
    fn sky_hub_a_hint() {
        let st = two();
        let (x, mut cfg) = (Default::default(), crate::config::Config::default());
        cfg.view.hub = crate::config::Look::Constellation;
        cfg.view.hub_all = crate::config::Look::Constellation;
        let all = crate::usage::Usage {
            project: Some("all sessions".into()),
            ..Default::default()
        };
        let keys = |u: &crate::usage::Usage, all: bool| {
            let i = Input {
                st: &st,
                u,
                x: &x,
                cfg: &cfg,
                now: 0,
                clock: "",
            };
            let v = HubView {
                all,
                ..Default::default()
            };
            let out = crate::panel::to_plain(&crate::hub::lines(&i, &v, 168, 49, false));
            out.lines().last().unwrap().to_string()
        };
        assert!(keys(&Default::default(), false).contains(" a all sessions"));
        assert!(keys(&all, true).contains(" a this session"));
        let fixed = keys(&all, false);
        assert!(
            !fixed.contains(" a ") && fixed.contains(" v details"),
            "{fixed}"
        );
    }

    #[test]
    fn star_is_deterministic_and_sparse() {
        let lit = (0..120u16)
            .flat_map(|x| (0..40u16).map(move |y| (x, y)))
            .filter(|&(x, y)| star(x, y, 0).is_some())
            .count();
        // About one cell in 30, lit 5/9 of the time: ~89 of 4800.
        assert!((50..=140).contains(&lit), "{lit}");
        assert_eq!(star(3, 4, 800), star(3, 4, 800));
    }

    /// `galaxy` alone over a 40×12 area.
    fn sky_all(st: &State, now: u64) -> Vec<String> {
        let (u, x, cfg) = Default::default();
        let i = Input {
            st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now,
            clock: "",
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        galaxy(&mut buf, area, &i);
        crate::panel::to_plain(&to_lines(&buf, false))
            .lines()
            .map(String::from)
            .collect()
    }
    fn session(kids: usize) -> State {
        let mut st = with_main(vec![kid("api", "main", 0)]);
        st.agents
            .extend((0..kids).map(|n| kid(&format!("api/{n}"), "api", n as u64 + 1)));
        st
    }

    #[test]
    fn sky_comet_within_one_body_is_not_drawn() {
        let mut st = session(1);
        st.messages.push_back(Msg {
            t: 10_000,
            from: "api/0".into(),
            to: "api".into(),
            text: "hi".into(),
        });
        let out = sky_all(&st, 11_500).join("\n");
        assert!(out.contains('✺') && !out.contains('✦'), "{out}");
        let mut st = with_main(vec![kid("scout", "main", 1), kid("deep", "scout", 2)]);
        st.messages.push_back(Msg {
            t: 10_000,
            from: "deep".into(),
            to: "scout".into(),
            text: "hi".into(),
        });
        assert!(!draw(&st, 11_500, 60, 20).join("\n").contains('✦'));
    }

    #[test]
    fn sky_sun_blinks_for_any_descendant() {
        let mut st = session(9);
        st.agents[10].waiting = Some((1, "Bash".into()));
        let out = sky_all(&st, 0);
        // Row 0 is the banner; the sun's `!` is below it.
        assert!(
            out[1..].iter().any(|l| l.contains('!')),
            "{}",
            out.join("\n")
        );
    }

    #[test]
    fn sky_helper_pulses_for_its_own_helpers() {
        let mut deep = kid("deep", "scout", 2);
        deep.waiting = Some((1, "Bash".into()));
        let st = with_main(vec![kid("scout", "main", 1), deep]);
        let out = draw(&st, 0, 60, 20).join("\n");
        assert!(out.contains("◉ scout"), "{out}");
    }

    #[test]
    fn sky_counts_helpers_out() {
        let mut st = two();
        st.agents[1].done = true;
        let last = |st: &State| draw(st, 0, 60, 20)[18].clone();
        let alone = with_main(vec![]);
        assert!(last(&alone).contains("no helpers yet"), "{}", last(&alone));
        assert!(last(&st).contains("1 helper out"), "{}", last(&st));
        st.agents[2].done = true;
        assert!(last(&st).contains("all helpers back"), "{}", last(&st));
        // Nothing but the words on the bottom row.
        let row = last(&st);
        let words = row
            .trim_matches('│')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(words, "main is waiting for you all helpers back");
    }
}

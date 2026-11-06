//! The hub: `ari watch`'s full-terminal mode (spec: docs/superpowers/specs/2026-11-05-hub-design.md).
//! Panes draw into a ratatui `Buffer`; `to_lines` turns it into the `Vec<Line>` watch draws.

mod agents;
pub mod all;
mod feed;
mod graph;
mod zoom;

use crate::bar::{self, Extras};
use crate::config::Config;
use crate::state::{Agent, State};
use crate::tokens::Ledger;
use crate::usage::Usage;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget};
use unicode_width::UnicodeWidthStr;

pub struct Input<'a> {
    pub st: &'a State,
    pub u: &'a Usage,
    pub x: &'a Extras,
    pub cfg: &'a Config,
    pub now: u64,
    pub clock: &'a str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Hub,
    Graph,
    Zoom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HubView {
    pub screen: Screen,
    /// Index into `order(st)`.
    pub sel: usize,
    pub show_done: bool,
    /// Zoom timeline lines scrolled up from the bottom.
    pub scroll: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Full,
    Compact,
    TooSmall,
}

pub fn fit(w: u16, h: u16) -> Fit {
    if w >= 160 && h >= 45 {
        Fit::Full
    } else if w >= 120 && h >= 30 {
        Fit::Compact
    } else {
        Fit::TooSmall
    }
}

/// Agents in tree order: main, then each agent followed by the ones it spawned, siblings by start.
pub fn order(st: &State) -> Vec<usize> {
    fn walk(st: &State, parent: &str, out: &mut Vec<usize>) {
        let mut kids: Vec<usize> = (0..st.agents.len())
            .filter(|&i| {
                let a = &st.agents[i];
                a.id != "main"
                    && a.parent.as_deref().unwrap_or("main") == parent
                    && !out.contains(&i)
            })
            .collect();
        kids.sort_by_key(|&i| st.agents[i].started);
        for k in kids {
            out.push(k);
            walk(st, &st.agents[k].id, out);
        }
    }
    let mut out: Vec<usize> = st
        .agents
        .iter()
        .position(|a| a.id == "main")
        .into_iter()
        .collect();
    walk(st, "main", &mut out);
    // Agents whose parent never showed up still get a row.
    let mut rest: Vec<usize> = (0..st.agents.len()).filter(|i| !out.contains(i)).collect();
    rest.sort_by_key(|&i| st.agents[i].started);
    out.extend(rest);
    out
}

pub fn lines(i: &Input, v: &HubView, w: u16, h: u16, color: bool) -> Vec<Line<'static>> {
    let f = fit(w, h);
    if f == Fit::TooSmall {
        return Vec::new();
    }
    let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
    match v.screen {
        Screen::Graph => graph_screen(&mut buf, i, v, f),
        Screen::Zoom if !i.st.agents.is_empty() => zoom_screen(&mut buf, i, v, f),
        _ => hub_screen(&mut buf, i, v, f),
    }
    to_lines(&buf, color)
}

/// Rows of cells as lines; wide symbols cover the cells after them. `color` false drops styles.
pub fn to_lines(buf: &Buffer, color: bool) -> Vec<Line<'static>> {
    let a = buf.area;
    (a.y..a.bottom())
        .map(|y| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let (mut text, mut cur) = (String::new(), Style::default());
            let mut x = a.x;
            while x < a.right() {
                let cell = &buf[(x, y)];
                let st = if color {
                    cell.style()
                } else {
                    Style::default()
                };
                if st != cur && !text.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut text), cur));
                }
                cur = st;
                let sym = cell.symbol();
                text.push_str(sym);
                x += (sym.width() as u16).max(1);
            }
            spans.push(Span::styled(text, cur));
            Line::from(spans)
        })
        .collect()
}

pub(crate) fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}
pub(crate) fn faint() -> Style {
    Style::new().fg(Color::DarkGray)
}
pub(crate) fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}
pub(crate) fn lead() -> Style {
    Style::new().fg(Color::Yellow)
}
pub(crate) fn agent() -> Style {
    Style::new().fg(Color::Cyan)
}
pub(crate) fn msg() -> Style {
    Style::new().fg(Color::Magenta)
}
pub(crate) fn ok() -> Style {
    Style::new().fg(Color::Green)
}
pub(crate) fn err() -> Style {
    Style::new().fg(Color::Red)
}
pub(crate) fn warn() -> Style {
    Style::new().fg(Color::Yellow)
}

/// `s` cut to `w` columns, ending in `…` when cut.
pub(crate) fn clip(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.into();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.to_string().width() + 1 > w {
            break;
        }
        out.push(c);
    }
    if w > 0 {
        out.push('…');
    }
    out
}

/// Write `s` at (x, y) within `max_w` columns; returns the column after it.
pub(crate) fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, max_w: u16, st: Style) -> u16 {
    if max_w == 0 || y >= buf.area.bottom() || x >= buf.area.right() {
        return x;
    }
    buf.set_stringn(x, y, clip(s, max_w as usize), max_w as usize, st)
        .0
}

/// Write `s` so it ends at column `right` (inclusive); returns where it starts.
pub(crate) fn rput(buf: &mut Buffer, right: u16, y: u16, s: &str, st: Style) -> u16 {
    let x = (right + 1).saturating_sub(s.width() as u16);
    put(buf, x, y, s, right + 1 - x, st);
    x
}

/// A rounded (or heavy) frame with a title left and right; clears and returns the inside.
pub(crate) fn frame(
    buf: &mut Buffer,
    r: Rect,
    title: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    style: Style,
    heavy: bool,
) -> Rect {
    Clear.render(r, buf);
    let mut t = vec![Span::styled(if heavy { "━ " } else { "─ " }, style)];
    t.extend(title);
    t.push(Span::raw(" "));
    let mut b = Block::bordered()
        .border_type(if heavy {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(style)
        .title_top(Line::from(t));
    if !right.is_empty() {
        let mut rt = vec![Span::raw(" ")];
        rt.extend(right);
        rt.push(Span::styled(if heavy { " ━" } else { " ─" }, style));
        b = b.title_top(Line::from(rt).right_aligned());
    }
    let inner = b.inner(r);
    b.render(r, buf);
    inner
}

/// The state glyph for an agent and its style: spinner, `✓`, `↻` (last call failed), `◉` (waiting), `?` (stale).
pub(crate) fn glyph(i: &Input, a: &Agent) -> (String, Style) {
    use crate::state::AgentStatus;
    let k = i.st.agents.iter().position(|x| x.id == a.id).unwrap_or(0);
    let spin = bar::SPINNER[((i.now / 100) as usize + k * 3) % bar::SPINNER.len()].to_string();
    if a.waiting.is_some() {
        return ("◉".into(), msg().add_modifier(Modifier::BOLD));
    }
    if let Some((active, v)) = a.session {
        return if active {
            (spin, agent())
        } else {
            (v.glyph().to_string(), crate::panel::verdict_style(v))
        };
    }
    if a.id == "main" {
        return if i.st.turn_active {
            (spin, lead())
        } else {
            let v = i.st.verdict_with(i.cfg.thresholds.stuck_repeats);
            (v.glyph().to_string(), crate::panel::verdict_style(v))
        };
    }
    match i.st.agent_status(a, i.now) {
        AgentStatus::Done => ("✓".into(), ok()),
        AgentStatus::Stale => ("?".into(), faint()),
        AgentStatus::Running if a.recent.back().is_some_and(|r| r.ok == Some(false)) => {
            ("↻".into(), warn())
        }
        AgentStatus::Running => (spin, agent()),
    }
}

/// Tool names colored by kind, as in the watch windows.
pub(crate) fn tool_style(tool: &str) -> Style {
    match tool {
        "Read" => Style::new().fg(Color::Blue),
        "Edit" | "Write" | "MultiEdit" => ok(),
        "Bash" => warn(),
        "Grep" | "Glob" => agent(),
        "Agent" => lead(),
        "SendMessage" => msg(),
        _ => dim(),
    }
}

/// Border and name color: lead, waiting, retrying, finished, running.
pub(crate) fn node_style(i: &Input, a: &Agent) -> Style {
    if a.id == "main" {
        lead()
    } else if a.waiting.is_some() {
        msg()
    } else if a.done {
        faint()
    } else if a.recent.back().is_some_and(|r| r.ok == Some(false)) {
        warn()
    } else {
        let _ = i;
        agent()
    }
}

pub(crate) fn ledger<'a>(i: &'a Input, a: &Agent) -> Option<&'a Ledger> {
    let t = i.x.tokens.as_ref()?;
    if a.id == "main" {
        Some(&t.main)
    } else {
        t.agents.get(&a.id)
    }
}

/// Context in use: (tokens, percent of the window).
pub(crate) fn ctx_of(i: &Input, a: &Agent) -> Option<(u64, f64)> {
    let used = ledger(i, a).map_or(0, |l| l.ctx);
    if a.id == "main"
        && let (Some(pct), Some(n)) = (i.u.ctx_pct, i.u.ctx_used)
    {
        return Some((n, pct));
    }
    // shortcut: subagents use the main window size; a subagent on a model with another window is off.
    let size = a.ctx_size.or(i.u.ctx_size).unwrap_or(200_000).max(1);
    (used > 0).then(|| (used, used as f64 * 100.0 / size as f64))
}

pub(crate) struct Areas {
    pub top: Rect,
    pub graph: Rect,
    pub table: Rect,
    pub tiles: Rect,
    pub messages: Rect,
    pub lanes: Option<Rect>,
    pub keys: Rect,
}

pub(crate) fn areas(w: u16, h: u16, fit: Fit) -> Areas {
    let full = fit == Fit::Full;
    let bottom = if full { 8 } else { 6 };
    let r = h.saturating_sub(3 + bottom);
    let band = if full { r * 20 / 38 } else { r / 2 };
    let gw = if full { w * 100 / 168 } else { w * 60 / 100 };
    let by = 2 + r;
    Areas {
        top: Rect::new(0, 0, w, 1),
        graph: Rect::new(0, 2, gw, band),
        table: Rect::new(gw, 2, w - gw, band),
        tiles: Rect::new(0, 2 + band, w, r - band),
        messages: Rect::new(0, by, if full { gw } else { w }, bottom),
        lanes: full.then(|| Rect::new(gw, by, w - gw, bottom)),
        keys: Rect::new(0, h - 1, w, 1),
    }
}

fn chrome(buf: &mut Buffer, i: &Input, a: &Areas, keys: &[(&str, String)]) {
    let w = a.top.width;
    let bar_bg = Style::new().bg(Color::Indexed(236));
    buf.set_style(a.top, bar_bg);
    let project =
        i.u.dir
            .as_deref()
            .map(crate::panel::tilde)
            .or_else(|| i.u.project.clone())
            .unwrap_or_default();
    let label = i.u.session_name.clone().or_else(|| i.st.label.clone());
    let mut x = put(buf, 0, 0, " ARI ", w, lead().add_modifier(Modifier::BOLD));
    x = put(buf, x, 0, "hub", w - x, bold());
    if i.x.app != crate::app::App::Claude {
        let st = Style::new()
            .fg(i.x.app.accent())
            .add_modifier(Modifier::BOLD);
        x = put(buf, x, 0, &format!("   {}", i.x.app.label()), w - x, st);
    }
    if !project.is_empty() {
        x = put(buf, x, 0, &format!("   {project}"), w - x, Style::default());
    }
    if let Some(l) = label {
        put(
            buf,
            x,
            0,
            &format!(" · {l}"),
            w.saturating_sub(x + 10),
            dim(),
        );
    }
    rput(buf, w - 2, 0, i.clock, dim());

    let text = bar::render(
        i.st,
        i.u,
        i.x,
        i.cfg,
        i.now,
        w.saturating_sub(2) as usize,
        false,
    );
    let (head, rest) = text
        .split_once('│')
        .map_or((text.as_str(), ""), |(h, r)| (h, r));
    let v = i.st.verdict_with(i.cfg.thresholds.stuck_repeats);
    let head = match i.st.waiting() {
        Some(a) => format!("{} {} ", head.trim_end(), a.label()),
        None => head.to_string(),
    };
    let mut x = put(buf, 1, 1, &head, w - 1, crate::panel::verdict_style(v));
    if !rest.is_empty() {
        x = put(buf, x, 1, "│", w - x, faint());
        put(buf, x, 1, rest, w - x, Style::default());
    }

    buf.set_style(a.keys, bar_bg);
    let y = a.keys.y;
    let mut x = 1;
    for (k, l) in keys {
        x = put(
            buf,
            x,
            y,
            &format!(" {k} "),
            w - x,
            lead().add_modifier(Modifier::BOLD),
        );
        x = put(buf, x, y, &format!("{l}  "), w - x, dim());
    }
    rput(buf, w - 2, y, &format!("{}×{}", w, a.keys.y + 1), faint());
}

/// One agent over the whole body.
fn zoom_screen(buf: &mut Buffer, i: &Input, v: &HubView, f: Fit) {
    let (w, h) = (buf.area.width, buf.area.height);
    let a = areas(w, h, f);
    let keys = [
        ("↑↓", "agent".to_string()),
        ("pgup pgdn", "scroll".into()),
        ("esc", "back".into()),
        ("g", "graph".into()),
        ("s", "settings".into()),
        ("q", "quit".into()),
    ];
    chrome(buf, i, &a, &keys);
    let ord = order(i.st);
    let k = ord[v.sel.min(ord.len() - 1)];
    zoom::draw(buf, Rect::new(0, 2, w, h - 3), i, &i.st.agents[k], v.scroll);
}

/// The graph over most of the height, the tasks board under it.
fn graph_screen(buf: &mut Buffer, i: &Input, v: &HubView, f: Fit) {
    let (w, h) = (buf.area.width, buf.area.height);
    let a = areas(w, h, f);
    let keys = [
        ("↑↓", "select".to_string()),
        ("⏎", "zoom".into()),
        ("h", "hub".into()),
        ("z", "show finished".into()),
        ("esc", "back".into()),
        ("s", "settings".into()),
        ("q", "quit".into()),
    ];
    chrome(buf, i, &a, &keys);
    let gh = (h - 3) * 31 / 46;
    let ord = order(i.st);
    let sel = ord.get(v.sel).map(|&k| i.st.agents[k].id.as_str());
    graph::draw(
        buf,
        Rect::new(0, 2, w, gh),
        i,
        sel,
        true,
        f == Fit::Full,
        v.show_done,
    );
    feed::tasks(buf, Rect::new(0, 2 + gh, w, h - 3 - gh), i);
}

fn hub_screen(buf: &mut Buffer, i: &Input, v: &HubView, f: Fit) {
    let (w, h) = (buf.area.width, buf.area.height);
    let a = areas(w, h, f);
    let done = i.st.agents.iter().filter(|a| a.done).count();
    let keys = [
        ("↑↓", "select".to_string()),
        ("⏎", "zoom".into()),
        ("g", "graph".into()),
        ("z", format!("show finished ({done})")),
        ("x", "back to panel".into()),
        ("s", "settings".into()),
        ("q", "quit".into()),
    ];
    chrome(buf, i, &a, &keys);
    let ord = order(i.st);
    let sel = ord.get(v.sel).map(|&k| i.st.agents[k].id.as_str());
    graph::draw(buf, a.graph, i, sel, false, f == Fit::Full, v.show_done);
    agents::table(buf, a.table, i, v.sel);
    agents::tiles(buf, a.tiles, i, v, if f == Fit::Full { 4 } else { 2 });
    feed::messages(buf, a.messages, i);
    if let Some(l) = a.lanes {
        feed::lanes(buf, l, i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Agent, State};
    use unicode_width::UnicodeWidthStr;

    pub(crate) fn kid(id: &str, parent: &str, started: u64) -> Agent {
        Agent {
            id: id.into(),
            kind: "Explore".into(),
            name: Some(id.into()),
            parent: Some(parent.into()),
            started,
            ..Default::default()
        }
    }
    pub(crate) fn with_main(kids: Vec<Agent>) -> State {
        let mut st = State::default();
        st.agents.push(Agent {
            id: "main".into(),
            kind: "main".into(),
            ..Default::default()
        });
        st.agents.extend(kids);
        st
    }
    pub(crate) fn render(st: &State, v: &HubView, w: u16, h: u16) -> Vec<String> {
        let (u, x, cfg) = Default::default();
        let i = Input {
            st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now: 100_000,
            clock: "13:04",
        };
        crate::panel::to_plain(&lines(&i, v, w, h, false))
            .lines()
            .map(String::from)
            .collect()
    }

    #[test]
    fn fit_boundaries() {
        assert_eq!(fit(160, 45), Fit::Full);
        assert_eq!(fit(159, 45), Fit::Compact);
        assert_eq!(fit(160, 44), Fit::Compact);
        assert_eq!(fit(120, 30), Fit::Compact);
        assert_eq!(fit(119, 30), Fit::TooSmall);
        assert_eq!(fit(120, 29), Fit::TooSmall);
    }

    #[test]
    fn order_is_tree_order() {
        let st = with_main(vec![
            kid("a3", "main", 3),
            kid("a2", "a1", 2),
            kid("a1", "main", 1),
        ]);
        let ids: Vec<_> = order(&st)
            .iter()
            .map(|&i| st.agents[i].id.as_str())
            .collect();
        assert_eq!(ids, ["main", "a1", "a2", "a3"]);
    }

    #[test]
    fn every_line_is_full_width() {
        let st = with_main(vec![kid("a1", "main", 1), kid("a2", "main", 2)]);
        for (w, h) in [(168, 49), (130, 36)] {
            let out = render(&st, &HubView::default(), w, h);
            assert_eq!(out.len(), h as usize, "{w}×{h}");
            for l in &out {
                assert_eq!(l.width(), w as usize, "{w}×{h}: {l:?}");
            }
        }
    }

    #[test]
    fn too_small_draws_nothing() {
        assert!(render(&State::default(), &HubView::default(), 100, 40).is_empty());
    }
}

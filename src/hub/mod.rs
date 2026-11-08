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
    /// `a`: every live session merged, in place of the watched one.
    pub all: bool,
    /// `b`: git panes in place of messages and activity (hub) or messages (zoom).
    pub git: bool,
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

/// `--all`: every live session merged into one state, the root `main` being you.
pub(crate) fn merged(i: &Input) -> bool {
    i.u.project.as_deref() == Some("all sessions")
}

/// The repo agent `a` works in, and the folder its file keys are anchored to: in `--all`
/// its session's (keys are `{dir}/{path}`), else the watched session's (keys are bare paths).
pub(crate) fn repo<'a>(i: &'a Input, a: &Agent) -> Option<(&'a crate::git::Git, &'a str)> {
    if !merged(i) {
        return i.x.git.as_ref().map(|g| (g, ""));
    }
    let k = i.st.agents.iter().position(|x| x.id == a.id)?;
    let key = &i.st.agents[top_of(i.st, k)].id;
    let r = i.x.repos.iter().find(|r| &r.key == key)?;
    Some((&r.git, r.dir.as_str()))
}

/// The plan row of agent `a`'s window: (done, total, what, task in progress). A session's
/// lead (main, or each session in `--all`) has its session's task list, else its checkbox
/// plan file; a subagent has the tasks it owns. None: no plan, no row.
pub(crate) fn plan_of(i: &Input, a: &Agent) -> Option<(usize, usize, String, Option<String>)> {
    let all = merged(i);
    let lead = if all {
        a.parent.as_deref() == Some("main")
    } else {
        a.id == "main"
    };
    // `--all` prefixes task ids with their session's key, as it does agent ids.
    let key = a.id.split('/').next().unwrap_or_default();
    let me = a.label();
    let tasks: Vec<_> =
        i.st.tasks
            .iter()
            .filter(|t| !all || t.id.split('/').next() == Some(key))
            .filter(|t| lead || t.owner.as_deref() == Some(me.as_str()))
            .collect();
    if tasks.is_empty() {
        let p = match all {
            _ if !lead => None,
            true => i.x.plans.iter().find(|p| p.0 == key).map(|p| &p.1),
            false => i.x.plan.as_ref(),
        }?;
        return Some((p.done, p.total, p.name.clone(), None));
    }
    let done = tasks.iter().filter(|t| t.status == "completed").count();
    let now = tasks.iter().find(|t| t.status == "in_progress");
    Some((
        done,
        tasks.len(),
        "tasks".into(),
        now.map(|t| t.subject.clone()),
    ))
}

/// A changed path as an agent's file key: anchored to `dir` in `--all`.
pub(crate) fn file_key(dir: &str, path: &str) -> String {
    if dir.is_empty() {
        path.to_string()
    } else {
        format!("{dir}/{path}")
    }
}

/// The agent's ancestor right under main (in `--all`, its session's lead); main maps to itself.
pub(crate) fn top_of(st: &State, k: usize) -> usize {
    let mut cur = k;
    // Bounded: a parent loop in bad data can't spin forever.
    for _ in 0..st.agents.len() {
        let p = st.agents[cur].parent.as_deref().unwrap_or("main");
        if st.agents[cur].id == "main" || p == "main" {
            break;
        }
        match st.agents.iter().position(|a| a.id == p) {
            Some(n) => cur = n,
            None => break,
        }
    }
    cur
}

/// Agents under `lead` in tree order with their depth (1 = spawned by `lead`).
pub(crate) fn subtree(st: &State, lead: usize) -> Vec<(usize, usize)> {
    let ord = order(st);
    let Some(at) = ord.iter().position(|&k| k == lead) else {
        return vec![];
    };
    let mut out = vec![];
    for &k in &ord[at + 1..] {
        let mut depth = 0;
        let mut cur = k;
        // Walk up to `lead`; a node outside its tree ends the run (tree order keeps them together).
        while cur != lead && depth <= st.agents.len() {
            let p = st.agents[cur].parent.as_deref().unwrap_or("main");
            match st.agents.iter().position(|a| a.id == p) {
                Some(n) => (cur, depth) = (n, depth + 1),
                None => break,
            }
        }
        if cur != lead {
            break;
        }
        out.push((k, depth));
    }
    out
}

/// Waits on a permission prompt, or (`--all`) its session needs you.
pub(crate) fn needs_you(a: &Agent) -> bool {
    a.waiting.is_some() || matches!(a.session, Some((_, crate::state::Verdict::NeedsYou)))
}

/// Border and name color by state. Main and (`--all`) each session take their verdict's colour,
/// as in the statusline; subagents: needs you, finished, stale, retrying, running.
pub(crate) fn node_style(i: &Input, a: &Agent) -> Style {
    use crate::state::AgentStatus;
    let verdict = match a.session {
        Some((_, v)) => Some(v),
        None if a.id == "main" => Some(i.st.verdict_with(i.cfg.thresholds.stuck_repeats)),
        None => None,
    };
    if needs_you(a) {
        msg()
    } else if let Some(v) = verdict {
        // The verdict's colour (and Idle's dim) without its bold or reverse.
        let mut s =
            crate::panel::verdict_style(v).remove_modifier(Modifier::BOLD | Modifier::REVERSED);
        s.sub_modifier = Modifier::empty();
        s
    } else if a.done || i.st.agent_status(a, i.now) == AgentStatus::Stale {
        faint()
    } else if a.recent.back().is_some_and(|r| r.ok == Some(false)) {
        warn()
    } else {
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
        ("b", git_key(v)),
        ("esc", "back".into()),
        ("g", "graph".into()),
        ("s", "settings".into()),
        ("q", "quit".into()),
    ];
    chrome(buf, i, &a, &keys);
    let ord = order(i.st);
    let k = ord[v.sel.min(ord.len() - 1)];
    zoom::draw(buf, Rect::new(0, 2, w, h - 3), i, &i.st.agents[k], v);
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
        ("c", "clear finished".into()),
        (
            "a",
            if v.all {
                "this session"
            } else {
                "all sessions"
            }
            .into(),
        ),
        ("b", git_key(v)),
        ("x", "back to panel".into()),
        ("s", "settings".into()),
        ("q", "quit".into()),
    ];
    // `ari hub --all` is merged already: nothing for `a` to switch to.
    let fixed = !v.all && merged(i);
    let keys: Vec<_> = keys
        .into_iter()
        .filter(|k| !(fixed && k.0 == "a"))
        .collect();
    chrome(buf, i, &a, &keys);
    let ord = order(i.st);
    let sel = ord.get(v.sel).map(|&k| i.st.agents[k].id.as_str());
    graph::draw(buf, a.graph, i, sel, false, f == Fit::Full, v.show_done);
    agents::table(buf, a.table, i, v.sel);
    let across = if merged(i) {
        (w / 76).max(1) // session cards are two panes wide
    } else if f == Fit::Full {
        4
    } else {
        2
    };
    agents::tiles(buf, a.tiles, i, v, across);
    if v.git {
        let (g, u) = match a.lanes {
            Some(l) => (a.messages, l),
            None => {
                let m = a.messages;
                let gw = m.width * 55 / 100;
                (
                    Rect { width: gw, ..m },
                    Rect::new(m.x + gw, m.y, m.width - gw, m.height),
                )
            }
        };
        if merged(i) {
            feed::repos(buf, g, i);
        } else {
            feed::git(buf, g, i, i.x.git.as_ref());
        }
        feed::uncommitted(buf, u, i);
    } else {
        feed::messages(buf, a.messages, i);
        if let Some(l) = a.lanes {
            feed::lanes(buf, l, i);
        }
    }
}

fn git_key(v: &HubView) -> String {
    if v.git { "messages" } else { "git" }.into()
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
    fn subtree_and_top_of_follow_parents() {
        let st = State {
            agents: vec![
                kid("s1", "main", 1),
                kid("rev", "s1", 2),
                kid("scout", "rev", 3),
                kid("s2", "main", 4),
            ],
            ..Default::default()
        };
        assert_eq!(subtree(&st, 0), [(1, 1), (2, 2)]);
        assert!(subtree(&st, 3).is_empty());
        assert_eq!((top_of(&st, 2), top_of(&st, 3)), (0, 3));
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
    fn needs_you_turns_the_box_purple() {
        use crate::state::Verdict;
        let mut st = with_main(vec![kid("a1", "main", 1), kid("s1", "main", 2)]);
        st.agents[0].waiting = Some((0, "Bash".into()));
        st.agents[2].session = Some((true, Verdict::NeedsYou));
        let (u, x, cfg) = Default::default();
        let i = Input {
            st: &st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now: 0,
            clock: "",
        };
        let style = |k: usize| node_style(&i, &st.agents[k]);
        assert_eq!(style(0), msg(), "main waiting");
        assert_eq!(style(1), agent(), "running kid");
        assert_eq!(style(2), msg(), "session needs you");
    }

    #[test]
    fn boxes_take_their_state_colour() {
        use crate::state::Verdict;
        let mut st = with_main(vec![
            kid("run", "main", 1),
            kid("stale", "main", 2),
            kid("done", "main", 2),
            kid("s1", "main", 3),
        ]);
        // Now is 1_000_000: `run` was just active, `stale` last at 0 (> STALE_MS ago).
        st.agents[1].last_t = 1_000_000;
        st.agents[3].done = true;
        st.agents[4].session = Some((false, Verdict::Blocked));
        let (u, x, cfg) = Default::default();
        let i = Input {
            st: &st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now: 1_000_000,
            clock: "",
        };
        let style = |k: usize| node_style(&i, &st.agents[k]);
        assert_eq!(style(0), dim(), "main with nothing going on: idle");
        assert_eq!(style(1), agent(), "running kid");
        assert_eq!(style(2), faint(), "stale kid");
        assert_eq!(style(3), faint(), "finished kid");
        assert_eq!(style(4).fg, Some(Color::Red), "blocked session");
    }

    #[test]
    fn plan_row_lead_gets_session_plan_subagent_its_own_tasks() {
        use crate::state::{PlanFile, Task};
        let mut st = with_main(vec![kid("impl", "main", 1), kid("rev", "main", 1)]);
        let (u, cfg) = Default::default();
        let mut x = Extras {
            plan: Some(PlanFile {
                name: "plan.md".into(),
                done: 1,
                total: 4,
            }),
            ..Default::default()
        };
        let plan = |st: &State, x: &Extras, k: usize| {
            let i = Input {
                st,
                u: &u,
                x,
                cfg: &cfg,
                now: 1,
                clock: "",
            };
            plan_of(&i, &st.agents[k]).map(|p| (p.0, p.1, p.2, p.3))
        };
        // No tasks: main falls back to the plan file; subagents have no row.
        assert_eq!(plan(&st, &x, 0), Some((1, 4, "plan.md".into(), None)));
        assert_eq!(plan(&st, &x, 1), None);
        let t = |id: &str, status: &str, owner: Option<&str>| Task {
            id: id.into(),
            subject: format!("task {id}"),
            status: status.into(),
            owner: owner.map(String::from),
            ..Default::default()
        };
        st.tasks = vec![
            t("1", "completed", Some("impl")),
            t("2", "in_progress", Some("impl")),
            t("3", "pending", None),
        ];
        x.plan = None;
        assert_eq!(
            plan(&st, &x, 0),
            Some((1, 3, "tasks".into(), Some("task 2".into())))
        );
        assert_eq!(
            plan(&st, &x, 1),
            Some((1, 2, "tasks".into(), Some("task 2".into())))
        );
        assert_eq!(plan(&st, &x, 2), None, "rev owns no tasks");
    }

    #[test]
    fn too_small_draws_nothing() {
        assert!(render(&State::default(), &HubView::default(), 100, 40).is_empty());
    }
}

//! The graph: lead on top, the agents it spawned below, spawn lines and message edges.

use super::{Input, bold, clip, dim, faint, frame, glyph, ledger, msg, node_style, put, rput};
use crate::bar::{cells, fmt_tokens};
use crate::state::{Agent, State};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::collections::BTreeMap;
use unicode_width::UnicodeWidthStr;

const MIN_W: u16 = 16;
const GAP: u16 = 4;
const PACKET_MS: u64 = 1600;

/// Messages between two agents, either way. `a < b` by id.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Edge {
    pub a: String,
    pub b: String,
    pub n: usize,
    pub last: String,
    pub from: String,
    pub t: u64,
}

/// One edge per pair of known agents that messaged; most messages first.
pub(crate) fn edges(st: &State) -> Vec<Edge> {
    let known = |id: &str| st.agents.iter().any(|a| a.id == id);
    let mut by: BTreeMap<(String, String), Edge> = BTreeMap::new();
    for m in st
        .messages
        .iter()
        .filter(|m| known(&m.from) && known(&m.to) && m.from != m.to)
    {
        let (a, b) = if m.from < m.to {
            (&m.from, &m.to)
        } else {
            (&m.to, &m.from)
        };
        let e = by.entry((a.clone(), b.clone())).or_insert_with(|| Edge {
            a: a.clone(),
            b: b.clone(),
            n: 0,
            last: String::new(),
            from: String::new(),
            t: 0,
        });
        e.n += 1;
        e.last.clone_from(&m.text);
        e.from.clone_from(&m.from);
        e.t = m.t;
    }
    let mut v: Vec<Edge> = by.into_values().collect();
    v.sort_by(|x, y| y.n.cmp(&x.n).then(y.t.cmp(&x.t)));
    v
}

pub(crate) struct Layout {
    pub lead: Rect,
    /// (agent index, node) for the agents drawn.
    pub nodes: Vec<(usize, Rect)>,
    /// Agents that didn't fit, drawn as one `+N more` node at `more_rect`.
    pub more: usize,
    pub more_rect: Option<Rect>,
}

fn node_size(big: bool) -> (u16, u16) {
    if big { (34, 7) } else { (20, 4) }
}

/// Lead centered one row below the top; kids in a row 3 rows under it.
pub(crate) fn layout(area: Rect, kids: &[usize], big: bool) -> Layout {
    let (pref, nh) = node_size(big);
    let inner_w = area.width.saturating_sub(2);
    let lw = pref.min(inner_w);
    let lead = Rect::new(area.x + (area.width - lw) / 2, area.y + 1, lw, nh);
    let fit = ((inner_w + GAP) / (MIN_W + GAP)).max(1) as usize;
    let (shown, more) = if kids.len() <= fit {
        (kids.len(), 0)
    } else {
        (fit - 1, kids.len() - (fit - 1))
    };
    let slots = (shown + (more > 0) as usize) as u16;
    if slots == 0 {
        return Layout {
            lead,
            nodes: vec![],
            more: 0,
            more_rect: None,
        };
    }
    let nw = ((inner_w - GAP * (slots - 1)) / slots).min(pref);
    let total = slots * nw + (slots - 1) * GAP;
    let x0 = area.x + 1 + (inner_w - total) / 2;
    let ky = lead.bottom() + 3;
    let rect = |k: u16| Rect::new(x0 + k * (nw + GAP), ky, nw, nh);
    Layout {
        lead,
        nodes: kids[..shown]
            .iter()
            .enumerate()
            .map(|(k, &i)| (i, rect(k as u16)))
            .collect(),
        more,
        more_rect: (more > 0).then(|| rect(slots - 1)),
    }
}

const U: u8 = 1;
const D: u8 = 2;
const L: u8 = 4;
const R: u8 = 8;

/// Box-drawing lines that join where they meet.
#[derive(Default)]
struct Lines {
    cells: BTreeMap<(u16, u16), (u8, bool, Style)>,
}

impl Lines {
    fn poke(&mut self, x: u16, y: u16, bits: u8, heavy: bool, st: Style) {
        let c = self.cells.entry((x, y)).or_insert((0, false, st));
        c.0 |= bits;
        if heavy && !c.1 {
            c.1 = true;
            c.2 = st;
        }
    }

    /// A straight run between two cells, both included; returns its cells in order.
    fn run(&mut self, from: (u16, u16), to: (u16, u16), heavy: bool, st: Style) -> Vec<(u16, u16)> {
        let mut out = vec![];
        if from.1 == to.1 {
            let (a, b) = (from.0.min(to.0), from.0.max(to.0));
            for x in a..=b {
                let bits = if x > a { L } else { 0 } | if x < b { R } else { 0 };
                self.poke(x, from.1, bits, heavy, st);
                out.push((x, from.1));
            }
            if from.0 > to.0 {
                out.reverse();
            }
        } else {
            let (a, b) = (from.1.min(to.1), from.1.max(to.1));
            for y in a..=b {
                let bits = if y > a { U } else { 0 } | if y < b { D } else { 0 };
                self.poke(from.0, y, bits, heavy, st);
                out.push((from.0, y));
            }
            if from.1 > to.1 {
                out.reverse();
            }
        }
        out
    }

    fn draw(&self, buf: &mut Buffer, clip_to: Rect) {
        for (&(x, y), &(bits, heavy, st)) in &self.cells {
            if clip_to.contains((x, y).into()) {
                buf[(x, y)].set_symbol(symbol(bits, heavy)).set_style(st);
            }
        }
    }
}

fn symbol(bits: u8, heavy: bool) -> &'static str {
    let (light, thick) = match bits {
        b if b == U | D => ("│", "┃"),
        b if b == L | R => ("─", "━"),
        b if b == D | R => ("╭", "┏"),
        b if b == D | L => ("╮", "┓"),
        b if b == U | R => ("╰", "┗"),
        b if b == U | L => ("╯", "┛"),
        b if b == U | D | R => ("├", "┣"),
        b if b == U | D | L => ("┤", "┫"),
        b if b == D | L | R => ("┬", "┳"),
        b if b == U | L | R => ("┴", "┻"),
        b if b == U | D | L | R => ("┼", "╋"),
        U | D => ("│", "┃"),
        L | R => ("─", "━"),
        _ => ("·", "·"),
    };
    if heavy { thick } else { light }
}

fn join(parts: Vec<Vec<(u16, u16)>>) -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = vec![];
    for p in parts {
        for c in p {
            if out.last() != Some(&c) {
                out.push(c);
            }
        }
    }
    out
}

fn node(buf: &mut Buffer, r: Rect, i: &Input, a: &Agent, sel: bool, big: bool) {
    let st = node_style(i, a);
    let (g, gs) = glyph(i, a);
    let subs =
        i.st.agents
            .iter()
            .filter(|x| x.parent.as_deref() == Some(&a.id) && a.id != "main")
            .count();
    let right = if subs > 0 {
        vec![Span::styled(format!("+{subs} sub"), dim())]
    } else {
        vec![]
    };
    let name = if a.done {
        dim()
    } else {
        st.add_modifier(Modifier::BOLD)
    };
    let inner = frame(
        buf,
        r,
        vec![
            Span::styled(g, gs),
            Span::styled(format!(" {}", a.label()), name),
        ],
        right,
        st,
        sel,
    );
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    let l = ledger(i, a);
    let model = l
        .and_then(|l| l.last_model.as_deref())
        .map(crate::tokens::short_model)
        .unwrap_or_else(|| a.kind.clone());
    let ctx = super::ctx_of(i, a);
    let act = if a.waiting.is_some() {
        "needs you".to_string()
    } else if a.done {
        a.result.clone().unwrap_or_else(|| "done".into())
    } else {
        a.act.clone().unwrap_or_else(|| "—".into())
    };
    let act_style = if a.done {
        faint()
    } else if a.waiting.is_some() {
        msg()
    } else {
        Style::default()
    };
    if !big {
        let mut row = model.split('-').next().unwrap_or("").to_string();
        if let Some((_, pct)) = ctx {
            row.push_str(&format!(" · ctx {pct:.0}%"));
        }
        put(buf, x, inner.y, &row, w, dim());
        put(buf, x, inner.y + 1, &act, w, act_style);
        return;
    }
    put(buf, x, inner.y, &format!("{model} · {}", a.kind), w, dim());
    if let Some((n, pct)) = ctx {
        let bar = cells(pct / 100.0, 14, '▰', '▱');
        put(
            buf,
            x,
            inner.y + 1,
            &format!("ctx {bar} {pct:.0}% {}", fmt_tokens(n)),
            w,
            dim(),
        );
    }
    put(buf, x, inner.y + 2, &act, w, act_style);
    let last = i.st.messages.iter().rev().find(|m| m.to == a.id);
    if let Some(m) = last {
        let from =
            i.st.agents
                .iter()
                .find(|x| x.id == m.from)
                .map_or(m.from.clone(), Agent::label);
        put(
            buf,
            x,
            inner.y + 3,
            &format!("✉ ← {from}: {}", m.text),
            w,
            msg(),
        );
    }
    let tok = l.map_or(String::new(), |l| {
        format!(" · {} tok", fmt_tokens(l.total()))
    });
    put(
        buf,
        x,
        inner.y + 4,
        &format!("{} calls{tok}", a.calls),
        w,
        faint(),
    );
}

/// Draw the graph pane into `area` (its frame included).
pub(crate) fn draw(
    buf: &mut Buffer,
    area: Rect,
    i: &Input,
    sel: Option<&str>,
    big: bool,
    labels: bool,
) {
    let st = i.st;
    let all = edges(st);
    let inner = frame(
        buf,
        area,
        vec![Span::styled("graph", bold())],
        vec![Span::styled(
            format!("{} agents · {} links", st.agents.len(), all.len()),
            dim(),
        )],
        faint(),
        false,
    );
    let mut kids: Vec<usize> = super::order(st)
        .into_iter()
        .filter(|&k| {
            st.agents[k].id != "main" && st.agents[k].parent.as_deref().unwrap_or("main") == "main"
        })
        .collect();
    kids.sort_by_key(|&k| (st.agents[k].done, st.agents[k].started));
    let lay = layout(area, &kids, big);
    let main = st.agents.iter().find(|a| a.id == "main");
    let legend_y = inner.bottom().saturating_sub(1);
    if lay.nodes.is_empty() {
        match main {
            Some(m) => node(buf, lay.lead, i, m, sel == Some("main"), big),
            None => {
                frame(
                    buf,
                    lay.lead,
                    vec![Span::styled("main", super::lead())],
                    vec![],
                    super::lead(),
                    false,
                );
            }
        }
        let t = "no agents yet";
        put(
            buf,
            area.x + (area.width - t.width() as u16) / 2,
            lay.lead.bottom() + 1,
            t,
            inner.width,
            dim(),
        );
        return;
    }
    let mut lines = Lines::default();
    let mut paths: Vec<(&Edge, Vec<(u16, u16)>)> = vec![];
    let tx = lay.lead.x + lay.lead.width / 2;
    let (top, bus) = (lay.lead.bottom(), lay.lead.bottom() + 1);
    let drop_x = |r: &Rect| r.x + r.width - 4;
    let mut drops: Vec<(String, u16)> = lay
        .nodes
        .iter()
        .map(|(k, r)| (st.agents[*k].id.clone(), drop_x(r)))
        .collect();
    if let Some(r) = lay.more_rect {
        drops.push((String::new(), drop_x(&r)));
    }
    let lo = drops.iter().map(|d| d.1).min().unwrap_or(tx).min(tx);
    let hi = drops.iter().map(|d| d.1).max().unwrap_or(tx).max(tx);
    lines.run((tx, top), (tx, bus), false, dim());
    lines.poke(tx, top, U, false, dim());
    lines.run((lo, bus), (hi, bus), false, dim());
    for (id, dx) in &drops {
        let done = st.agents.iter().any(|a| &a.id == id && a.done);
        let s = if done { faint() } else { dim() };
        lines.run((*dx, bus), (*dx, bus + 1), false, s);
        lines.poke(*dx, bus + 1, D, false, s);
        if done && *dx >= area.x + 11 {
            put(buf, dx - 10, bus + 1, "↩ result", 9, faint());
        }
    }
    // Lead ↔ kid messages ride that kid's spawn line, drawn heavy.
    for e in all.iter().filter(|e| e.a == "main" || e.b == "main") {
        let other = if e.a == "main" { &e.b } else { &e.a };
        let Some((_, dx)) = drops.iter().find(|d| &d.0 == other) else {
            continue;
        };
        let p = join(vec![
            lines.run((tx, top), (tx, bus), true, msg()),
            lines.run((tx, bus), (*dx, bus), true, msg()),
            lines.run((*dx, bus), (*dx, bus + 1), true, msg()),
        ]);
        if labels {
            put(buf, dx + 2, bus + 1, &format!("⇄ {}", e.n), 6, msg());
        }
        let p = if e.from == "main" {
            p
        } else {
            p.into_iter().rev().collect()
        };
        paths.push((e, p));
    }
    // Kid ↔ kid: up to two arcs under the row.
    let kb = lay.nodes[0].1.bottom();
    let center = |id: &str| {
        lay.nodes
            .iter()
            .find(|(k, _)| st.agents[*k].id == id)
            .map(|(_, r)| r.x + r.width / 2)
    };
    let pairs: Vec<&Edge> = all
        .iter()
        .filter(|e| e.a != "main" && e.b != "main")
        .collect();
    let mut drawn = vec![];
    for e in &pairs {
        let (Some(xa), Some(xb)) = (center(&e.a), center(&e.b)) else {
            continue;
        };
        if drawn.len() == 2 {
            break;
        }
        let k = drawn.len() as u16;
        let (xa, xb, ly) = (xa + 2 * k, xb + 2 * k, kb + 1 + k);
        if ly >= legend_y {
            break;
        }
        lines.poke(xa, kb, U, true, msg());
        lines.poke(xb, kb, U, true, msg());
        let p = join(vec![
            lines.run((xa, kb), (xa, ly), true, msg()),
            lines.run((xa, ly), (xb, ly), true, msg()),
            lines.run((xb, ly), (xb, kb), true, msg()),
        ]);
        let p = if e.from == e.a {
            p
        } else {
            p.into_iter().rev().collect()
        };
        paths.push((e, p));
        drawn.push((*e, xa, xb));
    }
    lines.draw(buf, inner);
    for (k, r) in &lay.nodes {
        let a = &st.agents[*k];
        node(buf, *r, i, a, sel == Some(a.id.as_str()), big);
    }
    if let Some(r) = lay.more_rect {
        let inner = frame(
            buf,
            r,
            vec![Span::styled(format!("+{} more", lay.more), bold())],
            vec![],
            faint(),
            false,
        );
        put(
            buf,
            inner.x + 1,
            inner.y,
            "see the agents table",
            inner.width.saturating_sub(2),
            faint(),
        );
    }
    if let Some(m) = main {
        node(buf, lay.lead, i, m, sel == Some("main"), big);
    }
    // Junctions on borders, drawn after the nodes.
    let lead_heavy = paths.iter().any(|(e, _)| e.a == "main" || e.b == "main");
    let lead_sel = sel == Some("main");
    buf[(tx, top - 1)].set_symbol(match (lead_sel, lead_heavy) {
        (true, _) => "┳",
        (false, true) => "┰",
        _ => "┬",
    });
    for (id, dx) in &drops {
        let heavy = paths
            .iter()
            .any(|(e, _)| (e.a == "main" || e.b == "main") && (&e.a == id || &e.b == id));
        let picked = sel.is_some_and(|s| s == id);
        buf[(*dx, bus + 2)].set_symbol(match (picked, heavy) {
            (true, _) => "┻",
            (false, true) => "┸",
            _ => "┴",
        });
    }
    for (_, xa, xb) in &drawn {
        buf[(*xa, kb - 1)].set_symbol("┰").set_style(msg());
        buf[(*xb, kb - 1)].set_symbol("┰").set_style(msg());
    }
    if labels {
        let mut y = kb + 1 + drawn.len() as u16;
        let name = |id: &str| {
            st.agents
                .iter()
                .find(|a| a.id == id)
                .map_or(id.to_string(), Agent::label)
        };
        for (e, xa, xb) in &drawn {
            if y >= legend_y {
                break;
            }
            let t = clip(
                &format!("⇄ {} · \"{}\"", e.n, e.last),
                inner.width as usize - 2,
            );
            let mid = (xa + xb) / 2;
            let x = mid
                .saturating_sub(t.width() as u16 / 2)
                .max(inner.x + 1)
                .min(inner.right().saturating_sub(t.width() as u16 + 1));
            put(buf, x, y, &t, inner.width, msg());
            y += 1;
        }
        let rest: Vec<String> = pairs
            .iter()
            .skip(drawn.len())
            .map(|e| format!("{}⇄{} {}", name(&e.a), name(&e.b), e.n))
            .collect();
        if !rest.is_empty() && y < legend_y {
            put(
                buf,
                inner.x + 1,
                y,
                &format!("also: {}", rest.join(" · ")),
                inner.width - 2,
                msg(),
            );
        }
    }
    // A message under 1.6 s old travels its edge.
    for (e, p) in &paths {
        let age = i.now.saturating_sub(e.t);
        if age < PACKET_MS && !p.is_empty() {
            let (x, y) = p[(age as usize * p.len() / PACKET_MS as usize).min(p.len() - 1)];
            if inner.contains((x, y).into()) {
                buf[(x, y)]
                    .set_symbol("●")
                    .set_style(msg().add_modifier(Modifier::BOLD));
            }
        }
    }
    let mut x = inner.x + 1;
    for (t, s) in [
        ("── spawn", dim()),
        ("   ━━ message", msg()),
        ("   ● in flight", msg()),
        ("   dim = finished", faint()),
    ] {
        x = put(buf, x, legend_y, t, inner.right().saturating_sub(x), s);
    }
    let _ = rput;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::tests::{kid, with_main};
    use crate::state::Msg;

    fn msg(t: u64, from: &str, to: &str, text: &str) -> Msg {
        Msg {
            t,
            from: from.into(),
            to: to.into(),
            text: text.into(),
        }
    }
    fn pane(st: &State, now: u64, w: u16, h: u16) -> String {
        let (u, x, cfg) = Default::default();
        let i = Input {
            st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now,
            clock: "",
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        let area = buf.area;
        draw(&mut buf, area, &i, Some("a1"), false, true);
        crate::panel::to_plain(&crate::hub::to_lines(&buf, false))
    }
    fn four() -> State {
        let mut st = with_main((1..=4).map(|n| kid(&format!("a{n}"), "main", n)).collect());
        st.agents[4].done = true;
        st.agents[4].ended_t = Some(5);
        st.messages.extend([
            msg(10, "a1", "a2", "2 failing"),
            msg(11, "a2", "a1", "fixed"),
            msg(12, "a1", "a2", "green now"),
            msg(13, "main", "a3", "hold"),
            msg(14, "a2", "ghost", "anyone?"),
        ]);
        st
    }

    #[test]
    fn layout_fits_four_and_overflows_twelve() {
        let area = Rect::new(0, 0, 100, 20);
        let l = layout(area, &[1, 2, 3, 4], false);
        assert_eq!(l.nodes.len(), 4);
        assert_eq!(l.more, 0);
        for (k, (_, r)) in l.nodes.iter().enumerate() {
            assert!(
                area.contains(r.as_position()) && r.right() <= area.right(),
                "{r:?}"
            );
            if let Some((_, next)) = l.nodes.get(k + 1) {
                assert!(r.right() < next.x, "overlap {r:?} {next:?}");
            }
        }
        let kids: Vec<usize> = (1..=12).collect();
        let l = layout(area, &kids, false);
        assert_eq!(l.more, 12 - l.nodes.len());
        assert!(l.more > 0 && l.more_rect.is_some());
        assert!(l.nodes.iter().all(|(_, r)| r.width >= 16));
        assert!(l.more_rect.unwrap().right() < area.right());
    }

    #[test]
    fn edges_skip_unknown_names() {
        let e = edges(&four());
        let pairs: Vec<_> = e
            .iter()
            .map(|e| (e.a.as_str(), e.b.as_str(), e.n))
            .collect();
        assert_eq!(pairs, [("a1", "a2", 3), ("a3", "main", 1)]);
        assert_eq!(e[0].last, "green now");
        assert_eq!(e[0].from, "a1");
    }

    #[test]
    fn graph_pane_snapshot() {
        insta::assert_snapshot!(pane(&four(), 100_000, 100, 20));
    }

    #[test]
    fn packet_moves_along_edge() {
        let mut st = four();
        st.messages.push_back(msg(99_200, "a2", "a1", "now"));
        // The legend has one `●`; a message in flight adds another.
        let dots = |now| pane(&st, now, 100, 20).matches('●').count();
        assert_eq!(dots(100_000), 2);
        assert_eq!(dots(101_200), 1);
    }

    #[test]
    fn no_agents_yet() {
        let out = pane(&with_main(vec![]), 1, 100, 20);
        assert!(out.contains("no agents yet"), "{out}");
        assert!(out.contains("main"));
        assert!(pane(&State::default(), 1, 100, 20).contains("no agents yet"));
    }
}

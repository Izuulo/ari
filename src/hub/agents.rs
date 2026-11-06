//! The agents table with its notes (waits, shared files), and the watch windows.

use super::{
    HubView, Input, agent, bold, ctx_of, dim, err, faint, frame, glyph, ledger, msg, node_style,
    ok, order, put, rput, warn,
};
use crate::bar::{cells, fmt_tokens};
use crate::panel::{age, dur};
use crate::state::{Agent, State};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::collections::BTreeMap;

pub(crate) fn mmss(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// `.3s`, `8.1s`, then `2m05s`.
pub(crate) fn short_ms(ms: u64) -> String {
    match ms {
        0..1000 => format!(".{}s", ms / 100),
        1000..60_000 => format!("{:.1}s", ms as f64 / 1000.0),
        _ => dur(ms),
    }
}

pub(crate) fn ctx_style(i: &Input, pct: f64) -> Style {
    let t = &i.cfg.thresholds;
    if pct >= t.ctx_crit as f64 {
        err()
    } else if pct >= t.ctx_warn as f64 {
        warn()
    } else {
        ok()
    }
}

/// Paths edited by two or more agents: path → [(agent label, edits)].
pub(crate) fn shared_files(st: &State) -> Vec<(String, Vec<(String, u32)>)> {
    let mut by: BTreeMap<&str, Vec<(String, u32)>> = BTreeMap::new();
    for a in &st.agents {
        for (p, f) in &a.files {
            by.entry(p).or_default().push((a.label(), f.edits));
        }
    }
    by.into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(p, v)| (p.to_string(), v))
        .collect()
}

/// What the agent is doing, in a few words, with its glyph.
pub(crate) fn state_text(i: &Input, a: &Agent) -> (String, Style) {
    let (g, gs) = glyph(i, a);
    let t = if a.waiting.is_some() {
        "needs you".to_string()
    } else if a.done {
        format!(
            "done {}",
            age(i.now.saturating_sub(a.ended_t.unwrap_or(a.last_t)))
        )
    } else if g == "?" {
        "stale".into()
    } else if g == "↻" {
        "retrying".into()
    } else if a.id == "main" && !i.st.turn_active {
        i.st.verdict_with(i.now, i.cfg.thresholds.stuck_repeats)
            .label()
            .to_lowercase()
    } else if a.running_since.is_some() {
        a.recent.back().map_or("working".into(), |r| r.tool.clone())
    } else if a.id == "main" {
        "waiting".into()
    } else {
        "thinking".into()
    };
    (format!("{g} {t}"), gs)
}

fn tree_prefix(st: &State, k: usize, last: bool) -> &'static str {
    if st.agents[k].id == "main" {
        "◆ "
    } else if last {
        "└ "
    } else {
        "├ "
    }
}

/// Notes under the agents: the oldest wait, shared files, cross-session conflicts, compaction.
fn notes(i: &Input) -> Vec<(String, Option<String>, Style)> {
    let mut out = vec![];
    if let Some(a) = i.st.waiting() {
        let (since, act) = a.waiting.clone().unwrap_or_default();
        out.push((
            format!("◉ {} is waiting on a permission prompt", a.label()),
            Some(format!("waiting {}", mmss(i.now.saturating_sub(since)))),
            msg().add_modifier(Modifier::BOLD),
        ));
        out.push((format!("  {act}"), None, Style::default()));
        out.push((
            "  approve or deny it in the Claude Code window".into(),
            None,
            dim(),
        ));
    } else if let Some(n) = &i.st.needs_you {
        out.push((
            format!("◉ main needs you: {n}"),
            None,
            msg().add_modifier(Modifier::BOLD),
        ));
    }
    for (p, who) in shared_files(i.st).into_iter().take(2) {
        out.push((format!("⚠ {} agents edited {p}", who.len()), None, warn()));
        let list: Vec<String> = who.iter().map(|(l, n)| format!("{l} ×{n}")).collect();
        out.push((format!("  {}", list.join(" · ")), None, dim()));
    }
    if let Some(c) = crate::conflicts::summary(&i.x.conflicts) {
        out.push((c, None, warn()));
    }
    if let Some(n) = i.x.compacts_in {
        out.push((format!("main compacts in ~{n} turns"), None, warn()));
    }
    out
}

/// The agents table: one row per agent in tree order, then notes. `sel` indexes `order`.
pub(crate) fn table(buf: &mut Buffer, area: Rect, i: &Input, sel: usize) {
    let st = i.st;
    let waiting = st.agents.iter().filter(|a| a.waiting.is_some()).count();
    let right = if waiting > 0 {
        vec![Span::styled(format!("◉ {waiting} needs you"), msg())]
    } else {
        vec![]
    };
    let inner = frame(
        buf,
        area,
        vec![Span::styled("agents", bold())],
        right,
        super::faint(),
        false,
    );
    let (x0, xr) = (inner.x + 1, inner.right().saturating_sub(2));
    let wide = inner.width >= 60;
    let col = |n: u16| x0 + n;
    let (c_model, c_state, c_ctx) = if wide { (12, 20, 35) } else { (12, 12, 27) };
    put(buf, col(0), inner.y, "agent", 11, faint());
    if wide {
        put(buf, col(c_model), inner.y, "model", 7, faint());
    }
    put(buf, col(c_state), inner.y, "state", 14, faint());
    put(buf, col(c_ctx), inner.y, "context", 15, faint());
    rput(buf, xr, inner.y, "tok", faint());
    let notes = notes(i);
    let ord = order(st);
    let room = (inner.height as usize)
        .saturating_sub(1 + if notes.is_empty() { 0 } else { notes.len() + 1 });
    let (start, shown) = if ord.len() <= room {
        (0, ord.len())
    } else {
        let shown = room.saturating_sub(1);
        (
            sel.saturating_sub(shown.saturating_sub(1))
                .min(ord.len() - shown),
            shown,
        )
    };
    for (row, &k) in ord.iter().enumerate().skip(start).take(shown) {
        let a = &st.agents[k];
        let y = inner.y + 1 + (row - start) as u16;
        let picked = row == sel;
        if picked {
            buf.set_style(
                Rect::new(inner.x, y, inner.width, 1),
                Style::new().add_modifier(Modifier::BOLD),
            );
            put(buf, inner.x, y, "▸", 1, bold());
        }
        let last = ord
            .get(row + 1)
            .is_none_or(|&n| st.agents[n].parent != a.parent);
        let x = put(buf, col(0), y, tree_prefix(st, k, last), 2, faint());
        let ns = if a.done {
            dim()
        } else {
            node_style(i, a).add_modifier(Modifier::BOLD)
        };
        put(buf, x, y, &a.label(), c_model.min(c_state) - 3, ns);
        let l = ledger(i, a);
        if wide {
            let m = l
                .and_then(|l| l.last_model.as_deref())
                .map(crate::tokens::short_model)
                .unwrap_or_default();
            put(
                buf,
                col(c_model),
                y,
                m.split('-').next().unwrap_or(""),
                7,
                dim(),
            );
        }
        let (t, ts) = state_text(i, a);
        put(buf, col(c_state), y, &t, c_ctx - c_state - 1, ts);
        if let Some((_, pct)) = ctx_of(i, a) {
            let s = if a.done { faint() } else { ctx_style(i, pct) };
            put(buf, col(c_ctx), y, &cells(pct / 100.0, 10, '▰', '▱'), 10, s);
            put(buf, col(c_ctx + 10), y, &format!("{pct:>4.0}%"), 5, s);
        }
        if let Some(l) = l.filter(|l| l.total() > 0) {
            rput(
                buf,
                xr,
                y,
                &fmt_tokens(l.total()),
                if a.done { dim() } else { Style::default() },
            );
        }
    }
    let mut y = inner.y + 1 + shown as u16;
    if shown < ord.len() {
        put(
            buf,
            col(0),
            y,
            &format!("+{} more", ord.len() - shown),
            inner.width - 2,
            faint(),
        );
        y += 1;
    }
    if notes.is_empty() {
        return;
    }
    let ry = inner.bottom().saturating_sub(notes.len() as u16 + 1).max(y);
    buf[(area.x, ry)].set_symbol("├");
    buf[(area.right() - 1, ry)].set_symbol("┤");
    for x in inner.x..inner.right() {
        buf[(x, ry)].set_symbol("─").set_style(faint());
    }
    for (n, (t, r, s)) in notes.iter().enumerate() {
        let y = ry + 1 + n as u16;
        if y >= inner.bottom() {
            break;
        }
        let rw = r.as_ref().map_or(0, |r| r.chars().count() as u16 + 1);
        put(buf, col(0), y, t, inner.width.saturating_sub(2 + rw), *s);
        if let Some(r) = r {
            rput(buf, xr, y, r, msg());
        }
    }
}

/// Watch windows for the page holding the selected agent; finished ones only with `show_done`.
pub(crate) fn tiles(buf: &mut Buffer, area: Rect, i: &Input, v: &HubView, across: u16) {
    let ord = order(i.st);
    let shown: Vec<usize> = ord
        .iter()
        .copied()
        .filter(|&k| v.show_done || !i.st.agents[k].done)
        .collect();
    let sel = ord.get(v.sel).copied();
    let pos = sel
        .and_then(|s| shown.iter().position(|&k| k == s))
        .unwrap_or(0);
    let page = pos / across as usize;
    let first = page * across as usize;
    let tw = area.width / across;
    let on_page: Vec<usize> = shown
        .iter()
        .skip(first)
        .take(across as usize)
        .copied()
        .collect();
    let more = shown.len().saturating_sub(first + on_page.len());
    for (n, &k) in on_page.iter().enumerate() {
        let n = n as u16;
        let w = if n + 1 == across {
            area.width - n * tw
        } else {
            tw
        };
        let r = Rect::new(area.x + n * tw, area.y, w, area.height);
        let last = n as usize + 1 == on_page.len();
        tile(
            buf,
            r,
            i,
            &i.st.agents[k],
            Some(k) == sel,
            if last { more } else { 0 },
        );
    }
}

fn tile(buf: &mut Buffer, r: Rect, i: &Input, a: &Agent, sel: bool, more: usize) {
    let st = node_style(i, a);
    let (g, gs) = glyph(i, a);
    let l = ledger(i, a);
    let model = l
        .and_then(|l| l.last_model.as_deref())
        .map(crate::tokens::short_model);
    let mut title = vec![
        Span::styled(g.clone(), gs),
        Span::styled(format!(" {}", a.label()), st.add_modifier(Modifier::BOLD)),
    ];
    if let Some(m) = model {
        title.push(Span::styled(format!(" · {m}"), dim()));
    }
    let right = if more > 0 {
        Span::styled(format!("+{more} more ›"), bold())
    } else {
        Span::styled(dur(i.now.saturating_sub(a.started)), dim())
    };
    let inner = frame(buf, r, title, vec![right], st, sel);
    if inner.height < 8 {
        return;
    }
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    let xr = inner.right().saturating_sub(2);
    let tok = l.map_or(String::new(), |l| {
        format!(" · {} tok", fmt_tokens(l.total()))
    });
    put(
        buf,
        x,
        inner.y,
        &format!("{} · {} calls{tok}", a.kind, a.calls),
        w,
        dim(),
    );
    let mut y = inner.y + 1;
    if let Some((n, pct)) = ctx_of(i, a) {
        let s = ctx_style(i, pct);
        let nx = put(buf, x, y, "ctx ", 4, faint());
        let nx = put(buf, nx, y, &cells(pct / 100.0, 20, '▰', '▱'), 20, s);
        put(
            buf,
            nx,
            y,
            &format!(" {pct:.0}% {}", fmt_tokens(n)),
            xr.saturating_sub(nx) + 1,
            s,
        );
    }
    y += 1;
    if let Some(l) = l {
        let mut fill: Vec<(&String, &u64)> = l
            .fill
            .iter()
            .filter(|(k, _)| k.as_str() != "base")
            .collect();
        fill.sort_by(|p, q| q.1.cmp(p.1));
        let mut nx = put(buf, x, y, "filled by", w, faint());
        for (k, v) in fill {
            let piece = format!(" {k} {}", fmt_tokens(*v));
            if nx + piece.chars().count() as u16 > xr + 1 {
                break;
            }
            nx = put(buf, nx, y, &piece, w, dim());
        }
    }
    y += 1;
    let rule = |buf: &mut Buffer, y: u16| {
        for cx in inner.x..inner.right() {
            buf[(cx, y)].set_symbol("┄").set_style(faint());
        }
    };
    rule(buf, y);
    let rows = inner.height.saturating_sub(7) as usize;
    let calls: Vec<_> = a.recent.iter().rev().take(rows).collect();
    for (n, c) in calls.iter().rev().enumerate() {
        let cy = y + 1 + n as u16;
        let mut nx = put(
            buf,
            x,
            cy,
            &format!("{:>3} ", age(i.now.saturating_sub(c.t))),
            4,
            faint(),
        );
        let tool: String = c.tool.chars().take(5).collect();
        nx = put(
            buf,
            nx,
            cy,
            &format!("{tool:<5} "),
            6,
            crate::hub::tool_style(&c.tool),
        );
        let (res, rs) = match c.ok {
            Some(true) => ("✓", ok()),
            Some(false) => ("✗", err()),
            None => ("…", dim()),
        };
        let d = c.dur.map(short_ms).unwrap_or_default();
        let tail = format!("{res} {d}");
        let target = c.act.split_once(' ').map_or(c.act.as_str(), |t| t.1);
        put(
            buf,
            nx,
            cy,
            target,
            (xr + 1).saturating_sub(nx + tail.chars().count() as u16 + 1),
            Style::default(),
        );
        let tx = rput(buf, xr, cy, &d, dim());
        rput(
            buf,
            if d.is_empty() {
                xr
            } else {
                tx.saturating_sub(2)
            },
            cy,
            res,
            rs,
        );
    }
    let y = inner.bottom() - 3;
    rule(buf, y);
    if let Some((since, act)) = &a.waiting {
        let nx = put(
            buf,
            x,
            y + 1,
            "◉ waiting on you  ",
            w,
            msg().add_modifier(Modifier::BOLD),
        );
        put(
            buf,
            nx,
            y + 1,
            act,
            (xr + 1).saturating_sub(nx + 6),
            Style::default(),
        );
        rput(buf, xr, y + 1, &mmss(i.now.saturating_sub(*since)), msg());
    } else if a.done {
        put(
            buf,
            x,
            y + 1,
            &format!("✓ done · {}", a.result.as_deref().unwrap_or("")),
            w,
            dim(),
        );
    } else {
        let act = a
            .act
            .clone()
            .unwrap_or_else(|| state_text(i, a).0[g.len() + 1..].to_string());
        let nx = put(buf, x, y + 1, &format!("{g} "), 2, gs);
        put(
            buf,
            nx,
            y + 1,
            &act,
            (xr + 1).saturating_sub(nx + 7),
            bold(),
        );
        let since = a.running_since.unwrap_or(a.last_t);
        if since > 0 {
            rput(buf, xr, y + 1, &dur(i.now.saturating_sub(since)), dim());
        }
    }
    if let Some(m) = i.st.messages.iter().rev().find(|m| m.to == a.id) {
        let from =
            i.st.agents
                .iter()
                .find(|x| x.id == m.from)
                .map_or(m.from.clone(), Agent::label);
        put(buf, x, y + 2, &format!("✉ ← {from}: {}", m.text), w, msg());
    }
    let _ = agent;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::HubView;
    use crate::hub::tests::{kid, render, with_main};
    use crate::state::FileRec;
    use unicode_width::UnicodeWidthStr;

    fn draw_with(st: &State, w: u16, h: u16, f: impl Fn(&mut Buffer, Rect, &Input)) -> String {
        let (u, x, cfg) = Default::default();
        let i = Input {
            st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now: 100_000,
            clock: "",
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        let area = buf.area;
        f(&mut buf, area, &i);
        crate::panel::to_plain(&crate::hub::to_lines(&buf, false))
    }

    fn six() -> State {
        with_main((1..=5).map(|n| kid(&format!("a{n}"), "main", n)).collect())
    }

    #[test]
    fn tiles_page_follows_selection() {
        let st = six();
        let tiles_for = |sel| {
            let v = HubView {
                sel,
                ..Default::default()
            };
            draw_with(&st, 168, 18, |b, r, i| tiles(b, r, i, &v, 4))
        };
        let first = tiles_for(0);
        let top = first.lines().next().unwrap();
        for name in ["main", "a1", "a2", "a3"] {
            assert!(top.contains(&format!(" {name}")), "{top}");
        }
        assert!(top.contains("+2 more ›"), "{top}");
        let second = tiles_for(5);
        let top = second.lines().next().unwrap();
        assert!(
            top.contains(" a4") && top.contains(" a5") && !top.contains(" a1 "),
            "{top}"
        );
    }

    #[test]
    fn table_folds_overflow() {
        let st = with_main((1..=30).map(|n| kid(&format!("a{n}"), "main", n)).collect());
        let out = draw_with(&st, 68, 12, |b, r, i| table(b, r, i, 0));
        assert!(out.contains("+") && out.contains("more"), "{out}");
    }

    #[test]
    fn wait_box_shows_exact_call() {
        let mut st = six();
        st.agents[1].waiting = Some((90_000, "$ curl -s x".into()));
        let out = draw_with(&st, 68, 20, |b, r, i| table(b, r, i, 0));
        assert!(
            out.contains("a1 is waiting on a permission prompt"),
            "{out}"
        );
        assert!(out.contains("$ curl -s x"), "{out}");
        assert!(
            out.contains("approve or deny it in the Claude Code window"),
            "{out}"
        );
        assert!(out.contains("waiting 0:10"), "{out}");
    }

    #[test]
    fn wide_chars_keep_borders() {
        let mut st = six();
        let wide = "修正する🙂 修正する🙂 修正する🙂 修正する🙂 修正する🙂";
        st.agents[1].act = Some(wide.into());
        st.agents[1].prompt = Some(wide.into());
        st.agents[1].recent.push_back(crate::state::CallRec {
            tool: "Read".into(),
            act: wide.into(),
            ok: Some(true),
            ..Default::default()
        });
        st.messages.push_back(crate::state::Msg {
            t: 1,
            from: "a2".into(),
            to: "a1".into(),
            text: wide.into(),
        });
        for l in render(&st, &HubView::default(), 168, 49) {
            assert_eq!(l.width(), 168, "{l}");
        }
    }

    #[test]
    fn shared_files_needs_two_agents() {
        let mut st = with_main(vec![kid("a1", "main", 1)]);
        st.agents[1].name = Some("impl".into());
        let f = |edits| FileRec {
            edits,
            ..Default::default()
        };
        st.agents[0].files.insert("a.rs".into(), f(1));
        st.agents[1].files.insert("a.rs".into(), f(2));
        st.agents[1].files.insert("b.rs".into(), f(1));
        assert_eq!(
            shared_files(&st),
            [(
                "a.rs".to_string(),
                vec![("main".to_string(), 1), ("impl".to_string(), 2)]
            )]
        );
    }
}

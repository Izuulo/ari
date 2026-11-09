//! Calm hub: the hub's layout in plain English, without the numbers.

use crate::hub::{
    HubView, Input, areas, chrome, dim, err, faint, fit, graph, merged, needs_you, order, put,
    subtree,
};
use crate::simple::{banner, draw_banner, glyph, phrase};
use crate::state::AgentStatus;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Block, BorderType, Clear, Widget},
};

/// The graph alone (or, merged, the session cards) under the bare chrome.
pub fn hub(buf: &mut Buffer, i: &Input, v: &HubView) {
    let (w, h) = (buf.area.width, buf.area.height);
    let all = if v.all {
        "this session"
    } else {
        "all sessions"
    };
    // The cards have no selection to move or open.
    let mut keys = if merged(i) {
        vec![]
    } else {
        vec![("↑↓", "select".to_string()), ("⏎", "zoom".into())]
    };
    // `ari hub --all` is merged already: nothing for `a` to switch to.
    if v.all || !merged(i) {
        keys.push(("a", all.into()));
    }
    for (k, l) in [
        ("x", "back to panel"),
        ("v", "constellation"),
        ("s", "settings"),
        ("q", "quit"),
    ] {
        keys.push((k, l.into()));
    }
    chrome(buf, i, &areas(w, h, fit(w, h)), &keys, true);
    let body = Rect::new(0, 2, w, h - 3);
    if merged(i) {
        cards(buf, body, i);
    } else {
        let ord = order(i.st);
        let sel = ord.get(v.sel).map(|&k| i.st.agents[k].id.as_str());
        let o = graph::Opts {
            big: true,
            show_done: v.show_done,
            calm: true,
            ..Default::default()
        };
        graph::draw(buf, body, i, sel, o);
    }
    if let Some(b) = banner(i.st, i.now) {
        let row = Rect::new(0, 2, w, 1);
        if b.on {
            Clear.render(row, buf);
        }
        draw_banner(buf, row, &b);
    }
}

/// One card per merged session, newest activity first, under a head count.
pub fn cards(buf: &mut Buffer, area: Rect, i: &Input) {
    let st = i.st;
    let mut sessions: Vec<usize> = (0..st.agents.len())
        .filter(|&k| st.agents[k].session.is_some())
        .collect();
    sessions.sort_by_key(|&k| std::cmp::Reverse(st.agents[k].last_t));
    let working = st
        .agents
        .iter()
        .filter(|a| a.session.is_none() && st.agent_status(a, i.now) == AgentStatus::Running)
        .count();
    let n = |k: usize, word: &str| format!("{k} {word}{}", if k == 1 { "" } else { "s" });
    let head = format!(
        "{} · {} working",
        n(sessions.len(), "session"),
        n(working, "agent")
    );
    put(
        buf,
        area.x + 2,
        area.y,
        &head,
        area.width.saturating_sub(2),
        dim(),
    );
    if sessions.is_empty() {
        put(
            buf,
            area.x + 2,
            area.y + 2,
            "no live sessions",
            area.width,
            dim(),
        );
        return;
    }
    let cols = (area.width / 30).max(1);
    let cw = area.width / cols;
    let cap = (cols * (area.height.saturating_sub(1) / 6)) as usize;
    let shown = if sessions.len() > cap {
        cap.saturating_sub(1)
    } else {
        sessions.len()
    };
    let slot = |n: usize| {
        let (c, r) = (n as u16 % cols, n as u16 / cols);
        Rect::new(area.x + c * cw, area.y + 1 + r * 6, cw - 1, 6)
    };
    for (n, &k) in sessions[..shown].iter().enumerate() {
        card(buf, slot(n), i, k);
    }
    if shown < sessions.len() && cap > 0 {
        let r = slot(shown);
        let more = format!("+{} more", sessions.len() - shown);
        put(
            buf,
            r.x + 2,
            r.y + 1,
            &more,
            r.width.saturating_sub(2),
            dim(),
        );
    }
}

fn card(buf: &mut Buffer, r: Rect, i: &Input, k: usize) {
    let st = i.st;
    let a = &st.agents[k];
    let Some((active, v)) = a.session else {
        return;
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(faint())
        .render(r, buf);
    let (x, w) = (r.x + 2, r.width.saturating_sub(4));
    let subs = subtree(st, k);
    let waiting = std::iter::once(k)
        .chain(subs.iter().map(|s| s.0))
        .find_map(|j| st.agents[j].waiting.as_ref().map(|w| w.1.clone()));
    let on = (i.now / 500).is_multiple_of(2);
    let (line, line_style) = if needs_you(a) || waiting.is_some() {
        // The merge keeps the first session's ask as `<name>: <ask>`.
        let ask = waiting.or_else(|| {
            let n = st.needs_you.as_deref()?;
            n.strip_prefix(&format!("{}: ", a.label()))
                .map(String::from)
        });
        put(buf, x, r.y + 1, if on { "!" } else { " " }, 1, err());
        let line = ask.map_or("needs you".into(), |s| format!("needs you: {s}"));
        (line, err())
    } else {
        let dot = if on { "●" } else { "◉" };
        put(buf, x, r.y + 1, dot, 1, crate::panel::verdict_style(v));
        let line = if active {
            phrase(st, a, i.now)
        } else {
            "waiting for you".into()
        };
        (line, dim())
    };
    put(
        buf,
        x + 2,
        r.y + 1,
        &a.label(),
        w.saturating_sub(2),
        Default::default(),
    );
    put(buf, x, r.y + 2, &line, w, line_style);
    let mut gx = x;
    for &(s, _) in &subs {
        let (g, gs) = glyph(i, &st.agents[s]);
        if gx + 1 >= x + w {
            break;
        }
        gx = put(buf, gx, r.y + 4, &g, 1, gs) + 1;
    }
}

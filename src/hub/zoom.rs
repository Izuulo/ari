//! One agent up close: its timeline, context window, messages, files, tasks and errors.

use super::agents::{ctx_style, mmss, shared_files, short_ms};
use super::{
    Input, bold, ctx_of, dim, err, faint, frame, glyph, ledger, msg, node_style, ok, put, rput,
    tool_style, warn,
};
use crate::bar::{cells, fmt_tokens};
use crate::panel::age;
use crate::state::{Agent, State};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

fn label(st: &State, id: &str) -> String {
    st.agents
        .iter()
        .find(|a| a.id == id)
        .map_or(id.to_string(), Agent::label)
}

/// One timeline row: left text and style, optional right text and style.
type Row = (String, Style, Option<(String, Style)>);

fn timeline(i: &Input, a: &Agent) -> Vec<Row> {
    enum E<'a> {
        Call(&'a crate::state::CallRec),
        Msg(&'a crate::state::Msg),
    }
    let mut ev: Vec<(u64, u8, E)> = a.recent.iter().map(|c| (c.t, 0, E::Call(c))).collect();
    ev.extend(
        i.st.messages
            .iter()
            .filter(|m| m.from == a.id || m.to == a.id)
            .map(|m| (m.t, 1, E::Msg(m))),
    );
    ev.sort_by_key(|e| (e.0, e.1));
    let mut out = vec![];
    for (t, _, e) in ev {
        let when = format!("{:>4}  ", age(i.now.saturating_sub(t)));
        match e {
            E::Call(c) => {
                let (res, rs) = match c.ok {
                    Some(true) => ("✓", ok()),
                    Some(false) => ("✗", err()),
                    None => ("…", dim()),
                };
                let d = c.dur.map(short_ms).unwrap_or_default();
                out.push((
                    format!("{when}{:<12} {}", c.tool, c.act),
                    Style::default(),
                    Some((format!("{res} {d}"), rs)),
                ));
                if let Some(m) = &c.msg {
                    out.push((format!("        └ {m}"), dim(), None));
                }
            }
            E::Msg(m) => {
                let (dir, who) = if m.from == a.id {
                    ("→", label(i.st, &m.to))
                } else {
                    ("←", label(i.st, &m.from))
                };
                out.push((
                    format!("{when}✉ {dir} {who:<9} \"{}\"", m.text),
                    msg(),
                    None,
                ));
            }
        }
    }
    out
}

fn rule(buf: &mut Buffer, inner: Rect, y: u16) {
    for x in inner.x..inner.right() {
        buf[(x, y)].set_symbol("┄").set_style(faint());
    }
}

/// Draw agent `a` over `area`; `scroll` moves the timeline up from its newest line.
pub(crate) fn draw(buf: &mut Buffer, area: Rect, i: &Input, a: &Agent, scroll: usize) {
    let st = node_style(i, a);
    let (g, gs) = glyph(i, a);
    let l = ledger(i, a);
    let model = l
        .and_then(|l| l.last_model.as_deref())
        .map(crate::tokens::short_model)
        .unwrap_or_default();
    let lw = area.width * 57 / 100;
    let left = Rect::new(area.x, area.y, lw, area.height);
    let tok = l.map_or(String::new(), |l| {
        format!(" · {} tok", fmt_tokens(l.total()))
    });
    let inner = frame(
        buf,
        left,
        vec![
            Span::styled(g.clone(), gs),
            Span::styled(format!(" {}", a.label()), st.add_modifier(Modifier::BOLD)),
            Span::styled(format!(" · {model} · {}", a.kind), dim()),
        ],
        vec![Span::styled(
            format!(
                "{} · {} calls{tok}",
                crate::panel::dur(i.now.saturating_sub(a.started)),
                a.calls
            ),
            dim(),
        )],
        st,
        true,
    );
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    let xr = inner.right().saturating_sub(2);
    let path = a.transcript.clone().or_else(|| {
        if a.id == "main" {
            i.u.transcript_path.clone()
        } else {
            None
        }
    });
    let head = match a.parent.as_deref() {
        _ if a.id == "main" => "started by you".to_string(),
        Some(p) => format!(
            "spawned by {} {} ago",
            label(i.st, p),
            age(i.now.saturating_sub(a.started))
        ),
        None => format!("started {} ago", age(i.now.saturating_sub(a.started))),
    };
    let head = match path {
        Some(p) => {
            // The last two parts name the file; full paths run off the row anyway.
            let tail: Vec<&str> = p.rsplit('/').take(2).collect();
            format!(
                "{head} · own context · …/{}/{}",
                tail.get(1).unwrap_or(&""),
                tail[0]
            )
        }
        None => format!("{head} · own context"),
    };
    put(buf, x, inner.y, &head, w, faint());
    let prompt = if a.id == "main" {
        i.st.label.clone()
    } else {
        a.prompt.clone()
    };
    put(
        buf,
        x,
        inner.y + 1,
        &format!("prompt  {}", prompt.unwrap_or_else(|| "—".into())),
        w,
        dim(),
    );
    rule(buf, inner, inner.y + 2);
    let rows = timeline(i, a);
    let room = inner.height.saturating_sub(6) as usize;
    let end = rows
        .len()
        .saturating_sub(scroll.min(rows.len().saturating_sub(room.saturating_sub(1))));
    let (start, note) = if end <= room {
        (0, false)
    } else {
        (end - room.saturating_sub(1), true)
    };
    let mut y = inner.y + 3;
    if note {
        put(
            buf,
            x,
            y,
            &format!("… {start} earlier lines (pgup)"),
            w,
            faint(),
        );
        y += 1;
    }
    for (t, s, r) in &rows[start..end] {
        let rw = r.as_ref().map_or(0, |r| r.0.chars().count() as u16 + 1);
        put(buf, x, y, t, w.saturating_sub(rw), *s);
        if let Some((rt, rs)) = r {
            rput(buf, xr, y, rt, *rs);
        }
        y += 1;
    }
    let by = inner.bottom() - 3;
    rule(buf, inner, by);
    if let Some((since, act)) = &a.waiting {
        let nx = put(
            buf,
            x,
            by + 1,
            "◉ waiting on you  ",
            w,
            msg().add_modifier(Modifier::BOLD),
        );
        put(
            buf,
            nx,
            by + 1,
            act,
            (xr + 1).saturating_sub(nx + 6),
            Style::default(),
        );
        rput(buf, xr, by + 1, &mmss(i.now.saturating_sub(*since)), msg());
    } else if a.done {
        let ago = age(i.now.saturating_sub(a.ended_t.unwrap_or(a.last_t)));
        put(
            buf,
            x,
            by + 1,
            &format!(
                "✓ returned {ago} ago · {}",
                a.result.as_deref().unwrap_or("")
            ),
            w,
            dim(),
        );
    } else {
        put(
            buf,
            x,
            by + 1,
            &format!("{g} {}", a.act.as_deref().unwrap_or("thinking")),
            w,
            bold(),
        );
    }
    let errs = i.st.errors.iter().filter(|e| e.agent == a.id).count();
    let ins = i.st.messages.iter().filter(|m| m.to == a.id).count();
    let outs = i.st.messages.iter().filter(|m| m.from == a.id).count();
    put(
        buf,
        x,
        by + 2,
        &format!(
            "{errs} errors · {ins} messages in · {outs} out · {} files",
            a.files.len()
        ),
        w,
        faint(),
    );

    let rx = left.right();
    let rw = area.right() - rx;
    let ch = 14.min(area.height / 3);
    context(buf, Rect::new(rx, area.y, rw, ch), i, a);
    let mh = (area.height - ch) * 2 / 5;
    messages(buf, Rect::new(rx, area.y + ch, rw, mh), i, a);
    let fh = (area.height - ch - mh) / 2;
    files(buf, Rect::new(rx, area.y + ch + mh, rw, fh), i, a);
    tasks(
        buf,
        Rect::new(rx, area.y + ch + mh + fh, rw, area.height - ch - mh - fh),
        i,
        a,
    );
}

fn cat_style(name: &str) -> Style {
    match name {
        "base" => dim(),
        "messages" => msg(),
        t => tool_style(t),
    }
}

fn context(buf: &mut Buffer, r: Rect, i: &Input, a: &Agent) {
    let size = if a.id == "main" { i.u.ctx_size } else { None }
        .or(i.u.ctx_size)
        .unwrap_or(200_000)
        .max(1);
    let ctx = ctx_of(i, a);
    let right = ctx.map(|(n, _)| {
        vec![Span::styled(
            format!("{} / {}", fmt_tokens(n), fmt_tokens(size)),
            dim(),
        )]
    });
    let inner = frame(
        buf,
        r,
        vec![Span::styled("context window", bold())],
        right.unwrap_or_default(),
        faint(),
        false,
    );
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    let Some((_, pct)) = ctx else {
        put(buf, x, inner.y, "no transcript read yet", w, faint());
        return;
    };
    let s = ctx_style(i, pct);
    let nx = put(buf, x, inner.y, "ctx ", 4, faint());
    let nx = put(
        buf,
        nx,
        inner.y,
        &cells(pct / 100.0, w.saturating_sub(10) as usize, '▰', '▱'),
        w,
        s,
    );
    put(
        buf,
        nx + 1,
        inner.y,
        &format!("{pct:.0}%"),
        5,
        s.add_modifier(Modifier::BOLD),
    );
    let mut note = format!("compacts near {}%", i.cfg.thresholds.compact_pct);
    if a.id == "main"
        && let Some(n) = i.x.compacts_in
    {
        note.push_str(&format!(" · ~{n} turns left"));
    }
    if a.done {
        note = "finished; its context was freed when it returned".into();
    }
    put(buf, x, inner.y + 1, &note, w, dim());
    let Some(l) = ledger(i, a).filter(|l| !l.fill.is_empty()) else {
        return;
    };
    put(buf, x, inner.y + 3, "what filled it", w, bold());
    let mut fill: Vec<(&String, &u64)> = l.fill.iter().collect();
    fill.sort_by(|p, q| (p.0 != "base").cmp(&(q.0 != "base")).then(q.1.cmp(p.1)));
    let mut cx = x;
    for (k, v) in &fill {
        let n = ((**v as f64 / size as f64) * w as f64).round().max(1.0) as u16;
        for _ in 0..n.min((x + w).saturating_sub(cx)) {
            put(buf, cx, inner.y + 4, "█", 1, cat_style(k));
            cx += 1;
        }
    }
    while cx < x + w {
        put(buf, cx, inner.y + 4, "░", 1, faint());
        cx += 1;
    }
    let half = w / 2;
    for (n, (k, v)) in fill.iter().enumerate() {
        let y = inner.y + 5 + (n / 2) as u16;
        if y + 2 >= inner.bottom() {
            break;
        }
        let cx = x + (n % 2) as u16 * half;
        let nx = put(buf, cx, y, "█ ", 2, cat_style(k));
        let name = match k.as_str() {
            "base" => "base: system, tools, CLAUDE.md".to_string(),
            "messages" => "messages and replies".into(),
            t => format!("{t} results"),
        };
        put(buf, nx, y, &name, half.saturating_sub(9), Style::default());
        rput(buf, cx + half - 2, y, &fmt_tokens(**v), dim());
    }
    let y = inner.bottom() - 2;
    put(
        buf,
        x,
        y,
        "estimate: each tool result is the jump in input tokens on the",
        w,
        faint(),
    );
    put(
        buf,
        x,
        y + 1,
        "next call; base is the first call",
        w,
        faint(),
    );
}

fn messages(buf: &mut Buffer, r: Rect, i: &Input, a: &Agent) {
    let ms: Vec<_> =
        i.st.messages
            .iter()
            .filter(|m| m.from == a.id || m.to == a.id)
            .collect();
    let ins = ms.iter().filter(|m| m.to == a.id).count();
    let right = vec![Span::styled(
        format!("{ins} in · {} out", ms.len() - ins),
        dim(),
    )];
    let inner = frame(
        buf,
        r,
        vec![Span::styled("messages", bold())],
        right,
        faint(),
        false,
    );
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    if ms.is_empty() {
        put(buf, x, inner.y, "none", w, faint());
    }
    let skip = ms.len().saturating_sub(inner.height as usize);
    for (n, m) in ms.iter().skip(skip).enumerate() {
        let (dir, who) = if m.from == a.id {
            ("→", label(i.st, &m.to))
        } else {
            ("←", label(i.st, &m.from))
        };
        let y = inner.y + n as u16;
        let nx = put(
            buf,
            x,
            y,
            &format!("{:>4} {dir} {who:<9} ", age(i.now.saturating_sub(m.t))),
            w,
            msg(),
        );
        put(
            buf,
            nx,
            y,
            &m.text,
            (x + w).saturating_sub(nx),
            if m.from == a.id {
                dim()
            } else {
                Style::default()
            },
        );
    }
}

fn files(buf: &mut Buffer, r: Rect, i: &Input, a: &Agent) {
    let inner = frame(
        buf,
        r,
        vec![Span::styled("files", bold())],
        vec![Span::styled("edits from hooks", dim())],
        faint(),
        false,
    );
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    if a.files.is_empty() {
        put(buf, x, inner.y, "no edits", w, faint());
    }
    let shared = shared_files(i.st);
    for (n, (p, f)) in a.files.iter().take(inner.height as usize).enumerate() {
        let y = inner.y + n as u16;
        let others: Vec<String> = shared
            .iter()
            .find(|(sp, _)| sp == p)
            .map(|(_, who)| {
                who.iter()
                    .map(|w| w.0.clone())
                    .filter(|l| *l != a.label())
                    .collect()
            })
            .unwrap_or_default();
        let tail = if others.is_empty() {
            String::new()
        } else {
            format!("⚠ {} edited it too", others.join(", "))
        };
        let nx = put(buf, x, y, p, w / 2, Style::default());
        let nx = put(
            buf,
            nx.max(x + w / 2),
            y,
            &format!(" +{}", f.added),
            8,
            ok(),
        );
        let nx = put(buf, nx, y, &format!(" −{}  ", f.removed), 8, err());
        put(buf, nx, y, &tail, (x + w).saturating_sub(nx), warn());
    }
}

fn tasks(buf: &mut Buffer, r: Rect, i: &Input, a: &Agent) {
    let inner = frame(
        buf,
        r,
        vec![Span::styled("tasks & errors", bold())],
        vec![],
        faint(),
        false,
    );
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    let mut y = inner.y;
    let me = a.label();
    for t in
        i.st.tasks
            .iter()
            .filter(|t| t.owner.as_deref() == Some(me.as_str()))
    {
        if y + 1 >= inner.bottom() {
            break;
        }
        let (gl, gs) = match t.status.as_str() {
            "completed" => ("✓".to_string(), ok()),
            "pending" => ("○".to_string(), dim()),
            _ => glyph(i, a),
        };
        let nx = put(buf, x, y, &format!("{gl} "), 2, gs);
        put(
            buf,
            nx,
            y,
            &format!("#{} {}", t.id, t.subject),
            w.saturating_sub(2),
            Style::default(),
        );
        y += 1;
    }
    let errs: Vec<_> = i.st.errors.iter().filter(|e| e.agent == a.id).collect();
    if errs.is_empty() {
        put(buf, x, y.min(inner.bottom() - 1), "no errors", w, faint());
        return;
    }
    for e in errs {
        if y + 1 >= inner.bottom() {
            break;
        }
        put(buf, x, y, &format!("✗ {} · {}", e.tool, e.msg), w, err());
        let state = match (e.resolved_t, &e.resolved_by) {
            (Some(t), by) => (
                format!(
                    "fixed {} ago by {}",
                    age(i.now.saturating_sub(t)),
                    by.as_deref().map_or("?".into(), |b| label(i.st, b))
                ),
                ok(),
            ),
            _ => (format!("open · {} retries", e.retries), err()),
        };
        put(buf, x + 2, y + 1, &state.0, w.saturating_sub(2), state.1);
        y += 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::tests::{kid, with_main};
    use crate::state::{CallRec, Msg};

    fn show(st: &State, x: &crate::bar::Extras, k: usize, scroll: usize) -> Vec<String> {
        let (u, cfg) = Default::default();
        let i = Input {
            st,
            u: &u,
            x,
            cfg: &cfg,
            now: 100_000,
            clock: "",
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 168, 46));
        let area = buf.area;
        draw(&mut buf, area, &i, &st.agents[k], scroll);
        crate::panel::to_plain(&crate::hub::to_lines(&buf, false))
            .lines()
            .map(String::from)
            .collect()
    }
    fn call(t: u64, act: &str) -> CallRec {
        CallRec {
            t,
            tool: "Read".into(),
            act: format!("reading {act}"),
            ok: Some(true),
            dur: Some(200),
            ..Default::default()
        }
    }
    fn row(out: &[String], s: &str) -> usize {
        out.iter()
            .position(|l| l.contains(s))
            .unwrap_or_else(|| panic!("{s} missing:\n{}", out.join("\n")))
    }

    #[test]
    fn zoom_merges_calls_and_messages() {
        let mut st = with_main(vec![kid("a1", "main", 1)]);
        st.agents[1].recent.extend([
            call(10_000, "one.rs"),
            call(30_000, "two.rs"),
            call(50_000, "three.rs"),
        ]);
        st.messages.push_back(Msg {
            t: 20_000,
            from: "main".into(),
            to: "a1".into(),
            text: "hello there".into(),
        });
        st.messages.push_back(Msg {
            t: 40_000,
            from: "a1".into(),
            to: "main".into(),
            text: "done here".into(),
        });
        let out = show(&st, &Default::default(), 1, 0);
        let order =
            ["one.rs", "hello there", "two.rs", "done here", "three.rs"].map(|s| row(&out, s));
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    }

    #[test]
    fn zoom_scrolls() {
        let mut st = with_main(vec![kid("a1", "main", 1)]);
        st.agents[1]
            .recent
            .extend((0..60).map(|n| call(n * 1000, &format!("f{n:02}.rs"))));
        let out = show(&st, &Default::default(), 1, 0);
        assert!(
            out.iter().any(|l| l.contains("earlier lines")),
            "{}",
            out.join("\n")
        );
        assert!(out.iter().any(|l| l.contains("f59.rs")));
        let up = show(&st, &Default::default(), 1, 10);
        assert!(
            !up.iter()
                .any(|l| l.contains("f59.rs") || l.contains("f50.rs"))
        );
        assert!(up.iter().any(|l| l.contains("f49.rs")));
    }

    #[test]
    fn zoom_tasks_stay_in_their_pane() {
        let mut st = with_main(vec![kid("a1", "main", 1)]);
        for n in 0..20 {
            st.tasks.push(crate::state::Task {
                id: n.to_string(),
                subject: format!("job {n}"),
                status: "pending".into(),
                owner: Some("a1".into()),
                ..Default::default()
            });
        }
        let out = show(&st, &Default::default(), 1, 0);
        let last = out.last().unwrap();
        assert!(
            !last.contains("job"),
            "tasks ran past the pane:\n{}",
            out.join("\n")
        );
        assert!(last.trim_end().ends_with('╯'), "{last}");
    }

    #[test]
    fn zoom_done_agent_returned() {
        let mut st = with_main(vec![kid("a1", "main", 1)]);
        st.agents[1].done = true;
        st.agents[1].ended_t = Some(90_000);
        st.agents[1].result = Some("3 call sites".into());
        let out = show(&st, &Default::default(), 1, 0);
        assert!(
            out.iter()
                .any(|l| l.contains("returned 10s ago") && l.contains("3 call sites")),
            "{}",
            out.join("\n")
        );
    }

    #[test]
    fn zoom_fill_is_labelled_estimate() {
        let st = with_main(vec![kid("a1", "main", 1)]);
        let mut l = crate::tokens::Ledger {
            ctx: 60_000,
            ..Default::default()
        };
        l.fill.insert("base".into(), 20_000);
        l.fill.insert("Read".into(), 40_000);
        let mut t = crate::tokens::TokenCache::default();
        t.agents.insert("a1".into(), l);
        let x = crate::bar::Extras {
            tokens: Some(t),
            ..Default::default()
        };
        let out = show(&st, &x, 1, 0).join("\n");
        assert!(
            out.contains("what filled it") && out.contains("Read") && out.contains("40k"),
            "{out}"
        );
        assert!(out.contains("estimate"), "{out}");
        assert!(out.contains("30%"), "{out}");
    }
}

//! The message log, activity lanes and the tasks board.

use super::{Input, bold, dim, faint, frame, glyph, msg, node_style, ok, order, put, rput, warn};
use crate::bar::SPINNER;
use crate::panel::age;
use crate::state::{Agent, LANE_MS, LANES, State};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

const SPARK: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

fn label(st: &State, id: &str) -> String {
    st.agents
        .iter()
        .find(|a| a.id == id)
        .map_or(id.to_string(), Agent::label)
}

/// Messages between agents, newest last; the newest is bold for 2.5 s.
pub(crate) fn messages(buf: &mut Buffer, area: Rect, i: &Input) {
    let st = i.st;
    let right = vec![Span::styled(
        format!("{} this session · newest last", st.messages.len()),
        dim(),
    )];
    let inner = frame(
        buf,
        area,
        vec![Span::styled("messages", bold())],
        right,
        faint(),
        false,
    );
    if st.messages.is_empty() {
        put(
            buf,
            inner.x + 1,
            inner.y,
            "no messages yet: agents talk with SendMessage",
            inner.width.saturating_sub(2),
            faint(),
        );
        return;
    }
    let rows = inner.height as usize;
    let skip = st.messages.len().saturating_sub(rows);
    let (x0, w) = (inner.x + 1, inner.width.saturating_sub(2));
    for (n, m) in st.messages.iter().skip(skip).enumerate() {
        let y = inner.y + n as u16;
        let fresh = n + skip + 1 == st.messages.len() && i.now.saturating_sub(m.t) < 2500;
        let base = if fresh {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut x = put(
            buf,
            x0,
            y,
            &format!("{:>4}  ", age(i.now.saturating_sub(m.t))),
            6,
            faint(),
        );
        x = put(
            buf,
            x,
            y,
            &format!("{:<9}", super::clip(&label(st, &m.from), 9)),
            9,
            bold(),
        );
        x = put(buf, x, y, " ━▸ ", 4, msg());
        x = put(
            buf,
            x,
            y,
            &format!("{:<9} ", super::clip(&label(st, &m.to), 9)),
            10,
            Style::default(),
        );
        put(buf, x, y, &m.text, (x0 + w).saturating_sub(x), base);
    }
}

/// Tool calls per agent over the last `LANES` × 12 s, scaled to the busiest cell.
pub(crate) fn lanes(buf: &mut Buffer, area: Rect, i: &Input) {
    let st = i.st;
    let inner = frame(
        buf,
        area,
        vec![Span::styled("activity", bold())],
        vec![Span::styled("last 10 min · 12 s a cell", dim())],
        faint(),
        false,
    );
    let n = (inner.width.saturating_sub(14) as usize).min(LANES);
    if n == 0 || inner.height < 2 {
        return;
    }
    let nb = i.now / LANE_MS;
    let first = (nb + 1).saturating_sub(n as u64);
    let count = |a: &Agent, b: u64| {
        a.lane
            .iter()
            .find(|(lb, _)| *lb == b)
            .map_or(0, |(_, c)| *c as u64)
    };
    let ord = order(st);
    let rows = &ord[..ord.len().min(inner.height as usize - 1)];
    let max = rows
        .iter()
        .flat_map(|&k| (first..=nb).map(move |b| count(&st.agents[k], b)))
        .max()
        .unwrap_or(0)
        .max(1);
    let x0 = inner.x + 1;
    for (r, &k) in rows.iter().enumerate() {
        let a = &st.agents[k];
        let y = inner.y + r as u16;
        let s = if a.done { faint() } else { node_style(i, a) };
        put(
            buf,
            x0,
            y,
            &format!("{:<9} ", super::clip(&a.label(), 9)),
            10,
            s,
        );
        for (j, b) in (first..=nb).enumerate() {
            let c = count(a, b);
            if c > 0 {
                let lvl = (1 + c * 7 / max).min(8) as usize;
                put(buf, x0 + 10 + j as u16, y, SPARK[lvl - 1], 1, s);
            }
        }
        let (g, gs) = glyph(i, a);
        put(buf, x0 + 11 + n as u16, y, &g, 1, gs);
    }
    let y = inner.bottom() - 1;
    let x = x0 + 10;
    put(
        buf,
        x,
        y,
        &format!("−{}m", n as u64 * LANE_MS / 60_000),
        5,
        faint(),
    );
    put(
        buf,
        x + n as u16 / 2 - 2,
        y,
        &format!("−{}m", n as u64 * LANE_MS / 120_000),
        5,
        faint(),
    );
    rput(buf, x + n as u16 - 1, y, "now", faint());
}

/// The task list as pending | in progress | done, with owners and what blocks a task.
pub(crate) fn tasks(buf: &mut Buffer, area: Rect, i: &Input) {
    let st = i.st;
    let (done, total) = st.tasks_done();
    let inner = frame(
        buf,
        area,
        vec![Span::styled("tasks", bold())],
        vec![Span::styled(
            format!("shared list · {done}/{total} done"),
            dim(),
        )],
        faint(),
        false,
    );
    if st.tasks.is_empty() {
        put(
            buf,
            inner.x + 1,
            inner.y,
            "no plan yet",
            inner.width.saturating_sub(2),
            faint(),
        );
        return;
    }
    let cw = area.width / 3;
    for c in 1..3 {
        let x = area.x + c * cw;
        buf[(x, area.y)].set_symbol("┬");
        for y in inner.y..inner.bottom() {
            buf[(x, y)].set_symbol("│").set_style(faint());
        }
        buf[(x, area.bottom() - 1)].set_symbol("┴");
    }
    let open = |id: &str| {
        st.tasks
            .iter()
            .any(|t| t.id == id && t.status != "completed")
    };
    let cols = [
        ("pending", dim()),
        ("in_progress", super::agent()),
        ("completed", ok()),
    ];
    for (c, (status, hs)) in cols.iter().enumerate() {
        let x = area.x + c as u16 * cw + 2;
        let w = cw.saturating_sub(3);
        let items: Vec<_> = st.tasks.iter().filter(|t| t.status == *status).collect();
        let head = match *status {
            "in_progress" => "in progress",
            "completed" => "done",
            s => s,
        };
        put(
            buf,
            x,
            inner.y,
            &format!("{head}  {}", items.len()),
            w,
            hs.add_modifier(Modifier::BOLD),
        );
        for (n, t) in items.iter().enumerate() {
            let y = inner.y + 2 + n as u16 * 3;
            if y + 1 >= inner.bottom() {
                put(
                    buf,
                    x,
                    y.min(inner.bottom() - 1),
                    &format!("+{} more", items.len() - n),
                    w,
                    faint(),
                );
                break;
            }
            let ss = if *status == "completed" {
                dim()
            } else {
                Style::default()
            };
            let nx = put(buf, x, y, &format!("#{} ", t.id), w, faint());
            put(buf, nx, y, &t.subject, w.saturating_sub(nx - x), ss);
            let blocked: Vec<&String> = t.blocked.iter().filter(|b| open(b)).collect();
            let (line, ls) = if *status == "pending" && !blocked.is_empty() {
                let ids: Vec<String> = blocked.iter().map(|b| format!("#{b}")).collect();
                (format!("⊘ blocked by {}", ids.join(", ")), warn())
            } else if let Some(o) = &t.owner {
                match st.agent_by_name(o) {
                    Some(a) => (format!("{o} {}", glyph(i, a).0), node_style(i, a)),
                    None => (o.clone(), dim()),
                }
            } else {
                ("unassigned".into(), faint())
            };
            put(buf, x + 3, y + 1, &line, w.saturating_sub(3), ls);
        }
    }
    let _ = (SPINNER, msg);
}

/// `b`: the watch panel's git rows, branch and working tree in the title.
pub(crate) fn git(buf: &mut Buffer, area: Rect, i: &Input) {
    let mut rows = crate::panel::git_rows(i.x, i.now);
    let right = if rows.is_empty() {
        vec![]
    } else {
        rows.remove(0).split_off(1)
    };
    let inner = frame(
        buf,
        area,
        vec![Span::styled("git", bold())],
        right,
        faint(),
        false,
    );
    let (x, w) = (inner.x + 1, inner.width.saturating_sub(2));
    if i.x.git.is_none() {
        put(buf, x, inner.y, "no repo here, or still loading", w, faint());
    }
    for (n, r) in rows.into_iter().take(inner.height as usize).enumerate() {
        buf.set_line(x, inner.y + n as u16, &r.into(), w);
    }
}

/// `b`: each agent's edits that git still shows as changed, then changes no agent made.
pub(crate) fn uncommitted(buf: &mut Buffer, area: Rect, i: &Input) {
    let changed = i.x.git.as_ref().map_or(&[][..], |g| &g.changed[..]);
    let inner = frame(
        buf,
        area,
        vec![Span::styled("uncommitted", bold())],
        vec![Span::styled(format!("{} files · by agent", changed.len()), dim())],
        faint(),
        false,
    );
    let (x0, w) = (inner.x + 1, inner.width.saturating_sub(2));
    if changed.is_empty() {
        put(buf, x0, inner.y, "working tree clean", w, faint());
        return;
    }
    let mut rows: Vec<(String, Style, Vec<&str>)> = order(i.st)
        .into_iter()
        .map(|k| &i.st.agents[k])
        .map(|a| {
            let mine: Vec<&str> = changed
                .iter()
                .filter(|c| a.files.contains_key(*c))
                .map(String::as_str)
                .collect();
            let s = if a.done { faint() } else { node_style(i, a) };
            (a.label(), s, mine)
        })
        .filter(|r| !r.2.is_empty())
        .collect();
    let other: Vec<&str> = changed
        .iter()
        .filter(|c| !i.st.agents.iter().any(|a| a.files.contains_key(*c)))
        .map(String::as_str)
        .collect();
    if !other.is_empty() {
        rows.push(("other".into(), dim(), other));
    }
    for (n, (who, s, files)) in rows.iter().take(inner.height as usize).enumerate() {
        let y = inner.y + n as u16;
        let x = put(buf, x0, y, &format!("{:<9} ", super::clip(who, 9)), 10, *s);
        let x = put(buf, x, y, &format!("{:>3}  ", files.len()), 5, warn());
        put(buf, x, y, &files.join(", "), (x0 + w).saturating_sub(x), Style::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::tests::{kid, with_main};
    use crate::state::{LANE_MS, Msg, Task};

    fn draw_with(
        st: &State,
        now: u64,
        w: u16,
        h: u16,
        f: impl Fn(&mut Buffer, Rect, &Input),
    ) -> Vec<String> {
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
        f(&mut buf, area, &i);
        crate::panel::to_plain(&crate::hub::to_lines(&buf, false))
            .lines()
            .map(String::from)
            .collect()
    }

    #[test]
    fn messages_newest_last() {
        let mut st = with_main(vec![kid("a1", "main", 1)]);
        for n in 0..8 {
            st.messages.push_back(Msg {
                t: n * 1000,
                from: "a1".into(),
                to: "main".into(),
                text: format!("m{n}"),
            });
        }
        st.messages.push_back(Msg {
            t: 9000,
            from: "a1".into(),
            to: "ghost".into(),
            text: "last".into(),
        });
        let out = draw_with(&st, 10_000, 100, 8, messages);
        let rows: Vec<&String> = out[1..7].iter().collect();
        assert!(rows[0].contains("m3"), "{out:#?}");
        assert!(
            rows[5].contains("ghost") && rows[5].contains("last"),
            "{out:#?}"
        );
        assert!(!out.iter().any(|l| l.contains("m2")));
    }

    #[test]
    fn lanes_scale_to_busiest() {
        let mut st = with_main(vec![kid("a1", "main", 1)]);
        let now = 100 * LANE_MS;
        st.agents[0].lane.push_back((100, 1));
        st.agents[1].lane.push_back((100, 4));
        let out = draw_with(&st, now, 68, 8, lanes);
        let row = |name: &str| {
            out.iter()
                .find(|l| l.contains(&format!(" {name} ")))
                .unwrap()
                .clone()
        };
        assert!(row("main").contains('▂'), "{out:#?}");
        assert!(row("a1").contains('█'), "{out:#?}");
    }

    #[test]
    fn tasks_board_owner_blocked() {
        let mut st = with_main(vec![kid("impl", "main", 1)]);
        let t = |id: &str, status: &str| Task {
            id: id.into(),
            subject: format!("task {id}"),
            status: status.into(),
            ..Default::default()
        };
        st.tasks.push(Task {
            owner: Some("impl".into()),
            ..t("4", "in_progress")
        });
        st.tasks.push(Task {
            blocked: vec!["4".into()],
            ..t("7", "pending")
        });
        st.tasks.push(t("1", "completed"));
        let out = draw_with(&st, 1, 168, 12, tasks);
        let col = 168 / 3;
        let at = |s: &str| {
            out.iter()
                .find_map(|l| l.find(s).map(|b| l[..b].chars().count()))
                .unwrap_or_else(|| panic!("{s} missing: {out:#?}"))
        };
        assert!(at("⊘ blocked by #4") < col);
        assert!((col..2 * col).contains(&at("impl")));
        assert!(at("#1 task 1") >= 2 * col);
    }

    #[test]
    fn uncommitted_by_agent() {
        let mut st = with_main(vec![kid("a1", "main", 1)]);
        st.agents[1].files.insert("src/x.rs".into(), Default::default());
        st.agents[1].files.insert("src/done.rs".into(), Default::default());
        let x = crate::bar::Extras {
            git: Some(crate::git::Git {
                branch: "main".into(),
                changed: vec!["src/x.rs".into(), "notes.md".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let (u, cfg) = Default::default();
        let i = Input {
            st: &st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now: 1,
            clock: "",
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 70, 6));
        let area = buf.area;
        uncommitted(&mut buf, area, &i);
        let out = crate::panel::to_plain(&crate::hub::to_lines(&buf, false));
        let row = |who: &str| out.lines().find(|l| l.contains(who)).unwrap().to_string();
        assert!(row("a1").contains("1  src/x.rs"), "{out}");
        assert!(!out.contains("done.rs"), "{out}");
        assert!(row("other").contains("notes.md"), "{out}");
        assert!(!out.lines().any(|l| l.contains("main ")), "{out}");
    }
}

//! The full panel, as styled lines. Pure: everything `ari watch` draws comes from here.

use crate::bar::{Extras, cells, countdown, fmt_tokens, spinner};
use crate::config::Config;
use crate::recap::Recap;
use crate::state::{AgentStatus, ErrorRec, State, Verdict};
use crate::usage::{Limit, Usage};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View {
    pub expand_errors: bool,
    pub expand_agents: bool,
    /// Show the recap (`Extras::recap`) in place of the sections.
    pub recap: bool,
}

type Row = Vec<Span<'static>>;

struct Section {
    title: Option<&'static str>,
    rows: Vec<Row>,
}

struct Ctx {
    color: bool,
    inner: usize,
}

impl Ctx {
    fn s(&self, style: Style) -> Style {
        if self.color { style } else { Style::default() }
    }
    fn span(&self, text: impl Into<String>, style: Style) -> Span<'static> {
        Span::styled(text.into(), self.s(style))
    }
    fn dim(&self, text: impl Into<String>) -> Span<'static> {
        self.span(text, Style::new().add_modifier(Modifier::DIM))
    }
}

pub fn face(v: Verdict) -> &'static str {
    match v {
        Verdict::OnTrack => "(•_•)",
        Verdict::NeedsYou => "(◉_◉)",
        Verdict::Blocked | Verdict::Retrying | Verdict::Stuck => "(>_<)",
        Verdict::Done => "(^_^)",
        Verdict::Idle => "(-_-)zz",
    }
}

pub fn verdict_style(v: Verdict) -> Style {
    let s = Style::new().add_modifier(Modifier::BOLD);
    match v {
        Verdict::NeedsYou => s.fg(Color::Magenta).add_modifier(Modifier::REVERSED),
        Verdict::Blocked => s.fg(Color::Red),
        Verdict::Retrying | Verdict::Stuck => s.fg(Color::Yellow),
        Verdict::OnTrack => s.fg(Color::Green),
        Verdict::Done => s.fg(Color::Cyan),
        Verdict::Idle => s.add_modifier(Modifier::DIM),
    }
}

fn level(pct: f64, cfg: &Config) -> Style {
    Style::new().fg(if pct >= cfg.thresholds.ctx_crit as f64 {
        Color::Red
    } else if pct >= cfg.thresholds.ctx_warn as f64 {
        Color::Yellow
    } else {
        Color::Green
    })
}

/// `/Users/me/ari/` → `~/ari`.
pub fn tilde(dir: &str) -> String {
    let dir = dir.trim_end_matches('/');
    let home = std::env::var("HOME").unwrap_or_default();
    match dir.strip_prefix(home.trim_end_matches('/')) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => dir.to_string(),
    }
}

/// Local wall-clock `HH:MM`.
pub fn local_hhmm(ms: u64) -> String {
    #[cfg(unix)]
    {
        let t = (ms / 1000) as libc::time_t;
        // SAFETY: localtime_r only writes into the tm we own.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
        }
    }
    let s = ms / 1000;
    format!("{:02}:{:02}", s / 3600 % 24, s / 60 % 60)
}

pub fn dur(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m{:02}s", s / 60, s % 60),
        _ => format!("{}h{:02}m", s / 3600, s / 60 % 60),
    }
}

pub fn age(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

fn tokens_k(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else {
        format!("{}k", n / 1000)
    }
}

/// The error's signature in words: `cargo build`, `edit main.rs`, `api rate_limit`.
pub fn sig_words(e: &ErrorRec) -> String {
    let (tool, key) = e.sig.split_once(':').unwrap_or((&e.sig, ""));
    match tool {
        "Bash" | "PowerShell" => key.to_string(),
        "StopFailure" => format!("api {key}"),
        _ if key.is_empty() => tool.to_lowercase(),
        _ => format!(
            "{} {}",
            tool.to_lowercase(),
            key.rsplit('/').next().unwrap_or(key)
        ),
    }
}

fn row_width(row: &[Span]) -> usize {
    row.iter().map(|s| s.content.width()).sum()
}

/// Cut a row to `max` display columns, ending in `…` when anything was dropped.
fn fit(row: Row, max: usize) -> Row {
    if row_width(&row) <= max {
        return row;
    }
    let mut out = Vec::new();
    let mut used = 0;
    for span in row {
        let w = span.content.width();
        if used + w < max {
            used += w;
            out.push(span);
            continue;
        }
        let mut text = String::new();
        for c in span.content.chars() {
            let cw = c.width().unwrap_or(0);
            if used + cw + 1 > max {
                break;
            }
            used += cw;
            text.push(c);
        }
        text.push('…');
        out.push(Span::styled(text, span.style));
        break;
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn lines(
    st: &State,
    u: &Usage,
    x: &Extras,
    cfg: &Config,
    view: View,
    now: u64,
    clock: &str,
    width: u16,
    height: u16,
    color: bool,
) -> Vec<Line<'static>> {
    let w = width as usize;
    let c = Ctx {
        color,
        inner: w.saturating_sub(4),
    };
    let v = st.verdict_with(now, cfg.thresholds.stuck_repeats);
    if w < 12 {
        let row = vec![c.span(format!("{} {}", v.glyph(), v.label()), verdict_style(v))];
        let out = vec![Line::from(fit(row, w))];
        return if cfg.ascii {
            crate::ascii::map_lines(out)
        } else {
            out
        };
    }

    let mut sections = vec![
        Section {
            title: None,
            rows: {
                let mut rows = header(
                    st,
                    x.plan.as_ref(),
                    u.session_name.as_deref(),
                    x.looks_active(),
                    &c,
                    v,
                    cfg.thresholds.stuck_repeats,
                );
                if let Some(s) = crate::conflicts::summary(&x.conflicts) {
                    rows.insert(
                        1.min(rows.len()),
                        vec![c.span(
                            s,
                            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                        )],
                    );
                }
                if let Some(b) = badges(st, u, &c) {
                    rows.insert(1.min(rows.len()), b);
                }
                rows
            },
        },
        Section {
            title: Some("agents"),
            rows: agents(st, x, &c, view, now),
        },
        Section {
            title: Some("errors"),
            rows: errors(st, &c, view, now),
        },
        Section {
            title: Some("usage"),
            rows: usage(st, u, x, cfg, &c, now),
        },
        Section {
            title: Some("session"),
            rows: session(st, u, x, cfg, &c, now),
        },
    ];
    if view.recap
        && let Some(r) = &x.recap
    {
        sections.truncate(1);
        sections.push(Section {
            title: Some("recap (r to close)"),
            rows: vec![vec![c.dim(format!("since {}", r.since))]],
        });
        sections.extend(recap_sections(r, &c, false));
    }
    collapse(&mut sections, height as usize);

    let border = c.dim("");
    let bstyle = border.style;
    // ╭─ ARI (•_•) ──── 12:41 ─╮
    let mut title = vec![
        Span::styled("╭─ ", bstyle),
        c.span("ARI", Style::new().add_modifier(Modifier::BOLD)),
    ];
    if cfg.face {
        title.push(Span::raw(" "));
        title.push(c.span(
            face(v),
            verdict_style(v).remove_modifier(Modifier::REVERSED),
        ));
    }
    let right = format!(" {clock} ─╮");
    // Which repo this panel is following, so several sessions can't be confused.
    let mut tail = Vec::new();
    if let Some(d) = &u.dir {
        let room = w.saturating_sub(row_width(&title) + right.width() + 5);
        if room >= 4 {
            tail.push(Span::raw(" "));
            tail.extend(fit(
                vec![c.span(
                    format!("cd {}", tilde(d)),
                    Style::new().add_modifier(Modifier::BOLD),
                )],
                room,
            ));
            tail.push(Span::styled(" ·", bstyle));
        }
    }
    title.push(Span::raw(" "));
    let fill = w.saturating_sub(row_width(&title) + row_width(&tail) + right.width());
    title.push(Span::styled("─".repeat(fill), bstyle));
    title.extend(tail);
    title.push(Span::styled(right, bstyle));
    frame(title, sections, &c, w, cfg.ascii)
}

/// The box around the sections: `title` is the full top row, then `├─ name ─┤` headers,
/// `│ row │` lines and the bottom edge.
fn frame(title: Row, sections: Vec<Section>, c: &Ctx, w: usize, ascii: bool) -> Vec<Line<'static>> {
    let bstyle = c.dim("").style;
    let mut out = vec![Line::from(fit(title, w))];
    for sec in sections {
        if let Some(t) = sec.title {
            let head = format!("├─ {t} ");
            let fill = w.saturating_sub(head.width() + 1);
            out.push(Line::from(vec![Span::styled(
                format!("{head}{}┤", "─".repeat(fill)),
                bstyle,
            )]));
        }
        for row in sec.rows {
            let row = fit(row, c.inner);
            let pad = c.inner.saturating_sub(row_width(&row));
            let mut spans = vec![Span::styled("│ ", bstyle)];
            spans.extend(row);
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(" │", bstyle));
            out.push(Line::from(spans));
        }
    }
    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(w.saturating_sub(2))),
        bstyle,
    )));
    if ascii {
        crate::ascii::map_lines(out)
    } else {
        out
    }
}

/// `ari recap` as a box: `╭─ ARI recap · since start ── cd ~/proj ─╮`, then the sections.
pub fn recap_lines(r: &Recap, width: u16, color: bool, ascii: bool) -> Vec<Line<'static>> {
    let w = (width as usize).max(12);
    let c = Ctx {
        color,
        inner: w.saturating_sub(4),
    };
    let bstyle = c.dim("").style;
    let mut title = vec![
        Span::styled("╭─ ", bstyle),
        c.span("ARI recap", Style::new().add_modifier(Modifier::BOLD)),
        c.dim(format!(" · since {} ", r.since)),
    ];
    let right = " ─╮";
    let mut tail = Vec::new();
    let room = w.saturating_sub(row_width(&title) + right.width() + 2);
    if !r.dir.is_empty() && room >= 4 {
        tail = fit(
            vec![c.span(
                format!(" cd {}", tilde(&r.dir)),
                Style::new().add_modifier(Modifier::BOLD),
            )],
            room,
        );
    }
    let fill = w.saturating_sub(row_width(&title) + row_width(&tail) + right.width());
    title.push(Span::styled("─".repeat(fill), bstyle));
    title.extend(tail);
    title.push(Span::styled(right, bstyle));
    frame(title, recap_sections(r, &c, true), &c, w, ascii)
}

/// The recap's sections; `header` adds the verdict and milestone rows (the panel has its own).
fn recap_sections(r: &Recap, c: &Ctx, header: bool) -> Vec<Section> {
    let mut out = Vec::new();
    if header {
        let mut rows =
            vec![vec![c.span(
                r.verdict.clone(),
                Style::new().add_modifier(Modifier::BOLD),
            )]];
        if let Some(t) = &r.title {
            rows.push(vec![c.dim(format!("milestone: {t}"))]);
        }
        out.push(Section { title: None, rows });
    }
    if r.events == 0 {
        out.push(Section {
            title: None,
            rows: vec![vec![
                c.dim(format!("nothing happened since {}", local_hhmm(r.from))),
            ]],
        });
        return out;
    }
    let ok = |t: String| vec![c.span("✓ ", Style::new().fg(Color::Green)), Span::raw(t)];
    let total = r.done.len() + r.open.len();
    if total > 0 {
        let mut rows = vec![vec![
            c.span(
                cells(r.done.len() as f64 / total as f64, 20, '█', '░'),
                Style::new().fg(Color::Green),
            ),
            Span::raw(format!(" {}/{total} tasks", r.done.len())),
        ]];
        rows.extend(r.done.iter().map(|t| ok(t.clone())));
        rows.extend(
            r.open
                .iter()
                .map(|t| vec![c.dim("◻ "), Span::raw(t.clone())]),
        );
        out.push(Section {
            title: Some("tasks"),
            rows,
        });
    }
    if r.errors_hit + r.fixed.len() + r.still_open.len() > 0 {
        let mut rows = vec![vec![Span::raw(format!(
            "{} hit · {} fixed · {} open",
            r.errors_hit,
            r.fixed.len(),
            r.still_open.len()
        ))]];
        rows.extend(
            r.fixed
                .iter()
                .map(|(what, by)| ok(format!("{what} · fixed by {by}"))),
        );
        rows.extend(r.still_open.iter().map(|what| {
            vec![
                c.span("✗ ", Style::new().fg(Color::Red)),
                Span::raw(what.clone()),
            ]
        }));
        out.push(Section {
            title: Some("errors"),
            rows,
        });
    }
    if !r.files.is_empty() {
        let mut rows: Vec<Row> = r
            .files
            .iter()
            .take(8)
            .map(|(f, n)| {
                let mut row = vec![Span::raw(f.clone())];
                if *n > 1 {
                    row.push(c.dim(format!(" ×{n}")));
                }
                row
            })
            .collect();
        if r.files.len() > 8 {
            rows.push(vec![c.dim(format!("and {} more", r.files.len() - 8))]);
        }
        out.push(Section {
            title: Some("files"),
            rows,
        });
    }
    if !r.commits.is_empty() {
        let rows = r
            .commits
            .iter()
            .map(|line| match line.split_once(' ') {
                Some((hash, msg)) => vec![
                    c.span(hash.to_string(), Style::new().fg(Color::Yellow)),
                    Span::raw(format!(" {msg}")),
                ],
                None => vec![Span::raw(line.clone())],
            })
            .collect();
        out.push(Section {
            title: Some("commits"),
            rows,
        });
    }
    let mut rows = Vec::new();
    let mut row = |label: &str, value: String| {
        rows.push(vec![c.dim(format!("{label:<9}")), Span::raw(value)]);
    };
    if let Some(t) = crate::recap::tests_line(r) {
        row("tests", t);
    }
    if let Some(t) = crate::recap::split_line(r) {
        row("time", t);
    }
    let mut cost = Vec::new();
    if let Some(v) = r.cost {
        cost.push(format!("${v:.2}"));
    }
    if let Some((a, d)) = r.lines {
        cost.push(format!("+{a} −{d} lines"));
    }
    if !cost.is_empty() {
        row("cost", cost.join(" · "));
    }
    if !r.models.is_empty() {
        row("models", r.models.join(", "));
    }
    if let Some(m) = crate::recap::mistakes_line(r) {
        row("mistakes", m);
    }
    if !rows.is_empty() {
        out.push(Section {
            title: Some("session"),
            rows,
        });
    }
    out
}

/// Drop or trim sections, lowest priority first, until the panel fits `height` rows.
fn collapse(secs: &mut Vec<Section>, height: usize) {
    let total = |s: &[Section]| {
        2 + s
            .iter()
            .map(|x| x.rows.len() + x.title.is_some() as usize)
            .sum::<usize>()
    };
    for title in ["session", "usage"] {
        if total(secs) > height {
            secs.retain(|s| s.title != Some(title));
        }
    }
    for title in ["errors", "agents"] {
        while total(secs) > height {
            let Some(s) = secs
                .iter_mut()
                .find(|s| s.title == Some(title) && s.rows.len() > 1)
            else {
                break;
            };
            s.rows.pop();
        }
    }
}

fn header(
    st: &State,
    plan: Option<&crate::state::PlanFile>,
    title: Option<&str>,
    active: bool,
    c: &Ctx,
    v: Verdict,
    repeats: u8,
) -> Vec<Row> {
    let mut rows = Vec::new();
    if st.first_t == 0 {
        rows.push(vec![if active {
            c.span(
                "● WORKING · no hooks yet, so no details",
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )
        } else {
            c.dim("◌ no hooks yet")
        }]);
        rows.push(vec![
            c.dim("start a new Claude Code session, or /reload-plugins"),
        ]);
        return rows;
    }
    let left = if v == Verdict::Done && st.tests_failing() {
        c.span(
            format!("{} {} · tests failing", v.glyph(), v.label()),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )
    } else {
        c.span(format!("{} {}", v.glyph(), v.label()), verdict_style(v))
    };
    let mut row = vec![left];
    // Claude Code's own session title beats a truncated first prompt.
    if let Some(label) = title.map(String::from).or_else(|| st.label.clone()) {
        let used = row_width(&row);
        let room = c.inner.saturating_sub(used + 2);
        if room >= 14 {
            let text = format!("milestone: {label}");
            let text = if text.width() > room {
                fit(vec![Span::raw(text)], room)[0].content.to_string()
            } else {
                text
            };
            row.push(Span::raw(" ".repeat(c.inner - used - text.width())));
            row.push(c.dim(text));
        }
    }
    rows.push(row);
    if v == Verdict::Stuck
        && let Some(why) = st.stuck_reason(repeats)
    {
        rows.push(vec![c.span(format!("↻ {why}"), verdict_style(v))]);
    }
    if let Some(msg) = &st.needs_you {
        rows.push(vec![
            c.span(format!("◉ {msg}"), verdict_style(Verdict::NeedsYou)),
        ]);
    }
    let (done, total) = st.tasks_done();
    if total == 0 {
        if let Some(p) = plan {
            let tail = format!(" {}/{} · {}", p.done, p.total, p.name);
            let n = c.inner.saturating_sub(tail.width()).min(20);
            rows.push(vec![
                c.span(
                    cells(p.done as f64 / p.total as f64, n, '█', '░'),
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw(tail),
            ]);
        }
    } else {
        let tail = format!(" {done}/{total} tasks");
        let n = c.inner.saturating_sub(tail.width()).min(20);
        rows.push(vec![
            c.span(
                cells(done as f64 / total as f64, n, '█', '░'),
                Style::new().fg(Color::Cyan),
            ),
            Span::raw(tail),
        ]);
    }
    if let Some(t) = st.now_task() {
        rows.push(vec![
            c.span("▸ now: ", Style::new().fg(Color::Cyan)),
            Span::raw(t.subject.clone()),
        ]);
    }
    rows
}

fn agents(st: &State, x: &Extras, c: &Ctx, view: View, now: u64) -> Vec<Row> {
    let tok = |id: &str| {
        let t = x.tokens.as_ref()?;
        let l = if id == "main" {
            Some(&t.main)
        } else {
            t.agents.get(id)
        }?;
        (l.total() > 0).then(|| format!(" · {} tok", fmt_tokens(l.total())))
    };
    let model = |id: &str| {
        let t = x.tokens.as_ref()?;
        let l = if id == "main" {
            Some(&t.main)
        } else {
            t.agents.get(id)
        }?;
        let m = l.last_model.as_deref().filter(|m| *m != "unknown")?;
        Some(c.dim(format!("{} · ", crate::tokens::short_model(m))))
    };
    let Some(main) = st.agents.first() else {
        return vec![vec![c.dim("no activity yet")]];
    };
    let subs: Vec<_> = st.agents.iter().skip(1).collect();
    // Finished agents worth showing: did some work, and finished in the last 10 minutes.
    let mut done: Vec<_> = subs
        .iter()
        .filter(|a| a.done)
        .filter(|a| a.calls > 0 || a.ended_t.unwrap_or(a.last_t) >= a.started + 1_000)
        .filter(|a| now.saturating_sub(a.ended_t.unwrap_or(a.last_t)) < 10 * 60_000)
        .collect();
    done.sort_by_key(|a| std::cmp::Reverse(a.ended_t));
    let recent_done: Vec<&str> = done.iter().take(3).map(|a| a.id.as_str()).collect();
    let shown: Vec<_> = subs
        .iter()
        .filter(|a| view.expand_agents || !a.done || recent_done.contains(&a.id.as_str()))
        .collect();
    let name_w = std::iter::once(4)
        .chain(shown.iter().map(|a| a.kind.width()))
        .max()
        .unwrap_or(4)
        .min(12);
    let pad = |s: &str| format!("{s:<name_w$}");

    let spin = c.span(spinner(now).to_string(), Style::new().fg(Color::Cyan));
    let main_glyph = if st.turn_active {
        spin.clone()
    } else {
        c.dim("◌")
    };
    // The command running now, with its timer; otherwise thinking or idle.
    let main_act = match (st.tool_running(main, now), st.turn_active) {
        (Some(ms), _) => format!("{} · {}", main.act.as_deref().unwrap_or("working"), dur(ms)),
        (None, true) if st.background > 0 => format!("{} in background", st.background),
        (None, true) => "thinking…".into(),
        (None, false) => "idle".into(),
    };
    let mut rows = vec![vec![
        main_glyph,
        Span::raw(" "),
        c.span(
            format!("{:<w$}", "main", w = name_w + 2),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        model("main").unwrap_or_default(),
        Span::raw(main_act),
        c.dim(tok("main").unwrap_or_default()),
    ]];

    for (i, a) in shown.iter().enumerate() {
        let branch = if i + 1 == shown.len() && shown.len() == subs.len() {
            "└─"
        } else {
            "├─"
        };
        let (glyph, text) = match st.agent_status(a, now) {
            AgentStatus::Running => (
                spin.clone(),
                format!(
                    "{} · {} call{} · {}{}",
                    match st.tool_running(a, now) {
                        Some(ms) =>
                            format!("{} {}", a.act.as_deref().unwrap_or("working"), dur(ms)),
                        None if a.calls == 0 => "starting".into(),
                        None => "thinking…".into(),
                    },
                    a.calls,
                    if a.calls == 1 { "" } else { "s" },
                    dur(now.saturating_sub(a.started)),
                    tok(&a.id).unwrap_or_default()
                ),
            ),
            AgentStatus::Done => (
                c.span("✓", Style::new().fg(Color::Green)),
                match &a.result {
                    Some(r) => format!(
                        "done {}{} · {r}",
                        dur(a.ended_t.unwrap_or(a.last_t).saturating_sub(a.started)),
                        tok(&a.id).unwrap_or_default()
                    ),
                    None => format!(
                        "done {}{}",
                        dur(a.ended_t.unwrap_or(a.last_t).saturating_sub(a.started)),
                        tok(&a.id).unwrap_or_default()
                    ),
                },
            ),
            AgentStatus::Stale => (
                c.span("◌", Style::new().fg(Color::Yellow)),
                format!(
                    "stale {} · {}",
                    age(now.saturating_sub(a.last_t)),
                    a.act.as_deref().unwrap_or("")
                ),
            ),
        };
        rows.push(vec![
            c.dim(branch),
            glyph,
            Span::raw(" "),
            Span::raw(pad(&crate::event::truncate(&a.kind, name_w))),
            Span::raw(" "),
            model(&a.id).unwrap_or_default(),
            Span::raw(text),
        ]);
    }
    let hidden = subs.len() - shown.len();
    if hidden > 0 {
        rows.push(vec![c.dim(format!("└─ … {hidden} more done (a)"))]);
    }
    rows
}

fn errors(st: &State, c: &Ctx, view: View, now: u64) -> Vec<Row> {
    let mut rows = errors_list(st, c, view, now);
    let time: u64 = st.errors.iter().map(|e| e.time_ms).sum();
    if time > 0 || st.reread > 0 {
        let failed: u32 = st.errors.iter().map(|e| e.retries + 1).sum();
        let s = if failed == 1 { "" } else { "s" };
        let mut parts = vec![format!("{failed} failed call{s}")];
        if time >= 1000 {
            parts.push(dur(time));
        }
        if st.reread > 0 {
            let s = if st.reread == 1 { "" } else { "s" };
            parts.push(format!("{} re-read{s}", st.reread));
        }
        rows.push(vec![
            c.dim(format!("cost of mistakes: {}", parts.join(" · "))),
        ]);
    }
    rows
}

fn errors_list(st: &State, c: &Ctx, view: View, now: u64) -> Vec<Row> {
    if st.errors.is_empty() {
        return vec![vec![c.dim("no errors")]];
    }
    let mut errs: Vec<&ErrorRec> = st.errors.iter().collect();
    errs.sort_by_key(|e| std::cmp::Reverse(e.last_t));
    let max = if view.expand_errors { 15 } else { 5 };
    let mut rows = Vec::new();
    for e in errs.iter().take(max) {
        let open = e.resolved_t.is_none();
        let glyph = if open {
            c.span("✗", Style::new().fg(Color::Red))
        } else {
            c.span("✓", Style::new().fg(Color::Green))
        };
        let mut text = sig_words(e);
        if !e.msg.is_empty() && e.msg != text {
            text = format!("{text} · {}", e.msg);
        }
        rows.push(vec![
            glyph,
            Span::raw(format!(" {:<3}  ", age(now.saturating_sub(e.last_t)))),
            Span::raw(text),
        ]);
        let mut detail = Vec::new();
        if e.retries > 0 {
            detail.push(format!("↻ retried {}×", e.retries));
        }
        match &e.resolved_by {
            Some(by) if !open => detail.push(format!("fixed by {by}")),
            _ if e.retries > 0 => detail.push("still failing".into()),
            _ => {}
        }
        if !detail.is_empty() {
            rows.push(vec![c.dim(format!("       {}", detail.join(" · ")))]);
        }
    }
    if errs.len() > max {
        rows.push(vec![c.dim(format!("… {} older (e)", errs.len() - max))]);
    }
    rows
}

fn meter(name: &str, pct: Option<f64>, tail: String, cfg: &Config, c: &Ctx) -> Row {
    let Some(p) = pct else {
        return vec![Span::raw(format!("{name:<4}")), c.dim("—")];
    };
    let label = format!(" {p:.0}%{tail}");
    let n = c.inner.saturating_sub(4 + label.width()).min(20);
    vec![
        Span::raw(format!("{name:<4}")),
        c.span(cells(p / 100.0, n, '▰', '▱'), level(p, cfg)),
        Span::raw(label),
    ]
}

#[allow(clippy::too_many_arguments)]
fn usage(st: &State, u: &Usage, x: &Extras, cfg: &Config, c: &Ctx, now: u64) -> Vec<Row> {
    let ctx_tail = match (u.ctx_used, u.ctx_size) {
        (Some(used), Some(size)) => format!("  {}/{}", tokens_k(used), tokens_k(size)),
        _ => String::new(),
    };
    let mut rows = vec![meter("ctx", u.ctx_pct, ctx_tail, cfg, c)];
    if let Some(n) = x.compacts_in.filter(|n| *n <= crate::bar::COMPACT_SOON) {
        let s = if n == 1 { "" } else { "s" };
        rows.push(vec![c.span(
            format!("    compacts in ~{n} turn{s}"),
            Style::new().fg(Color::Yellow),
        )]);
    }
    if let Some(t) = st.compacted_at {
        rows.push(vec![c.dim(format!("    ⟲ compacted {}", local_hhmm(t)))]);
    }
    let lim = |l: Option<Limit>| {
        l.map(|l| {
            if l.resets_at > 0 {
                format!("  {}", countdown(l.resets_at, now).replacen('↺', "↺ ", 1))
            } else {
                String::new()
            }
        })
    };
    rows.push(meter(
        "5h",
        u.five_h.map(|l| l.pct),
        lim(u.five_h).unwrap_or_default(),
        cfg,
        c,
    ));
    if let Some(eta) = x.burn_eta {
        rows.push(vec![c.span(
            format!("▲ at this pace: 5h limit in ~{}", age(eta)),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )]);
    }
    rows.push(meter(
        "7d",
        u.seven_d.map(|l| l.pct),
        lim(u.seven_d).unwrap_or_default(),
        cfg,
        c,
    ));
    rows
}

/// Model, effort, mode, worktree, PR and line counts in one dim row; each part only when known.
fn badges(st: &State, u: &Usage, c: &Ctx) -> Option<Row> {
    let mut parts: Vec<Span<'static>> = Vec::new();
    parts.extend(u.model.clone().map(|m| c.dim(m)));
    parts.extend(u.effort.clone().map(|e| c.dim(e)));
    if u.fast {
        parts.push(c.dim("fast"));
    }
    match st.permission_mode.as_deref() {
        None | Some("default") => {}
        Some("bypassPermissions") => parts.push(c.span(
            "BYPASS",
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Some(m) => parts.push(Span::raw(match m {
            "plan" => "PLAN".into(),
            "acceptEdits" => "EDITS".into(),
            "auto" => "AUTO".into(),
            "dontAsk" => "DON'T ASK".into(),
            other => other.to_string(),
        })),
    }
    parts.extend(u.worktree.as_ref().map(|w| c.dim(format!("⎇ {w}"))));
    if let Some(pr) = &u.pr {
        // GitLab writes merge requests as !N, GitHub PRs as #N.
        let (label, sign) = if pr.mr { ("MR", '!') } else { ("PR", '#') };
        let mut text = format!("{label} {sign}{}", pr.number);
        let mut style = Style::new();
        if let Some(state) = &pr.review_state {
            text += &format!(" {}", state.replace('_', " "));
            style = match state.as_str() {
                "approved" => Style::new().fg(Color::Green),
                "changes_requested" => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
                _ => Style::new().add_modifier(Modifier::DIM),
            };
        }
        parts.push(c.span(text, style));
    }
    if let (Some(a), Some(r)) = (u.lines_added, u.lines_removed)
        && a + r > 0
    {
        parts.push(c.dim(format!("+{a} −{r}")));
    }
    if parts.is_empty() {
        return None;
    }
    let mut row = Vec::new();
    for (i, p) in parts.into_iter().enumerate() {
        if i > 0 {
            row.push(c.dim(" · "));
        }
        row.push(p);
    }
    Some(row)
}

/// Cold warning and recent miss for the cache row; empty when there's nothing to say.
fn cache_suffix(u: &Usage, now: u64) -> String {
    let Some(c) = &u.cache else {
        return String::new();
    };
    let within = |t: u64| (t * 1000).saturating_sub(now) <= 10 * 60_000;
    let mut out = String::new();
    match c.expires_at {
        Some(e) if c.warm && within(e) => {
            let m = ((e * 1000).saturating_sub(now) / 60_000).max(1);
            out += &format!(" · cold in {m}m");
        }
        _ if !c.warm && c.observed => out += " · cold",
        _ => {}
    }
    if let (Some(t), Some(why)) = (c.last_miss_at, &c.last_miss)
        && now.saturating_sub(t * 1000) <= 10 * 60_000
    {
        out += &format!(" · last miss: {why}");
    }
    out
}

fn session(st: &State, u: &Usage, x: &Extras, cfg: &Config, c: &Ctx, now: u64) -> Vec<Row> {
    let mut rows = Vec::new();
    if let Some(t) = &x.tokens
        && t.total().total() > 0
    {
        let total = t.total();
        let turns = if st.turns == 1 {
            "1 turn".into()
        } else {
            format!("{} turns", st.turns)
        };
        rows.push(vec![
            c.dim("tokens  "),
            Span::raw(format!(
                "{} · {} out · {turns}",
                fmt_tokens(total.total()),
                fmt_tokens(total.output)
            )),
        ]);
        if let Some(hit) = total.hit() {
            let label = format!(" {:.0}% hit{}", hit * 100.0, cache_suffix(u, now));
            let n = c.inner.saturating_sub(8 + label.width()).min(20);
            rows.push(vec![
                c.dim("cache   "),
                c.span(cells(hit, n, '▰', '▱'), Style::new().fg(Color::Cyan)),
                Span::raw(label),
            ]);
        }
        if let Some(saved) = crate::tokens::savings(&total, u.cost_usd, &cfg.prices) {
            rows.push(vec![
                c.dim("        "),
                Span::raw(format!("saved ≈ ${saved:.2} vs uncached")),
            ]);
        }
        if let Some((from, to)) = t.drop_alert() {
            rows.push(vec![c.span(
                format!(
                    "⚠ cache hit fell {:.0}% → {:.0}% · compaction or a changed prompt",
                    from * 100.0,
                    to * 100.0
                ),
                Style::new().fg(Color::Yellow),
            )]);
        }
    }
    if let Some(last) = st.headline_run() {
        let mut row = vec![
            c.dim("tests   "),
            if last.fail > 0 {
                c.span(
                    format!("✗ {}/{}", last.fail, last.pass + last.fail),
                    Style::new().fg(Color::Red),
                )
            } else {
                c.span(format!("✓ {}", last.pass), Style::new().fg(Color::Green))
            },
        ];
        let full: Vec<_> = st.test_runs.iter().filter(|r| !r.partial).collect();
        let runs: Vec<_> = full.iter().rev().take(10).rev().collect();
        if runs.len() > 1 {
            let vals: Vec<f64> = runs.iter().map(|r| f64::from(r.pass)).collect();
            row.push(Span::raw(" · "));
            for (r, bar) in runs.iter().zip(crate::bar::spark(&vals).chars()) {
                let style = if r.fail > 0 {
                    Style::new().fg(Color::Red)
                } else {
                    Style::new()
                };
                row.push(c.span(bar.to_string(), style));
            }
            row.push(c.dim(format!(" last {} runs", runs.len())));
        }
        rows.push(row);
    }
    let wall = u
        .duration_ms
        .unwrap_or(st.last_t.saturating_sub(st.first_t));
    if wall > 0 {
        let sp = st.split(wall, u.api_duration_ms);
        // API time can overlap tool time; percentages are of whichever is larger.
        let total = wall.max(sp.claude + sp.tools + sp.waiting + sp.idle) as f64;
        let pct = |ms: u64| (ms as f64 * 100.0 / total).round();
        let mut text = dur(wall);
        for (name, ms) in [
            ("claude", sp.claude),
            ("tools", sp.tools),
            ("you", sp.waiting),
            ("idle", sp.idle),
            ("other", sp.other),
        ] {
            // Unattributed time only shows once it is big enough to matter.
            if pct(ms) > 0.0 && (name != "other" || pct(ms) >= 5.0) {
                text += &format!(" · {name} {:.0}%", pct(ms));
            }
        }
        rows.push(vec![c.dim("time    "), Span::raw(text)]);
    }
    let (done, _) = st.tasks_done();
    let cost = match u.cost_usd {
        Some(cost) if done > 0 => format!("${cost:.2} · ${:.2}/task done", cost / done as f64),
        Some(cost) => format!("${cost:.2}"),
        None => "—".into(),
    };
    rows.push(vec![c.dim("cost    "), Span::raw(cost)]);
    rows
}

pub fn to_plain(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Agent, Task};

    const NOW: u64 = 1_738_419_600_000;

    fn render(st: &State, u: &Usage, view: View, w: u16, h: u16) -> String {
        let out = lines(
            st,
            u,
            &Extras::default(),
            &Config::default(),
            view,
            NOW,
            "12:41",
            w,
            h,
            false,
        );
        for l in &out {
            assert!(
                l.width() <= w as usize,
                "line wider than {w}: {:?}",
                to_plain(std::slice::from_ref(l))
            );
        }
        to_plain(&out)
    }

    fn multi() -> (State, Usage) {
        let mut st = State {
            turn_active: true,
            turns: 2,
            first_t: NOW - 600_000,
            label: Some("auth refactor".into()),
            last_t: NOW,
            ..Default::default()
        };
        let agent = |id: &str, kind: &str| Agent {
            id: id.into(),
            kind: kind.into(),
            started: NOW - 72_000,
            last_t: NOW,
            ..Default::default()
        };
        st.agents = vec![
            Agent {
                act: Some("$ cargo test".into()),
                running_since: Some(NOW - 8_000),
                ..agent("main", "main")
            },
            Agent {
                act: Some("grep refresh_token".into()),
                running_since: Some(NOW - 3_000),
                calls: 14,
                ..agent("a1", "Explore")
            },
            Agent {
                done: true,
                ended_t: Some(NOW - 34_000),
                result: Some("12 passed".into()),
                ..agent("a2", "tests")
            },
            Agent {
                act: Some("reading big.log".into()),
                last_t: NOW - 6 * 60_000,
                ..agent("a3", "Plan")
            },
        ];
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
                subject: if i == 4 {
                    "hook up token refresh".into()
                } else {
                    format!("t{i}")
                },
                status: s.to_string(),
            });
        }
        st.errors = vec![
            ErrorRec {
                sig: "Bash:cargo build".into(),
                tool: "Bash".into(),
                msg: "error[E0382]: use of moved value".into(),
                last_t: NOW - 120_000,
                retries: 2,
                resolved_t: Some(NOW - 60_000),
                resolved_by: Some("main".into()),
                turn: 2,
                ..Default::default()
            },
            ErrorRec {
                sig: "Edit:/p/src/lib.rs".into(),
                tool: "Edit".into(),
                msg: "String to replace not found in file.".into(),
                last_t: NOW - 5_000,
                turn: 2,
                ..Default::default()
            },
        ];
        let u = Usage {
            ctx_pct: Some(71.0),
            ctx_used: Some(142_000),
            ctx_size: Some(200_000),
            five_h: Some(Limit {
                pct: 62.0,
                resets_at: NOW / 1000 + 6000,
            }),
            seven_d: None,
            cost_usd: Some(2.69),
            duration_ms: Some(840_000),
            api_duration_ms: Some(594_000),
            ..Default::default()
        };
        (st, u)
    }

    #[test]
    fn multi_agent_panel() {
        let (st, u) = multi();
        insta::assert_snapshot!(render(&st, &u, View::default(), 50, 40));
    }

    fn badge_usage() -> Usage {
        Usage {
            model: Some("Opus".into()),
            effort: Some("high".into()),
            fast: true,
            worktree: Some("wt".into()),
            pr: Some(crate::usage::Pr {
                number: 14,
                review_state: Some("approved".into()),
                mr: false,
            }),
            lines_added: Some(156),
            lines_removed: Some(23),
            ..Default::default()
        }
    }

    fn badge_text(st: &State, u: &Usage) -> Option<String> {
        let c = Ctx {
            color: true,
            inner: 60,
        };
        badges(st, u, &c).map(|r| r.iter().map(|s| s.content.as_ref()).collect())
    }

    #[test]
    fn badges_full_row() {
        let st = State {
            permission_mode: Some("auto".into()),
            ..Default::default()
        };
        assert_eq!(
            badge_text(&st, &badge_usage()).as_deref(),
            Some("Opus · high · fast · AUTO · ⎇ wt · PR #14 approved · +156 −23")
        );
        let (st, mut u) = multi();
        u.pr = Some(crate::usage::Pr {
            number: 3,
            review_state: Some("changes_requested".into()),
            mr: true,
        });
        let out = render(&st, &u, View::default(), 70, 40);
        let l: Vec<_> = out.lines().collect();
        assert!(l[2].contains("MR !3 changes requested"), "{out}");
        assert!(!out.contains("pr      "), "{out}");
    }

    #[test]
    fn badges_show_without_hooks() {
        let u = Usage {
            model: Some("Opus".into()),
            pr: Some(crate::usage::Pr {
                number: 14,
                review_state: Some("approved".into()),
                mr: false,
            }),
            ..Default::default()
        };
        let out = render(&State::default(), &u, View::default(), 70, 40);
        let l: Vec<_> = out.lines().collect();
        assert!(l[1].contains("no hooks yet"), "{out}");
        assert!(l[2].contains("Opus · PR #14 approved"), "{out}");
        assert!(l[3].contains("start a new Claude Code session"), "{out}");
    }

    #[test]
    fn badges_modes() {
        let mode = |m: &str| {
            let st = State {
                permission_mode: Some(m.into()),
                ..Default::default()
            };
            badge_text(&st, &Usage::default())
        };
        assert_eq!(mode("default"), None);
        assert_eq!(mode("plan").as_deref(), Some("PLAN"));
        assert_eq!(mode("dontAsk").as_deref(), Some("DON'T ASK"));
        assert_eq!(mode("weird").as_deref(), Some("weird"));
        let st = State {
            permission_mode: Some("bypassPermissions".into()),
            ..Default::default()
        };
        let c = Ctx {
            color: true,
            inner: 60,
        };
        let row = badges(&st, &Usage::default(), &c).unwrap();
        assert_eq!(row[0].content, "BYPASS");
        assert_eq!(row[0].style.fg, Some(Color::Red));
        let both_zero = Usage {
            lines_added: Some(0),
            lines_removed: Some(0),
            ..Default::default()
        };
        assert_eq!(badge_text(&State::default(), &both_zero), None);
    }

    #[test]
    fn narrow_and_short() {
        let (st, u) = multi();
        insta::assert_snapshot!("narrow_40", render(&st, &u, View::default(), 40, 40));
        insta::assert_snapshot!("short_14", render(&st, &u, View::default(), 50, 14));
        assert!(!render(&st, &u, View::default(), 50, 14).contains("├─ session"));
        render(&st, &u, View::default(), 30, 6);
        render(&st, &u, View::default(), 8, 3);
    }

    #[test]
    fn needs_you_and_empty() {
        let (mut st, u) = multi();
        st.needs_you = Some("Claude needs your permission to use Bash".into());
        let out = render(&st, &u, View::default(), 50, 40);
        assert!(
            out.contains("(◉_◉)")
                && out.contains("◉ NEEDS YOU")
                && out.contains("◉ Claude needs your permission"),
            "{out}"
        );
        insta::assert_snapshot!(
            "empty",
            render(
                &State::default(),
                &Usage::default(),
                View::default(),
                46,
                40
            )
        );
    }

    #[test]
    fn wide_text_truncates_by_display_width() {
        let (mut st, u) = multi();
        st.agents[0].act = Some("editing 日本語のファイル名がとても長いです🦀🦀🦀🦀🦀🦀.rs".into());
        let out = render(&st, &u, View::default(), 40, 40);
        assert!(out.contains('…'), "{out}");
    }

    #[test]
    fn stuck_reason_in_header() {
        let (mut st, u) = multi();
        st.errors[1].retries = 3;
        st.recent.push_back((false, st.errors[1].sig.clone()));
        let out = render(&st, &u, View::default(), 50, 40);
        assert!(
            out.contains("↻ STUCK") && out.contains("↻ edit lib.rs failed 4× in a row"),
            "{out}"
        );
    }

    #[test]
    fn test_history_row_and_done_failing() {
        use crate::state::TestRun;
        let (mut st, u) = multi();
        for (pass, fail) in [(10, 0), (40, 0), (69, 0), (67, 2)] {
            st.test_runs.push_back(TestRun {
                t: NOW,
                pass,
                fail,
                ..Default::default()
            });
        }
        let out = render(&st, &u, View::default(), 60, 60);
        assert!(out.contains("tests   ✗ 2/69 · ▁▄█▇ last 4 runs"), "{out}");
        st.test_runs.push_back(TestRun {
            t: NOW,
            pass: 1,
            partial: true,
            ..Default::default()
        });
        let out = render(&st, &u, View::default(), 60, 60);
        assert!(
            out.contains("▁▄█▇ last 4 runs"),
            "partial runs stay off the trend: {out}"
        );
        st.test_runs.pop_back();
        st.turn_active = false;
        st.background = 0;
        st.agents.iter_mut().for_each(|a| a.done = true);
        st.tasks
            .iter_mut()
            .for_each(|t| t.status = "completed".into());
        st.errors.clear();
        st.recent.clear();
        st.last_main_result = None;
        assert_eq!(st.verdict(NOW), crate::state::Verdict::Done);
        let out = render(&st, &u, View::default(), 60, 60);
        assert!(out.contains("DONE · tests failing"), "{out}");
    }

    #[test]
    fn time_split_other_only_from_5_percent() {
        let (mut st, mut u) = multi();
        st.time.tools_ms = 22_000;
        u.duration_ms = Some(100_000);
        u.api_duration_ms = Some(61_000);
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(out.contains("tools 22% · other 17%"), "{out}");
        u.api_duration_ms = Some(75_000);
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(!out.contains("other"), "{out}");
    }

    #[test]
    fn time_split_row() {
        let (mut st, mut u) = multi();
        st.time.tools_ms = 22_000;
        st.time.waiting_ms = 9_000;
        st.time.idle_ms = 8_000;
        u.duration_ms = Some(100_000);
        u.api_duration_ms = Some(61_000);
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(
            out.contains("time    1m40s · claude 61% · tools 22% · you 9% · idle 8%"),
            "{out}"
        );
    }

    #[test]
    fn mistake_cost_footer() {
        let (mut st, u) = multi();
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(!out.contains("cost of mistakes"), "{out}");
        st.errors[0].retries = 1;
        st.errors[0].time_ms = 5_000;
        st.reread = 1;
        let failed: u32 = st.errors.iter().map(|e| e.retries + 1).sum();
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(
            out.contains(&format!(
                "cost of mistakes: {failed} failed calls · 5s · 1 re-read"
            )),
            "{out}"
        );
    }

    #[test]
    fn no_plan_row_without_tasks() {
        let (mut st, u) = multi();
        st.tasks.clear();
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(!out.contains("no plan yet"), "{out}");
    }

    #[test]
    fn cheap_mistakes_drop_the_time() {
        let (mut st, u) = multi();
        st.errors[0].retries = 1;
        st.errors.iter_mut().for_each(|e| e.time_ms = 0);
        st.errors[0].time_ms = 400;
        st.reread = 1;
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(
            out.contains("cost of mistakes: 3 failed calls · 1 re-read"),
            "{out}"
        );
        assert!(!out.contains("· 0s"), "{out}");
    }

    fn cache_row(c: Option<crate::usage::CacheState>) -> String {
        use crate::tokens::{Ledger, TokenCache};
        let (st, mut u) = multi();
        u.cache = c;
        let x = Extras {
            tokens: Some(TokenCache {
                main: Ledger {
                    input: 1,
                    cache_read: 99,
                    output: 5,
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let out = to_plain(&lines(
            &st,
            &u,
            &x,
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            90,
            60,
            false,
        ));
        let r: String = out
            .lines()
            .find(|l| l.contains("cache   "))
            .unwrap()
            .trim_end_matches('│')
            .trim_end()
            .to_string();
        r
    }

    #[test]
    fn cache_suffix() {
        use crate::usage::CacheState;
        let s = NOW / 1000;
        let warm = |exp: u64| CacheState {
            warm: true,
            observed: true,
            expires_at: Some(exp),
            ..Default::default()
        };
        assert!(cache_row(Some(warm(s + 4 * 60 + 10))).ends_with("% hit · cold in 4m"));
        assert!(cache_row(Some(warm(s + 20 * 60))).ends_with("% hit"));
        let cold = CacheState {
            observed: true,
            ..Default::default()
        };
        assert!(cache_row(Some(cold)).ends_with("% hit · cold"));
        let miss = CacheState {
            warm: true,
            last_miss_at: Some(s - 120),
            last_miss: Some("ttl_expired_1h".into()),
            ..Default::default()
        };
        assert!(cache_row(Some(miss)).ends_with("% hit · last miss: ttl_expired_1h"));
        assert!(cache_row(None).ends_with("% hit"));
    }

    #[test]
    fn long_kind_is_cut_and_columns_align() {
        use crate::tokens::{Ledger, TokenCache};
        let (mut st, u) = multi();
        st.agents[1].kind = "general-purpose".into();
        let led = |m: &str| Ledger {
            output: 1,
            last_model: Some(m.into()),
            ..Default::default()
        };
        let mut agents = std::collections::BTreeMap::new();
        agents.insert(st.agents[1].id.clone(), led("claude-haiku-5-5"));
        let x = Extras {
            tokens: Some(TokenCache {
                main: led("claude-opus-5-5-20261001"),
                agents,
                ..Default::default()
            }),
            ..Default::default()
        };
        let out = to_plain(&lines(
            &st,
            &u,
            &x,
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            90,
            60,
            false,
        ));
        assert!(out.contains("general-pur… "), "{out}");
        let col = |l: &str, m: &str| {
            l.chars()
                .take_while(|_| true)
                .collect::<Vec<_>>()
                .windows(m.chars().count())
                .position(|w| w.iter().collect::<String>() == m)
        };
        let main = out.lines().find(|l| l.contains(" main ")).unwrap();
        let sub = out.lines().find(|l| l.contains("general-pur")).unwrap();
        assert_eq!(col(main, "opus-5-5"), col(sub, "haiku-5-5"), "{out}");
    }

    #[test]
    fn model_per_agent() {
        use crate::tokens::{Ledger, TokenCache};
        let (st, u) = multi();
        let led = |m: &str| Ledger {
            output: 1,
            last_model: Some(m.into()),
            ..Default::default()
        };
        let sub = st.agents[1].id.clone();
        let mut agents = std::collections::BTreeMap::new();
        agents.insert(sub, led("claude-haiku-5-5"));
        let x = Extras {
            tokens: Some(TokenCache {
                main: led("claude-opus-5-5-20261001"),
                agents,
                ..Default::default()
            }),
            ..Default::default()
        };
        let out = to_plain(&lines(
            &st,
            &u,
            &x,
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            90,
            60,
            false,
        ));
        let row = |name: &str| {
            out.lines()
                .find(|l| l.contains(name))
                .unwrap_or("")
                .to_string()
        };
        assert!(row(" main ").contains(" opus-5-5 · $ cargo test"), "{out}");
        assert!(row(&st.agents[1].kind).contains("haiku-5-5 · "), "{out}");
    }

    #[test]
    fn compaction_forecast_in_usage() {
        let (st, u) = multi();
        let x = Extras {
            compacts_in: Some(4),
            ..Default::default()
        };
        let out = to_plain(&lines(
            &st,
            &u,
            &x,
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            70,
            60,
            false,
        ));
        assert!(out.contains("│     compacts in ~4 turns"), "{out}");
        let later = Extras {
            compacts_in: Some(11),
            ..Default::default()
        };
        let out = to_plain(&lines(
            &st,
            &u,
            &later,
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            70,
            60,
            false,
        ));
        assert!(!out.contains("compacts in"), "{out}");
    }

    #[test]
    fn hides_instant_and_old_finished_agents() {
        use crate::state::Agent;
        let (mut st, u) = multi();
        let done = |id: &str, calls: u32, started: u64, ended: u64| Agent {
            id: id.into(),
            kind: id.into(),
            calls,
            started,
            last_t: ended,
            ended_t: Some(ended),
            done: true,
            result: Some(format!("{id} result")),
            ..Default::default()
        };
        st.agents.retain(|a| a.id == "main");
        st.agents.push(done("instant", 0, NOW - 5_000, NOW - 5_000));
        st.agents
            .push(done("oldone", 9, NOW - 30 * 60_000, NOW - 20 * 60_000));
        st.agents
            .push(done("recent", 4, NOW - 90_000, NOW - 60_000));
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(out.contains("recent result"), "{out}");
        assert!(!out.contains("instant result"), "{out}");
        assert!(!out.contains("oldone result"), "{out}");
        assert!(out.contains("… 2 more done (a)"), "{out}");
        let all = View {
            expand_agents: true,
            ..View::default()
        };
        let out = render(&st, &u, all, 70, 60);
        assert!(
            out.contains("instant result") && out.contains("oldone result"),
            "{out}"
        );
    }

    #[test]
    fn conflict_under_header() {
        let (st, u) = multi();
        let x = Extras {
            conflicts: vec![crate::conflicts::Conflict {
                path: "/x/a.rs".into(),
                other: "tankobon #bbbb".into(),
            }],
            ..Default::default()
        };
        let out = to_plain(&lines(
            &st,
            &u,
            &x,
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            70,
            60,
            false,
        ));
        assert_eq!(
            out.lines().nth(2).unwrap_or(""),
            format!("│ {:<66} │", "⚠ a.rs also in tankobon #bbbb"),
            "{out}"
        );
    }

    #[test]
    fn recap_view_replaces_sections() {
        let (st, u) = multi();
        let x = Extras {
            recap: Some(sample_recap()),
            ..Default::default()
        };
        let view = View {
            recap: true,
            ..View::default()
        };
        let out = to_plain(&lines(
            &st,
            &u,
            &x,
            &Config::default(),
            view,
            NOW,
            "12:41",
            40,
            30,
            false,
        ));
        for l in out.lines() {
            assert!(l.width() <= 40, "{l}");
        }
        assert!(out.contains("├─ recap (r to close) "), "{out}");
        assert!(out.contains("│ ✓ scaffold"), "{out}");
        assert!(out.contains("├─ errors "), "{out}");
        assert!(!out.contains("├─ agents"), "{out}");
        insta::assert_snapshot!("recap_view", out);
        // Recap toggled on before it's built: the normal sections stay.
        let out = to_plain(&lines(
            &st,
            &u,
            &Extras::default(),
            &Config::default(),
            view,
            NOW,
            "12:41",
            40,
            30,
            false,
        ));
        assert!(out.contains("├─ agents"), "{out}");
    }

    fn sample_recap() -> crate::recap::Recap {
        crate::recap::Recap {
            dir: "/x/proj".into(),
            title: Some("auth refactor".into()),
            since: "start".into(),
            events: 40,
            verdict: "✓ DONE".into(),
            done: vec!["scaffold".into(), "build".into()],
            open: vec!["write docs".into()],
            errors_hit: 1,
            fixed: vec![("cargo test".into(), "main".into())],
            tests: Some((
                Some(crate::state::TestRun {
                    t: 1,
                    pass: 58,
                    fail: 0,
                    ..Default::default()
                }),
                crate::state::TestRun {
                    t: 2,
                    pass: 71,
                    fail: 0,
                    ..Default::default()
                },
            )),
            files: vec![("src/a.rs".into(), 2), ("src/b.rs".into(), 1)],
            commits: vec!["abc1234 feat: retry the token refresh".into()],
            wall_ms: 21 * 60_000,
            cost: Some(2.69),
            lines: Some((156, 23)),
            models: vec!["opus-5-5 (main)".into(), "haiku-5-5 (2 agents)".into()],
            failed_calls: 1,
            failed_ms: 2_000,
            ..Default::default()
        }
    }

    #[test]
    fn recap_box() {
        let r = sample_recap();
        let out = recap_lines(&r, 60, false, false);
        for l in &out {
            assert!(l.width() <= 60, "{:?}", to_plain(std::slice::from_ref(l)));
        }
        let text = to_plain(&out);
        insta::assert_snapshot!("recap_box", text);
        let narrow = to_plain(&recap_lines(&r, 30, false, false));
        assert!(narrow.lines().all(|l| l.width() <= 30), "{narrow}");
        let asc = to_plain(&recap_lines(&r, 60, false, true));
        assert!(asc.starts_with("+- ARI recap"), "{asc}");
    }

    #[test]
    fn ascii_keeps_layout() {
        let (st, u) = multi();
        let uni = lines(
            &st,
            &u,
            &Extras::default(),
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            50,
            40,
            false,
        );
        let cfg = Config {
            ascii: true,
            ..Config::default()
        };
        let asc = lines(
            &st,
            &u,
            &Extras::default(),
            &cfg,
            View::default(),
            NOW,
            "12:41",
            50,
            40,
            false,
        );
        assert_eq!(uni.len(), asc.len());
        for (a, b) in uni.iter().zip(&asc) {
            assert_eq!(a.width(), b.width());
        }
        let text = to_plain(&asc);
        assert!(text.starts_with("+- ARI (o_o) -"), "{text}");
        insta::assert_snapshot!("ascii", text);
    }

    #[test]
    fn project_and_session_title_in_header() {
        let (st, mut u) = multi();
        u.dir = Some("/x/tankobon/".into());
        u.session_name = Some("next milestone".into());
        let out = render(&st, &u, View::default(), 50, 40);
        let first: Vec<&str> = out.lines().take(2).collect();
        assert!(first[0].starts_with("╭─ ARI (•_•) ─"), "{out}");
        assert!(first[0].contains("─ cd /x/tankobon · "), "{out}");
        assert!(first[1].ends_with("milestone: next milestone │"), "{out}");
        render(&st, &u, View::default(), 20, 10);
    }

    #[test]
    fn helpers() {
        assert_eq!(
            (dur(5_000), dur(594_000), dur(3_900_000)),
            ("5s".into(), "9m54s".into(), "1h05m".into())
        );
        let e = |sig: &str| ErrorRec {
            sig: sig.into(),
            ..Default::default()
        };
        assert_eq!(sig_words(&e("Bash:cargo build")), "cargo build");
        assert_eq!(sig_words(&e("Edit:/a/b/main.rs")), "edit main.rs");
        assert_eq!(sig_words(&e("StopFailure:rate_limit")), "api rate_limit");
        assert_eq!(sig_words(&e("WebFetch")), "webfetch");
        assert_eq!(local_hhmm(NOW).len(), 5);
        let home = std::env::var("HOME").unwrap();
        assert_eq!(tilde(&format!("{home}/ari/")), "~/ari");
        assert_eq!(tilde(&format!("{home}x/ari")), format!("{home}x/ari"));
    }
}

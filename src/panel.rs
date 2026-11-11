//! The full panel, as styled lines. Pure: everything `ari watch` draws comes from here.

use crate::bar::{Extras, cells, countdown, fmt_tokens, spinner};
use crate::config::{Config, Look};
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
    /// `c`: finished subagents that ended at or before this time (ms) are hidden.
    pub cleared: u64,
    /// Show the recap (`Extras::recap`) in place of the sections.
    pub recap: bool,
    /// `ari watch --all`: the highlighted row, and whether its session is open.
    pub sel: usize,
    pub open: bool,
    /// The hub (`x`) instead of the panel.
    pub hub: Option<crate::hub::HubView>,
    /// The settings screen (`s`) over everything, at this row of `settings::items()`.
    pub settings: Option<usize>,
    /// A settings change the key handler queued: 1 forward, -1 back, 0 reset.
    pub edit: Option<i8>,
    /// `v`: a queued view change, taken by the frame like `edit`.
    pub next_look: bool,
    /// The view of the screen on show, set by the frame before keys are read.
    pub look: Look,
    /// Calm hub `--all`'s session cards are on show: no selection to move or open.
    pub cards: bool,
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

/// Local weekday, `Sun`…`Sat`.
pub fn local_weekday(ms: u64) -> &'static str {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    #[cfg(unix)]
    {
        let t = (ms / 1000) as libc::time_t;
        // SAFETY: localtime_r only writes into the tm we own.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return DAYS[tm.tm_wday as usize % 7];
        }
    }
    DAYS[((ms / 86_400_000 + 4) % 7) as usize] // 1970-01-01 was a Thursday
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
    if !view.recap && cfg.view.watch != Look::Full {
        let i = crate::hub::Input {
            st,
            u,
            x,
            cfg,
            now,
            clock,
        };
        let out = if cfg.view.watch == Look::Calm {
            crate::simple::calm::panel(&i, width, height, color, "constellation")
        } else {
            crate::simple::sky::panel(&i, width, height, color)
        };
        return if cfg.ascii {
            crate::ascii::map_lines(out)
        } else {
            out
        };
    }
    let w = width as usize;
    let c = Ctx {
        color,
        inner: w.saturating_sub(4),
    };
    // Wide panes split into two columns (not the recap view): agents/turns | usage/session/errors/git.
    let two = w >= 120 && !(view.recap && x.recap.is_some());
    let col = |inner| Ctx { color, inner };
    let (lc, rc) = if two {
        let l = (w - 7) / 2;
        (col(l), col(w - 7 - l))
    } else {
        (col(c.inner), col(c.inner))
    };
    let v = st.verdict_with(cfg.thresholds.stuck_repeats);
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
                let mut rows = header(st, x, u.session_name.as_deref(), &c, v, cfg);
                if let Some(s) = crate::conflicts::summary(&x.conflicts) {
                    rows.insert(
                        1.min(rows.len()),
                        vec![c.span(
                            s,
                            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                        )],
                    );
                }
                if let Some(r) = risky_row(st, &c, now) {
                    rows.insert(1.min(rows.len()), r);
                }
                if let Some(b) = badges(st, u, &c) {
                    rows.insert(1.min(rows.len()), b);
                }
                if st.first_t > 0
                    && matches!(v, Verdict::Idle | Verdict::Done)
                    && let Some(a) = x.tokens.as_ref().and_then(|t| t.main.away.as_ref())
                {
                    rows.push(vec![c.dim(format!("“{a}”"))]);
                }
                rows
            },
        },
        Section {
            title: Some("agents"),
            rows: agents(st, x, &lc, view, now),
        },
        Section {
            title: Some("calls"),
            rows: calls(st, &lc, now),
        },
        Section {
            title: Some("turns"),
            rows: turns(st, x, &lc, now),
        },
        Section {
            title: Some("usage"),
            rows: usage(st, u, x, cfg, &rc, now),
        },
        Section {
            title: Some("session"),
            rows: session(st, u, x, cfg, &rc, now),
        },
        Section {
            title: Some("errors"),
            rows: errors(st, &rc, view, now),
        },
        Section {
            title: Some("git"),
            rows: git(x.git.as_ref(), &rc, now),
        },
    ];
    // A section with nothing to say, or switched off in config, is left out.
    sections.retain(|s| {
        s.title
            .is_none_or(|t| !s.rows.is_empty() && !hidden(cfg, t))
    });
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
    let cols = if two {
        let (mut left, mut right): (Vec<_>, Vec<_>) = sections
            .split_off(1)
            .into_iter()
            .partition(|s| matches!(s.title, Some("agents" | "calls" | "turns")));
        // Each column gets the height left under the full-width header rows.
        let h = (height as usize).saturating_sub(sections[0].rows.len());
        collapse(&mut left, h);
        // Errors get little room on the right: trim them before git/session/usage go.
        trim(&mut right, "errors", h);
        collapse(&mut right, h);
        Some((left, right))
    } else {
        collapse(&mut sections, height as usize);
        None
    };

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
                vec![c.span(tilde(d), Style::new().add_modifier(Modifier::BOLD))],
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
    match cols {
        Some((l, r)) => frame_two(title, sections.remove(0), l, r, &c, w, cfg.ascii),
        None => frame(title, sections, &c, w, cfg.ascii),
    }
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

/// [`frame`] with two columns under the full-width `top`:
/// `│ left │ right │`, headers joined as `├─ agents ──┼─ usage ──┤`, bottom `╰──┴──╯`.
fn frame_two(
    title: Row,
    top: Section,
    left: Vec<Section>,
    right: Vec<Section>,
    c: &Ctx,
    w: usize,
    ascii: bool,
) -> Vec<Line<'static>> {
    let bstyle = c.dim("").style;
    let lw = (w - 7) / 2;
    let rw = w - 7 - lw;
    let mut out = frame(title, vec![top], c, w, false);
    out.pop(); // the bottom edge; ours has the divider in it
    // A column as cells `cw + 2` wide: (is a header, spans).
    let cells = |secs: Vec<Section>, cw: usize| {
        let mut v: Vec<(bool, Row)> = Vec::new();
        for sec in secs {
            if let Some(t) = sec.title {
                let head = format!("─ {t} ");
                let fill = (cw + 2).saturating_sub(head.width());
                v.push((
                    true,
                    vec![Span::styled(format!("{head}{}", "─".repeat(fill)), bstyle)],
                ));
            }
            for row in sec.rows {
                let row = fit(row, cw);
                let pad = cw.saturating_sub(row_width(&row));
                let mut spans = vec![Span::raw(" ")];
                spans.extend(row);
                spans.push(Span::raw(" ".repeat(pad + 1)));
                v.push((false, spans));
            }
        }
        v
    };
    let (l, r) = (cells(left, lw), cells(right, rw));
    for i in 0..l.len().max(r.len()) {
        let blank = |cw: usize| (false, vec![Span::raw(" ".repeat(cw + 2))]);
        let (lh, lrow) = l.get(i).cloned().unwrap_or_else(|| blank(lw));
        let (rh, rrow) = r.get(i).cloned().unwrap_or_else(|| blank(rw));
        let mid = match (lh, rh) {
            (true, true) if i == 0 => "┬",
            (true, true) => "┼",
            (true, false) => "┤",
            (false, true) => "├",
            (false, false) => "│",
        };
        let edge = |h: bool, a: &'static str| Span::styled(if h { a } else { "│" }, bstyle);
        let mut spans = vec![edge(lh, "├")];
        spans.extend(lrow);
        spans.push(Span::styled(mid, bstyle));
        spans.extend(rrow);
        spans.push(edge(rh, "┤"));
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(
        format!("╰{}┴{}╯", "─".repeat(lw + 2), "─".repeat(rw + 2)),
        bstyle,
    )));
    if ascii {
        crate::ascii::map_lines(out)
    } else {
        out
    }
}

/// `ari recap` as a box: `╭─ ARI recap · since start ── ~/proj ─╮`, then the sections.
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
                format!(" {}", tilde(&r.dir)),
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
            rows.push(vec![c.dim(t.clone())]);
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
                cells(r.done.len() as f64 / total as f64, 20, '▰', '▱'),
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
                .map(|(what, _)| ok(format!("{what} · passed on retry"))),
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
    if !r.risky.is_empty() {
        let warn = Style::new().fg(Color::Yellow);
        let row = |k: &crate::state::Risky| {
            let mut t = format!("{} · {}", k.cmd, k.rule);
            if k.agent != "main" {
                t += &format!(" · {}", k.agent);
            }
            if !k.ok {
                t += " · failed";
            }
            vec![c.span("⚠ ", warn), Span::raw(t)]
        };
        out.push(Section {
            title: Some("risky"),
            rows: r.risky.iter().map(row).collect(),
        });
    }
    if !r.test_flags.is_empty() {
        let warn = Style::new().fg(Color::Yellow);
        out.push(Section {
            title: Some("check"),
            rows: r
                .test_flags
                .iter()
                .map(|f| vec![c.span("⚠ ", warn), Span::raw(f.clone())])
                .collect(),
        });
    }
    if !r.groups.is_empty() {
        use crate::recap::{FILES_PER_GROUP, GROUPS_SHOWN, REVIEW_SHOWN};
        let warn = Style::new().fg(Color::Yellow);
        let order = crate::recap::review_order(&r.groups);
        let flagged = order.iter().filter(|f| f.look.is_some()).count();
        let mut head = format!("{} edited", order.len());
        if flagged > 0 {
            head += &format!(" · {flagged} to look at");
        }
        let mut rows: Vec<Row> = vec![vec![Span::raw(head)]];
        let hidden = r.groups.len().saturating_sub(GROUPS_SHOWN);
        if hidden > 0 {
            rows.push(vec![c.dim(format!("and {hidden} earlier"))]);
        }
        for g in &r.groups[hidden..] {
            rows.push(vec![c.dim("› "), Span::raw(g.cause.clone())]);
            for f in g.files.iter().take(FILES_PER_GROUP) {
                let mut row = vec![Span::raw(format!("  {}", f.path))];
                if f.edits > 1 {
                    row.push(c.dim(format!(" ×{}", f.edits)));
                }
                if f.added > 0 {
                    row.push(c.dim(format!("  +{}", f.added)));
                }
                if f.removed > 0 {
                    row.push(c.dim(format!(" −{}", f.removed)));
                }
                if let Some(l) = f.look {
                    row.push(c.span(format!(" · {l}"), warn));
                }
                rows.push(row);
            }
            if g.files.len() > FILES_PER_GROUP {
                rows.push(vec![
                    c.dim(format!("  and {} more", g.files.len() - FILES_PER_GROUP)),
                ]);
            }
        }
        out.push(Section {
            title: Some("files"),
            rows,
        });
        if order.len() >= 2 {
            let mut rows: Vec<Row> = order
                .iter()
                .take(REVIEW_SHOWN)
                .enumerate()
                .map(|(i, f)| {
                    let mut row = vec![Span::raw(format!("{}. {}", i + 1, f.path))];
                    if let Some(l) = f.look {
                        row.push(c.span(format!(" · {l}"), warn));
                    }
                    row
                })
                .collect();
            if order.len() > REVIEW_SHOWN {
                rows.push(vec![
                    c.dim(format!("and {} more", order.len() - REVIEW_SHOWN)),
                ]);
            }
            out.push(Section {
                title: Some("review"),
                rows,
            });
        }
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
    let total = panel_height;
    // Turns go before calls: the calls are what you're watching.
    for title in ["turns", "calls"] {
        while total(secs) > height {
            let Some(s) = secs.iter_mut().find(|s| s.title == Some(title)) else {
                break;
            };
            s.rows.pop();
            if s.rows.is_empty() {
                secs.retain(|s| s.title != Some(title));
            }
        }
    }
    for title in ["git", "session", "usage"] {
        if total(secs) > height {
            secs.retain(|s| s.title != Some(title));
        }
    }
    for title in ["errors", "agents"] {
        trim(secs, title, height);
    }
}

/// Pop rows off section `title`, keeping at least one, until the panel fits `height`.
fn trim(secs: &mut [Section], title: &str, height: usize) {
    while panel_height(secs) > height {
        let Some(s) = secs
            .iter_mut()
            .find(|s| s.title == Some(title) && s.rows.len() > 1)
        else {
            break;
        };
        s.rows.pop();
    }
}

fn panel_height(s: &[Section]) -> usize {
    2 + s
        .iter()
        .map(|x| x.rows.len() + x.title.is_some() as usize)
        .sum::<usize>()
}

/// Cost between two times from the sampled cumulative cost; never negative.
fn cost_between(samples: &[crate::burn::Sample], a: u64, b: u64) -> Option<f64> {
    let at = |t: u64| samples.iter().rev().find(|s| s.t <= t)?.cost;
    Some((at(b)? - at(a)?).max(0.0))
}

/// Main's recent tool calls, newest first, laid out like the hub's agent card:
/// age, tool, target, then result and duration on the right.
fn calls(st: &State, c: &Ctx, now: u64) -> Vec<Row> {
    use crate::hub::agents::short_ms;
    let Some(main) = st.agents.iter().find(|a| a.id == "main") else {
        return Vec::new();
    };
    main.recent
        .iter()
        .rev()
        .map(|r| {
            let (res, rs) = match r.ok {
                Some(true) => ("✓", Style::new().fg(Color::Green)),
                Some(false) => ("✗", Style::new().fg(Color::Red)),
                None => ("…", Style::new().add_modifier(Modifier::DIM)),
            };
            let d = r
                .dur
                .map(|d| format!(" {}", short_ms(d)))
                .unwrap_or_default();
            let tool: String = r.tool.chars().take(5).collect();
            let target = r.act.split_once(' ').map_or(r.act.as_str(), |t| t.1);
            let room = c.inner.saturating_sub(12 + d.len());
            let target = crate::event::truncate(target, room);
            let pad = room.saturating_sub(target.width());
            vec![
                c.dim(format!("{:>3} ", age(now.saturating_sub(r.t)))),
                c.span(format!("{tool:<5} "), crate::hub::tool_style(&r.tool)),
                Span::raw(format!("{target}{}", " ".repeat(pad))),
                c.span(format!(" {res}"), rs),
                c.dim(d),
            ]
        })
        .collect()
}

/// Turns and compactions, newest first.
fn turns(st: &State, x: &Extras, c: &Ctx, now: u64) -> Vec<Row> {
    if st.turn_log.len() < 2 {
        return Vec::new();
    }
    let mut items: Vec<(u64, Row)> = Vec::new();
    for r in &st.turn_log {
        let end = r.end.unwrap_or(now);
        let mut t = format!(
            "{}  {}  {} call{}",
            local_hhmm(r.start),
            dur(end.saturating_sub(r.start)),
            r.calls,
            if r.calls == 1 { "" } else { "s" }
        );
        let style = match (r.end, r.ok) {
            (None, _) => Style::new().fg(Color::Cyan),
            (_, false) => Style::new().fg(Color::Red),
            _ => Style::new(),
        };
        let mut row = Vec::new();
        if r.failed > 0 {
            row.push(c.span(format!("{t}  "), style));
            row.push(c.span(format!("✗ {}", r.failed), Style::new().fg(Color::Red)));
            t = String::new();
        }
        if let Some(usd) = r.end.and_then(|e| cost_between(&x.samples, r.start, e)) {
            t.push_str(&format!("  ${usd:.2}"));
        }
        t.push_str(&format!("  {}", r.prompt));
        row.push(c.span(t, if row.is_empty() { style } else { Style::new() }));
        items.push((r.start, row));
    }
    for (t, kind) in &st.compactions {
        let before = x
            .samples
            .iter()
            .rev()
            .find(|s| s.t <= *t)
            .and_then(|s| s.ctx);
        let after = x.samples.iter().find(|s| s.t > *t).and_then(|s| s.ctx);
        let ctx = match (before, after) {
            (Some(b), Some(a)) => format!(" · ctx {b:.0}% → {a:.0}%"),
            _ => String::new(),
        };
        let text = format!("⟲ {} compacted ({kind}){ctx}", local_hhmm(*t));
        items.push((*t, vec![c.dim(text)]));
    }
    items.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    items.into_iter().map(|(_, r)| r).collect()
}

/// Switched off with `hide_panel = ["<id>"]`.
fn hidden(cfg: &Config, id: &str) -> bool {
    cfg.hide_panel.iter().any(|h| h == id)
}

fn header(
    st: &State,
    x: &Extras,
    title: Option<&str>,
    c: &Ctx,
    v: Verdict,
    cfg: &Config,
) -> Vec<Row> {
    let (plan, active, app) = (x.plan.as_ref(), x.looks_active(), x.app);
    let repeats = cfg.thresholds.stuck_repeats;
    let mut rows = Vec::new();
    if st.first_t == 0 {
        rows.push(vec![if active {
            c.span(
                "● WORKING · not tracked, so no details",
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )
        } else {
            c.dim("– not tracked")
        }]);
        rows.push(vec![
            c.dim("no hook events · new session or /reload-plugins"),
        ]);
        return rows;
    }
    let left = if let Some(note) = st.verdict_note(v) {
        c.span(
            format!("{} {} · {note}", v.glyph(), v.label()),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )
    } else {
        c.span(format!("{} {}", v.glyph(), v.label()), verdict_style(v))
    };
    let mut row = vec![left];
    if app != crate::app::App::Claude {
        row.push(c.span(
            format!(" · {}", app.label()),
            Style::new().fg(app.accent()).add_modifier(Modifier::BOLD),
        ));
    }
    // Claude Code's own session title beats a truncated first prompt.
    if let Some(label) = title.map(String::from).or_else(|| st.label.clone())
        && c.inner.saturating_sub(row_width(&row)) >= 8
    {
        row.push(c.dim(format!(" · {label}")));
    }
    rows.push(row);
    if v == Verdict::Stuck
        && let Some(why) = st.stuck_reason(repeats)
    {
        let burned = crate::bar::stuck_tokens(st, x, repeats)
            .map(|n| format!(" · {} tok while stuck", crate::bar::fmt_tokens(n)))
            .unwrap_or_default();
        rows.push(vec![c.span(format!("↻ {why}{burned}"), verdict_style(v))]);
    }
    if let Some(r) = st.redoing() {
        rows.push(vec![c.span(
            format!("⟲ redoing work since compaction · {}× · {}", r.n, r.last),
            Style::new().fg(Color::Yellow),
        )]);
    }
    let flags = st.turn_flags();
    for f in flags.iter().take(3) {
        let file = std::path::Path::new(&f.path)
            .file_name()
            .map_or(f.path.as_str(), |n| n.to_str().unwrap_or(&f.path));
        rows.push(vec![c.span(
            format!("⚠ {} · {file}", f.kind),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )]);
    }
    if flags.len() > 3 {
        rows.push(vec![c.dim(format!("  +{} more", flags.len() - 3))]);
    }
    if let Some(msg) = &st.needs_you {
        rows.push(vec![
            c.span(format!("◉ {msg}"), verdict_style(Verdict::NeedsYou)),
        ]);
    }
    if let Some(r) = &st.resume {
        let mut t = "↻ resumed".to_string();
        if let Some(ms) = r.away_ms {
            t += &format!(" after {}", age(ms));
        }
        if r.cold == Some(true) {
            t += " · cache cold";
        }
        if let Some(usd) = r.rebuild_usd {
            t += &format!(" · ~${usd:.2} to rebuild");
        }
        rows.push(vec![if r.cold == Some(true) {
            c.span(t, Style::new().fg(Color::Yellow))
        } else {
            c.dim(t)
        }]);
    }
    let (done, total) = st.tasks_done();
    if hidden(cfg, "tasks") {
        // switched off: no checklist bar
    } else if total == 0 {
        if let Some(p) = plan {
            let tail = format!(" {}/{} · {}", p.done, p.total, p.name);
            let n = c.inner.saturating_sub(tail.width()).min(20);
            rows.push(vec![
                c.span(
                    cells(p.done as f64 / p.total as f64, n, '▰', '▱'),
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
                cells(done as f64 / total as f64, n, '▰', '▱'),
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

/// The subagents the panel lists: running and stale ones, plus the 3 most recent that finished
/// in the last 5 minutes after doing some work (all of them are folded away with `expand`).
pub(crate) fn shown_agents(st: &State, expand: bool, now: u64) -> Vec<&crate::state::Agent> {
    let subs = || st.agents.iter().skip(1);
    let mut done: Vec<_> = subs()
        .filter(|a| a.done)
        .filter(|a| a.calls > 0 || a.ended_t.unwrap_or(a.last_t) >= a.started + 1_000)
        .filter(|a| now.saturating_sub(a.ended_t.unwrap_or(a.last_t)) < 5 * 60_000)
        .collect();
    done.sort_by_key(|a| std::cmp::Reverse(a.ended_t));
    let recent: Vec<&str> = done.iter().take(3).map(|a| a.id.as_str()).collect();
    subs()
        .filter(|a| !a.done || (!expand && recent.contains(&a.id.as_str())))
        .collect()
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
    let shown = shown_agents(st, view.expand_agents, now);
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

    // Background subagents already have rows below; list shells, monitors, workflows.
    for b in st
        .background_names
        .iter()
        .filter(|b| !b.starts_with("subagent"))
    {
        rows.push(vec![c.dim(format!("  ↳ {b}"))]);
    }
    for k in &st.crons {
        rows.push(vec![c.dim(format!("  ◷ {k}"))]);
    }

    let first_sub = rows.len();
    for a in shown.iter() {
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
                c.span("?", Style::new().fg(Color::Yellow)),
                format!(
                    "stale {} · {}",
                    age(now.saturating_sub(a.last_t)),
                    a.act.as_deref().unwrap_or("")
                ),
            ),
        };
        rows.push(vec![
            Span::raw(""),
            glyph,
            Span::raw(" "),
            Span::raw(pad(&crate::event::truncate(&a.kind, name_w))),
            Span::raw(" "),
            model(&a.id).unwrap_or_default(),
            Span::raw(text),
        ]);
    }
    let ledger = |id: &str| x.tokens.as_ref().and_then(|t| t.agents.get(id));
    let toks = |ids: &[&crate::state::Agent]| -> u64 {
        ids.iter()
            .filter_map(|a| ledger(&a.id))
            .map(|l| l.total())
            .sum()
    };
    let tok_part = |n: u64| {
        if n > 0 {
            format!(" · {} tok", fmt_tokens(n))
        } else {
            String::new()
        }
    };
    let mut fold = None;
    if view.expand_agents {
        // One row per (kind, model), newest group first; instant no-call helpers fold together.
        let is_helper = |a: &crate::state::Agent| {
            a.calls == 0 && a.ended_t.unwrap_or(a.last_t) < a.started + 1_000
        };
        let mut all: Vec<&crate::state::Agent> = subs.iter().copied().filter(|a| a.done).collect();
        all.sort_by_key(|a| std::cmp::Reverse(a.ended_t.unwrap_or(a.last_t)));
        let mut groups: Vec<((String, String), Vec<&crate::state::Agent>)> = vec![];
        let mut helpers = 0;
        for a in all {
            if is_helper(a) {
                helpers += 1;
                continue;
            }
            let m = ledger(&a.id)
                .and_then(|l| l.last_model.as_deref())
                .filter(|m| *m != "unknown")
                .map(crate::tokens::short_model)
                .unwrap_or_default();
            let key = (a.kind.clone(), m);
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.1.push(a),
                None => groups.push((key, vec![a])),
            }
        }
        let n_x = |n: usize| {
            if n > 1 {
                format!("{n}× ")
            } else {
                String::new()
            }
        };
        let mut line = |text: String| {
            rows.push(vec![
                Span::raw(""),
                c.span("✓", Style::new().fg(Color::Green)),
                Span::raw(format!(" {text}")),
            ])
        };
        for ((kind, m), g) in &groups {
            let time: u64 = g
                .iter()
                .map(|a| a.ended_t.unwrap_or(a.last_t).saturating_sub(a.started))
                .sum();
            let m = if m.is_empty() {
                String::new()
            } else {
                format!(" · {m}")
            };
            line(format!(
                "{}{kind}{m}{} · {}",
                n_x(g.len()),
                tok_part(toks(g)),
                dur(time)
            ));
        }
        if helpers > 0 {
            line(format!("{}helpers", n_x(helpers)));
        }
    } else if subs.len() > shown.len() {
        // One quiet count; the kinds and tokens live behind `a`.
        fold = Some(vec![
            c.dim(format!("└─ ✓ {} done (a)", subs.len() - shown.len())),
        ]);
    }
    let last = rows.len().saturating_sub(1);
    for (i, r) in rows.iter_mut().enumerate().skip(first_sub) {
        r[0] = c.dim(if i == last && fold.is_none() {
            "└─"
        } else {
            "├─"
        });
    }
    rows.extend(fold);
    rows
}

/// Open errors one by one; fixed ones fold into a count (listed with `e`), with the time
/// failed calls took and the files read twice.
fn errors(st: &State, c: &Ctx, view: View, now: u64) -> Vec<Row> {
    let mut errs: Vec<&ErrorRec> = st.errors.iter().collect();
    errs.sort_by_key(|e| std::cmp::Reverse(e.last_t));
    let (open, fixed): (Vec<_>, Vec<_>) = errs.into_iter().partition(|e| e.resolved_t.is_none());
    let max = if view.expand_errors { 15 } else { 5 };
    let text = |e: &ErrorRec| {
        let w = sig_words(e);
        if e.msg.is_empty() || e.msg == w {
            w
        } else {
            format!("{w} · {}", e.msg)
        }
    };
    let when = |e: &ErrorRec| format!(" {:<3}  ", age(now.saturating_sub(e.last_t)));
    let mut rows = Vec::new();
    for e in open.iter().take(max) {
        rows.push(vec![
            c.span("✗", Style::new().fg(Color::Red)),
            Span::raw(when(e)),
            Span::raw(text(e)),
        ]);
        if e.retries > 0 {
            rows.push(vec![
                c.dim(format!("       ↻ retried {}× · still failing", e.retries)),
            ]);
        }
    }
    if open.len() > max {
        rows.push(vec![c.dim(format!("… {} older (e)", open.len() - max))]);
    }
    if view.expand_errors {
        for e in fixed.iter().take(max) {
            rows.push(vec![c.dim(format!(
                " {}{} · passed on retry",
                when(e),
                text(e)
            ))]);
        }
    }
    let mut parts = Vec::new();
    if !fixed.is_empty() {
        parts.push(format!("{} fixed", fixed.len()));
    }
    let time: u64 = st.errors.iter().map(|e| e.time_ms).sum();
    if time >= 1000 {
        parts.push(format!("{} on failed calls", dur(time)));
    }
    if st.reread > 0 {
        let s = if st.reread == 1 { "" } else { "s" };
        parts.push(format!("{} file{s} read twice", st.reread));
    }
    if rows.is_empty() {
        parts.insert(
            0,
            if fixed.is_empty() {
                "no errors"
            } else {
                "no open errors"
            }
            .into(),
        );
    }
    if !parts.is_empty() {
        let hint = if !fixed.is_empty() && !view.expand_errors {
            " (e)"
        } else {
            ""
        };
        rows.push(vec![c.dim(format!("{}{hint}", parts.join(" · ")))]);
    }
    rows
}

fn meter(name: &str, pct: Option<f64>, tail: String, cfg: &Config, c: &Ctx) -> Option<Row> {
    let p = pct?;
    let label = format!(" {p:.0}%{tail}");
    let n = c.inner.saturating_sub(4 + label.width()).min(20);
    Some(vec![
        Span::raw(format!("{name:<4}")),
        c.span(cells(p / 100.0, n, '▰', '▱'), level(p, cfg)),
        Span::raw(label),
    ])
}

const TREND_MS: u64 = 30 * 60_000;
/// Context already used at start, as a percent of the window, that earns the `▲` warning.
pub const START_LARGE_PCT: f64 = 20.0;

/// `at start 31k (16%)`: the first request's input side; `None` when unknown.
pub(crate) fn start_note(start: u64, window: u64) -> Option<(String, bool)> {
    if start == 0 {
        return None;
    }
    let pct = start as f64 * 100.0 / window.max(1) as f64;
    let n = format!("at start {} ({pct:.0}%)", fmt_tokens(start));
    Some(if pct >= START_LARGE_PCT {
        (format!("▲ {n} · system, tools, MCP, CLAUDE.md"), true)
    } else {
        (n, false)
    })
}

/// Samples from the 30 min before the newest one that carry a value, as (t, value).
fn recent(
    samples: &[crate::burn::Sample],
    f: fn(&crate::burn::Sample) -> Option<f64>,
) -> Vec<(u64, f64)> {
    let v: Vec<_> = samples.iter().filter_map(|s| Some((s.t, f(s)?))).collect();
    let from = v
        .iter()
        .map(|p| p.0)
        .max()
        .unwrap_or(0)
        .saturating_sub(TREND_MS);
    v.into_iter().filter(|p| p.0 >= from).collect()
}

/// 12 equal time buckets over the window, empty ones carrying the last value; none if flat.
fn trend(pts: &[(u64, f64)]) -> Option<String> {
    let from = pts.iter().map(|p| p.0).max()?.saturating_sub(TREND_MS);
    let mut buckets = [None; 12];
    for &(t, v) in pts {
        buckets[((t.saturating_sub(from) * 12 / TREND_MS) as usize).min(11)] = Some(v);
    }
    let mut last = None;
    let v: Vec<f64> = buckets
        .iter()
        .filter_map(|b| {
            last = b.or(last);
            last
        })
        .collect();
    (v.len() >= 3 && v.iter().any(|x| *x != v[0])).then(|| crate::bar::spark(&v))
}

/// $/h from the first to the last point, over at least 10 min.
fn rate(pts: &[(u64, f64)]) -> Option<f64> {
    let first = pts.iter().min_by_key(|p| p.0)?;
    let last = pts.iter().max_by_key(|p| p.0)?;
    let span = last.0.saturating_sub(first.0);
    (span >= 10 * 60_000).then(|| ((last.1 - first.1) / (span as f64 / 3.6e6)).max(0.0))
}

#[allow(clippy::too_many_arguments)]
fn usage(st: &State, u: &Usage, x: &Extras, cfg: &Config, c: &Ctx, now: u64) -> Vec<Row> {
    let mut ctx_tail = match (u.ctx_used, u.ctx_size) {
        (Some(used), Some(size)) => format!("  {}/{}", tokens_k(used), tokens_k(size)),
        _ => String::new(),
    };
    if let Some(t) = trend(&recent(&x.samples, |s| s.ctx)) {
        ctx_tail += &format!(" {t}");
    }
    let mut rows = Vec::new();
    // The warning that matters most leads the section.
    if let Some(eta) = x.burn_eta {
        rows.push(vec![c.span(
            format!("▲ at this pace: 5h limit in ~{}", age(eta)),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )]);
    }
    if let Some(eta) = x.week_eta {
        let at = now + eta;
        rows.push(vec![c.span(
            format!(
                "▲ 7d runs out {} ~{} at this pace",
                local_weekday(at),
                local_hhmm(at)
            ),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )]);
    }
    rows.extend(meter("ctx", u.ctx_pct, ctx_tail, cfg, c));
    // Only under a ctx meter: without the statusline it would hang under another row.
    let start = x
        .tokens
        .as_ref()
        .filter(|_| u.ctx_pct.is_some())
        .map_or(0, |t| t.main.start);
    if let Some((text, large)) = start_note(start, u.ctx_size.unwrap_or(200_000)) {
        let text = format!("    {text}");
        rows.push(vec![if large {
            c.span(text, Style::new().fg(Color::Yellow))
        } else {
            c.dim(text)
        }]);
    }
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
    rows.extend(meter(
        "5h",
        u.five_h.map(|l| l.pct),
        lim(u.five_h).unwrap_or_default(),
        cfg,
        c,
    ));
    rows.extend(meter(
        "7d",
        u.seven_d.map(|l| l.pct),
        lim(u.seven_d).unwrap_or_default(),
        cfg,
        c,
    ));
    rows
}

/// Model, effort, mode, worktree, PR and line counts in one dim row; each part only when known.
/// The latest risky command that ran, and how many older ones are kept.
fn risky_row(st: &State, c: &Ctx, now: u64) -> Option<Row> {
    let r = st.risky.back()?;
    let mut s = format!(
        "⚠ ran {} · {} · {}",
        r.cmd,
        r.rule,
        age(now.saturating_sub(r.t))
    );
    if st.risky.len() > 1 {
        s += &format!(" · +{} more", st.risky.len() - 1);
    }
    let warn = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    Some(vec![c.span(s, warn)])
}

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
            "bypass permissions",
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Some(m) => parts.push(c.dim(match m {
            "plan" => "plan mode".into(),
            "acceptEdits" => "accept edits".into(),
            "auto" => "auto mode".into(),
            "dontAsk" => "don't-ask mode".into(),
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
    let within = |t: u64| t.saturating_mul(1000).saturating_sub(now) <= 10 * 60_000;
    let mut out = String::new();
    match c.expires_at {
        Some(e) if c.warm && within(e) => {
            let m = (e.saturating_mul(1000).saturating_sub(now) / 60_000).max(1);
            out += &format!(" · cold in {m}m");
        }
        _ if !c.warm && c.observed => out += " · cold",
        _ => {}
    }
    if let (Some(t), Some(why)) = (c.last_miss_at, &c.last_miss)
        && now.saturating_sub(t.saturating_mul(1000)) <= 10 * 60_000
    {
        out += &format!(" · last miss: {why}");
    }
    out
}

/// Cost, tokens and cache, tests, time: a short ledger, cost first.
fn session(st: &State, u: &Usage, x: &Extras, cfg: &Config, c: &Ctx, now: u64) -> Vec<Row> {
    let mut rows = Vec::new();
    if let Some(cost) = u.cost_usd {
        let (done, _) = st.tasks_done();
        let mut text = format!("${cost:.2}");
        if done > 0 {
            text += &format!(" · ${:.2}/task done", cost / done as f64);
        }
        let costs = recent(&x.samples, |s| s.cost);
        if let Some(r) = rate(&costs) {
            text += &format!(" · ${r:.2}/h");
        }
        if let Some(t) = trend(&costs) {
            text += &format!(" {t}");
        }
        rows.push(vec![c.dim("cost    "), Span::raw(text)]);
    }
    if let Some(t) = &x.tokens
        && t.total().total() > 0
    {
        let total = t.total();
        let mut text = format!(
            "{} · {} out",
            fmt_tokens(total.total()),
            fmt_tokens(total.output)
        );
        if let Some(hit) = total.hit() {
            text += &format!(" · cache {:.0}% hit", hit * 100.0);
            if x.app == crate::app::App::Claude
                && let Some(saved) = crate::tokens::savings(&total, u.cost_usd, &cfg.prices)
            {
                text += &format!(", saved ${saved:.2}");
            }
            text += &cache_suffix(u, now);
        }
        rows.push(vec![c.dim("tokens  "), Span::raw(text)]);
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
    for d in &x.quality {
        rows.push(vec![
            c.span(format!("⚠ {}", d.text()), Style::new().fg(Color::Yellow)),
        ]);
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
            (
                if x.app == crate::app::App::Claude {
                    "claude"
                } else {
                    "model"
                },
                sp.claude,
            ),
            ("tools", sp.tools),
            ("you", sp.waiting),
            ("idle", sp.idle),
        ] {
            // Unattributed time is left out: it explains nothing.
            if pct(ms) > 0.0 {
                text += &format!(" · {name} {:.0}%", pct(ms));
            }
        }
        rows.push(vec![c.dim("time    "), Span::raw(text)]);
    }
    rows
}

/// The git section's rows, colored, for the hub's git panes.
pub(crate) fn git_rows(g: Option<&crate::git::Git>, now: u64) -> Vec<Vec<Span<'static>>> {
    git(
        g,
        &Ctx {
            color: true,
            inner: 0,
        },
        now,
    )
}

/// Branch and working tree, then recent commits and PRs, each led by its age.
fn git(g: Option<&crate::git::Git>, c: &Ctx, now: u64) -> Vec<Row> {
    let Some(g) = g else { return vec![] };
    let mut head = g.branch.clone();
    if g.ahead > 0 {
        head += &format!(" ↑{}", g.ahead);
    }
    if g.behind > 0 {
        head += &format!(" ↓{}", g.behind);
    }
    let tree = match g.changed.len() {
        0 => "clean".to_string(),
        n => format!("{n} changed"),
    };
    let mut rows = vec![vec![
        c.dim("branch  "),
        c.span(head, Style::new().add_modifier(Modifier::BOLD)),
        c.dim(format!(" · {tree}")),
    ]];
    let ago = |t: u64| c.dim(format!("{:>3}  ", age(now.saturating_sub(t))));
    for k in &g.commits {
        rows.push(vec![
            ago(k.t),
            c.dim(format!("{} ", k.hash)),
            Span::raw(k.subject.clone()),
        ]);
    }
    for p in &g.prs {
        let (state, color) = match (p.state.as_str(), p.draft) {
            ("OPEN", true) => ("draft ", Color::DarkGray),
            ("OPEN", _) => ("open  ", Color::Green),
            ("MERGED", _) => ("merged", Color::Magenta),
            _ => ("closed", Color::Red),
        };
        let t = crate::git::parse_utc(&p.updated).unwrap_or(now);
        rows.push(vec![
            ago(t),
            c.span(
                format!("#{} ", p.number),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            c.span(format!("{state} "), Style::new().fg(color)),
            Span::raw(p.title.clone()),
        ]);
    }
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
pub(crate) mod tests {
    use super::*;
    use crate::state::{Agent, Task};

    pub(crate) const NOW: u64 = 1_738_419_600_000;

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

    #[test]
    fn simple_watch_dispatches_unless_recap() {
        let mut cfg = Config::default();
        cfg.view.watch = crate::config::Look::Constellation;
        let draw = |view| {
            let out = lines(
                &State::default(),
                &Usage::default(),
                &Extras::default(),
                &cfg,
                view,
                NOW,
                "12:41",
                80,
                30,
                false,
            );
            to_plain(&out)
        };
        assert!(draw(View::default()).contains("no agents yet"));
        let recap = View {
            recap: true,
            ..Default::default()
        };
        assert!(!draw(recap).contains("no agents yet"));
    }

    pub(crate) fn multi() -> (State, Usage) {
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
                ..Default::default()
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
    fn resume_row() {
        let (mut st, u) = multi();
        assert!(!render(&st, &u, View::default(), 100, 30).contains("resumed"));
        st.resume = Some(crate::state::Resume::default());
        let out = render(&st, &u, View::default(), 100, 30);
        assert!(
            out.contains("↻ resumed\n") || out.contains("↻ resumed "),
            "{out}"
        );
        assert!(!out.contains("resumed after"));
        st.resume = Some(crate::state::Resume {
            away_ms: Some(3 * 3_600_000),
            cold: Some(true),
            rebuild_usd: Some(0.4),
        });
        let out = render(&st, &u, View::default(), 100, 30);
        assert!(
            out.contains("↻ resumed after 3h · cache cold · ~$0.40 to rebuild"),
            "{out}"
        );
    }

    #[test]
    fn background_and_cron_rows() {
        let (mut st, u) = multi();
        let base = render(&st, &u, View::default(), 100, 30);
        assert!(!base.contains('↳') && !base.contains('◷'));
        st.background_names = vec!["subagent: hidden".into(), "shell: npm run dev".into()];
        st.crons = vec!["*/5 * * * * · check deploy".into()];
        let out = render(&st, &u, View::default(), 100, 30);
        assert!(out.contains("↳ shell: npm run dev"), "{out}");
        assert!(!out.contains("subagent: hidden"));
        assert!(out.contains("◷ */5 * * * * · check deploy"), "{out}");
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
            Some("Opus · high · fast · auto mode · ⎇ wt · PR #14 approved · +156 −23")
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
        assert!(l[1].contains("– not tracked"), "{out}");
        assert!(l[2].contains("Opus · PR #14 approved"), "{out}");
        assert!(l[3].contains("new session or /reload-plugins"), "{out}");
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
        assert_eq!(mode("plan").as_deref(), Some("plan mode"));
        assert_eq!(mode("dontAsk").as_deref(), Some("don't-ask mode"));
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
        assert_eq!(row[0].content, "bypass permissions");
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

    fn turn_state() -> (State, Extras) {
        use crate::state::TurnRec;
        let m = 60_000;
        let t0 = NOW - 60 * m;
        let rec = |start, end: Option<u64>, calls, failed, ok, p: &str| TurnRec {
            start,
            end,
            calls,
            failed,
            ok,
            prompt: p.into(),
        };
        let st = State {
            first_t: t0,
            last_t: NOW,
            turn_log: [
                rec(t0, Some(t0 + 252_000), 23, 2, true, "fix the panel"),
                rec(t0 + 20 * m, Some(t0 + 21 * m), 1, 0, false, "second"),
                rec(NOW - 5_000, None, 4, 0, true, "third"),
            ]
            .into(),
            compactions: [
                (t0 + 10 * m, "auto".to_string()),
                (t0 + 15 * m, "manual".into()),
            ]
            .into(),
            ..Default::default()
        };
        let s = |t, ctx, cost| crate::burn::Sample {
            t,
            ctx,
            cost,
            ..Default::default()
        };
        let x = Extras {
            samples: vec![
                s(t0 - 1, Some(10.0), Some(1.0)),
                s(t0 + 252_000, Some(91.0), Some(1.41)),
                s(t0 + 11 * m, Some(18.0), Some(2.0)),
            ],
            ..Default::default()
        };
        (st, x)
    }

    #[test]
    fn weekday_names() {
        assert!(["Wed", "Thu"].contains(&local_weekday(0)));
        let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        for ms in (0..7 * 4).map(|i| i * 6 * 3_600_000) {
            assert!(days.contains(&local_weekday(ms)));
        }
        // Noon UTC keeps seven days distinct in any time zone.
        let mut seen: Vec<_> = (0..7)
            .map(|d| local_weekday(43_200_000 + d * 86_400_000))
            .collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 7);
    }

    #[test]
    fn week_runs_out_row() {
        let (st, u) = (State::default(), Usage::default());
        let eta = 30 * 3_600_000;
        let go = |week_eta| {
            let x = Extras {
                week_eta,
                ..Default::default()
            };
            to_plain(&lines(
                &st,
                &u,
                &x,
                &Config::default(),
                View::default(),
                NOW,
                "12:41",
                80,
                40,
                false,
            ))
        };
        let want = format!(
            "▲ 7d runs out {} ~{} at this pace",
            local_weekday(NOW + eta),
            local_hhmm(NOW + eta)
        );
        assert!(go(Some(eta)).contains(&want), "{}", go(Some(eta)));
        assert!(!go(None).contains("7d runs out"));
    }

    #[test]
    fn trends() {
        let mk = |pts: Vec<(u64, f64, f64)>| Extras {
            samples: pts
                .into_iter()
                .map(|(t, ctx, cost)| crate::burn::Sample {
                    t,
                    ctx: Some(ctx),
                    cost: Some(cost),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let u = Usage {
            cost_usd: Some(5.0),
            ctx_pct: Some(46.0),
            ..Default::default()
        };
        let go = |x: &Extras| {
            to_plain(&lines(
                &State::default(),
                &u,
                x,
                &Config::default(),
                View::default(),
                NOW,
                "12:41",
                100,
                40,
                false,
            ))
        };
        let row = |out: &str, k: &str| out.lines().find(|l| l.contains(k)).unwrap().to_string();
        let sparks = |l: &str| l.chars().filter(|c| "▁▂▃▄▅▆▇█".contains(*c)).count();
        // A sample every 6 s for 40 min: flat for 10 min, then ctx rising and
        // cost stepping $0.83 every 5 min ($10/h, but a burst in the last minute).
        let min = 60_000;
        let pts = |flat_ctx: bool| {
            (0..=400u64)
                .map(|i| i * 6_000)
                .map(|t| {
                    let on = t.saturating_sub(10 * min);
                    let ctx = if flat_ctx {
                        46.0
                    } else {
                        20.0 + on as f64 / 1e5
                    };
                    (t, ctx, (on / (5 * min)) as f64 * 10.0 / 12.0)
                })
                .collect::<Vec<_>>()
        };
        let out = go(&mk(pts(false)));
        let cost = row(&out, "cost");
        assert!(
            cost.contains("$5.00 · $10.00/h ▁"),
            "older samples ignored: {cost}"
        );
        assert!(cost.trim_end_matches([' ', '│']).ends_with('█'), "{cost}");
        assert_eq!(sparks(&cost), 12, "{cost}");
        let ctx = row(&out, "ctx");
        assert!(ctx.contains(" ▁") && sparks(&ctx) == 12, "{ctx}");
        let flat = go(&mk(pts(true)));
        assert_eq!(sparks(&row(&flat, "ctx")), 0, "flat ctx draws no sparkline");
        // Under 10 min of samples: no rate.
        let short = go(&mk(pts(false)
            .into_iter()
            .filter(|p| p.0 >= 31 * min)
            .collect()));
        assert!(!short.contains("/h"), "{short}");
        // Two points: no sparkline either.
        let two = go(&mk(vec![(0, 10.0, 1.0), (60_000, 20.0, 2.0)]));
        assert!(!two.contains('▁'), "{two}");
        // Out-of-order timestamps must not panic.
        go(&mk(vec![
            (5 * min, 30.0, 3.0),
            (1_000, 10.0, 1.0),
            (20 * min, 20.0, 2.0),
        ]));
    }

    fn render_x(st: &State, x: &Extras, w: u16, h: u16) -> String {
        to_plain(&lines(
            st,
            &Usage::default(),
            x,
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            w,
            h,
            false,
        ))
    }

    #[test]
    fn turn_timeline() {
        let (mut st, x) = turn_state();
        let out = render_x(&st, &x, 80, 40);
        let l = |k: &str| out.lines().position(|l| l.contains(k)).unwrap_or(999);
        assert!(
            l("third") < l("second") && l("second") < l("fix the panel"),
            "{out}"
        );
        assert!(
            out.contains("4m12s  23 calls  ✗ 2  $0.41  fix the panel"),
            "{out}"
        );
        assert!(
            out.contains("1 call  ") && out.contains("5s  4 calls  third"),
            "{out}"
        );
        assert!(
            out.contains("⟲ ") && out.contains("compacted (auto) · ctx 91% → 18%"),
            "{out}"
        );
        assert!(
            out.contains("compacted (manual)") && !out.contains("(manual) · ctx"),
            "{out}"
        );
        assert!(
            l("second") < l("compacted (manual)")
                && l("compacted (manual)") < l("compacted (auto)"),
            "{out}"
        );
        st.turn_log.truncate(1);
        assert!(!render_x(&st, &x, 80, 40).contains("├─ turns"));
    }

    #[test]
    fn turns_collapse_first() {
        let (mut st, u) = multi();
        let (t, x) = turn_state();
        st.turn_log = t.turn_log;
        for i in 0..30 {
            st.turn_log.push_back(crate::state::TurnRec {
                start: NOW - 1000 + i,
                prompt: format!("p{i}"),
                ..Default::default()
            });
        }
        let r = |h| {
            to_plain(&lines(
                &st,
                &u,
                &x,
                &Config::default(),
                View::default(),
                NOW,
                "12:41",
                50,
                h,
                false,
            ))
        };
        let rows = |o: &str| {
            o.lines()
                .filter(|l| l.contains(" calls") || l.contains(" call "))
                .count()
        };
        let full = r(100);
        let mid = r(full.lines().count() as u16 - 5);
        assert!(full.contains("├─ turns"));
        assert!(
            mid.contains("├─ turns") && rows(&mid) == rows(&full) - 5,
            "{mid}"
        );
        assert!(
            mid.contains("├─ usage") && mid.contains("├─ session"),
            "{mid}"
        );
        let short = r(14);
        assert!(!short.contains("├─ turns"), "{short}");
        let mut bare = st.clone();
        bare.turn_log.clear();
        let want = to_plain(&lines(
            &bare,
            &u,
            &x,
            &Config::default(),
            View::default(),
            NOW,
            "12:41",
            50,
            14,
            false,
        ));
        assert_eq!(short, want);
    }

    #[test]
    fn needs_you_and_empty() {
        let (mut st, u) = multi();
        st.needs_you = Some("Claude needs your permission to use Bash".into());
        let out = render(&st, &u, View::default(), 50, 40);
        assert!(
            out.contains("◉ NEEDS YOU") && out.contains("◉ Claude needs your permission"),
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
    fn stuck_row_shows_tokens() {
        let (mut st, _) = multi();
        st.errors[1].retries = 3;
        st.errors[1].first_t = 20;
        st.recent.push_back((false, st.errors[1].sig.clone()));
        let samples = [(10, 100), (20, 150), (30, 400)]
            .iter()
            .map(|&(t, tok)| crate::burn::Sample {
                t,
                tok: Some(tok),
                ..Default::default()
            })
            .collect();
        let x = Extras {
            samples,
            ..Default::default()
        };
        let out = render_x(&st, &x, 60, 40);
        assert!(
            out.contains("↻ edit lib.rs failed 4× in a row · 250 tok while stuck"),
            "{out}"
        );
        let out = render_x(&st, &Extras::default(), 60, 40);
        assert!(
            out.contains("↻ edit lib.rs failed 4× in a row") && !out.contains("while stuck"),
            "{out}"
        );
    }

    #[test]
    fn start_note_copy() {
        assert_eq!(
            start_note(31_000, 200_000),
            Some(("at start 31k (16%)".into(), false))
        );
        assert_eq!(
            start_note(46_000, 200_000),
            Some((
                "▲ at start 46k (23%) · system, tools, MCP, CLAUDE.md".into(),
                true
            ))
        );
        assert!(start_note(40_000, 200_000).unwrap().1);
        assert_eq!(
            start_note(46_000, 1_000_000).unwrap(),
            ("at start 46k (5%)".into(), false)
        );
        assert_eq!(start_note(0, 200_000), None);
    }

    #[test]
    fn ctx_at_start_row() {
        let (st, mut u) = multi();
        u.ctx_pct = Some(40.0);
        u.ctx_size = Some(200_000);
        let at = |u: &Usage, start: u64, w: u16| {
            let x = Extras {
                tokens: Some(crate::tokens::TokenCache {
                    main: crate::tokens::Ledger {
                        start,
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            };
            let out = lines(
                &st,
                u,
                &x,
                &Config::default(),
                View::default(),
                NOW,
                "12:41",
                w,
                60,
                false,
            );
            for l in &out {
                assert!(l.width() <= w as usize, "line wider than {w}");
            }
            to_plain(&out)
        };
        let out = at(&u, 46_000, 80);
        let l = |k: &str| out.lines().position(|l| l.contains(k)).unwrap_or(999);
        assert!(l("▲ at start 46k (23%)") < 999, "{out}");
        assert!(l("ctx ") < l("▲ at start 46k (23%)"), "{out}");
        assert!(!at(&u, 0, 80).contains("at start"));
        u.ctx_pct = None;
        assert!(
            !at(&u, 46_000, 80).contains("at start"),
            "no ctx meter, no row"
        );
        at(&u, 46_000, 40);
    }

    #[test]
    fn redo_row_in_header() {
        let (mut st, _) = multi();
        st.redo = Some(crate::state::Redo {
            since: 1,
            n: 3,
            last: "editing a.rs".into(),
            ..Default::default()
        });
        let out = render_x(&st, &Extras::default(), 80, 40);
        assert!(
            out.contains("⟲ redoing work since compaction · 3× · editing a.rs"),
            "{out}"
        );
        let c = Ctx {
            color: true,
            inner: 80,
        };
        let v = st.verdict();
        let rows = header(&st, &Extras::default(), None, &c, v, &Config::default());
        let span = rows
            .iter()
            .flatten()
            .find(|s| s.content.starts_with('⟲'))
            .unwrap();
        assert_eq!(span.style.fg, Some(Color::Yellow));
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
        assert_eq!(st.verdict(), crate::state::Verdict::Done);
        let out = render(&st, &u, View::default(), 60, 60);
        assert!(out.contains("DONE · tests failing"), "{out}");
    }

    #[test]
    fn risky_row_under_the_verdict() {
        let (mut st, u) = multi();
        assert!(!render(&st, &u, View::default(), 80, 40).contains("⚠ ran"));
        for (t, rule, cmd) in [
            (
                NOW - 180_000,
                "discards uncommitted work",
                "git reset --hard",
            ),
            (NOW - 60_000, "delete outside repo", "rm -rf ../x"),
        ] {
            st.apply(&crate::event::Event {
                t,
                e: "PostToolUse".into(),
                tool: Some("Bash".into()),
                ok: Some(true),
                risk: Some((rule.into(), cmd.into())),
                ..Default::default()
            });
        }
        let out = render(&st, &u, View::default(), 80, 40);
        assert!(
            out.contains("⚠ ran rm -rf ../x · delete outside repo · 1m · +1 more"),
            "{out}"
        );
        let cfg = Config {
            ascii: true,
            ..Config::default()
        };
        let asc = to_plain(&lines(
            &st,
            &u,
            &Extras::default(),
            &cfg,
            View::default(),
            NOW,
            "12:41",
            80,
            40,
            false,
        ));
        assert!(asc.contains("! ran rm -rf ../x"), "{asc}");
    }

    #[test]
    fn test_flags_in_header() {
        let (mut st, u) = multi();
        let t = st.turn_start();
        for (i, kind) in ["skip added", "deleted", "edited while failing", "deleted"]
            .iter()
            .enumerate()
        {
            st.test_flags.push_back(crate::state::TestFlag {
                t,
                path: format!("/r/tests/t{i}.py"),
                kind: (*kind).into(),
            });
        }
        let out = render(&st, &u, View::default(), 80, 60);
        assert!(out.contains("⚠ skip added · t0.py"), "{out}");
        assert!(out.contains("⚠ edited while failing · t2.py"), "{out}");
        assert!(!out.contains("t3.py"), "{out}");
        assert!(out.contains("+1 more"), "{out}");
    }

    #[test]
    fn away_summary_row_only_when_quiet() {
        use crate::tokens::{Ledger, TokenCache};
        let (mut st, u) = multi();
        let x = Extras {
            tokens: Some(TokenCache {
                main: Ledger {
                    away: Some("Next, wire the desktop".into()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let show = |st: &State| {
            to_plain(&lines(
                st,
                &u,
                &x,
                &Config::default(),
                View::default(),
                NOW,
                "12:41",
                60,
                60,
                false,
            ))
        };
        assert!(!show(&st).contains("“Next, wire"), "on track");
        st.turn_active = false;
        st.background = 0;
        st.agents.iter_mut().for_each(|a| a.done = true);
        st.tasks
            .iter_mut()
            .for_each(|t| t.status = "completed".into());
        st.errors.clear();
        st.recent.clear();
        st.last_main_result = None;
        assert!(show(&st).contains("“Next, wire the desktop”"), "done");
    }

    #[test]
    fn time_split_leaves_out_unattributed_time() {
        let (mut st, mut u) = multi();
        st.time.tools_ms = 22_000;
        u.duration_ms = Some(100_000);
        u.api_duration_ms = Some(61_000);
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(out.contains("claude 61% · tools 22%"), "{out}");
        assert!(!out.contains("other"), "{out}");
    }

    #[test]
    fn git_section() {
        let (st, _) = multi();
        let mut x = Extras::default();
        assert!(!render_x(&st, &x, 80, 60).contains("─ git"));
        x.git = Some(crate::git::Git {
            branch: "main".into(),
            ahead: 2,
            changed: vec!["a".into(), "b".into(), "c".into()],
            commits: vec![crate::git::Commit {
                hash: "1ec1c52".into(),
                subject: "Merge pull request #38".into(),
                t: NOW - 7_200_000,
            }],
            prs: vec![crate::git::Pr {
                number: 39,
                title: "git section".into(),
                state: "OPEN".into(),
                draft: false,
                updated: "1970-01-01T00:00:00Z".into(),
            }],
            ..Default::default()
        });
        let out = render_x(&st, &x, 80, 60);
        assert!(out.contains("─ git"), "{out}");
        assert!(out.contains("branch  main ↑2 · 3 changed"), "{out}");
        assert!(out.contains(" 2h  1ec1c52 Merge pull request #38"), "{out}");
        assert!(out.contains("#39 open   git section"), "{out}");
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
        assert!(!out.contains("on failed calls"), "{out}");
        st.errors[0].retries = 1;
        st.errors[0].time_ms = 5_000;
        st.reread = 1;
        let failed: u32 = st.errors.iter().map(|e| e.retries + 1).sum();
        let out = render(&st, &u, View::default(), 70, 60);
        assert!(failed > 0);
        assert!(
            out.contains("5s on failed calls · 1 file read twice"),
            "{out}"
        );
    }

    fn render_cfg(st: &State, u: &Usage, x: &Extras, cfg: &Config) -> String {
        to_plain(&lines(
            st,
            u,
            x,
            cfg,
            View::default(),
            NOW,
            "12:41",
            50,
            60,
            false,
        ))
    }

    #[test]
    fn calls_section_lists_main_calls() {
        use crate::state::CallRec;
        let (mut st, u) = multi();
        let call = |t, act: &str, ok, dur| CallRec {
            t,
            tool: "Bash".into(),
            act: act.into(),
            ok,
            dur,
            ..Default::default()
        };
        st.agents[0].recent = [
            call(NOW - 30_000, "$ cargo build", Some(true), Some(4_000)),
            call(NOW - 20_000, "$ cargo fmt", Some(false), Some(1_000)),
            call(NOW - 8_000, "$ cargo test", None, None),
        ]
        .into();
        let out = render_cfg(&st, &u, &Extras::default(), &Config::default());
        let l = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s}: {out}"));
        assert!(out.contains("├─ calls"), "{out}");
        let calls = &out[l("├─ calls")..l("├─ usage")];
        assert!(!calls.contains('$'), "{out}");
        assert!(
            l(" 8s Bash  cargo test ") < l(" 20s Bash  cargo fmt "),
            "{out}"
        );
        assert!(
            out.contains("cargo fmt") && out.contains("✗ 1.0s │"),
            "{out}"
        );
        assert!(l("cargo fmt") < l("cargo build"), "{out}");
        let hid = render_cfg(&st, &u, &Extras::default(), &hiding(&["calls"]));
        assert!(!hid.contains("├─ calls"), "{hid}");
    }

    #[test]
    fn quality_rows_only_when_deviating() {
        let (st, u) = multi();
        let x = Extras {
            quality: vec![crate::quality::Deviation {
                what: "reads/edit",
                now: 2.0,
                usual: 6.1,
            }],
            ..Extras::default()
        };
        let out = render_cfg(&st, &u, &x, &Config::default());
        assert!(out.contains("⚠ reads/edit 2.0 · your usual 6.1"), "{out}");
        let none = render_cfg(&st, &u, &Extras::default(), &Config::default());
        assert!(!none.contains("your usual"), "{none}");
        let hid = render_cfg(&st, &u, &x, &hiding(&["session"]));
        assert!(!hid.contains("your usual"), "{hid}");
    }

    fn hiding(ids: &[&str]) -> Config {
        Config {
            hide_panel: ids.iter().map(|s| s.to_string()).collect(),
            ..Config::default()
        }
    }

    #[test]
    fn statusline_hide_leaves_the_panel_alone() {
        let (st, u) = multi();
        let cfg = Config {
            hide: vec!["agents".into(), "tasks".into(), "errors".into()],
            ..Config::default()
        };
        let out = render_cfg(&st, &u, &Extras::default(), &cfg);
        assert!(out.contains("├─ agents"), "{out}");
        assert!(out.contains("├─ errors"), "{out}");
        assert!(out.contains("4/7 tasks"), "{out}");
    }

    #[test]
    fn hide_drops_panel_sections() {
        let (st, u) = multi();
        let x = Extras::default();
        let all = render_cfg(&st, &u, &x, &Config::default());
        for t in ["├─ agents", "├─ errors", "├─ usage", "├─ session"] {
            assert!(all.contains(t), "{t} in {all}");
        }
        let out = render_cfg(&st, &u, &x, &hiding(&["agents", "usage", "turns"]));
        assert!(!out.contains("├─ agents"), "{out}");
        assert!(!out.contains("├─ usage"), "{out}");
        assert!(out.contains("├─ errors"), "{out}");
        assert!(out.contains("├─ session"), "{out}");
    }

    #[test]
    fn hide_tasks_drops_the_checklist_bar() {
        let (mut st, u) = multi();
        let x = Extras::default();
        assert!(render_cfg(&st, &u, &x, &Config::default()).contains("4/7 tasks"));
        let out = render_cfg(&st, &u, &x, &hiding(&["tasks"]));
        assert!(!out.contains("tasks"), "{out}");
        st.tasks.clear();
        let x = Extras {
            plan: Some(crate::state::PlanFile {
                name: "plan.md".into(),
                done: 2,
                total: 5,
            }),
            ..Extras::default()
        };
        assert!(render_cfg(&st, &u, &x, &Config::default()).contains("2/5 · plan.md"));
        assert!(!render_cfg(&st, &u, &x, &hiding(&["tasks"])).contains("plan.md"));
    }

    #[test]
    fn features_on_by_default() {
        assert!(Config::default().hide.is_empty());
        assert!(Config::default().hide_panel.is_empty());
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
        assert!(!out.contains("on failed calls"), "{out}");
        assert!(out.contains("1 file read twice"), "{out}");
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
            .find(|l| l.contains("tokens  "))
            .unwrap()
            .trim_end_matches('│')
            .trim_end()
            .to_string();
        // What follows the hit rate, minus the savings: just the cache's own notes.
        let tail = r.split_once("% hit").unwrap().1;
        let notes = tail.find(" · ").map_or("", |i| &tail[i..]);
        format!("% hit{notes}")
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
        let huge = CacheState {
            last_miss_at: Some(u64::MAX),
            last_miss: Some("x".into()),
            ..warm(u64::MAX)
        };
        cache_row(Some(huge)); // a garbage usage.json must not panic
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
        assert!(out.contains("└─ ✓ 2 done (a)"), "{out}");
        let all = View {
            expand_agents: true,
            ..View::default()
        };
        let out = render(&st, &u, all, 70, 60);
        assert!(
            out.contains("✓ oldone") && out.contains("✓ helpers"),
            "{out}"
        );
    }

    fn long_list() -> (State, Usage, Extras) {
        use crate::state::Agent;
        use crate::tokens::{Ledger, TokenCache};
        let (mut st, u) = multi();
        st.agents.retain(|a| a.id == "main");
        let mut led = std::collections::BTreeMap::new();
        let mut add =
            |st: &mut State, id: String, kind: &str, calls: u32, model: Option<&str>, i: u64| {
                let ended = NOW - 60_000 - i * 1_000;
                st.agents.push(Agent {
                    id: id.clone(),
                    kind: kind.into(),
                    calls,
                    started: ended - if calls > 0 { 30_000 } else { 100 },
                    last_t: ended,
                    ended_t: Some(ended),
                    done: true,
                    ..Default::default()
                });
                if let Some(m) = model {
                    led.insert(
                        id,
                        Ledger {
                            output: 1000,
                            last_model: Some(m.into()),
                            ..Default::default()
                        },
                    );
                }
            };
        for i in 0..2 {
            add(
                &mut st,
                format!("o{i}"),
                "general-purpose",
                3,
                Some("claude-opus-5-5"),
                i,
            );
        }
        for i in 0..30 {
            add(
                &mut st,
                format!("s{i}"),
                "general-purpose",
                3,
                Some("claude-sonnet-5-5"),
                10 + i,
            );
        }
        for i in 0..3 {
            add(&mut st, format!("h{i}"), "agent", 0, None, 50 + i);
        }
        st.agents.push(Agent {
            id: "run".into(),
            kind: "runner".into(),
            calls: 1,
            started: NOW - 5_000,
            last_t: NOW - 1_000,
            ..Default::default()
        });
        let x = Extras {
            tokens: Some(TokenCache {
                agents: led,
                ..Default::default()
            }),
            ..Default::default()
        };
        (st, u, x)
    }

    fn render_ex(st: &State, u: &Usage, x: &Extras, view: View) -> String {
        to_plain(&lines(
            st,
            u,
            x,
            &Config::default(),
            view,
            NOW,
            "12:41",
            90,
            80,
            false,
        ))
    }

    #[test]
    fn long_list_default_folds_with_summary() {
        let (st, u, x) = long_list();
        let out = render_ex(&st, &u, &x, View::default());
        assert!(out.contains("runner"), "{out}");
        assert!(out.contains("└─ ✓ 32 done (a)"), "{out}");
    }

    #[test]
    fn long_list_expanded_groups() {
        let (st, u, x) = long_list();
        let v = View {
            expand_agents: true,
            ..View::default()
        };
        let out = render_ex(&st, &u, &x, v);
        assert!(
            out.contains("✓ 30× general-purpose · sonnet-5-5 · 30k tok"),
            "{out}"
        );
        assert!(
            out.contains("✓ 2× general-purpose · opus-5-5 · 2.0k tok"),
            "{out}"
        );
        assert!(out.contains("✓ 3× helpers"), "{out}");
        assert!(out.contains("runner"), "{out}");
        let rows = out
            .lines()
            .filter(|l| l.contains("─✓") || l.contains("─⠋"))
            .count();
        assert_eq!(rows, 4, "{out}");
    }

    #[test]
    fn two_done_agents_show_without_fold() {
        let (mut st, u, x) = long_list();
        st.agents
            .retain(|a| a.id == "main" || a.id == "o0" || a.id == "o1");
        let out = render_ex(&st, &u, &x, View::default());
        assert!(!out.contains("more done"), "{out}");
        assert_eq!(out.matches("general-pur").count(), 2, "{out}");
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
            files: vec![("src/a.rs".into(), 2), ("tests/a.rs".into(), 1)],
            groups: vec![crate::recap::Group {
                cause: "“fix the token refresh”".into(),
                files: vec![
                    crate::recap::Changed {
                        path: "src/a.rs".into(),
                        edits: 2,
                        added: 30,
                        removed: 4,
                        look: None,
                    },
                    crate::recap::Changed {
                        path: "tests/a.rs".into(),
                        edits: 1,
                        added: 12,
                        removed: 0,
                        look: Some("test"),
                    },
                ],
            }],
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
        assert!(text.starts_with("+- ARI -"), "{text}");
        insta::assert_snapshot!("ascii", text);
    }

    #[test]
    fn two_columns_when_wide() {
        let (st, u) = multi();
        let at = |w: u16, ascii: bool| {
            let cfg = Config {
                ascii,
                ..Config::default()
            };
            lines(
                &st,
                &u,
                &Extras::default(),
                &cfg,
                View::default(),
                NOW,
                "12:41",
                w,
                40,
                false,
            )
        };
        let wide = at(140, false);
        for l in &wide {
            assert_eq!(l.width(), 140, "{:?}", to_plain(std::slice::from_ref(l)));
        }
        let text = to_plain(&wide);
        assert!(
            text.lines()
                .any(|l| l.contains("─ agents ") && l.contains("─ usage ")),
            "{text}"
        );
        let asc = at(140, true);
        assert_eq!(asc.len(), wide.len());
        for l in &asc {
            assert_eq!(l.width(), 140);
        }
        let narrow = to_plain(&at(119, false));
        assert!(
            !narrow
                .lines()
                .any(|l| l.contains("─ agents ") && l.contains("─ usage ")),
            "{narrow}"
        );
        assert!(narrow.contains("├─ agents ─"), "{narrow}");
        insta::assert_snapshot!("wide_140", text);
    }

    #[test]
    fn short_wide_collapses_per_column() {
        let (st, u) = multi();
        let out = render(&st, &u, View::default(), 140, 12);
        assert!(out.lines().count() <= 12, "{out}");
    }

    #[test]
    fn project_and_session_title_in_header() {
        let (st, mut u) = multi();
        u.dir = Some("/x/tankobon/".into());
        u.session_name = Some("next milestone".into());
        let out = render(&st, &u, View::default(), 50, 40);
        let first: Vec<&str> = out.lines().take(2).collect();
        assert!(first[0].starts_with("╭─ ARI ─"), "{out}");
        assert!(first[0].contains("─ /x/tankobon · "), "{out}");
        assert!(first[1].contains("● ON TRACK · next milestone"), "{out}");
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

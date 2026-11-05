//! The full panel, as styled lines. Pure: everything `ari watch` draws comes from here.

use crate::bar::{Extras, cells, countdown, fmt_tokens, spinner};
use crate::config::Config;
use crate::state::{AgentStatus, ErrorRec, State, Verdict};
use crate::usage::{Limit, Usage};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View {
    pub expand_errors: bool,
    pub expand_agents: bool,
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
fn tilde(dir: &str) -> String {
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

fn age(ms: u64) -> String {
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
            rows: session(st, u, x, cfg, &c),
        },
    ];
    collapse(&mut sections, height as usize);

    let border = c.dim("");
    let bstyle = border.style;
    let mut out = Vec::new();
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
    out.push(Line::from(fit(title, w)));

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
    if cfg.ascii {
        crate::ascii::map_lines(out)
    } else {
        out
    }
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
        rows.push(vec![c.dim("no plan yet")]);
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
            Span::raw(pad(&a.kind)),
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
        let mut parts = vec![format!("{failed} failed call{s}"), dur(time)];
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

fn session(st: &State, u: &Usage, x: &Extras, cfg: &Config, c: &Ctx) -> Vec<Row> {
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
            let label = format!(" {:.0}% hit", hit * 100.0);
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
    if let Some(last) = st.test_runs.back() {
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
        let runs: Vec<_> = st.test_runs.iter().rev().take(10).rev().collect();
        if runs.len() > 1 {
            let lo = runs.iter().map(|r| r.pass).min().unwrap_or(0);
            let hi = runs.iter().map(|r| r.pass).max().unwrap_or(0);
            row.push(Span::raw(" · "));
            for r in &runs {
                let i = if hi > lo {
                    (r.pass - lo) * 7 / (hi - lo)
                } else {
                    7
                };
                let bar = "▁▂▃▄▅▆▇█".chars().nth(i as usize).unwrap_or('█');
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
        ] {
            if pct(ms) > 0.0 {
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
            st.test_runs.push_back(TestRun { t: NOW, pass, fail });
        }
        let out = render(&st, &u, View::default(), 60, 60);
        assert!(out.contains("tests   ✗ 2/69 · ▁▄█▇ last 4 runs"), "{out}");
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

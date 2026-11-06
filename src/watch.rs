//! `ari watch`: the full panel in a terminal pane.

use crate::panel::{self, View};
use crate::sessions::{self, SessionInfo};
use crate::state::{State, Verdict};
use crate::usage::{self, Usage};
use crate::{bar, burn, config, hook, paths, state, tokens};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Alignment;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const FAST: Duration = Duration::from_millis(100); // 10 fps while something animates
const SLOW: Duration = Duration::from_secs(1); // 1 fps idle
const REPICK: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq)]
enum Mode {
    Follow(String),
    Fixed(String),
    All,
}

struct App {
    root: PathBuf,
    mode: Mode,
    cfg: config::Config,
    view: View,
    dir: Option<PathBuf>,
    st: State,
    usage: Usage,
    x: bar::Extras,
    mtimes: (Option<SystemTime>, Option<SystemTime>),
    picked_at: Option<Instant>,
    /// When the recap view was last built, and whether events arrived since.
    recap_built: Option<Instant>,
    recap_dirty: bool,
    last_verdict: Option<Verdict>,
    color: bool,
    /// A live terminal: the `--all` list is selectable (not `--once`).
    interactive: bool,
    /// The session opened from the `--all` list: it stays put while the urgency order shifts.
    pinned: Option<PathBuf>,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn load_usage(dir: &Path) -> Usage {
    std::fs::read(dir.join("usage.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .map(|v| usage::from_value(&v))
        .unwrap_or_default()
}

impl App {
    fn refresh(&mut self) {
        if self.picked_at.is_none_or(|t| t.elapsed() >= REPICK) {
            self.picked_at = Some(Instant::now());
            let dir = match &self.mode {
                Mode::Follow(cwd) => {
                    sessions::pick(&sessions::list(&self.root), cwd).map(|s| s.dir.clone())
                }
                Mode::Fixed(id) => paths::session_dir(&self.root, id),
                Mode::All => None,
            };
            if dir != self.dir {
                self.dir = dir;
                self.mtimes = (None, None);
                self.st = State::default();
                self.usage = Usage::default();
                self.x = bar::Extras::default();
            }
        }
        let Some(dir) = &self.dir else { return };
        let now = (
            mtime(&dir.join("events.jsonl")),
            mtime(&dir.join("usage.json")),
        );
        let changed = now != self.mtimes;
        self.recap_dirty |= now.0 != self.mtimes.0;
        // Built on demand: a full replay plus `git log` is too much for every refresh.
        if !self.view.recap {
            self.x.recap = None;
            self.recap_built = None;
        } else if recap_due(self.recap_built, self.recap_dirty) {
            self.x.recap = Some(crate::recap::build(
                dir,
                crate::recap::Since::Prompt,
                hook::now_ms(),
            ));
            self.recap_built = Some(Instant::now());
            self.recap_dirty = false;
        }
        if changed {
            self.mtimes = now;
            self.st = state::load_session(dir);
            self.usage = load_usage(dir);
            self.x.tokens = Some(tokens::update(
                dir,
                &self.st,
                tokens::transcript_for(dir, &self.usage).as_deref(),
            ));
            let now = hook::now_ms();
            self.x.burn_eta = burn::projection(&burn::load(dir, now), now);
            self.x.samples = burn::recent(dir);
            self.x.compacts_in = burn::forecast(dir, &self.st, &self.cfg);
            self.x.plan = state::plan_file(&self.st);
        }
    }

    fn animating(&self, now: u64) -> bool {
        self.mode == Mode::All || self.st.turn_active || self.st.running_agents(now) > 0
    }

    fn cycle(&mut self) {
        let now = hook::now_ms();
        let live: Vec<SessionInfo> = sessions::list(&self.root)
            .into_iter()
            .filter(|s| sessions::is_live(s, now))
            .collect();
        if live.is_empty() {
            return;
        }
        let cur = self
            .dir
            .as_ref()
            .and_then(|d| live.iter().position(|s| &s.dir == d));
        let next = &live[cur.map_or(0, |i| (i + 1) % live.len())];
        self.mode = Mode::Fixed(next.id.clone());
        self.picked_at = None;
    }

    fn lines(&mut self, width: u16, height: u16) -> Vec<Line<'static>> {
        self.frame_lines(width, height).0
    }

    /// The frame's lines, and the entry count while the `--all` list is what's drawn.
    fn frame_lines(&mut self, width: u16, height: u16) -> (Vec<Line<'static>>, Option<usize>) {
        let now = hook::now_ms();
        if self.mode == Mode::All {
            if !self.view.open {
                self.pinned = None;
            } else if self.pinned.is_none() {
                let entries = all_entries(&self.root, &self.cfg, now);
                self.pinned = entries.get(self.view.sel).map(|e| e.dir.clone());
            }
            if let Some(dir) = self.pinned.clone() {
                if dir.is_dir() {
                    let st = state::load_session(&dir);
                    let usage = load_usage(&dir);
                    let x = bar::Extras {
                        tokens: Some(tokens::cached(&dir)),
                        samples: burn::recent(&dir),
                        plan: state::plan_file(&st),
                        ..Default::default()
                    };
                    return (self.panel(&dir, &st, &usage, x, width, height, now), None);
                }
                self.pinned = None; // gone: back to the list, tui_loop clears `open`
            }
            let entries = all_entries(&self.root, &self.cfg, now);
            let sel = self.interactive.then_some(self.view.sel);
            let lines = list_lines(&entries, sel, &self.cfg, now, width);
            return (lines, self.interactive.then_some(entries.len()));
        }
        let Some(dir) = &self.dir else {
            return (waiting(&self.mode), None);
        };
        let lines = self.panel(
            dir,
            &self.st,
            &self.usage,
            self.x.clone(),
            width,
            height,
            now,
        );
        (lines, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn panel(
        &self,
        dir: &Path,
        st: &State,
        usage: &Usage,
        mut x: bar::Extras,
        width: u16,
        height: u16,
        now: u64,
    ) -> Vec<Line<'static>> {
        x.conflicts = crate::conflicts::cached(&self.root, dir, now);
        x.transcript_age_ms =
            tokens::transcript_for(dir, usage).and_then(|t| tokens::transcript_age(&t, now));
        // Live, the keys are listed under the panel; `--once` has no keys.
        let hint = self.interactive.then(|| {
            let back = if self.mode == Mode::All {
                "esc list"
            } else {
                "tab next session"
            };
            let text = format!("e errors · a agents · r recap · {back} · q quit");
            let text = crate::event::truncate(&text, width as usize);
            let text = if self.cfg.ascii {
                crate::ascii::map(&text)
            } else {
                text
            };
            let dim = Style::new().add_modifier(Modifier::DIM);
            Line::from(Span::styled(
                text,
                if self.color { dim } else { Style::default() },
            ))
        });
        let mut lines = panel::lines(
            st,
            usage,
            &x,
            &self.cfg,
            self.view,
            now,
            &panel::local_hhmm(now),
            width,
            height.saturating_sub(hint.is_some() as u16),
            self.color,
        );
        lines.extend(hint);
        lines
    }
}

fn waiting(mode: &Mode) -> Vec<Line<'static>> {
    let what = match mode {
        Mode::Follow(cwd) => format!("waiting for a Claude Code session in {cwd}"),
        Mode::Fixed(id) => format!("no session {id}"),
        Mode::All => String::new(),
    };
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "ARI (-_-)zz",
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(what),
        Line::from(Span::styled(
            "ari watch --all lists every session · q quits",
            Style::new().add_modifier(Modifier::DIM),
        )),
    ]
}

/// One session in `ari watch --all`.
struct Entry {
    /// Urgency: the verdict's order (NEEDS YOU first), unhooked idle sessions last.
    rank: u8,
    verdict: String,
    style: Style,
    project: String,
    title: Option<String>,
    detail: String,
    ctx: Option<f64>,
    cost: Option<f64>,
    age_ms: u64,
    dir: PathBuf,
    /// Account rate limits as this session last saw them.
    limits: Option<(Option<usage::Limit>, Option<usage::Limit>)>,
}

fn all_entries(root: &Path, cfg: &config::Config, now: u64) -> Vec<Entry> {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let mut entries = Vec::new();
    for s in sessions::list(root)
        .into_iter()
        .filter(|s| sessions::is_live(s, now))
    {
        let u = load_usage(&s.dir);
        let st = state::load_session(&s.dir);
        let x = bar::Extras {
            transcript_age_ms: tokens::transcript_for(&s.dir, &u)
                .and_then(|t| tokens::transcript_age(&t, now)),
            conflicts: crate::conflicts::cached(root, &s.dir, now),
            plan: state::plan_file(&st),
            ..Default::default()
        };
        // The row's own columns and the header carry these; "✗ 0" is noise in a list.
        let mut row_cfg = cfg.clone();
        row_cfg.max_width = 0;
        row_cfg.ascii = false;
        row_cfg
            .hide
            .extend(["5h", "7d", "ctx", "cost", "tokens"].map(String::from));
        if st.open_errors() == 0 {
            row_cfg.hide.push("errors".into());
        }
        let row = bar::render(&st, &u, &x, &row_cfg, now, 10_000, false);
        let (verdict, detail) = row.split_once(" │ ").unwrap_or((row.as_str(), ""));
        let v = st.verdict_with(now, cfg.thresholds.stuck_repeats);
        let (rank, style) = if st.first_t > 0 {
            (v as u8, panel::verdict_style(v))
        } else if x.looks_active() {
            (
                Verdict::OnTrack as u8,
                Style::new()
                    .fg(ratatui::style::Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (Verdict::Idle as u8 + 1, dim)
        };
        let mut detail = match (&st.needs_you, rank == Verdict::NeedsYou as u8) {
            (Some(ask), true) => ask.clone(),
            _ => detail.replace(" │ ", " · "),
        };
        if detail.is_empty()
            && rank >= Verdict::Done as u8
            && let Some(a) = tokens::cached(&s.dir).main.away
        {
            detail = format!("“{a}”");
        }
        entries.push(Entry {
            rank,
            verdict: verdict.to_string(),
            style,
            project: u
                .project
                .clone()
                .or_else(|| s.cwd.as_deref()?.rsplit('/').next().map(String::from))
                .unwrap_or_else(|| s.id.chars().take(8).collect()),
            title: u.session_name.clone(),
            // What it's asking beats what it was doing.
            detail,
            ctx: u.ctx_pct,
            cost: u.cost_usd,
            age_ms: now.saturating_sub(s.last_ms),
            dir: s.dir.clone(),
            limits: (u.five_h.is_some() || u.seven_d.is_some()).then_some((u.five_h, u.seven_d)),
        });
    }
    // Stable: equally urgent sessions keep most-recent-first.
    entries.sort_by_key(|e| e.rank);
    entries
}

/// The `--all` list; `sel` highlights a row and adds the key hint (None for `--once`).
fn list_lines(
    entries: &[Entry],
    sel: Option<usize>,
    cfg: &config::Config,
    now: u64,
    width: u16,
) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthStr;
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let paint = |s: Style| if color { s } else { Style::default() };
    let dim = Style::new().add_modifier(Modifier::DIM);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    // Rate limits are per account: show one session's once, not one per row.
    let limits = entries
        .iter()
        .filter(|e| e.limits.is_some())
        .min_by_key(|e| e.age_ms)
        .and_then(|e| e.limits);
    let total: f64 = entries.iter().filter_map(|e| e.cost).sum();

    let w = width as usize;
    let needs = entries
        .iter()
        .filter(|e| e.rank == Verdict::NeedsYou as u8)
        .count();
    let mut head = vec![
        Span::styled("ARI", paint(bold)),
        Span::raw(format!(
            " · {} live session{}",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" }
        )),
    ];
    if needs > 0 {
        head.push(Span::raw(" · "));
        head.push(Span::styled(
            format!("{needs} need{} you", if needs == 1 { "s" } else { "" }),
            paint(panel::verdict_style(Verdict::NeedsYou)),
        ));
    }
    let mut acct = Vec::new();
    if let Some((five, seven)) = limits {
        for (name, l) in [("5h", five), ("7d", seven)] {
            if let Some(l) = l {
                let reset = if l.resets_at > 0 {
                    format!(" {}", bar::countdown(l.resets_at, now))
                } else {
                    String::new()
                };
                acct.push(format!("{name} {:.0}%{reset}", l.pct));
            }
        }
    }
    if total > 0.0 {
        acct.push(format!("${total:.2} total"));
    }
    let acct = acct.join(" · ");
    let used: usize = head.iter().map(|s| s.content.width()).sum();
    if !acct.is_empty() && used + 2 + acct.width() <= w {
        head.push(Span::raw(" ".repeat(w - used - acct.width())));
        head.push(Span::styled(acct, paint(dim)));
    }
    let mut out = vec![Line::from(head), Line::from("")];

    // Columns: verdict │ project · title │ what it's doing │ ctx, cost, last active.
    let vw = entries
        .iter()
        .map(|e| e.verdict.width())
        .max()
        .unwrap_or(0)
        .min(22);
    let right = if w >= 70 { 26 } else { 0 };
    let label = |e: &Entry| match &e.title {
        Some(t) => format!("{} · {t}", e.project),
        None => e.project.clone(),
    };
    let full = entries.iter().map(|e| label(e).width()).max().unwrap_or(0);
    // Which session it is matters more than what it's doing: the label gets room first.
    let mut lw = full
        .min(36)
        .min(w.saturating_sub(vw + right + 4 + 12))
        .max(12);
    let mut room = w.saturating_sub(vw + lw + right + 4);
    // Too little room to read what it's doing: that goes on its own line under the session.
    let below = room < 24;
    if below {
        lw = full.min(w.saturating_sub(vw + right + 4)).max(12);
        room = w.saturating_sub(vw + 2);
    }
    for (i, e) in entries.iter().enumerate() {
        let quiet = e.rank >= Verdict::Done as u8;
        let mut spans = vec![
            Span::styled(e.verdict.clone(), paint(e.style)),
            Span::raw(" ".repeat(vw.saturating_sub(e.verdict.width()) + 2)),
        ];
        let text = crate::event::truncate(&label(e), lw);
        let (p, t) = text.split_at(text.floor_char_boundary(e.project.len()));
        spans.push(Span::styled(
            p.to_string(),
            paint(if quiet { dim } else { bold }),
        ));
        spans.push(Span::styled(t.to_string(), paint(dim)));
        let detail = Span::styled(
            crate::event::truncate(&e.detail, room),
            paint(if quiet { dim } else { Style::default() }),
        );
        spans.push(Span::raw(" ".repeat(lw.saturating_sub(text.width()) + 2)));
        let mut under = None;
        if below {
            if !e.detail.is_empty() {
                under = Some(Line::from(vec![Span::raw(" ".repeat(vw + 2)), detail]));
            }
        } else if right > 0 {
            let pad = room.saturating_sub(detail.content.width());
            spans.push(detail);
            spans.push(Span::raw(" ".repeat(pad)));
        } else {
            spans.push(detail);
        }
        if right > 0 {
            let ctx = e.ctx.map(|p| format!("ctx {p:.0}%")).unwrap_or_default();
            let cost = e.cost.map(|c| format!("${c:.2}")).unwrap_or_default();
            spans.push(Span::styled(
                format!("  {ctx:>7} {cost:>8} {:>4} ago", panel::age(e.age_ms)),
                paint(dim),
            ));
        }
        for line in std::iter::once(Line::from(spans)).chain(under) {
            out.push(if sel == Some(i) {
                line.patch_style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                line
            });
        }
    }
    if sel.is_some() {
        out.push(Line::from(Span::styled(
            "↑↓ select · enter open · esc back · q quit",
            dim,
        )));
    }
    if cfg.ascii {
        crate::ascii::map_lines(out)
    } else {
        out
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("ari watch: {msg}");
    std::process::exit(2)
}

pub fn run(args: &[String]) -> ! {
    let mut mode = None;
    let mut once = false;
    let mut ascii = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--all" => mode = Some(Mode::All),
            "--once" => once = true,
            "--ascii" => ascii = true,
            "--session" => {
                let id = it.next().unwrap_or_else(|| fail("--session needs an id"));
                if !paths::valid_session_id(id) {
                    fail("invalid session id");
                }
                mode = Some(Mode::Fixed(id.clone()));
            }
            other => fail(&format!(
                "unknown option {other} (try --session <id>, --all, --once, --ascii)"
            )),
        }
    }
    let root = paths::state_root().unwrap_or_else(|| fail("HOME is not set"));
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut app = App {
        root,
        mode: mode.unwrap_or(Mode::Follow(cwd)),
        cfg: {
            let mut c = config::load();
            c.ascii |= ascii;
            c
        },
        view: View::default(),
        dir: None,
        st: State::default(),
        usage: Usage::default(),
        x: bar::Extras::default(),
        mtimes: (None, None),
        picked_at: None,
        recap_built: None,
        recap_dirty: false,
        last_verdict: None,
        color: std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
        interactive: false,
        pinned: None,
    };
    app.refresh();

    if once || !std::io::stdout().is_terminal() {
        let width = std::env::var("COLUMNS")
            .ok()
            .and_then(|c| c.parse().ok())
            .unwrap_or(80);
        println!("{}", panel::to_plain(&app.lines(width, u16::MAX)));
        std::process::exit(0)
    }

    app.interactive = true;
    let result = run_tui(|view, tab, w, h| {
        if tab {
            app.cycle();
        }
        app.view = view;
        app.refresh();
        let now = hook::now_ms();
        let v = app.st.verdict_with(now, app.cfg.thresholds.stuck_repeats);
        if app.cfg.bell && v == Verdict::NeedsYou && app.last_verdict.is_some_and(|l| l != v) {
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x07").and_then(|_| out.flush());
        }
        app.last_verdict = Some(v);
        let align = if app.dir.is_none() && app.mode != Mode::All {
            Alignment::Center
        } else {
            Alignment::Left
        };
        let (lines, list_len) = app.frame_lines(w, h);
        (lines, app.animating(now), align, list_len)
    });
    if let Err(e) = result {
        fail(&e.to_string());
    }
    std::process::exit(0)
}

/// Apply a key to the view; true means quit. `list_len` is `Some` while the `--all` list shows.
fn key(view: &mut View, code: KeyCode, list_len: Option<usize>) -> bool {
    match (code, list_len) {
        (KeyCode::Char('q'), _) => return true,
        (KeyCode::Esc, _) if view.open => view.open = false,
        (KeyCode::Esc, _) if view.recap => view.recap = false,
        (KeyCode::Esc, _) => return true,
        (KeyCode::Up | KeyCode::Char('k'), Some(_)) => view.sel = view.sel.saturating_sub(1),
        (KeyCode::Down | KeyCode::Char('j'), Some(n)) => {
            view.sel = (view.sel + 1).min(n.saturating_sub(1))
        }
        (KeyCode::Enter, Some(n)) if n > 0 => view.open = true,
        (KeyCode::Char('e'), _) => view.expand_errors ^= true,
        (KeyCode::Char('a'), _) => view.expand_agents ^= true,
        (KeyCode::Char('r'), _) => view.recap ^= true,
        _ => {}
    }
    false
}

/// A frame: lines to draw, whether something animates (10 fps vs 1 fps), alignment, and
/// `Some(entries)` while the selectable `--all` list is drawn.
pub type Frame = (Vec<Line<'static>>, bool, Alignment, Option<usize>);

/// Own the terminal (alternate screen, raw mode, restored on exit or panic) and redraw
/// until `q`. `frame(view, tab_pressed, width, height)` produces each frame.
pub fn run_tui(mut frame: impl FnMut(View, bool, u16, u16) -> Frame) -> std::io::Result<()> {
    let mut term = ratatui::init();
    let result = tui_loop(&mut term, &mut frame);
    ratatui::restore();
    result
}

fn tui_loop(
    term: &mut ratatui::DefaultTerminal,
    frame: &mut impl FnMut(View, bool, u16, u16) -> Frame,
) -> std::io::Result<()> {
    let mut view = View::default();
    let mut tab = false;
    let mut list_len;
    loop {
        let size = term.size()?;
        let (lines, animating, align, n) =
            frame(view, std::mem::take(&mut tab), size.width, size.height);
        list_len = n;
        if let Some(n) = n {
            view.sel = view.sel.min(n.saturating_sub(1));
            view.open = false; // a list frame means nothing is open (or it vanished)
        }
        term.draw(|f| f.render_widget(Paragraph::new(lines).alignment(align), f.area()))?;
        if !event::poll(if animating { FAST } else { SLOW })? {
            continue;
        }
        if let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(());
            }
            if k.code == KeyCode::Tab {
                tab = true;
            } else if key(&mut view, k.code, list_len) {
                return Ok(());
            }
        }
    }
}

/// Build the recap view now: never built yet, or new events and the last build is 5 s old.
fn recap_due(built: Option<Instant>, dirty: bool) -> bool {
    built.is_none_or(|b| dirty && b.elapsed() >= Duration::from_secs(5))
}

#[cfg(test)]
mod key_tests {
    use super::*;

    #[test]
    fn all_list_puts_needs_you_first_and_limits_in_the_header() {
        let root = std::env::temp_dir().join(format!("ari-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let now = hook::now_ms();
        let session = |id: &str, title: &str, events: &[String]| {
            let d = root.join("sessions").join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("events.jsonl"), events.join("\n") + "\n").unwrap();
            let u = serde_json::json!({"session_id": id, "session_name": title,
                "workspace": {"project_dir": "/x/proj"}, "cost": {"total_cost_usd": 1.5},
                "rate_limits": {"seven_day": {"used_percentage": 58.0, "resets_at": 0}}});
            std::fs::write(d.join("usage.json"), u.to_string()).unwrap();
        };
        let ev = |dt: u64, rest: &str| format!(r#"{{"t":{},{rest}}}"#, now - dt);
        let start = ev(60_000, r#""e":"SessionStart""#);
        let prompt = ev(50_000, r#""e":"UserPromptSubmit","msg":"go""#);
        session(
            "aaaa",
            "asks",
            &[
                start.clone(),
                prompt.clone(),
                ev(
                    40_000,
                    r#""e":"Notification","kind":"permission_prompt","msg":"Claude needs your permission to use Bash""#,
                ),
            ],
        );
        session("bbbb", "works", &[start, prompt]);
        let out = panel::to_plain(&list_lines(
            &all_entries(&root, &config::Config::default(), now),
            None,
            &config::Config::default(),
            now,
            110,
        ));
        let _ = std::fs::remove_dir_all(&root);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].contains("2 live sessions · 1 needs you"), "{out}");
        assert!(lines[0].ends_with("7d 58% · $3.00 total"), "{out}");
        assert!(
            lines[2].starts_with("◉ NEEDS YOU")
                && lines[2].contains("proj · asks")
                && lines[2].contains("Claude needs your permission to use Bash"),
            "{out}"
        );
        assert!(
            lines[3].contains("proj · works") && lines[3].contains("$1.50"),
            "{out}"
        );
        assert_eq!(
            out.matches("7d").count(),
            1,
            "limits once, not per row: {out}"
        );
    }

    #[test]
    fn quiet_all_row_shows_the_away_summary() {
        let root = std::env::temp_dir().join(format!("ari-away-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let now = hook::now_ms();
        let d = root.join("sessions").join("cccc");
        std::fs::create_dir_all(&d).unwrap();
        let ev = |dt: u64, rest: &str| format!(r#"{{"t":{},{rest}}}"#, now - dt);
        let evs = [
            ev(60_000, r#""e":"SessionStart""#),
            ev(50_000, r#""e":"UserPromptSubmit","msg":"go""#),
            ev(40_000, r#""e":"Stop""#),
        ];
        std::fs::write(d.join("events.jsonl"), evs.join("\n") + "\n").unwrap();
        std::fs::write(d.join("usage.json"), r#"{"session_id":"cccc"}"#).unwrap();
        let c = tokens::TokenCache {
            v: tokens::CACHE_VERSION,
            main: tokens::Ledger {
                away: Some("Next, wire the desktop".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        std::fs::write(d.join("tokens.json"), serde_json::to_vec(&c).unwrap()).unwrap();
        let out = panel::to_plain(&list_lines(
            &all_entries(&root, &config::Config::default(), now),
            None,
            &config::Config::default(),
            now,
            110,
        ));
        let _ = std::fs::remove_dir_all(&root);
        assert!(out.contains("“Next, wire the desktop”"), "{out}");
    }

    #[test]
    fn recap_rebuilds_only_on_new_events_and_not_too_often() {
        use std::time::Duration;
        let now = Instant::now();
        assert!(recap_due(None, false), "first build");
        assert!(!recap_due(Some(now), false), "nothing new");
        assert!(
            !recap_due(Some(now), true),
            "new events, but built just now"
        );
        let old = now.checked_sub(Duration::from_secs(6)).unwrap();
        assert!(recap_due(Some(old), true));
        assert!(!recap_due(Some(old), false));
    }

    #[test]
    fn recap_key_and_escape() {
        let mut v = View::default();
        assert!(!key(&mut v, KeyCode::Char('r'), None));
        assert!(v.recap);
        assert!(
            !key(&mut v, KeyCode::Esc, None),
            "esc closes the recap first"
        );
        assert!(!v.recap);
        assert!(key(&mut v, KeyCode::Esc, None), "then quits");
        assert!(key(&mut v, KeyCode::Char('q'), None));
    }

    #[test]
    fn list_keys_clamp_open_and_escape() {
        let mut v = View::default();
        for _ in 0..3 {
            key(&mut v, KeyCode::Down, Some(2));
        }
        assert_eq!(v.sel, 1);
        key(&mut v, KeyCode::Char('k'), Some(2));
        assert_eq!(v.sel, 0);
        key(&mut v, KeyCode::Up, Some(2));
        assert_eq!(v.sel, 0);
        let mut e = View::default();
        key(&mut e, KeyCode::Enter, Some(0));
        key(&mut e, KeyCode::Down, Some(0));
        assert!(!e.open && e.sel == 0);
        key(&mut v, KeyCode::Enter, Some(2));
        assert!(v.open);
        assert!(
            !key(&mut v, KeyCode::Esc, Some(2)),
            "esc closes the session"
        );
        assert!(!v.open);
        assert!(key(&mut v, KeyCode::Esc, Some(2)), "esc in the list quits");
    }

    #[test]
    fn opened_session_stays_pinned_when_urgency_reorders() {
        let root = std::env::temp_dir().join(format!("ari-pin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let now = hook::now_ms();
        let ev = |dt: u64, rest: &str| format!(r#"{{"t":{},{rest}}}"#, now - dt);
        let write = |id: &str, extra: Option<String>| {
            let d = root.join("sessions").join(id);
            std::fs::create_dir_all(&d).unwrap();
            let mut evs = vec![
                ev(60_000, r#""e":"SessionStart""#),
                ev(50_000, r#""e":"UserPromptSubmit","msg":"go""#),
            ];
            evs.extend(extra);
            std::fs::write(d.join("events.jsonl"), evs.join("\n") + "\n").unwrap();
            let u = serde_json::json!({"session_id": id, "session_name": format!("name-{id}")});
            std::fs::write(d.join("usage.json"), u.to_string()).unwrap();
        };
        write("aaaa", None);
        write("bbbb", None);
        let mut app = App {
            root: root.clone(),
            mode: Mode::All,
            cfg: config::Config::default(),
            view: View::default(),
            dir: None,
            st: State::default(),
            usage: Usage::default(),
            x: bar::Extras::default(),
            mtimes: (None, None),
            picked_at: None,
            recap_built: None,
            recap_dirty: false,
            last_verdict: None,
            color: false,
            interactive: true,
            pinned: None,
        };
        let (_, n) = app.frame_lines(100, 40);
        assert_eq!(n, Some(2));
        app.view.open = true;
        let title = |app: &mut App| panel::to_plain(&app.frame_lines(100, 40).0);
        let first = title(&mut app);
        let open_id = app.pinned.clone().unwrap();
        let other = if open_id.ends_with("aaaa") {
            "bbbb"
        } else {
            "aaaa"
        };
        // The other session now needs you, so it sorts above the opened one.
        write(
            other,
            Some(ev(
                1_000,
                r#""e":"Notification","kind":"permission_prompt","msg":"ok?""#,
            )),
        );
        let second = title(&mut app);
        assert_eq!(app.pinned, Some(open_id.clone()));
        assert!(first.contains(&format!(
            "name-{}",
            open_id.file_name().unwrap().to_string_lossy()
        )));
        assert!(second.contains(&format!(
            "name-{}",
            open_id.file_name().unwrap().to_string_lossy()
        )));
        // Gone: back to the list.
        std::fs::remove_dir_all(&open_id).unwrap();
        let (_, n) = app.frame_lines(100, 40);
        assert_eq!((n, app.pinned.is_none()), (Some(1), true));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_marks_the_selected_row_and_ends_with_a_hint() {
        let e = |p: &str| Entry {
            rank: 0,
            verdict: "ON TRACK".into(),
            style: Style::default(),
            project: p.into(),
            title: None,
            detail: String::new(),
            ctx: None,
            cost: None,
            age_ms: 0,
            dir: PathBuf::new(),
            limits: None,
        };
        let es = [e("one"), e("two")];
        let cfg = config::Config::default();
        let out = list_lines(&es, Some(1), &cfg, 0, 100);
        let rev = |l: &Line| l.style.add_modifier.contains(Modifier::REVERSED);
        assert!(!rev(&out[2]) && rev(&out[3]));
        let last = out.last().unwrap();
        assert_eq!(
            panel::to_plain(std::slice::from_ref(last)),
            "↑↓ select · enter open · esc back · q quit"
        );
        assert!(last.spans[0].style.add_modifier.contains(Modifier::DIM));
        let mut old = e("old");
        old.rank = 0;
        old.age_ms = 9_000_000;
        old.limits = Some((
            Some(usage::Limit {
                pct: 11.0,
                resets_at: 0,
            }),
            None,
        ));
        let mut fresh = e("fresh");
        fresh.rank = 3;
        fresh.limits = Some((
            Some(usage::Limit {
                pct: 77.0,
                resets_at: 0,
            }),
            None,
        ));
        let head = panel::to_plain(&list_lines(&[old, fresh], None, &cfg, 0, 100)[..1]);
        assert!(head.contains("5h 77%") && !head.contains("11%"), "{head}");
        let once = list_lines(&es, None, &cfg, 0, 100);
        assert_eq!(once.len(), 4, "no hint without a selection");
        let ascii = config::Config {
            ascii: true,
            ..Default::default()
        };
        let out = list_lines(&es, Some(1), &ascii, 0, 100);
        assert!(!rev(&out[2]) && rev(&out[3]), "--ascii keeps the selection");
    }

    #[test]
    fn narrow_list_puts_the_detail_under_the_session() {
        let e = |w: &str| Entry {
            rank: 0,
            verdict: "ON TRACK".into(),
            style: Style::default(),
            project: "tankobon".into(),
            title: Some("Phone unable to open the reader".into()),
            detail: w.into(),
            ctx: Some(5.0),
            cost: Some(1.0),
            age_ms: 0,
            dir: PathBuf::new(),
            limits: None,
        };
        let cfg = config::Config::default();
        let out = panel::to_plain(&list_lines(
            &[e("$ cargo test"), e("")],
            Some(0),
            &cfg,
            0,
            80,
        ));
        let rows: Vec<&str> = out.lines().skip(2).collect();
        assert!(
            rows[0].contains("tankobon · Phone unable to open the reader"),
            "{out}"
        );
        assert_eq!(rows[1].trim(), "$ cargo test", "{out}");
        assert!(
            rows[2].contains("tankobon"),
            "no empty line for no detail: {out}"
        );
        let wide = panel::to_plain(&list_lines(&[e("$ cargo test")], None, &cfg, 0, 140));
        assert_eq!(
            wide.lines().count(),
            3,
            "one line per session when wide: {wide}"
        );
    }

    #[test]
    fn long_project_names_truncate_on_a_char_boundary() {
        for (n, w) in [(35, 120), (36, 120), (37, 120), (12, 40), (13, 40)] {
            let e = Entry {
                rank: 0,
                verdict: "ON TRACK".into(),
                style: Style::default(),
                project: "p".repeat(n),
                title: Some("a session name".into()),
                detail: String::new(),
                ctx: None,
                cost: None,
                age_ms: 0,
                dir: PathBuf::new(),
                limits: None,
            };
            let out = list_lines(&[e], None, &config::Config::default(), 0, w);
            assert!(panel::to_plain(&out).contains('…'), "{n} at {w}");
        }
    }
}

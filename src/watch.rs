//! `ari watch`: the full panel in a terminal pane.

use crate::hub::{self, Fit, HubView, Screen};
use crate::panel::{self, View};
use crate::sessions::{self, SessionInfo};
use crate::state::{State, Verdict};
use crate::usage::{self, Usage};
use crate::{bar, burn, config, hook, paths, settings, state, theme, tokens};
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
    /// What's drawn with: `file` plus command-line flags.
    cfg: config::Config,
    /// config.toml as last read or saved; the settings screen edits this.
    file: config::Config,
    ascii_flag: bool,
    cfg_path: Option<PathBuf>,
    cfg_mtime: Option<SystemTime>,
    /// Why config.toml couldn't be read (the last good config stays in use).
    cfg_error: Option<String>,
    settings_note: Option<String>,
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
    fn derive_cfg(&mut self) {
        self.cfg = self.file.clone();
        self.cfg.ascii |= self.ascii_flag;
    }

    /// Pick up config.toml when it changed on disk (a hand edit, another window).
    fn reload_config(&mut self) {
        let Some(p) = &self.cfg_path else { return };
        let m = mtime(p);
        if m == self.cfg_mtime {
            return;
        }
        self.cfg_mtime = m;
        match config::load_from(p) {
            Ok(c) => {
                self.file = c;
                self.cfg_error = None;
            }
            Err(e) => self.cfg_error = Some(e),
        }
        self.derive_cfg();
    }

    /// Apply the settings change `key` queued, then save it.
    fn apply_edit(&mut self, view: &mut View) {
        let (Some(dir), Some(sel)) = (view.edit.take(), view.settings) else {
            return;
        };
        // A hand edit since the last frame must win over (or block) this save.
        self.reload_config();
        if self.cfg_error.is_some() {
            self.settings_note = Some("fix config.toml first (or delete it)".into());
            return;
        }
        let Some(item) = settings::items().into_iter().nth(sel) else {
            return;
        };
        if dir == 0 {
            settings::reset(&mut self.file, item.key);
        } else {
            settings::step(&mut self.file, item.key, dir);
        }
        self.settings_note = None;
        if let Some(p) = &self.cfg_path {
            match config::save_to(p, &self.file) {
                // Our own save isn't a hand edit to reload.
                Ok(()) => self.cfg_mtime = mtime(p),
                Err(e) => self.settings_note = Some(format!("couldn't save: {e}")),
            }
        }
        self.derive_cfg();
    }

    fn refresh(&mut self) {
        self.reload_config();
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
        self.mode == Mode::All
            || self.st.turn_active
            || self.st.running_agents(now) > 0
            // A message in flight on the hub's graph.
            || self.st.messages.back().is_some_and(|m| now.saturating_sub(m.t) < 2000)
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
        let lines = self.frame_lines(width, height).0;
        self.themed(lines)
    }

    /// The settings screen colours itself; everything else gets the theme here.
    fn themed(&self, lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
        if self.view.settings.is_some() {
            lines
        } else {
            theme::lines(lines, &theme::Palette::new(&self.cfg))
        }
    }

    fn settings_lines(&self, sel: usize, w: u16, h: u16) -> Vec<Line<'static>> {
        let now = hook::now_ms();
        let preview = self
            .dir
            .as_ref()
            .filter(|_| w >= settings::PREVIEW_MIN_W && self.mode != Mode::All)
            .map(|_| {
                let pw = w - settings::LIST_W - 1;
                // shortcut: the statusline row is shown uncoloured; parse its ANSI if colour matters.
                let row = bar::render(
                    &self.st,
                    &self.usage,
                    &self.x,
                    &self.cfg,
                    now,
                    pw.into(),
                    false,
                );
                let dim = Style::new().add_modifier(Modifier::DIM);
                let mut lines = vec![
                    Line::styled(
                        "statusline",
                        if self.color { dim } else { Style::default() },
                    ),
                    Line::from(row),
                    Line::default(),
                ];
                lines.extend(panel::lines(
                    &self.st,
                    &self.usage,
                    &self.x,
                    &self.cfg,
                    View::default(),
                    now,
                    &panel::local_hhmm(now),
                    pw,
                    h.saturating_sub(5),
                    self.color,
                ));
                theme::lines(lines, &theme::Palette::new(&self.cfg))
            });
        let path = self
            .cfg_path
            .as_ref()
            .map(|p| panel::tilde(&p.to_string_lossy()))
            .unwrap_or_default();
        let lines = settings::lines(
            // Rows show the file, not command-line flags (`--ascii`).
            &settings::Screen {
                cfg: &self.file,
                sel,
                path: &path,
                error: self.cfg_error.as_deref(),
                note: self.settings_note.as_deref(),
                preview,
                color: self.color,
            },
            w,
            h,
        );
        if self.cfg.ascii {
            crate::ascii::map_lines(lines)
        } else {
            lines
        }
    }

    /// The frame's lines, and the entry count while the `--all` list is what's drawn.
    fn frame_lines(&mut self, width: u16, height: u16) -> (Vec<Line<'static>>, Option<usize>) {
        if let Some(sel) = self.view.settings {
            return (self.settings_lines(sel, width, height), None);
        }
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
                    return self.view_lines(&dir, &st, &usage, x, width, height, now);
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
        let dir = dir.clone();
        self.view_lines(
            &dir,
            &self.st,
            &self.usage,
            self.x.clone(),
            width,
            height,
            now,
        )
    }

    /// The hub when it's open and fits, else the panel (with a note when the hub doesn't fit).
    /// The count is the hub's agents while the hub draws.
    #[allow(clippy::too_many_arguments)]
    fn view_lines(
        &self,
        dir: &Path,
        st: &State,
        usage: &Usage,
        mut x: bar::Extras,
        width: u16,
        height: u16,
        now: u64,
    ) -> (Vec<Line<'static>>, Option<usize>) {
        let Some(hv) = self.view.hub else {
            return (
                self.panel(dir, st, usage, x, width, height, now, None),
                None,
            );
        };
        if hub::fit(width, height) == Fit::TooSmall {
            let note = format!("hub needs 120×30 · this window is {width}×{height} · x panel");
            return (
                self.panel(dir, st, usage, x, width, height, now, Some(note)),
                None,
            );
        }
        x.conflicts = crate::conflicts::cached(&self.root, dir, now);
        x.transcript_age_ms =
            tokens::transcript_for(dir, usage).and_then(|t| tokens::transcript_age(&t, now));
        let clock = panel::local_hhmm(now);
        let i = hub::Input {
            st,
            u: usage,
            x: &x,
            cfg: &self.cfg,
            now,
            clock: &clock,
        };
        let lines = hub::lines(&i, &hv, width, height, self.color);
        let lines = if self.cfg.ascii {
            crate::ascii::map_lines(lines)
        } else {
            lines
        };
        (lines, Some(st.agents.len().max(1)))
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
        note: Option<String>,
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
            let text = note.unwrap_or_else(|| {
                format!("e errors · a agents · r recap · x hub · s settings · {back} · q quit")
            });
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
    let mut start_hub = false;
    let mut ascii = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--all" => mode = Some(Mode::All),
            "--once" => once = true,
            "--ascii" => ascii = true,
            "--hub" => start_hub = true,
            "--session" => {
                let id = it.next().unwrap_or_else(|| fail("--session needs an id"));
                if !paths::valid_session_id(id) {
                    fail("invalid session id");
                }
                mode = Some(Mode::Fixed(id.clone()));
            }
            other => fail(&format!(
                "unknown option {other} (try --session <id>, --all, --once, --ascii, --hub)"
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
        // Filled from config.toml by the first `refresh`.
        cfg: config::Config {
            ascii,
            ..Default::default()
        },
        file: config::Config::default(),
        ascii_flag: ascii,
        cfg_path: config::path(),
        cfg_mtime: None,
        cfg_error: None,
        settings_note: None,
        view: View {
            hub: start_hub.then(HubView::default),
            ..Default::default()
        },
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
        let env = |k: &str, d: u16| {
            std::env::var(k)
                .ok()
                .and_then(|c| c.parse().ok())
                .unwrap_or(d)
        };
        let (width, height) = if start_hub {
            (env("COLUMNS", 168), env("LINES", 49))
        } else {
            (env("COLUMNS", 80), u16::MAX)
        };
        println!("{}", panel::to_plain(&app.lines(width, height)));
        std::process::exit(0)
    }

    app.interactive = true;
    let first = app.view;
    let result = run_tui(first, |view, tab, w, h| {
        if tab {
            app.cycle();
        }
        app.apply_edit(view);
        if view.settings.is_none() {
            app.settings_note = None;
        }
        app.view = *view;
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
        let (lines, agents) = app.frame_lines(w, h);
        let lines = app.themed(lines);
        // The `--all` list's length rides in the same slot; the hub's count is separate.
        let (list_len, agents) = if app.mode == Mode::All && !app.view.open {
            (agents, None)
        } else {
            (None, agents)
        };
        (lines, app.animating(now), align, list_len, agents)
    });
    if let Err(e) = result {
        fail(&e.to_string());
    }
    std::process::exit(0)
}

/// Apply a key to the view; true means quit. `list_len` is `Some` while the `--all` list shows,
/// `agents` while the hub draws.
fn key(view: &mut View, code: KeyCode, list_len: Option<usize>, agents: Option<usize>) -> bool {
    // Settings sit over everything and keep every key while open.
    if let Some(sel) = view.settings {
        let last = settings::items().len() - 1;
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc | KeyCode::Char('s') => view.settings = None,
            KeyCode::Up | KeyCode::Char('k') => view.settings = Some(sel.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => view.settings = Some((sel + 1).min(last)),
            KeyCode::Right | KeyCode::Char('l' | ' ') | KeyCode::Enter => view.edit = Some(1),
            KeyCode::Left | KeyCode::Char('h') => view.edit = Some(-1),
            KeyCode::Char('d') => view.edit = Some(0),
            _ => {}
        }
        return false;
    }
    if code == KeyCode::Char('s') {
        view.settings = Some(0);
        return false;
    }
    // The `--all` list keeps its keys; the hub's apply only while it draws (`agents` is Some).
    if let Some(h) = view.hub.as_mut()
        && list_len.is_none()
    {
        let Some(n) = agents.map(|n| n.max(1)) else {
            match code {
                KeyCode::Char('q') => return true,
                KeyCode::Char('x') => view.hub = None,
                _ => {}
            }
            return false;
        };
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('x') => view.hub = None,
            KeyCode::Esc if h.screen == Screen::Hub => view.hub = None,
            KeyCode::Esc | KeyCode::Char('h') => {
                h.screen = Screen::Hub;
                h.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                h.sel = h.sel.saturating_sub(1);
                h.scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                h.sel = (h.sel + 1).min(n - 1);
                h.scroll = 0;
            }
            KeyCode::Enter => {
                h.screen = Screen::Zoom;
                h.scroll = 0;
            }
            KeyCode::Char('g') => h.screen = Screen::Graph,
            KeyCode::Char('z') => h.show_done ^= true,
            KeyCode::PageUp => h.scroll += 10,
            KeyCode::PageDown => h.scroll = h.scroll.saturating_sub(10),
            _ => {}
        }
        return false;
    }
    if code == KeyCode::Char('x') && list_len.is_none() {
        view.hub = Some(HubView::default());
        return false;
    }
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

/// A frame: lines to draw, whether something animates (10 fps vs 1 fps), alignment,
/// `Some(entries)` while the selectable `--all` list is drawn, and `Some(agents)` while the hub is.
pub type Frame = (
    Vec<Line<'static>>,
    bool,
    Alignment,
    Option<usize>,
    Option<usize>,
);

/// Own the terminal (alternate screen, raw mode, restored on exit or panic) and redraw
/// until `q`. `frame(view, tab_pressed, width, height)` produces each frame.
pub fn run_tui(
    view: View,
    mut frame: impl FnMut(&mut View, bool, u16, u16) -> Frame,
) -> std::io::Result<()> {
    let mut term = ratatui::init();
    let result = tui_loop(&mut term, view, &mut frame);
    ratatui::restore();
    result
}

fn tui_loop(
    term: &mut ratatui::DefaultTerminal,
    mut view: View,
    frame: &mut impl FnMut(&mut View, bool, u16, u16) -> Frame,
) -> std::io::Result<()> {
    let mut tab = false;
    let mut list_len;
    let mut hub_agents;
    loop {
        let size = term.size()?;
        let (lines, animating, align, n, agents) =
            frame(&mut view, std::mem::take(&mut tab), size.width, size.height);
        list_len = n;
        hub_agents = agents;
        if let (Some(h), Some(a)) = (view.hub.as_mut(), agents) {
            h.sel = h.sel.min(a.saturating_sub(1));
        }
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
            } else if key(&mut view, k.code, list_len, hub_agents) {
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
    use crate::hub::{HubView, Screen};

    fn hub_view(screen: Screen, sel: usize) -> View {
        View {
            hub: Some(HubView {
                screen,
                sel,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn x_toggles_hub() {
        let mut v = View::default();
        assert!(!key(&mut v, KeyCode::Char('x'), None, None));
        assert_eq!(v.hub, Some(HubView::default()));
        key(&mut v, KeyCode::Char('x'), None, Some(3));
        assert_eq!(v.hub, None);
        key(&mut v, KeyCode::Char('x'), Some(4), None);
        assert_eq!(v.hub, None, "the --all list keeps x to itself");
    }

    #[test]
    fn s_opens_settings_everywhere() {
        for (mut v, list, agents) in [
            (View::default(), None, None),
            (hub_view(Screen::Hub, 2), None, Some(3)),
            (hub_view(Screen::Zoom, 0), None, None),
            (View::default(), Some(4), None),
        ] {
            let hub = v.hub;
            assert!(!key(&mut v, KeyCode::Char('s'), list, agents));
            assert_eq!(v.settings, Some(0));
            assert_eq!(v.hub, hub);
        }
    }

    #[test]
    fn settings_keys() {
        let last = crate::settings::items().len() - 1;
        let mut v = hub_view(Screen::Graph, 1);
        key(&mut v, KeyCode::Char('s'), None, Some(3));
        key(&mut v, KeyCode::Up, None, None);
        assert_eq!(v.settings, Some(0));
        key(&mut v, KeyCode::Down, None, None);
        key(&mut v, KeyCode::Char('j'), None, None);
        assert_eq!(v.settings, Some(2));
        for _ in 0..100 {
            key(&mut v, KeyCode::Down, None, None);
        }
        assert_eq!(v.settings, Some(last));
        for (k, want) in [
            (KeyCode::Right, 1),
            (KeyCode::Char('l'), 1),
            (KeyCode::Enter, 1),
            (KeyCode::Char(' '), 1),
            (KeyCode::Left, -1),
            (KeyCode::Char('h'), -1),
            (KeyCode::Char('d'), 0),
        ] {
            v.edit = None;
            key(&mut v, k, None, None);
            assert_eq!(v.edit, Some(want), "{k:?}");
        }
        assert_eq!(
            v.hub,
            hub_view(Screen::Graph, 1).hub,
            "keys stay in settings"
        );
        key(&mut v, KeyCode::Esc, None, None);
        assert_eq!(v.settings, None);
        assert_eq!(v.hub, hub_view(Screen::Graph, 1).hub);
        key(&mut v, KeyCode::Char('s'), None, Some(3));
        key(&mut v, KeyCode::Char('s'), None, None);
        assert_eq!(v.settings, None);
        key(&mut v, KeyCode::Char('s'), None, Some(3));
        assert!(key(&mut v, KeyCode::Char('q'), None, None));
    }

    #[test]
    fn esc_walks_back() {
        let mut v = hub_view(Screen::Zoom, 1);
        assert!(!key(&mut v, KeyCode::Esc, None, Some(3)));
        assert_eq!(v.hub.map(|h| h.screen), Some(Screen::Hub));
        assert!(!key(&mut v, KeyCode::Esc, None, Some(3)));
        assert_eq!(v.hub, None);
        assert!(
            key(&mut v, KeyCode::Esc, None, None),
            "esc on the panel quits"
        );
    }

    #[test]
    fn list_keys_work_with_hub_flag() {
        let mut v = hub_view(Screen::Hub, 0);
        key(&mut v, KeyCode::Down, Some(3), None);
        assert_eq!(v.sel, 1, "the --all list still navigates");
        key(&mut v, KeyCode::Enter, Some(3), None);
        assert!(v.open);
    }

    #[test]
    fn hidden_hub_ignores_keys() {
        // `agents` is None while the too-small notice shows the panel.
        let mut v = hub_view(Screen::Zoom, 2);
        for k in [
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::Enter,
            KeyCode::Char('g'),
            KeyCode::PageUp,
        ] {
            key(&mut v, k, None, None);
        }
        assert_eq!(v, hub_view(Screen::Zoom, 2));
        key(&mut v, KeyCode::Char('x'), None, None);
        assert_eq!(v.hub, None);
    }

    #[test]
    fn hub_selection_clamps() {
        let mut v = hub_view(Screen::Hub, 2);
        key(&mut v, KeyCode::Down, None, Some(3));
        assert_eq!(v.hub.unwrap().sel, 2);
        key(&mut v, KeyCode::Up, None, Some(3));
        assert_eq!(v.hub.unwrap().sel, 1);
        v.hub.as_mut().unwrap().scroll = 5;
        key(&mut v, KeyCode::Enter, None, Some(3));
        assert_eq!(
            (v.hub.unwrap().screen, v.hub.unwrap().scroll),
            (Screen::Zoom, 0)
        );
        key(&mut v, KeyCode::PageUp, None, Some(3));
        assert_eq!(v.hub.unwrap().scroll, 10);
        key(&mut v, KeyCode::Char('g'), None, Some(3));
        assert_eq!(v.hub.unwrap().screen, Screen::Graph);
        key(&mut v, KeyCode::Char('z'), None, Some(3));
        assert!(v.hub.unwrap().show_done);
        assert!(key(&mut v, KeyCode::Char('q'), None, Some(3)));
    }

    fn app_on(root: &Path, id: &str, events: &[String]) -> App {
        let d = root.join("sessions").join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("events.jsonl"), events.join("\n") + "\n").unwrap();
        std::fs::write(d.join("usage.json"), format!(r#"{{"session_id":"{id}"}}"#)).unwrap();
        let mut app = App {
            root: root.to_path_buf(),
            mode: Mode::Fixed(id.into()),
            cfg: config::Config::default(),
            file: config::Config::default(),
            ascii_flag: false,
            cfg_path: None,
            cfg_mtime: None,
            cfg_error: None,
            settings_note: None,
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
        app.refresh();
        app
    }

    /// A fresh session plus a config file path of its own (tests run in parallel).
    fn app_with_cfg(name: &str, text: Option<&str>) -> (App, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("ari-cfg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = root.join("config.toml");
        std::fs::create_dir_all(&root).unwrap();
        if let Some(t) = text {
            std::fs::write(&p, t).unwrap();
        }
        let mut app = app_on(&root, "s1", &[]);
        app.cfg_path = Some(p.clone());
        app.refresh();
        (app, root, p)
    }

    fn edit(app: &mut App, key: &str, dir: i8) {
        let i = crate::settings::items()
            .iter()
            .position(|it| it.key == key)
            .unwrap();
        let mut v = View {
            settings: Some(i),
            edit: Some(dir),
            ..View::default()
        };
        app.apply_edit(&mut v);
        assert_eq!(v.edit, None, "the edit is taken");
    }

    #[test]
    fn edit_saves_and_applies() {
        let (mut app, root, p) = app_with_cfg("edit", None);
        assert!(!app.cfg.face);
        edit(&mut app, "face", 1);
        assert!(app.cfg.face);
        assert_eq!(config::load_from(&p), Ok(app.file.clone()));
        assert!(app.file.face);
        edit(&mut app, "face", 0);
        assert!(!app.cfg.face);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ascii_flag_is_not_saved() {
        let (mut app, root, p) = app_with_cfg("ascii", None);
        app.ascii_flag = true;
        edit(&mut app, "bell", 1);
        assert!(app.cfg.ascii && app.cfg.bell);
        let saved = config::load_from(&p).unwrap();
        assert!(!saved.ascii && saved.bell);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn broken_file_is_never_overwritten() {
        let (mut app, root, p) = app_with_cfg("broken", Some("face = [[["));
        assert!(app.cfg_error.is_some());
        edit(&mut app, "bell", 1);
        assert_eq!(
            app.settings_note.as_deref(),
            Some("fix config.toml first (or delete it)")
        );
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "face = [[[");
        assert!(!app.cfg.bell);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn edit_right_after_a_hand_edit_keeps_it() {
        let (mut app, root, p) = app_with_cfg("race", None);
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&p, "[chain]\ncommands = [\"mine\"]\n").unwrap();
        edit(&mut app, "bell", 1); // no refresh in between
        let saved = config::load_from(&p).unwrap();
        assert_eq!(saved.chain.commands, ["mine"]);
        assert!(saved.bell);
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&p, "face = [[[").unwrap();
        edit(&mut app, "bell", 1);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "face = [[[");
        assert_eq!(
            app.settings_note.as_deref(),
            Some("fix config.toml first (or delete it)")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ascii_row_shows_the_file_under_the_flag() {
        let (mut app, root, p) = app_with_cfg("asciirow", None);
        app.ascii_flag = true;
        app.derive_cfg();
        let i = crate::settings::items()
            .iter()
            .position(|it| it.key == "ascii")
            .unwrap();
        app.view.settings = Some(i);
        let row = |app: &mut App| {
            panel::to_plain(&app.lines(100, 40))
                .lines()
                .find(|l| l.contains("ascii glyphs"))
                .unwrap()
                .to_string()
        };
        assert!(row(&mut app).contains("off"), "{}", row(&mut app));
        edit(&mut app, "ascii", 1);
        assert!(row(&mut app).contains("on"), "{}", row(&mut app));
        assert!(config::load_from(&p).unwrap().ascii);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn hand_edit_reloads() {
        let (mut app, root, p) = app_with_cfg("reload", Some("theme = \"soft\"\n"));
        assert_eq!(app.cfg.theme, "soft");
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&p, "theme = \"mono\"\n").unwrap();
        app.refresh();
        assert_eq!(app.cfg.theme, "mono");
        std::fs::write(&p, "theme = [").unwrap();
        app.refresh();
        assert_eq!(
            app.cfg.theme, "mono",
            "a broken edit keeps the last good config"
        );
        assert!(app.cfg_error.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn settings_preview_shows_the_statusline_row() {
        let (mut app, root, _) = app_with_cfg("barprev", None);
        let i = crate::settings::items()
            .iter()
            .position(|it| it.key == "hide.ctx")
            .unwrap();
        app.view.settings = Some(i);
        let text = panel::to_plain(&app.lines(130, 40));
        assert!(text.contains("│statusline"), "{text}");
        assert!(text.contains("ctx —"), "{text}");
        edit(&mut app, "hide.ctx", 1);
        let text = panel::to_plain(&app.lines(130, 40));
        assert!(!text.contains("ctx —"), "{text}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn settings_frame_draws_the_screen() {
        let (mut app, root, _) = app_with_cfg("frame", None);
        app.view.settings = Some(0);
        let text = panel::to_plain(&app.lines(130, 40));
        assert!(text.contains("ARI settings"), "{text}");
        assert!(text.contains("▸ theme"), "{text}");
        assert!(text.contains("│╭─ ARI"), "preview beside the list:\n{text}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn too_small_keeps_hub_screen() {
        let root = std::env::temp_dir().join(format!("ari-hub-small-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let now = hook::now_ms();
        let mut app = app_on(
            &root,
            "dddd",
            &[format!(
                r#"{{"t":{now},"e":"UserPromptSubmit","msg":"go"}}"#
            )],
        );
        app.view = hub_view(Screen::Zoom, 2);
        let (small, n) = app.frame_lines(100, 40);
        let (big, m) = app.frame_lines(168, 49);
        let _ = std::fs::remove_dir_all(&root);
        let small = panel::to_plain(&small);
        assert!(
            small.contains("hub needs 120×30 · this window is 100×40 · x panel"),
            "{small}"
        );
        assert_eq!(n, None);
        assert_eq!(app.view, hub_view(Screen::Zoom, 2));
        assert!(panel::to_plain(&big).contains("ARI hub"));
        assert_eq!(m, Some(1));
    }

    #[test]
    fn hub_strip_matches_statusline_without_hooks() {
        let root = std::env::temp_dir().join(format!("ari-hub-nohooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = root.join("sessions").join("eeee");
        std::fs::create_dir_all(&d).unwrap();
        let t = d.join("t.jsonl");
        std::fs::write(&t, "{}\n").unwrap();
        let mut app = app_on(&root, "eeee", &[]);
        // After app_on, which writes a bare usage.json.
        let u = format!(
            r#"{{"session_id":"eeee","transcript_path":"{}"}}"#,
            t.display()
        );
        std::fs::write(d.join("usage.json"), u).unwrap();
        app.mtimes = (None, None);
        app.refresh();
        app.view = hub_view(Screen::Hub, 0);
        let out = panel::to_plain(&app.frame_lines(168, 49).0);
        let _ = std::fs::remove_dir_all(&root);
        assert!(out.lines().nth(1).unwrap().contains("WORKING"), "{out}");
    }

    #[test]
    fn ascii_hub_is_ascii() {
        use crate::state::{Agent, Msg};
        let mut st = State::default();
        st.agents.push(Agent {
            id: "main".into(),
            kind: "main".into(),
            ..Default::default()
        });
        for (n, id) in ["a1", "a2", "a3", "a4", "a5", "a6"].iter().enumerate() {
            st.agents.push(Agent {
                id: (*id).into(),
                kind: "Explore".into(),
                parent: Some("main".into()),
                started: n as u64,
                ..Default::default()
            });
        }
        st.agents[1].waiting = Some((1, "$ curl x".into()));
        st.agents[3].done = true;
        st.turn_active = true;
        st.messages.push_back(Msg {
            t: 99_500,
            from: "a1".into(),
            to: "a2".into(),
            text: "hi".into(),
        });
        st.messages.push_back(Msg {
            t: 99_000,
            from: "main".into(),
            to: "a3".into(),
            text: "ok".into(),
        });
        let (u, x) = (Usage::default(), bar::Extras::default());
        let cfg = config::Config {
            ascii: true,
            ..Default::default()
        };
        for screen in [Screen::Hub, Screen::Graph, Screen::Zoom] {
            let i = crate::hub::Input {
                st: &st,
                u: &u,
                x: &x,
                cfg: &cfg,
                now: 100_000,
                clock: "12:00",
            };
            let v = HubView {
                screen,
                sel: 1,
                ..Default::default()
            };
            let out = panel::to_plain(&crate::ascii::map_lines(crate::hub::lines(
                &i, &v, 168, 49, false,
            )));
            let bad: String = out.chars().filter(|c| !c.is_ascii()).collect();
            assert!(bad.is_empty(), "{screen:?}: {bad:?}");
        }
    }

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
        assert!(!key(&mut v, KeyCode::Char('r'), None, None));
        assert!(v.recap);
        assert!(
            !key(&mut v, KeyCode::Esc, None, None),
            "esc closes the recap first"
        );
        assert!(!v.recap);
        assert!(key(&mut v, KeyCode::Esc, None, None), "then quits");
        assert!(key(&mut v, KeyCode::Char('q'), None, None));
    }

    #[test]
    fn list_keys_clamp_open_and_escape() {
        let mut v = View::default();
        for _ in 0..3 {
            key(&mut v, KeyCode::Down, Some(2), None);
        }
        assert_eq!(v.sel, 1);
        key(&mut v, KeyCode::Char('k'), Some(2), None);
        assert_eq!(v.sel, 0);
        key(&mut v, KeyCode::Up, Some(2), None);
        assert_eq!(v.sel, 0);
        let mut e = View::default();
        key(&mut e, KeyCode::Enter, Some(0), None);
        key(&mut e, KeyCode::Down, Some(0), None);
        assert!(!e.open && e.sel == 0);
        key(&mut v, KeyCode::Enter, Some(2), None);
        assert!(v.open);
        assert!(
            !key(&mut v, KeyCode::Esc, Some(2), None),
            "esc closes the session"
        );
        assert!(!v.open);
        assert!(
            key(&mut v, KeyCode::Esc, Some(2), None),
            "esc in the list quits"
        );
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
            file: config::Config::default(),
            ascii_flag: false,
            cfg_path: None,
            cfg_mtime: None,
            cfg_error: None,
            settings_note: None,
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

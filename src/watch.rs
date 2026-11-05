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
    last_verdict: Option<Verdict>,
    color: bool,
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
        if now != self.mtimes {
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

    fn lines(&self, width: u16, height: u16) -> Vec<Line<'static>> {
        let now = hook::now_ms();
        if self.mode == Mode::All {
            return all_lines(&self.root, &self.cfg, now, width);
        }
        if self.dir.is_none() {
            return waiting(&self.mode);
        }
        let mut x = self.x.clone();
        x.transcript_age_ms = self
            .dir
            .as_deref()
            .and_then(|d| tokens::transcript_for(d, &self.usage))
            .and_then(|t| tokens::transcript_age(&t, now));
        panel::lines(
            &self.st,
            &self.usage,
            &x,
            &self.cfg,
            self.view,
            now,
            &panel::local_hhmm(now),
            width,
            height,
            self.color,
        )
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

fn all_lines(root: &Path, cfg: &config::Config, now: u64, width: u16) -> Vec<Line<'static>> {
    let live: Vec<SessionInfo> = sessions::list(root)
        .into_iter()
        .filter(|s| sessions::is_live(s, now))
        .collect();
    let mut out = vec![Line::from(Span::styled(
        format!(
            "ARI · {} live session{}",
            live.len(),
            if live.len() == 1 { "" } else { "s" }
        ),
        Style::new().add_modifier(Modifier::BOLD),
    ))];
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    // The label column already names the project.
    let mut cfg = cfg.clone();
    cfg.hide.push("project".into());
    let cfg = &cfg;
    // Fixed-width label column: project, then Claude Code's title for the session.
    let col = (width as usize / 3).clamp(12, 36);
    for s in live {
        let u = load_usage(&s.dir);
        let project = u
            .project
            .clone()
            .or_else(|| {
                s.cwd
                    .as_deref()
                    .and_then(|c| c.rsplit('/').next())
                    .map(String::from)
            })
            .unwrap_or_else(|| s.id.chars().take(8).collect());
        let label = match &u.session_name {
            Some(n) => format!("{project} · {n}"),
            None => project.clone(),
        };
        let label = crate::event::truncate(&label, col);
        let pad = col.saturating_sub(unicode_width::UnicodeWidthStr::width(label.as_str()));
        let st = state::load_session(&s.dir);
        let x = bar::Extras {
            transcript_age_ms: tokens::transcript_for(&s.dir, &u)
                .and_then(|t| tokens::transcript_age(&t, now)),
            ..Default::default()
        };
        let row = bar::render(
            &st,
            &u,
            &x,
            cfg,
            now,
            (width as usize).saturating_sub(col + 2),
            false,
        );
        // Color the verdict like the panel does, so busy sessions stand out from idle ones.
        let style = if st.first_t > 0 {
            panel::verdict_style(st.verdict_with(now, cfg.thresholds.stuck_repeats))
        } else if x.looks_active() {
            Style::new()
                .fg(ratatui::style::Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        };
        let (verdict, rest) = row
            .split_once(" │ ")
            .map_or((row.as_str(), ""), |(a, b)| (a, b));
        let rest = if rest.is_empty() {
            String::new()
        } else {
            format!(" │ {rest}")
        };
        let (head, tail) = label.split_at(project.len().min(label.len()));
        out.push(Line::from(vec![
            Span::styled(head.to_string(), Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(tail.to_string(), Style::new().add_modifier(Modifier::DIM)),
            Span::raw(" ".repeat(pad + 2)),
            Span::styled(
                verdict.to_string(),
                if color { style } else { Style::default() },
            ),
            Span::raw(rest),
        ]));
    }
    out
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
        last_verdict: None,
        color: std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
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
        (app.lines(w, h), app.animating(now), align)
    });
    if let Err(e) = result {
        fail(&e.to_string());
    }
    std::process::exit(0)
}

/// A frame: lines to draw, whether something animates (10 fps vs 1 fps), alignment.
pub type Frame = (Vec<Line<'static>>, bool, Alignment);

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
    loop {
        let size = term.size()?;
        let (lines, animating, align) =
            frame(view, std::mem::take(&mut tab), size.width, size.height);
        term.draw(|f| f.render_widget(Paragraph::new(lines).alignment(align), f.area()))?;
        if !event::poll(if animating { FAST } else { SLOW })? {
            continue;
        }
        if let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                KeyCode::Char('e') => view.expand_errors ^= true,
                KeyCode::Char('a') => view.expand_agents ^= true,
                KeyCode::Tab => tab = true,
                _ => {}
            }
        }
    }
}

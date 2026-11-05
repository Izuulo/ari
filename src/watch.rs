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
        panel::lines(
            &self.st,
            &self.usage,
            &self.x,
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
    for s in live {
        let name = s
            .cwd
            .as_deref()
            .and_then(|c| c.rsplit('/').next())
            .unwrap_or(&s.id)
            .to_string();
        let row = bar::render(
            &state::load_session(&s.dir),
            &load_usage(&s.dir),
            &bar::Extras::default(),
            cfg,
            now,
            (width as usize).saturating_sub(name.len() + 2),
            false,
        );
        out.push(Line::from(vec![
            Span::styled(name, Style::new().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::raw(row),
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
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--all" => mode = Some(Mode::All),
            "--once" => once = true,
            "--session" => {
                let id = it.next().unwrap_or_else(|| fail("--session needs an id"));
                if !paths::valid_session_id(id) {
                    fail("invalid session id");
                }
                mode = Some(Mode::Fixed(id.clone()));
            }
            other => fail(&format!(
                "unknown option {other} (try --session <id>, --all, --once)"
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
        cfg: config::load(),
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

    let mut term = ratatui::init();
    let result = (|| -> std::io::Result<()> {
        loop {
            app.refresh();
            let now = hook::now_ms();
            let v = app.st.verdict_with(now, app.cfg.thresholds.stuck_repeats);
            if app.cfg.bell && v == Verdict::NeedsYou && app.last_verdict.is_some_and(|l| l != v) {
                let mut out = std::io::stdout();
                out.write_all(b"\x07")?;
                out.flush()?;
            }
            app.last_verdict = Some(v);
            term.draw(|f| {
                let area = f.area();
                let lines = app.lines(area.width, area.height);
                let align = if app.dir.is_none() && app.mode != Mode::All {
                    Alignment::Center
                } else {
                    Alignment::Left
                };
                f.render_widget(Paragraph::new(lines).alignment(align), area);
            })?;
            if !event::poll(if app.animating(now) { FAST } else { SLOW })? {
                continue;
            }
            if let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char('e') => app.view.expand_errors ^= true,
                    KeyCode::Char('a') => app.view.expand_agents ^= true,
                    KeyCode::Tab => app.cycle(),
                    _ => {}
                }
            }
        }
    })();
    ratatui::restore();
    if let Err(e) = result {
        fail(&e.to_string());
    }
    std::process::exit(0)
}

//! `ari hub --all`: every live session merged into one state the hub draws
//! (spec: docs/superpowers/specs/2026-11-06-hub-all-design.md).

use crate::app::App;
use crate::sessions;
use crate::state::{self, Agent, State};
use crate::tokens::{self, TokenCache};
use crate::usage::{self, Usage};
use std::path::Path;

/// One live session, ready to merge.
pub struct Part {
    /// The first 8 chars of the session id.
    pub key: String,
    pub name: String,
    pub st: State,
    pub app: App,
    pub u: Usage,
    pub tokens: TokenCache,
}

/// Live sessions under `root` (the `--all` list's set), most recently active first.
pub fn load(root: &Path, now: u64) -> Vec<Part> {
    sessions::list(root)
        .into_iter()
        .filter(|s| sessions::is_live(s, now))
        .map(|s| {
            let u = usage::load(&s.dir);
            let key: String = s.id.chars().take(8).collect();
            let name = u
                .session_name
                .clone()
                .or_else(|| u.project.clone())
                .or_else(|| s.cwd.as_deref()?.rsplit('/').next().map(String::from))
                .unwrap_or_else(|| key.clone());
            Part {
                st: state::load_session(&s.dir),
                app: s.app,
                tokens: tokens::cached(&s.dir),
                key,
                name,
                u,
            }
        })
        .collect()
}

/// One state over all sessions: the root `main` is you, each session's lead becomes `<key>`
/// under it, and every other agent `<key>/<id>`. `parts` are most recently active first.
pub fn merge(parts: Vec<Part>, now: u64) -> (State, Usage, TokenCache) {
    let mut st = State {
        label: Some(format!("{} sessions", parts.len())),
        ..Default::default()
    };
    st.agents.push(Agent {
        id: "main".into(),
        // The graph node's second line.
        kind: format!("{} sessions", parts.len()),
        name: Some("you".into()),
        ..Default::default()
    });
    let mut u = Usage {
        project: Some("all sessions".into()),
        ..Default::default()
    };
    let mut tok = TokenCache::default();
    let mut limits: Vec<(
        App,
        Option<crate::usage::Limit>,
        Option<crate::usage::Limit>,
    )> = vec![];
    // shortcut: another session is addressed by its name as ARI knows it; aliases miss.
    let names: Vec<(String, String)> = parts
        .iter()
        .map(|p| (p.name.to_lowercase(), p.key.clone()))
        .collect();
    for p in parts {
        let key = &p.key;
        let id = |a: &str| {
            if a == "main" {
                key.clone()
            } else {
                format!("{key}/{a}")
            }
        };
        let s = p.st;
        let size = p.u.ctx_size;
        // Paths are relative to each session's folder; anchor them so two repos' README.md differ.
        let anchor = |a: &mut Agent| {
            if let Some(dir) = &p.u.dir {
                a.files = std::mem::take(&mut a.files)
                    .into_iter()
                    .map(|(k, f)| match k.starts_with('/') {
                        true => (k, f),
                        false => (format!("{dir}/{k}"), f),
                    })
                    .collect();
            }
        };
        let mut lead = s
            .agents
            .iter()
            .find(|a| a.id == "main")
            .cloned()
            .unwrap_or_else(|| Agent {
                started: s.first_t,
                ..Default::default()
            });
        if lead.started == 0 {
            lead.started = now; // live by its transcript, no hook events yet
        }
        lead.id = key.clone();
        lead.kind = "session".into();
        lead.app = p.app;
        lead.name = Some(p.name.clone());
        lead.parent = Some("main".into());
        // A live session isn't stale while its subagents work.
        lead.last_t = lead.last_t.max(s.last_t);
        lead.ctx_size = size;
        // shortcut: the default stuck threshold; pass the config's if a custom one reads wrong here.
        let repeats = crate::config::Config::default().thresholds.stuck_repeats;
        lead.session = Some((s.turn_active, s.verdict_with(repeats)));
        anchor(&mut lead);
        st.agents.push(lead);
        for a in s.agents.iter().filter(|a| a.id != "main") {
            let mut a = a.clone();
            a.parent = Some(id(a.parent.as_deref().unwrap_or("main")));
            a.id = id(&a.id);
            a.ctx_size = size;
            anchor(&mut a);
            st.agents.push(a);
        }
        let known = |to: &str| to == "main" || s.agents.iter().any(|a| a.id == to);
        st.messages.extend(s.messages.iter().map(|m| {
            let mut m = m.clone();
            m.from = id(&m.from);
            m.to = if known(&m.to) {
                id(&m.to)
            } else {
                let to = m.to.to_lowercase();
                names
                    .iter()
                    .find(|(n, _)| *n == to)
                    .map_or(m.to, |(_, k)| k.clone())
            };
            m
        }));
        st.errors.extend(s.errors.iter().map(|e| {
            let mut e = e.clone();
            e.agent = id(&e.agent);
            // `open_errors` counts the merged turn (0): this session's current turn maps there.
            e.turn = (e.turn != s.turns) as u32;
            e
        }));
        st.tasks.extend(s.tasks.iter().map(|t| {
            let mut t = t.clone();
            t.id = format!("{key}/{}", t.id);
            t.blocked = t.blocked.iter().map(|b| format!("{key}/{b}")).collect();
            t
        }));
        if st.needs_you.is_none() {
            st.needs_you = s.needs_you.as_ref().map(|n| format!("{}: {n}", p.name));
        }
        st.turn_active |= s.turn_active;
        st.turn_failed |= s.turn_failed;
        if s.first_t > 0 && (st.first_t == 0 || s.first_t < st.first_t) {
            st.first_t = s.first_t;
        }
        st.last_t = st.last_t.max(s.last_t);
        // Dollars are Claude Code's; other apps' tokens are counted, not priced.
        if p.app == App::Claude
            && let Some(c) = p.u.cost_usd
        {
            *u.cost_usd.get_or_insert(0.0) += c;
        }
        // Limits are per account: each app's most recent session has the freshest.
        if (p.u.five_h.is_some() || p.u.seven_d.is_some())
            && !limits.iter().any(|(a, ..)| *a == p.app)
        {
            limits.push((p.app, p.u.five_h, p.u.seven_d));
        }
        tok.agents.insert(key.clone(), p.tokens.main);
        for (a, l) in p.tokens.agents {
            tok.agents.insert(id(&a), l);
        }
    }
    if let Some((_, five, seven)) = limits.iter().find(|l| l.0 == App::Claude) {
        (u.five_h, u.seven_d) = (*five, *seven);
    }
    if limits.iter().any(|l| l.0 != App::Claude) {
        limits.sort_by_key(|l| l.0 != App::Claude); // stable: Claude first
        u.app_limits = limits
            .into_iter()
            .map(|(a, f, s)| (a.label().to_string(), f, s))
            .collect();
    }
    let span = |t: u64| if t == 0 { now } else { t };
    (st.agents[0].started, st.agents[0].last_t) = (span(st.first_t), span(st.last_t));
    st.messages.make_contiguous().sort_by_key(|m| m.t);
    (st, u, tok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::tests::{kid, with_main};
    use crate::state::{Agent, Msg, State, Verdict};
    use crate::tokens::{Ledger, TokenCache};
    use crate::usage::Usage;

    fn part(key: &str, name: &str, st: State) -> Part {
        Part {
            key: key.into(),
            name: name.into(),
            st,
            app: App::Claude,
            u: Usage::default(),
            tokens: TokenCache::default(),
        }
    }
    fn two() -> (Part, Part) {
        (
            part("aaaaaaaa", "api", with_main(vec![kid("rev", "main", 5)])),
            part("bbbbbbbb", "web", with_main(vec![kid("rev", "main", 6)])),
        )
    }
    fn agent<'a>(st: &'a State, id: &str) -> &'a Agent {
        st.agents.iter().find(|a| a.id == id).unwrap()
    }

    #[test]
    fn empty_is_root_only() {
        let (st, u, _) = merge(vec![], 0);
        assert_eq!(st.agents.len(), 1);
        assert_eq!(st.agents[0].label(), "you");
        assert_eq!(u.project.as_deref(), Some("all sessions"));
    }

    #[test]
    fn sessions_hang_under_root_and_ids_dont_collide() {
        let (mut a, b) = two();
        a.tokens.agents.insert("rev".into(), Ledger::default());
        let (st, _, t) = merge(vec![a, b], 0);
        let ids: Vec<_> = st.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "main",
                "aaaaaaaa",
                "aaaaaaaa/rev",
                "bbbbbbbb",
                "bbbbbbbb/rev"
            ]
        );
        assert_eq!(st.agents[1].parent.as_deref(), Some("main"));
        assert_eq!(st.agents[1].label(), "api");
        assert_eq!(st.agents[2].parent.as_deref(), Some("aaaaaaaa"));
        assert!(t.agents.contains_key("aaaaaaaa") && t.agents.contains_key("aaaaaaaa/rev"));
        assert_eq!(st.label.as_deref(), Some("2 sessions"));
        assert_eq!(
            st.agents[0].kind, "2 sessions",
            "the root node's second line"
        );
    }

    #[test]
    fn session_without_events_still_has_a_node() {
        let (st, _, _) = merge(vec![part("cccccccc", "cli", State::default())], 0);
        assert_eq!(agent(&st, "cccccccc").label(), "cli");
    }

    #[test]
    fn messages_map_and_cross_sessions_by_name() {
        let (mut a, b) = two();
        let m = |from: &str, to: &str, t| Msg {
            t,
            from: from.into(),
            to: to.into(),
            text: "hi".into(),
        };
        a.st.messages.extend([
            m("rev", "main", 1),
            m("main", "Web", 2),
            m("main", "nobody", 3),
        ]);
        let (st, _, _) = merge(vec![a, b], 0);
        let got: Vec<_> = st
            .messages
            .iter()
            .map(|m| (m.from.as_str(), m.to.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("aaaaaaaa/rev", "aaaaaaaa"),
                ("aaaaaaaa", "bbbbbbbb"),
                ("aaaaaaaa", "nobody")
            ]
        );
    }

    #[test]
    fn each_session_keeps_its_window_size() {
        let (mut a, b) = two();
        a.u.ctx_size = Some(1_000_000);
        a.tokens.main = Ledger {
            ctx: 100_000,
            ..Default::default()
        };
        let (st, u, tokens) = merge(vec![a, b], 0);
        assert_eq!(agent(&st, "aaaaaaaa").ctx_size, Some(1_000_000));
        assert_eq!(agent(&st, "aaaaaaaa/rev").ctx_size, Some(1_000_000));
        assert_eq!(agent(&st, "bbbbbbbb").ctx_size, None);
        let (x, cfg) = (
            crate::bar::Extras {
                tokens: Some(tokens),
                ..Default::default()
            },
            Default::default(),
        );
        let i = crate::hub::Input {
            st: &st,
            u: &u,
            x: &x,
            cfg: &cfg,
            now: 0,
            clock: "",
        };
        assert_eq!(
            crate::hub::ctx_of(&i, agent(&st, "aaaaaaaa")),
            Some((100_000, 10.0))
        );
    }

    #[test]
    fn a_waiting_agent_makes_the_merge_need_you() {
        let (a, mut b) = two();
        b.st.agents[1].waiting = Some((1, "Bash rm".into()));
        let (st, _, _) = merge(vec![a, b], 0);
        assert_eq!(st.verdict_with(3), Verdict::NeedsYou);
    }

    fn input<'a>(
        st: &'a State,
        u: &'a Usage,
        x: &'a crate::bar::Extras,
        cfg: &'a crate::config::Config,
        now: u64,
    ) -> crate::hub::Input<'a> {
        crate::hub::Input {
            st,
            u,
            x,
            cfg,
            now,
            clock: "",
        }
    }

    #[test]
    fn session_nodes_show_the_session_state() {
        let (mut a, b) = two();
        a.st.turn_active = true;
        let idle = b.st.verdict_with(3);
        let (st, u, _) = merge(vec![a, b], 0);
        let (x, cfg) = Default::default();
        let i = input(&st, &u, &x, &cfg, 0);
        let busy = crate::hub::agents::state_text(&i, agent(&st, "aaaaaaaa")).0;
        assert!(busy.ends_with("thinking"), "{busy}");
        let quiet = crate::hub::agents::state_text(&i, agent(&st, "bbbbbbbb")).0;
        assert_eq!(
            quiet,
            format!("{} {}", idle.glyph(), idle.label().to_lowercase())
        );
        // Even long idle, a live session isn't stale.
        let i = input(&st, &u, &x, &cfg, 10 * crate::state::STALE_MS);
        let later = crate::hub::agents::state_text(&i, agent(&st, "bbbbbbbb")).0;
        assert_eq!(later, quiet);
        assert_eq!(st.running_agents(0), 2, "the two revs, not the sessions");
    }

    #[test]
    fn current_turn_errors_still_count() {
        let (mut a, b) = two();
        a.st.turns = 3;
        a.st.errors.push(crate::state::ErrorRec {
            agent: "main".into(),
            turn: 3,
            ..Default::default()
        });
        a.st.errors.push(crate::state::ErrorRec {
            agent: "main".into(),
            turn: 2,
            ..Default::default()
        });
        let (st, _, _) = merge(vec![a, b], 0);
        assert_eq!(st.open_errors(), 1);
    }

    #[test]
    fn same_relative_path_in_two_repos_is_not_shared() {
        let files = |dir: &str| {
            let (mut p, _) = two();
            p.u.dir = Some(dir.into());
            p.st.agents[0]
                .files
                .insert("README.md".into(), Default::default());
            p
        };
        let (mut a, mut b) = (files("/a"), files("/b"));
        b.key = "bbbbbbbb".into();
        let (st, _, _) = merge(vec![a, b], 0);
        assert!(crate::hub::agents::shared_files(&st).is_empty());
        (a, b) = (files("/a"), files("/a"));
        b.key = "bbbbbbbb".into();
        let (st, _, _) = merge(vec![a, b], 0);
        assert_eq!(crate::hub::agents::shared_files(&st).len(), 1);
    }

    #[test]
    fn nothing_started_reads_as_now() {
        let (st, _, _) = merge(vec![part("cccccccc", "cli", State::default())], 77);
        assert_eq!(st.agents[0].started, 77);
        assert_eq!(agent(&st, "cccccccc").started, 77);
    }

    #[test]
    fn root_spans_the_sessions() {
        let (mut a, mut b) = two();
        (a.st.first_t, a.st.last_t) = (50, 90);
        (b.st.first_t, b.st.last_t) = (20, 70);
        let (st, _, _) = merge(vec![a, b], 0);
        assert_eq!((st.agents[0].started, st.agents[0].last_t), (20, 90));
    }
}

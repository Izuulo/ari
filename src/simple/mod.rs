//! The simple views (Calm, Constellation): plain-English verbs and the alert banner.

use crate::event::runs_tests;
use crate::hub::{err, put};
use crate::state::{Agent, AgentStatus, State};
use ratatui::{buffer::Buffer, layout::Rect, style::Modifier};

/// What the agent is doing, in words with no paths, commands or numbers.
pub fn phrase(st: &State, a: &Agent, now: u64) -> String {
    let s = match st.agent_status(a, now) {
        _ if a.waiting.is_some() => "needs you",
        AgentStatus::Done => {
            let green = a
                .recent
                .iter()
                .rev()
                .find(|r| r.tool == "Bash" && runs_tests(&r.act))
                .is_some_and(|r| r.ok == Some(true));
            if green {
                "finished · all green"
            } else {
                "finished"
            }
        }
        AgentStatus::Stale => "taking a break",
        AgentStatus::Running if a.id == "main" && !st.turn_active => "waiting for you",
        AgentStatus::Running => match a.recent.back() {
            Some(r) if r.ok == Some(false) => "hit a snag",
            Some(r) if a.running_since.is_some() => return tool_verb(&r.tool, &r.act),
            _ => "thinking",
        },
    };
    s.into()
}

fn tool_verb(tool: &str, act: &str) -> String {
    match tool {
        "Bash" if runs_tests(act) => "running the tests".into(),
        "Bash" => "running a command".into(),
        "Read" => "reading".into(),
        "Grep" | "Glob" => "searching the code".into(),
        "Edit" | "Write" | "MultiEdit" | "apply_patch" => "writing code".into(),
        "Task" | "Agent" => "sending out a helper".into(),
        "SendMessage" => format!("messaging {}", act.strip_prefix("message ").unwrap_or(act)),
        "WebFetch" | "WebSearch" => "browsing the web".into(),
        "TodoWrite" | "TaskCreate" | "TaskUpdate" => "planning".into(),
        "Skill" => "using a skill".into(),
        _ => "working".into(),
    }
}

/// Animated trailing dots, one step per 300 ms.
pub fn dots(now: u64) -> &'static str {
    [".", "..", "...", ""][(now / 300 % 4) as usize]
}

pub struct Banner {
    pub text: String,
    pub on: bool,
}

pub fn banner(st: &State, now: u64) -> Option<Banner> {
    let mut waiting = st.agents.iter().filter(|a| a.waiting.is_some());
    let text = if let Some(a) = waiting.next() {
        let more = waiting.count();
        let action = a.waiting.as_ref().map_or("", |w| w.1.as_str());
        let plus = if more > 0 {
            format!(" (+{more})")
        } else {
            String::new()
        };
        format!("! {} needs you: {action}{plus}", a.label())
    } else if let Some(ask) = &st.needs_you {
        format!("! needs you: {ask}")
    } else {
        let e = st
            .errors
            .iter()
            .filter(|e| now.saturating_sub(e.last_t) <= 5000)
            .max_by_key(|e| e.last_t)?;
        format!("✗ {} hit a snag", e.agent)
    };
    Some(Banner {
        text,
        on: (now / 500).is_multiple_of(2),
    })
}

/// The full-width top row; blank while `!on`, so it blinks.
pub fn draw_banner(buf: &mut Buffer, area: Rect, b: &Banner) {
    if b.on {
        put(
            buf,
            area.x,
            area.y,
            &b.text,
            area.width,
            err().add_modifier(Modifier::BOLD),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::tests::{kid, with_main};
    use crate::state::{CallRec, ErrorRec, STALE_MS};

    fn rec(tool: &str, act: &str, ok: Option<bool>) -> CallRec {
        CallRec {
            tool: tool.into(),
            act: act.into(),
            ok,
            ..Default::default()
        }
    }
    fn running(tool: &str, act: &str) -> (State, Agent) {
        let mut a = kid("k", "main", 0);
        a.last_t = 1000;
        a.running_since = Some(1000);
        a.recent.push_back(rec(tool, act, None));
        (with_main(vec![]), a)
    }
    fn say(tool: &str, act: &str) -> String {
        let (st, a) = running(tool, act);
        phrase(&st, &a, 2000)
    }

    #[test]
    fn verbs() {
        for (tool, act, want) in [
            ("Bash", "cargo test", "running the tests"),
            ("Bash", "ls", "running a command"),
            ("Read", "src/a.rs", "reading"),
            ("Grep", "foo", "searching the code"),
            ("Edit", "src/a.rs", "writing code"),
            ("WebFetch", "fetch http://x/1", "browsing the web"),
            ("Mystery", "x", "working"),
            ("SendMessage", "message scout", "messaging scout"),
        ] {
            let p = say(tool, act);
            assert_eq!(p, want);
            assert!(!p.contains(['/', '$']) && !p.chars().any(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn between_tools_is_thinking() {
        let (st, mut a) = running("Read", "x");
        a.running_since = None;
        assert_eq!(phrase(&st, &a, 2000), "thinking");
    }

    #[test]
    fn finished() {
        let (st, mut a) = running("Bash", "cargo test");
        a.done = true;
        a.recent[0].ok = Some(true);
        assert_eq!(phrase(&st, &a, 2000), "finished · all green");
        a.recent[0].ok = Some(false);
        assert_eq!(phrase(&st, &a, 2000), "finished");
        a.recent.clear();
        assert_eq!(phrase(&st, &a, 2000), "finished");
    }

    #[test]
    fn states() {
        let (st, mut a) = running("Read", "x");
        assert_eq!(phrase(&st, &a, 1000 + STALE_MS + 1), "taking a break");
        a.recent[0].ok = Some(false);
        assert_eq!(phrase(&st, &a, 2000), "hit a snag");
        a.waiting = Some((1, "Bash".into()));
        assert_eq!(phrase(&st, &a, 2000), "needs you");
        let mut st = with_main(vec![]);
        st.turn_active = false;
        let m = st.agents[0].clone();
        assert_eq!(phrase(&st, &m, 2000), "waiting for you");
    }

    #[test]
    fn dots_cycle() {
        assert_eq!([0, 300, 600, 900].map(dots), [".", "..", "...", ""]);
    }

    #[test]
    fn banner_priority_and_blink() {
        let mut a = kid("a", "main", 0);
        a.waiting = Some((1, "run it".into()));
        let mut b = kid("b", "main", 0);
        b.waiting = Some((1, "x".into()));
        let st = with_main(vec![a, b]);
        let t = banner(&st, 0).unwrap().text;
        assert!(t.starts_with("! ") && t.ends_with("(+1)"), "{t}");

        let mut st = with_main(vec![]);
        st.errors.push(ErrorRec {
            agent: "k".into(),
            last_t: 10_000,
            ..Default::default()
        });
        assert!(banner(&st, 14_000).unwrap().text.contains("hit a snag"));
        assert!(banner(&st, 16_000).is_none());
        assert!(banner(&st, 0).unwrap().on);
        assert!(!banner(&st, 10_500).unwrap().on);
    }

    #[test]
    fn draw_blinks() {
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        let mut b = Banner {
            text: "! hi".into(),
            on: false,
        };
        draw_banner(&mut buf, area, &b);
        assert_eq!(buf[(0, 0)].symbol(), " ");
        b.on = true;
        draw_banner(&mut buf, area, &b);
        assert_eq!(buf[(0, 0)].symbol(), "!");
    }
}

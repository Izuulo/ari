//! The reducer: a pure fold of events into session state, plus the verdict.

use crate::event::Event;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const STALE_MS: u64 = 300_000;
const RECENT: usize = 10;
const MAX_ERRORS: usize = 100;
const CACHE_VERSION: u32 = 3;
const CALLS: usize = 30;
const STUCK_WINDOW: usize = 10;
const FILE_EDITS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
/// Bookkeeping tools that repeat by design.
const NOT_LOOPS: [&str; 6] = [
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    "TodoWrite",
    "ToolSearch",
];
/// Notification types that mean Claude is waiting on the user (docs/DECISIONS.md D5).
const NEEDS_YOU: [&str; 5] = [
    "permission_prompt",
    "idle_prompt",
    "agent_needs_input",
    "elicitation_dialog",
    "elicitation_url_dialog",
];

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct State {
    pub turn_active: bool,
    pub turn_failed: bool,
    pub turns: u32,
    pub label: Option<String>,
    pub needs_you: Option<String>,
    pub tasks: Vec<Task>,
    pub agents: Vec<Agent>,
    pub errors: Vec<ErrorRec>,
    /// Last tool results across all agents: (ok, signature).
    pub recent: VecDeque<(bool, String)>,
    pub last_main_result: Option<(bool, String)>,
    /// Last tool calls across agents, for loop detection.
    pub calls: VecDeque<Call>,
    pub compacted_at: Option<u64>,
    pub first_t: u64,
    pub last_t: u64,
    pub ended: bool,
    /// Background tasks (shells, agents) still running at the last Stop.
    pub background: u32,
    /// Recent test runs, oldest first.
    pub test_runs: VecDeque<TestRun>,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct TestRun {
    pub t: u64,
    pub pass: u32,
    pub fail: u32,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Task {
    pub id: String,
    pub subject: String,
    pub status: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Agent {
    pub id: String,
    pub kind: String,
    pub started: u64,
    pub last_t: u64,
    pub calls: u32,
    pub act: Option<String>,
    pub done: bool,
    pub result: Option<String>,
    pub transcript: Option<String>,
    pub ended_t: Option<u64>,
    /// When the tool it's running now started; `None` between tools.
    pub running_since: Option<u64>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ErrorRec {
    pub sig: String,
    pub tool: String,
    pub msg: String,
    pub kind: Option<String>,
    pub agent: String,
    pub first_t: u64,
    pub last_t: u64,
    pub retries: u32,
    pub resolved_t: Option<u64>,
    pub resolved_by: Option<String>,
    pub turn: u32,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Call {
    pub agent: String,
    pub tool: String,
    pub h: u64,
    pub act: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    NeedsYou,
    Blocked,
    Stuck,
    Retrying,
    OnTrack,
    Done,
    Idle,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::NeedsYou => "NEEDS YOU",
            Verdict::Blocked => "BLOCKED",
            Verdict::Stuck => "STUCK",
            Verdict::Retrying => "RETRYING",
            Verdict::OnTrack => "ON TRACK",
            Verdict::Done => "DONE",
            Verdict::Idle => "IDLE",
        }
    }

    pub fn glyph(self) -> char {
        match self {
            Verdict::NeedsYou => '◉',
            Verdict::Blocked => '✗',
            Verdict::Stuck | Verdict::Retrying => '↻',
            Verdict::OnTrack => '●',
            Verdict::Done => '✓',
            Verdict::Idle => '◌',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Done,
    Stale,
}

impl State {
    pub fn apply(&mut self, ev: &Event) {
        if self.first_t == 0 {
            self.first_t = ev.t;
        }
        self.last_t = self.last_t.max(ev.t);
        if self.agents.is_empty() {
            self.agents.push(Agent {
                id: "main".into(),
                kind: "main".into(),
                started: ev.t,
                ..Default::default()
            });
        }
        let aid = ev.a.as_deref().unwrap_or("main");
        if ev.a.is_some() || ev.tool.is_some() {
            let a = self.agent_mut(aid, ev);
            a.last_t = ev.t;
        }
        match ev.e.as_str() {
            "UserPromptSubmit" => {
                let msg = ev.msg.as_deref().unwrap_or("");
                self.needs_you = None;
                self.start_turn();
                if !msg.starts_with("<task-notification>") {
                    self.turns += 1;
                    self.turn_failed = false;
                    if self.label.is_none() && !msg.is_empty() {
                        self.label = Some(crate::event::truncate(msg, 40));
                    }
                }
            }
            "PreToolUse" => {
                self.needs_you = None;
                self.start_turn();
                let a = self.agent_mut(aid, ev);
                a.calls += 1;
                a.running_since = Some(ev.t);
                a.act = ev.act.clone();
                self.calls.push_back(Call {
                    agent: aid.into(),
                    tool: ev.tool.clone().unwrap_or_default(),
                    h: ev.h.unwrap_or(0),
                    act: ev.act.clone().unwrap_or_default(),
                });
                if self.calls.len() > CALLS {
                    self.calls.pop_front();
                }
            }
            "PostToolUse" | "PostToolUseFailure" => {
                self.needs_you = None;
                if let Some((pass, fail)) = ev.tests {
                    self.test_runs.push_back(TestRun {
                        t: ev.t,
                        pass,
                        fail,
                    });
                    if self.test_runs.len() > 20 {
                        self.test_runs.pop_front();
                    }
                }
                self.agent_mut(aid, ev).running_since = None;
                self.tool_result(ev, aid);
                self.update_tasks(ev);
            }
            "SubagentStart" => {
                let a = self.agent_mut(aid, ev);
                a.started = ev.t;
                a.done = false;
            }
            "SubagentStop" => {
                let a = self.agent_mut(aid, ev);
                a.done = true;
                a.ended_t = Some(ev.t);
                a.result = ev.msg.clone();
                a.transcript = ev.path.clone();
                a.act = None;
                a.running_since = None;
            }
            "Notification" => {
                if ev.kind.as_deref().is_some_and(|k| NEEDS_YOU.contains(&k)) {
                    self.needs_you = Some(
                        ev.msg
                            .clone()
                            .or_else(|| ev.kind.clone())
                            .unwrap_or_default(),
                    );
                }
            }
            "PreCompact" => self.compacted_at = Some(ev.t),
            "Stop" => {
                self.needs_you = None;
                for e in self
                    .errors
                    .iter_mut()
                    .filter(|e| e.tool == "StopFailure" && e.resolved_t.is_none())
                {
                    e.resolved_t = Some(ev.t);
                    e.resolved_by = Some("main".into());
                }
                self.background = ev.bg.unwrap_or(0);
                self.agents[0].running_since = None;
                if self.background == 0 {
                    self.turn_active = false;
                    self.agents[0].act = None;
                }
            }
            "StopFailure" => {
                self.turn_active = false;
                self.turn_failed = true;
                self.agents[0].act = None;
                self.agents[0].running_since = None;
                let kind = ev.kind.clone().unwrap_or_else(|| "unknown".into());
                self.add_error(ErrorRec {
                    sig: format!("StopFailure:{kind}"),
                    tool: "StopFailure".into(),
                    msg: ev.msg.clone().unwrap_or_else(|| kind.clone()),
                    kind: Some(kind),
                    agent: "main".into(),
                    first_t: ev.t,
                    last_t: ev.t,
                    turn: self.turns,
                    ..Default::default()
                });
            }
            "TaskCreated" | "TaskCompleted" => self.update_tasks(ev),
            "SessionStart" => self.ended = false,
            "SessionEnd" => {
                self.ended = true;
                self.turn_active = false;
                self.background = 0;
                for a in &mut self.agents {
                    a.running_since = None;
                }
            }
            _ => {}
        }
    }

    fn start_turn(&mut self) {
        self.turn_active = true;
        self.ended = false;
    }

    fn agent_mut(&mut self, id: &str, ev: &Event) -> &mut Agent {
        let i = match self.agents.iter().position(|a| a.id == id) {
            Some(i) => i,
            None => {
                self.agents.push(Agent {
                    id: id.into(),
                    kind: ev.at.clone().unwrap_or_else(|| "agent".into()),
                    started: ev.t,
                    last_t: ev.t,
                    ..Default::default()
                });
                self.agents.len() - 1
            }
        };
        let a = &mut self.agents[i];
        if let Some(at) = &ev.at
            && id != "main"
        {
            a.kind = at.clone();
        }
        a
    }

    fn agent_label(&self, id: &str) -> String {
        self.agents
            .iter()
            .find(|a| a.id == id)
            .map_or_else(|| id.to_string(), |a| a.kind.clone())
    }

    fn tool_result(&mut self, ev: &Event, aid: &str) {
        let ok = ev.ok == Some(true);
        let sig = ev
            .sig
            .clone()
            .or_else(|| ev.tool.clone())
            .unwrap_or_default();
        if ev.kind.as_deref() == Some("interrupt") {
            return;
        }
        self.recent.push_back((ok, sig.clone()));
        if self.recent.len() > RECENT {
            self.recent.pop_front();
        }
        if aid == "main" {
            self.last_main_result = Some((ok, sig.clone()));
        }
        let who = self.agent_label(aid);
        if ok {
            for e in self
                .errors
                .iter_mut()
                .filter(|e| e.sig == sig && e.resolved_t.is_none())
            {
                e.resolved_t = Some(ev.t);
                e.resolved_by = Some(who.clone());
            }
            return;
        }
        let msg = ev.msg.clone().unwrap_or_default();
        if let Some(e) = self
            .errors
            .iter_mut()
            .rev()
            .find(|e| e.sig == sig && e.resolved_t.is_none())
        {
            e.retries += 1;
            e.last_t = ev.t;
            e.msg = msg;
            e.kind = ev.kind.clone();
            return;
        }
        self.add_error(ErrorRec {
            sig,
            tool: ev.tool.clone().unwrap_or_default(),
            msg,
            kind: ev.kind.clone(),
            agent: who,
            first_t: ev.t,
            last_t: ev.t,
            turn: self.turns,
            ..Default::default()
        });
    }

    fn add_error(&mut self, e: ErrorRec) {
        self.errors.push(e);
        if self.errors.len() > MAX_ERRORS {
            let i = self
                .errors
                .iter()
                .position(|e| e.resolved_t.is_some())
                .unwrap_or(0);
            self.errors.remove(i);
        }
    }

    fn update_tasks(&mut self, ev: &Event) {
        if let Some(todos) = &ev.todos {
            self.tasks = todos
                .iter()
                .enumerate()
                .map(|(i, t)| Task {
                    id: format!("todo-{i}"),
                    subject: t.c.clone(),
                    status: t.s.clone(),
                })
                .collect();
        }
        let Some(t) = &ev.task else { return };
        if t.status.as_deref() == Some("deleted") {
            self.tasks.retain(|x| x.id != t.id);
            return;
        }
        let task = match self.tasks.iter_mut().find(|x| x.id == t.id) {
            Some(x) => x,
            None => {
                self.tasks.push(Task {
                    id: t.id.clone(),
                    subject: format!("task #{}", t.id),
                    status: "pending".into(),
                });
                self.tasks.last_mut().unwrap()
            }
        };
        if let Some(s) = &t.subject {
            task.subject = s.clone();
        }
        if let Some(s) = &t.status {
            task.status = s.clone();
        }
    }

    fn unresolved(&self, sig: &str) -> bool {
        self.errors
            .iter()
            .any(|e| e.sig == sig && e.resolved_t.is_none())
    }

    pub fn verdict(&self, now: u64) -> Verdict {
        self.verdict_with(now, 3)
    }

    /// Why Claude looks stuck, if it does (PRD 7.4). `repeats` = `thresholds.stuck_repeats`.
    pub fn stuck_reason(&self, repeats: u8) -> Option<String> {
        let repeats = repeats.max(2) as usize;
        // Rule B: the same signature failing again and again without a success in between.
        if let Some(e) = self.errors.iter().rev().find(|e| {
            e.resolved_t.is_none()
                && e.retries as usize + 1 >= repeats
                && self.recent.iter().any(|(ok, sig)| !ok && *sig == e.sig)
        }) {
            return Some(format!(
                "{} failed {}× in a row",
                crate::panel::sig_words(e),
                e.retries + 1
            ));
        }
        // Rule A: one agent repeating an identical call with no file edit in between.
        let mut agents: Vec<&str> = self.calls.iter().map(|c| c.agent.as_str()).collect();
        agents.dedup();
        for agent in agents {
            let mine: Vec<&Call> = self.calls.iter().filter(|c| c.agent == agent).collect();
            let window = &mine[mine.len().saturating_sub(STUCK_WINDOW)..];
            for c in window
                .iter()
                .filter(|c| !NOT_LOOPS.contains(&c.tool.as_str()))
            {
                let same: Vec<usize> = (0..window.len())
                    .filter(|&i| window[i].tool == c.tool && window[i].h == c.h)
                    .collect();
                if same.len() < repeats {
                    continue;
                }
                let (first, last) = (same[0], same[same.len() - 1]);
                let edited = (first..=last)
                    .any(|i| !same.contains(&i) && FILE_EDITS.contains(&window[i].tool.as_str()));
                if !edited {
                    return Some(format!("same call {}× · {}", same.len(), c.act));
                }
            }
        }
        None
    }

    pub fn verdict_with(&self, _now: u64, repeats: u8) -> Verdict {
        if self.needs_you.is_some() {
            return Verdict::NeedsYou;
        }
        if self.turn_failed {
            return Verdict::Blocked;
        }
        if !self.turn_active {
            // Ended right after a failure that never got fixed: Claude gave up on it.
            if let Some((false, sig)) = &self.last_main_result
                && self
                    .errors
                    .iter()
                    .any(|e| &e.sig == sig && e.resolved_t.is_none() && e.turn == self.turns)
            {
                return Verdict::Blocked;
            }
            let (done, total) = self.tasks_done();
            return if total > 0 && done == total {
                Verdict::Done
            } else {
                Verdict::Idle
            };
        }
        if self.stuck_reason(repeats).is_some() {
            return Verdict::Stuck;
        }
        if self
            .recent
            .iter()
            .any(|(ok, sig)| !ok && self.unresolved(sig))
        {
            return Verdict::Retrying;
        }
        Verdict::OnTrack
    }

    /// The latest test run had failures.
    pub fn tests_failing(&self) -> bool {
        self.test_runs.back().is_some_and(|r| r.fail > 0)
    }

    pub fn tasks_done(&self) -> (usize, usize) {
        (
            self.tasks
                .iter()
                .filter(|t| t.status == "completed")
                .count(),
            self.tasks.len(),
        )
    }

    pub fn now_task(&self) -> Option<&Task> {
        self.tasks.iter().find(|t| t.status == "in_progress")
    }

    pub fn agent_status(&self, a: &Agent, now: u64) -> AgentStatus {
        if a.done {
            AgentStatus::Done
        } else if a.id != "main" && now.saturating_sub(a.last_t) > STALE_MS {
            AgentStatus::Stale
        } else {
            AgentStatus::Running
        }
    }

    /// How long the agent's current tool has been running, if one is.
    pub fn tool_running(&self, a: &Agent, now: u64) -> Option<u64> {
        a.running_since
            .filter(|_| self.agent_status(a, now) == AgentStatus::Running)
            .map(|t| now.saturating_sub(t))
    }

    pub fn running_agents(&self, now: u64) -> usize {
        self.agents
            .iter()
            .filter(|a| a.id != "main" && self.agent_status(a, now) == AgentStatus::Running)
            .count()
    }

    /// Unresolved errors from the current turn.
    pub fn open_errors(&self) -> usize {
        self.errors
            .iter()
            .filter(|e| e.resolved_t.is_none() && e.turn == self.turns)
            .count()
    }
}

#[derive(Serialize, Deserialize)]
struct Cache {
    v: u32,
    offset: u64,
    state: State,
}

/// Bring the cached state for a session dir up to date with its events.jsonl.
/// Only complete lines are consumed, so a hook mid-write is picked up next time.
pub fn load_session(dir: &Path) -> State {
    let cache_path = dir.join("state.json");
    let mut cache = std::fs::read(&cache_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Cache>(&b).ok())
        .filter(|c| c.v == CACHE_VERSION)
        .unwrap_or(Cache {
            v: CACHE_VERSION,
            offset: 0,
            state: State::default(),
        });
    let Ok(mut f) = std::fs::File::open(dir.join("events.jsonl")) else {
        return cache.state;
    };
    let len = f.metadata().map_or(0, |m| m.len());
    if len < cache.offset {
        cache = Cache {
            v: CACHE_VERSION,
            offset: 0,
            state: State::default(),
        };
    }
    if len == cache.offset {
        return cache.state;
    }
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(cache.offset)).is_err() || f.read_to_end(&mut buf).is_err() {
        return cache.state;
    }
    let Some(end) = buf.iter().rposition(|b| *b == b'\n').map(|i| i + 1) else {
        return cache.state;
    };
    for line in buf[..end].split(|b| *b == b'\n') {
        if let Ok(ev) = serde_json::from_slice::<Event>(line) {
            cache.state.apply(&ev);
        }
    }
    cache.offset += end as u64;
    if let Ok(bytes) = serde_json::to_vec(&cache) {
        let _ = crate::paths::write_atomic(&cache_path, &bytes);
    }
    cache.state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{TaskRef, Todo};

    fn ev(e: &str) -> Event {
        Event {
            t: 1_000,
            e: e.into(),
            ..Default::default()
        }
    }
    fn tool(e: &str, sig: &str, ok: Option<bool>) -> Event {
        Event {
            tool: Some("Bash".into()),
            sig: Some(sig.into()),
            ok,
            act: Some(format!("$ {sig}")),
            ..ev(e)
        }
    }
    fn run(events: &[Event]) -> State {
        let mut s = State::default();
        for e in events {
            s.apply(e);
        }
        s
    }
    fn prompt(msg: &str) -> Event {
        Event {
            msg: Some(msg.into()),
            ..ev("UserPromptSubmit")
        }
    }
    fn stop(bg: u32) -> Event {
        Event {
            bg: Some(bg),
            ..ev("Stop")
        }
    }
    fn task(id: &str, subject: Option<&str>, status: Option<&str>) -> Event {
        Event {
            task: Some(TaskRef {
                id: id.into(),
                subject: subject.map(Into::into),
                status: status.map(Into::into),
            }),
            ..ev("TaskCreated")
        }
    }

    fn call(agent: &str, tool: &str, h: u64, act: &str) -> Event {
        Event {
            a: (agent != "main").then(|| agent.to_string()),
            tool: Some(tool.into()),
            h: Some(h),
            act: Some(act.into()),
            ..ev("PreToolUse")
        }
    }

    #[test]
    fn stuck_on_identical_calls() {
        let r = call("main", "Read", 7, "reading a.rs");
        let s = run(&[prompt("go"), r.clone(), r.clone(), r.clone()]);
        assert_eq!(s.verdict(0), Verdict::Stuck);
        assert_eq!(
            s.stuck_reason(3).as_deref(),
            Some("same call 3× · reading a.rs")
        );
        assert_eq!(s.verdict_with(0, 4), Verdict::OnTrack);
        let edit = call("main", "Edit", 9, "editing a.rs");
        let s = run(&[prompt("go"), r.clone(), edit, r.clone(), r.clone()]);
        assert_eq!(
            s.verdict(0),
            Verdict::OnTrack,
            "an edit between repeats is progress"
        );
        let t = call("main", "TaskUpdate", 1, "updating plan");
        assert_eq!(
            run(&[prompt("go"), t.clone(), t.clone(), t]).verdict(0),
            Verdict::OnTrack
        );
        let par: Vec<Event> = ["a1", "a2", "a3"]
            .iter()
            .map(|a| call(a, "Read", 7, "reading a.rs"))
            .collect();
        let mut evs = vec![prompt("go")];
        evs.extend(par);
        assert_eq!(
            run(&evs).verdict(0),
            Verdict::OnTrack,
            "parallel agents don't add up"
        );
        let mut s = run(&[prompt("go"), r.clone(), r.clone(), r]);
        s.apply(&stop(0));
        assert_ne!(s.verdict(0), Verdict::Stuck);
    }

    #[test]
    fn stuck_on_repeated_failure() {
        let fail = Event {
            sig: Some("Bash:cargo build".into()),
            ..tool("PostToolUseFailure", "Bash:cargo build", Some(false))
        };
        let mut s = run(&[prompt("go"), fail.clone(), fail.clone()]);
        assert_eq!(s.verdict(0), Verdict::Retrying);
        s.apply(&fail);
        assert_eq!(s.verdict(0), Verdict::Stuck);
        assert_eq!(
            s.stuck_reason(3).as_deref(),
            Some("cargo build failed 3× in a row")
        );
        s.apply(&tool("PostToolUse", "Bash:cargo build", Some(true)));
        assert_eq!(s.verdict(0), Verdict::OnTrack);
    }

    #[test]
    fn tracks_the_running_tool() {
        let pre = Event {
            t: 1_000,
            ..call("main", "Bash", 1, "$ cargo test")
        };
        let mut s = run(&[prompt("go"), pre]);
        assert_eq!(s.tool_running(&s.agents[0], 13_000), Some(12_000));
        let post = Event {
            t: 14_000,
            ..tool("PostToolUse", "Bash:cargo test", Some(true))
        };
        s.apply(&post);
        assert_eq!(s.tool_running(&s.agents[0], 15_000), None);
        s.apply(&Event {
            bg: Some(2),
            ..stop(0)
        });
        assert_eq!((s.background, s.turn_active), (2, true));
    }

    #[test]
    fn needs_you_until_next_tool() {
        let note = Event {
            kind: Some("permission_prompt".into()),
            msg: Some("Claude needs your permission".into()),
            ..ev("Notification")
        };
        let mut s = run(&[prompt("go"), note]);
        assert_eq!(s.verdict(0), Verdict::NeedsYou);
        s.apply(&tool("PreToolUse", "Bash:ls", None));
        assert_eq!(s.verdict(0), Verdict::OnTrack);
        let other = Event {
            kind: Some("auth_success".into()),
            ..ev("Notification")
        };
        s.apply(&other);
        assert_eq!(s.verdict(0), Verdict::OnTrack);
    }

    #[test]
    fn stop_failure_blocks_until_next_prompt() {
        let sf = Event {
            kind: Some("rate_limit".into()),
            ..ev("StopFailure")
        };
        let mut s = run(&[prompt("go"), sf]);
        assert_eq!(s.verdict(0), Verdict::Blocked);
        assert_eq!(s.open_errors(), 1);
        s.apply(&prompt("again"));
        assert_eq!(s.verdict(0), Verdict::OnTrack);
        s.apply(&stop(0));
        assert!(s.errors[0].resolved_t.is_some());
    }

    #[test]
    fn retry_then_fix() {
        let fail = tool("PostToolUseFailure", "Bash:cargo build", Some(false));
        let mut s = run(&[prompt("go"), fail.clone(), fail]);
        assert_eq!(s.verdict(0), Verdict::Retrying);
        assert_eq!((s.errors.len(), s.errors[0].retries), (1, 1));
        s.apply(&tool("PostToolUse", "Bash:cargo build", Some(true)));
        assert_eq!(s.verdict(0), Verdict::OnTrack);
        assert_eq!(s.errors[0].resolved_by.as_deref(), Some("main"));
        assert_eq!(s.open_errors(), 0);
    }

    #[test]
    fn old_errors_go_cold() {
        let mut s = run(&[
            prompt("go"),
            tool("PostToolUseFailure", "Bash:grep x", Some(false)),
        ]);
        for i in 0..10 {
            s.apply(&tool("PostToolUse", &format!("Bash:ok {i}"), Some(true)));
        }
        assert_eq!(s.verdict(0), Verdict::OnTrack);
        assert_eq!(s.open_errors(), 1);
        s.apply(&stop(0));
        assert_eq!(s.verdict(0), Verdict::Idle);
    }

    #[test]
    fn ending_on_a_failure_blocks() {
        let s = run(&[
            prompt("go"),
            tool("PostToolUseFailure", "Bash:make", Some(false)),
            stop(0),
        ]);
        assert_eq!(s.verdict(0), Verdict::Blocked);
    }

    #[test]
    fn interrupts_are_not_errors() {
        let mut f = tool("PostToolUseFailure", "Bash:sleep", Some(false));
        f.kind = Some("interrupt".into());
        let s = run(&[prompt("go"), f]);
        assert!(s.errors.is_empty());
        assert_eq!(s.verdict(0), Verdict::OnTrack);
    }

    #[test]
    fn background_agents_keep_turn_active() {
        let s = run(&[prompt("go"), task("1", Some("a"), None), stop(1)]);
        assert_eq!(s.verdict(0), Verdict::OnTrack);
    }

    #[test]
    fn done_vs_idle() {
        let s = run(&[
            prompt("go"),
            task("1", Some("a"), Some("completed")),
            stop(0),
        ]);
        assert_eq!(s.verdict(0), Verdict::Done);
        let s = run(&[
            prompt("go"),
            task("1", Some("a"), Some("completed")),
            task("2", Some("b"), None),
            stop(0),
        ]);
        assert_eq!(s.verdict(0), Verdict::Idle);
        let s = run(&[prompt("go"), stop(0)]);
        assert_eq!(s.verdict(0), Verdict::Idle);
    }

    #[test]
    fn test_runs_history() {
        let t = |pass, fail| Event {
            tests: Some((pass, fail)),
            ..tool("PostToolUse", "Bash:cargo test", Some(true))
        };
        let mut s = run(&[prompt("go"), t(5, 0), t(6, 1)]);
        assert_eq!(s.test_runs.len(), 2);
        assert_eq!(s.test_runs.back().map(|r| (r.pass, r.fail)), Some((6, 1)));
        for i in 0..30 {
            s.apply(&t(i, 0));
        }
        assert_eq!(s.test_runs.len(), 20, "capped");
    }

    #[test]
    fn done_with_failing_tests() {
        let red = Event {
            tests: Some((67, 2)),
            ..tool("PostToolUseFailure", "Bash:cargo test", Some(false))
        };
        let s = run(&[
            prompt("go"),
            task("1", Some("a"), Some("completed")),
            red,
            stop(0),
        ]);
        assert!(s.tests_failing());
        let green = Event {
            tests: Some((69, 0)),
            ..tool("PostToolUse", "Bash:cargo test", Some(true))
        };
        let s = run(&[
            prompt("go"),
            task("1", Some("a"), Some("completed")),
            green,
            stop(0),
        ]);
        assert!(!s.tests_failing());
        assert_eq!(s.verdict(0), Verdict::Done);
    }

    #[test]
    fn old_cache_version_rebuilds() {
        let dir = std::env::temp_dir().join(format!("ari-state-v2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let line = crate::event::to_line(&prompt("go"));
        std::fs::write(dir.join("events.jsonl"), &line).unwrap();
        let stale = format!(
            "{{\"v\":2,\"offset\":{},\"state\":{{\"turns\":99}}}}",
            line.len()
        );
        std::fs::write(dir.join("state.json"), stale).unwrap();
        assert_eq!(load_session(&dir).turns, 1);
    }

    #[test]
    fn synthetic_prompts_dont_count() {
        let s = run(&[
            prompt("fix the auth bug"),
            stop(1),
            prompt("<task-notification> <task-id>x</task-id>"),
        ]);
        assert_eq!(s.turns, 1);
        assert_eq!(s.label.as_deref(), Some("fix the auth bug"));
        assert!(s.turn_active);
    }

    #[test]
    fn task_lifecycle() {
        let mut s = run(&[task("1", Some("write"), None), task("2", Some("run"), None)]);
        s.apply(&Event {
            task: Some(TaskRef {
                id: "1".into(),
                subject: None,
                status: Some("in_progress".into()),
            }),
            ..ev("PostToolUse")
        });
        assert_eq!(s.now_task().unwrap().subject, "write");
        s.apply(&task("2", None, Some("deleted")));
        assert_eq!(s.tasks_done(), (0, 1));
        let todos = Event {
            todos: Some(vec![
                Todo {
                    c: "x".into(),
                    s: "completed".into(),
                },
                Todo {
                    c: "y".into(),
                    s: "pending".into(),
                },
            ]),
            ..ev("PostToolUse")
        };
        s.apply(&todos);
        assert_eq!(s.tasks_done(), (1, 2));
    }

    #[test]
    fn orphan_and_stale_agents() {
        let sub = Event {
            a: Some("ab5f".into()),
            at: Some("Explore".into()),
            ..tool("PreToolUse", "Bash:ls", None)
        };
        let s = run(&[sub]);
        assert_eq!(s.agents.len(), 2);
        assert_eq!(
            (s.agents[1].kind.as_str(), s.agents[1].calls),
            ("Explore", 1)
        );
        assert_eq!(s.running_agents(1_000), 1);
        assert_eq!(
            s.agent_status(&s.agents[1], 1_000 + 6 * 60_000),
            AgentStatus::Stale
        );
        assert_eq!(s.running_agents(1_000 + 6 * 60_000), 0);
        let stop_only = Event {
            a: Some("zz".into()),
            at: Some("Plan".into()),
            msg: Some("done".into()),
            ..ev("SubagentStop")
        };
        let s = run(&[stop_only]);
        assert_eq!(s.agent_status(&s.agents[1], 0), AgentStatus::Done);
    }

    #[test]
    fn incremental_cache() {
        let dir = std::env::temp_dir().join(format!("ari-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let line = |e: &Event| crate::event::to_line(e);
        let full = line(&tool("PostToolUseFailure", "Bash:make", Some(false)));
        let (half_a, half_b) = full.split_at(10);
        let text = line(&prompt("go"))
            + &line(&task("1", Some("a"), None))
            + &line(&task("2", Some("b"), None))
            + half_a;
        std::fs::write(dir.join("events.jsonl"), text).unwrap();
        let s = load_session(&dir);
        assert_eq!((s.tasks.len(), s.errors.len()), (2, 0));
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("events.jsonl"))
            .unwrap()
            .write_all(half_b.as_bytes())
            .unwrap();
        let s = load_session(&dir);
        assert_eq!((s.tasks.len(), s.errors.len(), s.turns), (2, 1, 1));
        std::fs::write(dir.join("state.json"), "{corrupt").unwrap();
        assert_eq!(load_session(&dir), s);
    }
}

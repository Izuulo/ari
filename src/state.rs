//! The reducer: a pure fold of events into session state, plus the verdict.

use crate::event::Event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const STALE_MS: u64 = 300_000;
const RECENT: usize = 10;
const MAX_ERRORS: usize = 100;
const CACHE_VERSION: u32 = 5;
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
/// Prompts where Claude is blocked on you mid-turn; `idle_prompt` is idle time, not this.
const WAITS_ON_YOU: [&str; 3] = [
    "permission_prompt",
    "elicitation_dialog",
    "elicitation_url_dialog",
];

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
    pub time: TimeSplit,
    /// "agent\tpath" → input hashes read since the file last changed.
    pub reads: BTreeMap<String, Vec<u64>>,
    /// Reads repeating an identical earlier read of an unchanged file.
    pub reread: u32,
    /// Absolute path → last successful edit time, for conflicts across sessions.
    pub edits: BTreeMap<String, u64>,
    /// Start times of the last real prompts (turns).
    pub prompts: VecDeque<u64>,
}

/// Wall time not spent by Claude itself, folded from events.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TimeSplit {
    /// Main agent's tool time; subagents run in parallel and would double-count.
    pub tools_ms: u64,
    /// A permission or input prompt was open.
    pub waiting_ms: u64,
    /// Between Stop and the next prompt.
    pub idle_ms: u64,
    pub waiting_since: Option<u64>,
    pub idle_since: Option<u64>,
    /// End of the latest counted main-agent tool call, so overlapping calls count once.
    pub tools_until: u64,
}

/// Checkbox progress of a Markdown plan the session wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanFile {
    pub name: String,
    pub done: usize,
    pub total: usize,
}

/// With no task list (the task tools are off by default), the newest Markdown file this
/// session wrote that has `- [ ]` checkboxes stands in as its plan.
pub fn plan_file(st: &State) -> Option<PlanFile> {
    if st.tasks_done().1 > 0 {
        return None;
    }
    let mut md: Vec<_> = st
        .edits
        .iter()
        .filter(|(p, _)| p.ends_with(".md"))
        .collect();
    md.sort_by_key(|(_, t)| std::cmp::Reverse(**t));
    md.into_iter().take(5).find_map(|(p, _)| {
        let (done, total) = checkboxes(&std::fs::read_to_string(p).ok()?);
        (total > 0).then(|| PlanFile {
            name: p.rsplit('/').next().unwrap_or(p).to_string(),
            done,
            total,
        })
    })
}

/// (checked, total) Markdown task-list boxes: `- [ ]`, `- [x]`, `* [X]`.
pub fn checkboxes(text: &str) -> (usize, usize) {
    let (mut done, mut total) = (0, 0);
    for l in text.lines().map(str::trim_start) {
        let Some(rest) = l.strip_prefix("- [").or_else(|| l.strip_prefix("* [")) else {
            continue;
        };
        if rest.get(1..2) != Some("]") {
            continue;
        }
        match rest.get(..1) {
            Some(" ") => total += 1,
            Some("x" | "X") => {
                total += 1;
                done += 1;
            }
            _ => {}
        }
    }
    (done, total)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    pub claude: u64,
    pub tools: u64,
    pub waiting: u64,
    pub idle: u64,
    /// Wall time none of the above accounts for.
    pub other: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct TestRun {
    pub t: u64,
    pub pass: u32,
    pub fail: u32,
    pub partial: bool,
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
    /// Σ duration of this error's failed calls.
    pub time_ms: u64,
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
        if let Some(since) = self.time.waiting_since.take() {
            // Approving fires no hook; a Post event's tool started when the wait ended.
            let end = match ev.e.as_str() {
                "PostToolUse" | "PostToolUseFailure" => ev.t.saturating_sub(ev.dur.unwrap_or(0)),
                _ => ev.t,
            };
            self.time.waiting_ms += end.saturating_sub(since);
        }
        let aid = ev.a.as_deref().unwrap_or("main");
        if ev.a.is_some() || ev.tool.is_some() {
            let a = self.agent_mut(aid, ev);
            a.last_t = ev.t;
        }
        match ev.e.as_str() {
            "UserPromptSubmit" => {
                if let Some(since) = self.time.idle_since.take() {
                    self.time.idle_ms += ev.t.saturating_sub(since);
                }
                let msg = ev.msg.as_deref().unwrap_or("");
                self.needs_you = None;
                self.start_turn();
                if !msg.starts_with("<task-notification>") {
                    self.prompts.push_back(ev.t);
                    if self.prompts.len() > 6 {
                        self.prompts.pop_front();
                    }
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
                if ev.a.is_none() {
                    let t = &mut self.time;
                    let start = ev.t.saturating_sub(ev.dur.unwrap_or(0)).max(t.tools_until);
                    t.tools_ms += ev.t.saturating_sub(start);
                    t.tools_until = t.tools_until.max(ev.t);
                }
                self.track_reads(ev, aid);
                if let Some((pass, fail)) = ev.tests {
                    self.test_runs.push_back(TestRun {
                        t: ev.t,
                        pass,
                        fail,
                        partial: ev.partial,
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
                if ev
                    .kind
                    .as_deref()
                    .is_some_and(|k| WAITS_ON_YOU.contains(&k))
                {
                    self.time.waiting_since = Some(ev.t);
                }
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
                // You are idle after Stop even while background tasks run.
                self.time.idle_since.get_or_insert(ev.t);
                if self.background == 0 {
                    self.turn_active = false;
                    self.agents[0].act = None;
                }
            }
            "StopFailure" => {
                self.time.idle_since.get_or_insert(ev.t);
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
            e.time_ms += ev.dur.unwrap_or(0);
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
            time_ms: ev.dur.unwrap_or(0),
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

    fn track_reads(&mut self, ev: &Event, aid: &str) {
        let Some((tool, path)) = ev.sig.as_deref().and_then(|s| s.split_once(':')) else {
            return;
        };
        match tool {
            "Read" if ev.ok == Some(true) => {
                let seen = self.reads.entry(format!("{aid}\t{path}")).or_default();
                let h = ev.h.unwrap_or(0);
                if seen.contains(&h) {
                    self.reread += 1;
                } else {
                    seen.push(h);
                }
            }
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" if ev.ok == Some(true) => {
                self.edits.insert(path.to_string(), ev.t);
                self.reads.retain(|k, _| !k.ends_with(&format!("\t{path}")));
            }
            _ => {}
        }
    }

    /// Wall time split four ways. Claude is the API time when known, else what's left.
    pub fn split(&self, wall_ms: u64, api_ms: Option<u64>) -> Split {
        let t = &self.time;
        let others = t.tools_ms + t.waiting_ms + t.idle_ms;
        let claude = api_ms.unwrap_or(wall_ms.saturating_sub(others));
        Split {
            claude,
            tools: t.tools_ms,
            waiting: t.waiting_ms,
            idle: t.idle_ms,
            other: wall_ms.saturating_sub(claude + others),
        }
    }

    /// The latest test run had failures.
    pub fn tests_failing(&self) -> bool {
        self.test_runs.back().is_some_and(|r| r.fail > 0)
    }

    /// The run to headline: the latest if it failed, else the latest full run.
    pub fn headline_run(&self) -> Option<&TestRun> {
        self.test_runs
            .back()
            .filter(|r| r.fail > 0)
            .or_else(|| self.test_runs.iter().rev().find(|r| !r.partial))
            .or(self.test_runs.back())
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
    #[test]
    fn checkboxes_count_task_list_items_only() {
        let md = "- [ ] a\n  - [x] b\n* [X] c\n- [?] no\n- [] no\n[ ] no\n- [ ]";
        assert_eq!(super::checkboxes(md), (2, 4));
    }

    #[test]
    fn newest_markdown_plan_stands_in_for_tasks() {
        let dir = std::env::temp_dir().join(format!("ari-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plan = dir.join("m6-richness.md");
        let notes = dir.join("notes.md");
        std::fs::write(&plan, "- [x] one\n- [x] two\n- [ ] three\n").unwrap();
        std::fs::write(&notes, "no boxes here\n").unwrap();
        let mut st = super::State::default();
        st.edits.insert(plan.to_string_lossy().into(), 1);
        st.edits.insert(notes.to_string_lossy().into(), 2); // newer, but no checkboxes
        st.edits
            .insert(dir.join("a.rs").to_string_lossy().into(), 3);
        let got = super::plan_file(&st);
        st.tasks.push(super::Task::default());
        let with_tasks = super::plan_file(&st);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            got,
            Some(super::PlanFile {
                name: "m6-richness.md".into(),
                done: 2,
                total: 3
            })
        );
        assert_eq!(with_tasks, None, "a real task list wins");
    }

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
    fn stop_failure_and_background_stop_start_idle() {
        let sf = Event {
            kind: Some("rate_limit".into()),
            ..ev("StopFailure")
        };
        for stopper in [sf, stop(1)] {
            let mut s = run(&[prompt("go"), stopper]);
            s.apply(&Event {
                t: 61_000,
                ..prompt("again")
            });
            assert_eq!(s.time.idle_ms, 60_000);
        }
    }

    #[test]
    fn split_other_is_the_remainder() {
        let mut s = State::default();
        s.time.tools_ms = 10;
        s.time.idle_ms = 20;
        let sp = s.split(100, Some(30));
        assert_eq!(sp.other, 40);
        assert_eq!(s.split(50, Some(30)).other, 0);
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
    fn time_split() {
        let at = |t: u64, e: Event| Event { t, ..e };
        let post = |t: u64, dur: u64, agent: Option<&str>| Event {
            t,
            dur: Some(dur),
            a: agent.map(Into::into),
            ..tool("PostToolUse", "Bash:make", Some(true))
        };
        let needs = Event {
            kind: Some("permission_prompt".into()),
            ..ev("Notification")
        };
        let s = run(&[
            at(0, prompt("go")),
            post(5_000, 5_000, None),
            post(9_000, 4_000, Some("sub1")),
            at(10_000, needs),
            at(40_000, call("main", "Bash", 1, "$ make")),
            at(50_000, stop(0)),
            at(110_000, prompt("again")),
        ]);
        assert_eq!(
            (s.time.tools_ms, s.time.waiting_ms, s.time.idle_ms),
            (5_000, 30_000, 60_000),
            "subagent tool time not counted"
        );
        let sp = s.split(120_000, Some(20_000));
        assert_eq!(
            (sp.claude, sp.tools, sp.waiting, sp.idle),
            (20_000, 5_000, 30_000, 60_000)
        );
        let sp = s.split(120_000, None);
        assert_eq!(sp.claude, 25_000, "remainder when api time is unknown");
        assert_eq!(s.split(50_000, None).claude, 0, "never negative");
    }

    #[test]
    fn mistake_cost() {
        let fail = |dur| Event {
            dur: Some(dur),
            ..tool("PostToolUseFailure", "Bash:make", Some(false))
        };
        let s = run(&[prompt("go"), fail(2_000), fail(3_000)]);
        assert_eq!(s.errors[0].time_ms, 5_000);
        let read = |agent: &str, path: &str| Event {
            a: (agent != "main").then(|| agent.to_string()),
            tool: Some("Read".into()),
            sig: Some(format!("Read:{path}")),
            h: Some(path.len() as u64),
            ok: Some(true),
            ..ev("PostToolUse")
        };
        let edit = |path: &str| Event {
            tool: Some("Edit".into()),
            sig: Some(format!("Edit:{path}")),
            ok: Some(true),
            ..ev("PostToolUse")
        };
        assert_eq!(run(&[read("main", "/a"), read("main", "/a")]).reread, 1);
        assert_eq!(
            run(&[read("main", "/a"), edit("/a"), read("main", "/a")]).reread,
            0,
            "the file changed in between"
        );
        assert_eq!(run(&[read("main", "/a"), read("sub1", "/a")]).reread, 0);
    }

    #[test]
    fn prompt_times_kept() {
        let at = |t: u64, e: Event| Event { t, ..e };
        let mut evs = vec![at(1, prompt("<task-notification>x"))];
        evs.extend((0..10).map(|i| at(10 + i, prompt("go"))));
        let s = run(&evs);
        assert_eq!(s.prompts.len(), 6, "capped");
        assert_eq!(s.prompts.back(), Some(&19));
        assert!(!s.prompts.contains(&1), "synthetic prompts are not turns");
    }

    #[test]
    fn idle_prompt_is_not_waiting_on_you() {
        let at = |t: u64, e: Event| Event { t, ..e };
        let idle = Event {
            kind: Some("idle_prompt".into()),
            ..ev("Notification")
        };
        let s = run(&[
            at(0, prompt("go")),
            at(10_000, stop(0)),
            at(70_000, idle),
            at(130_000, prompt("back")),
        ]);
        assert_eq!((s.time.waiting_ms, s.time.idle_ms), (0, 120_000));
    }

    #[test]
    fn permission_wait_ends_when_the_tool_starts() {
        let at = |t: u64, e: Event| Event { t, ..e };
        let perm = Event {
            kind: Some("permission_prompt".into()),
            ..ev("Notification")
        };
        // Approved at 15 s; the tool then ran 60 s and reported at 75 s.
        let post = Event {
            dur: Some(60_000),
            ..tool("PostToolUse", "Bash:cargo build", Some(true))
        };
        let s = run(&[at(0, prompt("go")), at(10_000, perm), at(75_000, post)]);
        assert_eq!((s.time.waiting_ms, s.time.tools_ms), (5_000, 60_000));
    }

    #[test]
    fn overlapping_tools_count_once() {
        let post = |t: u64, dur: u64| Event {
            t,
            dur: Some(dur),
            ..tool("PostToolUse", "Agent", Some(true))
        };
        // Two 10 s calls in parallel, 1 s apart: 11 s of wall time.
        let s = run(&[post(10_000, 10_000), post(11_000, 10_000)]);
        assert_eq!(s.time.tools_ms, 11_000);
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

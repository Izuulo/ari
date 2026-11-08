//! The reducer: a pure fold of events into session state, plus the verdict.

use crate::event::Event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

pub const STALE_MS: u64 = 300_000;
const RECENT: usize = 10;
const MAX_ERRORS: usize = 100;
const CACHE_VERSION: u32 = 12;
const CALLS: usize = 30;
const STUCK_WINDOW: usize = 10;
const RECENT_CALLS: usize = 40;
pub const LANE_MS: u64 = 12_000;
pub const LANES: usize = 50;
const MESSAGES: usize = 60;
const SPAWNS: usize = 16;
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
    /// Running background tasks by name, from the main agent's last Stop.
    pub background_names: Vec<String>,
    pub crons: Vec<String>,
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
    /// The last real turns, oldest first.
    pub turn_log: VecDeque<TurnRec>,
    /// Compactions: (time, trigger), oldest first.
    pub compactions: VecDeque<(u64, String)>,
    /// Last permission mode a hook reported.
    pub permission_mode: Option<String>,
    /// Set by a resumed session's start, cleared at the main agent's Stop.
    pub resume: Option<Resume>,
    /// Messages between agents (SendMessage), oldest first.
    pub messages: VecDeque<Msg>,
    /// Agent tool calls whose subagent hasn't shown up yet.
    pub spawns: VecDeque<Spawn>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Resume {
    pub away_ms: Option<u64>,
    pub cold: Option<bool>,
    pub rebuild_usd: Option<f64>,
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

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TurnRec {
    pub start: u64,
    /// None while the turn runs.
    pub end: Option<u64>,
    pub calls: u32,
    pub failed: u32,
    /// False when the turn ended in a StopFailure.
    pub ok: bool,
    pub prompt: String,
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
    pub owner: Option<String>,
    pub blocked: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Agent {
    /// Hub `--all`: the app of the session this node belongs to.
    #[serde(skip)]
    pub app: crate::app::App,
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
    /// The name its spawn gave it (SendMessage addresses agents by it).
    pub name: Option<String>,
    /// Id of the agent that spawned it.
    pub parent: Option<String>,
    pub prompt: Option<String>,
    /// Its last tool calls, oldest first.
    pub recent: VecDeque<CallRec>,
    /// A permission prompt it's waiting on: (since, action).
    pub waiting: Option<(u64, String)>,
    /// Tool calls per `LANE_MS` bucket: (bucket, count), oldest first.
    pub lane: VecDeque<(u64, u16)>,
    /// Files it edited, by path as shown.
    pub files: BTreeMap<String, FileRec>,
    /// Its context window when known (`ari hub --all` sets each session's); else the session's.
    pub ctx_size: Option<u64>,
    /// `ari hub --all` only: a session's lead carries (turn active, verdict) of its session.
    #[serde(skip)]
    pub session: Option<(bool, Verdict)>,
}

impl Agent {
    pub fn label(&self) -> String {
        if self.id == "main" {
            self.name.clone().unwrap_or_else(|| "main".into())
        } else {
            self.name.clone().unwrap_or_else(|| self.kind.clone())
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CallRec {
    pub t: u64,
    /// tool_use_id, to pair a result with its call when calls overlap.
    pub id: Option<String>,
    pub tool: String,
    pub act: String,
    pub ok: Option<bool>,
    pub dur: Option<u64>,
    /// The failure's first line.
    pub msg: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Msg {
    pub t: u64,
    /// Agent ids (`main` for the lead); `to` stays as written when no agent has that name.
    pub from: String,
    pub to: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Spawn {
    /// The Agent call's tool_use_id.
    pub id: String,
    pub by: String,
    pub name: Option<String>,
    pub kind: String,
    pub prompt: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct FileRec {
    pub edits: u32,
    pub added: u32,
    pub removed: u32,
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

    pub fn id(self) -> &'static str {
        match self {
            Verdict::NeedsYou => "needs_you",
            Verdict::Blocked => "blocked",
            Verdict::Stuck => "stuck",
            Verdict::Retrying => "retrying",
            Verdict::OnTrack => "on_track",
            Verdict::Done => "done",
            Verdict::Idle => "idle",
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
        if ev.pm.is_some() && ev.a.is_none() {
            self.permission_mode.clone_from(&ev.pm);
        }
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
        if matches!(
            ev.e.as_str(),
            "PreToolUse"
                | "PostToolUse"
                | "PostToolUseFailure"
                | "Stop"
                | "SubagentStop"
                | "StopFailure"
        ) && let Some(a) = self.agents.iter_mut().find(|a| a.id == aid)
        {
            a.waiting = None;
        }
        match ev.e.as_str() {
            "UserPromptSubmit" => {
                if let Some(since) = self.time.idle_since.take() {
                    self.time.idle_ms += ev.t.saturating_sub(since);
                }
                let msg = ev.msg.as_deref().unwrap_or("");
                self.needs_you = None;
                for a in &mut self.agents {
                    a.waiting = None;
                }
                self.start_turn();
                if !crate::event::is_synthetic(msg) {
                    // An Esc interrupt fires no Stop; the next prompt ends that turn.
                    self.end_turn(ev.t, true);
                    self.prompts.push_back(ev.t);
                    if self.prompts.len() > 6 {
                        self.prompts.pop_front();
                    }
                    self.turns += 1;
                    self.turn_failed = false;
                    self.turn_log.push_back(TurnRec {
                        start: ev.t,
                        ok: true,
                        prompt: msg.into(),
                        ..Default::default()
                    });
                    if self.turn_log.len() > 50 {
                        self.turn_log.pop_front();
                    }
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
                a.recent.push_back(CallRec {
                    t: ev.t,
                    id: ev.id.clone(),
                    tool: ev.tool.clone().unwrap_or_default(),
                    act: ev.act.clone().unwrap_or_default(),
                    ..Default::default()
                });
                if a.recent.len() > RECENT_CALLS {
                    a.recent.pop_front();
                }
                let b = ev.t / LANE_MS;
                match a.lane.back_mut() {
                    Some((lb, n)) if *lb == b => *n = n.saturating_add(1),
                    _ => a.lane.push_back((b, 1)),
                }
                while a
                    .lane
                    .front()
                    .is_some_and(|(lb, _)| *lb + LANES as u64 <= b)
                {
                    a.lane.pop_front();
                }
                match ev.tool.as_deref() {
                    Some("Agent") => {
                        self.spawns.push_back(Spawn {
                            id: ev.id.clone().unwrap_or_default(),
                            by: aid.into(),
                            name: ev.name.clone(),
                            kind: ev.kind.clone().unwrap_or_default(),
                            prompt: ev.msg.clone(),
                        });
                        if self.spawns.len() > SPAWNS {
                            self.spawns.pop_front();
                        }
                    }
                    Some("SendMessage") if ev.to.is_some() => {
                        let to = self.resolve(ev.to.as_deref().unwrap_or(""));
                        self.messages.push_back(Msg {
                            t: ev.t,
                            from: aid.into(),
                            to,
                            text: ev.msg.clone().unwrap_or_default(),
                        });
                        if self.messages.len() > MESSAGES {
                            self.messages.pop_front();
                        }
                    }
                    _ => {}
                }
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
                    if let Some(r) = self.turn_log.back_mut().filter(|r| r.end.is_none()) {
                        r.calls += 1;
                        r.failed += (ev.e == "PostToolUseFailure") as u32;
                    }
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
                let a = self.agent_mut(aid, ev);
                a.running_since = None;
                let tool = ev.tool.as_deref().unwrap_or("");
                // Parallel calls of one tool: match by tool_use_id, else the oldest still open.
                let k = a
                    .recent
                    .iter()
                    .position(|r| r.ok.is_none() && r.id.is_some() && r.id == ev.id)
                    .or_else(|| {
                        a.recent
                            .iter()
                            .position(|r| r.ok.is_none() && r.tool == tool)
                    });
                if let Some(r) = k.map(|k| &mut a.recent[k]) {
                    r.ok = ev.ok;
                    r.dur = ev.dur;
                    r.msg = ev.msg.clone().filter(|_| ev.ok == Some(false));
                }
                if let (Some((added, removed)), Some(act)) = (ev.lines, &ev.act) {
                    let path = act.strip_prefix("editing ").unwrap_or(act);
                    let f = a.files.entry(path.to_string()).or_default();
                    f.edits += 1;
                    f.added += added;
                    f.removed += removed;
                }
                if tool == "Agent" {
                    self.spawns.retain(|s| Some(&s.id) != ev.id.as_ref());
                }
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
            "PermissionRequest" => {
                let a = self.agent_mut(aid, ev);
                a.waiting = Some((ev.t, ev.act.clone().unwrap_or_default()));
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
            "PreCompact" => {
                self.compacted_at = Some(ev.t);
                self.compactions
                    .push_back((ev.t, ev.kind.clone().unwrap_or_else(|| "auto".into())));
                if self.compactions.len() > 20 {
                    self.compactions.pop_front();
                }
            }
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
                if ev.a.is_none() {
                    self.end_turn(ev.t, true);
                    self.resume = None;
                    if let Some(b) = &ev.bgs {
                        self.background_names = b.clone();
                    }
                    if let Some(c) = &ev.crons {
                        self.crons = c.clone();
                    }
                }
                self.agents[0].running_since = None;
                // You are idle after Stop even while background tasks run.
                self.time.idle_since.get_or_insert(ev.t);
                if self.background == 0 {
                    self.turn_active = false;
                    self.agents[0].act = None;
                }
            }
            // A subagent's API failure (e.g. an expired login) ends that agent, not main's turn.
            "StopFailure" if aid != "main" => {
                let a = self.agent_mut(aid, ev);
                a.done = true;
                a.ended_t = Some(ev.t);
                a.result = ev.msg.clone();
                a.act = None;
                a.running_since = None;
                let kind = ev.kind.clone().unwrap_or_else(|| "unknown".into());
                self.add_error(ErrorRec {
                    sig: format!("StopFailure:{kind}"),
                    tool: "StopFailure".into(),
                    msg: ev.msg.clone().unwrap_or_else(|| kind.clone()),
                    kind: Some(kind),
                    agent: aid.into(),
                    first_t: ev.t,
                    last_t: ev.t,
                    turn: self.turns,
                    ..Default::default()
                });
            }
            "StopFailure" => {
                self.end_turn(ev.t, false);
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
            "SessionStart" => {
                self.ended = false;
                self.resume = (ev.kind.as_deref() == Some("resume")).then_some(Resume {
                    away_ms: ev.dur,
                    cold: ev.cold,
                    rebuild_usd: ev.usd,
                });
            }
            "SessionEnd" => {
                self.ended = true;
                self.end_turn(ev.t, true);
                self.turn_active = false;
                self.background = 0;
                self.background_names.clear();
                self.crons.clear();
                for a in &mut self.agents {
                    a.running_since = None;
                }
            }
            _ => {}
        }
    }

    fn end_turn(&mut self, t: u64, ok: bool) {
        if let Some(r) = self.turn_log.back_mut().filter(|r| r.end.is_none()) {
            r.end = Some(t);
            r.ok = ok;
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
                // A new agent claims the oldest pending spawn of its type, else the oldest.
                let claim = self
                    .spawns
                    .iter()
                    .position(|s| Some(&s.kind) == ev.at.as_ref())
                    .or_else(|| (!self.spawns.is_empty()).then_some(0))
                    .and_then(|k| self.spawns.remove(k));
                self.agents.push(Agent {
                    id: id.into(),
                    kind: ev.at.clone().unwrap_or_else(|| "agent".into()),
                    started: ev.t,
                    last_t: ev.t,
                    name: claim.as_ref().and_then(|c| c.name.clone()),
                    parent: claim.as_ref().map(|c| c.by.clone()),
                    prompt: claim.and_then(|c| c.prompt),
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

    /// A SendMessage recipient as an agent id; unknown names stay as written.
    fn resolve(&self, to: &str) -> String {
        match to {
            "team-lead" | "main" | "lead" => "main".into(),
            _ => self
                .agent_by_name(to)
                .map_or_else(|| to.into(), |a| a.id.clone()),
        }
    }

    pub fn agent_by_name(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.name.as_deref() == Some(name))
    }

    /// The agent with the oldest open permission wait.
    pub fn waiting(&self) -> Option<&Agent> {
        self.agents
            .iter()
            .filter(|a| a.waiting.is_some())
            .min_by_key(|a| a.waiting.as_ref().map(|w| w.0))
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
                    ..Default::default()
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
                    ..Default::default()
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
        if t.owner.is_some() {
            task.owner.clone_from(&t.owner);
        }
        if !t.blocked.is_empty() {
            task.blocked.clone_from(&t.blocked);
        }
    }

    fn unresolved(&self, sig: &str) -> bool {
        self.errors
            .iter()
            .any(|e| e.sig == sig && e.resolved_t.is_none())
    }

    pub fn verdict(&self) -> Verdict {
        self.verdict_with(3)
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

    pub fn verdict_with(&self, repeats: u8) -> Verdict {
        if self.needs_you.is_some() || self.waiting().is_some() {
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

    /// Drop subagents that finished at or before `cut` (ms); `u64::MAX` drops every finished one.
    pub fn clear_done(&mut self, cut: u64) {
        self.agents
            .retain(|a| a.id == "main" || !a.done || a.ended_t.unwrap_or(a.last_t) > cut);
    }

    pub fn agent_status(&self, a: &Agent, now: u64) -> AgentStatus {
        if a.done {
            AgentStatus::Done
        } else if a.id != "main" && a.session.is_none() && now.saturating_sub(a.last_t) > STALE_MS {
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
            .filter(|a| {
                a.id != "main"
                    && a.session.is_none()
                    && self.agent_status(a, now) == AgentStatus::Running
            })
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
    let Some((restart, bytes)) = crate::paths::new_lines(&dir.join("events.jsonl"), cache.offset)
    else {
        return cache.state;
    };
    if restart {
        cache = Cache {
            v: CACHE_VERSION,
            offset: 0,
            state: State::default(),
        };
    }
    if bytes.is_empty() {
        return cache.state;
    }
    for line in bytes.split(|b| *b == b'\n') {
        if let Ok(ev) = serde_json::from_slice::<Event>(line) {
            cache.state.apply(&ev);
        }
    }
    cache.offset += bytes.len() as u64;
    if let Ok(bytes) = serde_json::to_vec(&cache) {
        let _ = crate::paths::write_atomic(&cache_path, &bytes);
    }
    cache.state
}

#[cfg(test)]
mod tests {
    #[test]
    fn clear_done_keeps_main_running_and_later() {
        let ag = |id: &str, done: bool, end: u64| Agent {
            id: id.into(),
            done,
            ended_t: Some(end),
            ..Default::default()
        };
        let mut st = State {
            agents: vec![
                ag("main", true, 1),
                ag("old", true, 5),
                ag("new", true, 20),
                ag("run", false, 1),
            ],
            ..Default::default()
        };
        st.clear_done(10);
        let ids: Vec<_> = st.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["main", "new", "run"]);
        st.clear_done(u64::MAX);
        let ids: Vec<_> = st.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["main", "run"]);
    }

    #[test]
    fn resume_set_then_cleared_on_stop() {
        let mut st = State::default();
        st.apply(&Event {
            kind: Some("resume".into()),
            dur: Some(10_800_000),
            cold: Some(true),
            usd: Some(0.4),
            ..ev("SessionStart")
        });
        let r = st.resume.clone().unwrap();
        assert_eq!(
            (r.away_ms, r.cold, r.rebuild_usd),
            (Some(10_800_000), Some(true), Some(0.4))
        );
        st.apply(&ev("Stop"));
        assert_eq!(st.resume, None);
        st.apply(&Event {
            kind: Some("startup".into()),
            ..ev("SessionStart")
        });
        assert_eq!(st.resume, None);
    }

    #[test]
    fn permission_mode_is_last_seen() {
        let mut st = State::default();
        let ev = |t, pm: Option<&str>| Event {
            t,
            e: "Stop".into(),
            pm: pm.map(String::from),
            ..Default::default()
        };
        st.apply(&ev(1, Some("plan")));
        st.apply(&ev(2, None));
        assert_eq!(st.permission_mode.as_deref(), Some("plan"));
        st.apply(&ev(3, Some("auto")));
        assert_eq!(st.permission_mode.as_deref(), Some("auto"));
        st.apply(&Event {
            a: Some("sub1".into()),
            ..ev(4, Some("plan"))
        });
        assert_eq!(
            st.permission_mode.as_deref(),
            Some("auto"),
            "a subagent's mode is not the session's"
        );
    }

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

    #[test]
    fn verdict_ids() {
        use Verdict::*;
        let all = [
            (NeedsYou, "needs_you"),
            (Blocked, "blocked"),
            (Stuck, "stuck"),
            (Retrying, "retrying"),
            (OnTrack, "on_track"),
            (Done, "done"),
            (Idle, "idle"),
        ];
        for (v, id) in all {
            assert_eq!(v.id(), id);
        }
        for id in crate::config::NOTIFY_ON {
            assert!(all.iter().any(|(v, _)| v.id() == id), "{id}");
        }
    }

    #[test]
    fn a_passing_script_does_not_fix_a_different_failing_one() {
        use serde_json::json;
        let ev = |t, v: serde_json::Value| crate::event::normalize(&v, t).unwrap();
        let bash = |cmd: &str| json!({ "command": cmd });
        let mut st = State::default();
        st.apply(&ev(
            1,
            json!({"hook_event_name": "UserPromptSubmit", "prompt": "build"}),
        ));
        st.apply(&ev(
            2,
            json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash",
            "tool_input": bash("npm run build"), "error": "Exit code 1\nerror TS2304"}),
        ));
        st.apply(&ev(
            3,
            json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
            "tool_input": bash("npm run lint")}),
        ));
        assert_eq!(
            st.errors[0].resolved_t, None,
            "lint passing doesn't fix the build"
        );
        // Different scripts failing once each is not "the same command failing again".
        for (t, cmd) in [(5, "npm run test"), (6, "npm run lint")] {
            st.apply(&ev(
                t,
                json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash",
                "tool_input": bash(cmd), "error": "Exit code 1\nfailed"}),
            ));
        }
        assert_eq!(st.stuck_reason(3), None);
        st.apply(&ev(7, json!({"hook_event_name": "Stop"})));
        assert_eq!(st.open_errors(), 3);
    }

    fn at(t: u64, e: &str, a: Option<&str>) -> Event {
        Event {
            t,
            e: e.into(),
            a: a.map(String::from),
            ..Default::default()
        }
    }
    fn spawn(t: u64, id: &str, name: Option<&str>, kind: &str) -> Event {
        Event {
            tool: Some("Agent".into()),
            id: Some(id.into()),
            name: name.map(String::from),
            kind: Some(kind.into()),
            msg: Some(format!("do {id}")),
            ..at(t, "PreToolUse", None)
        }
    }
    fn start(t: u64, a: &str, kind: &str) -> Event {
        Event {
            at: Some(kind.into()),
            ..at(t, "SubagentStart", Some(a))
        }
    }
    fn send(t: u64, from: Option<&str>, to: &str, text: &str) -> Event {
        Event {
            tool: Some("SendMessage".into()),
            to: Some(to.into()),
            msg: Some(text.into()),
            ..at(t, "PreToolUse", from)
        }
    }
    fn agent<'a>(s: &'a State, id: &str) -> &'a Agent {
        s.agents.iter().find(|a| a.id == id).unwrap()
    }

    #[test]
    fn spawn_claims_name_parent_and_prompt() {
        let s = run(&[
            spawn(1, "tu1", Some("impl"), "Explore"),
            start(2, "a1", "Explore"),
        ]);
        let a = agent(&s, "a1");
        assert_eq!(a.name.as_deref(), Some("impl"));
        assert_eq!(a.parent.as_deref(), Some("main"));
        assert_eq!(a.prompt.as_deref(), Some("do tu1"));
        assert!(s.spawns.is_empty());
        assert_eq!(a.label(), "impl");
        assert_eq!(s.agents[0].label(), "main");
    }

    #[test]
    fn spawn_matches_by_type_then_oldest() {
        let s = run(&[
            spawn(1, "tu1", Some("p"), "Plan"),
            spawn(2, "tu2", Some("e"), "Explore"),
            start(3, "a1", "Explore"),
            start(4, "a2", "Other"),
        ]);
        assert_eq!(agent(&s, "a1").name.as_deref(), Some("e"));
        assert_eq!(agent(&s, "a2").name.as_deref(), Some("p"));
        // A woken agent starts again under its id and claims nothing.
        let mut s = s;
        s.apply(&spawn(5, "tu3", Some("x"), "Explore"));
        s.apply(&start(6, "a1", "Explore"));
        assert_eq!(agent(&s, "a1").name.as_deref(), Some("e"));
        assert_eq!(s.spawns.len(), 1);
        let unnamed = run(&[spawn(1, "tu1", None, "Explore"), start(2, "a1", "Explore")]);
        assert_eq!(agent(&unnamed, "a1").label(), "Explore");
    }

    #[test]
    fn send_message_resolves_names() {
        let s = run(&[
            spawn(1, "tu1", Some("impl"), "general-purpose"),
            start(2, "a1", "general-purpose"),
            spawn(3, "tu2", Some("tester"), "general-purpose"),
            start(4, "a2", "general-purpose"),
            send(5, Some("a2"), "impl", "2 failing"),
            send(6, Some("a1"), "team-lead", "done"),
            send(7, None, "ghost", "hi"),
        ]);
        let m: Vec<_> = s
            .messages
            .iter()
            .map(|m| (m.from.as_str(), m.to.as_str(), m.text.as_str()))
            .collect();
        assert_eq!(
            m,
            [
                ("a2", "a1", "2 failing"),
                ("a1", "main", "done"),
                ("main", "ghost", "hi")
            ]
        );
        assert_eq!(s.agent_by_name("tester").map(|a| a.id.as_str()), Some("a2"));
    }

    #[test]
    fn recent_calls_record_result_and_cap() {
        let pre = tool("PreToolUse", "cargo test", None);
        let fail = Event {
            dur: Some(300),
            msg: Some("exit 1".into()),
            ..tool("PostToolUseFailure", "cargo test", Some(false))
        };
        let s = run(&[pre.clone(), fail]);
        let r = s.agents[0].recent.back().unwrap();
        assert_eq!(
            (r.ok, r.dur, r.msg.as_deref()),
            (Some(false), Some(300), Some("exit 1"))
        );
        assert_eq!(r.tool, "Bash");
        let s = run(&vec![pre; 45]);
        assert_eq!(s.agents[0].recent.len(), 40);
    }

    #[test]
    fn permission_wait_is_per_agent() {
        let wait = Event {
            tool: Some("Bash".into()),
            act: Some("$ curl -s x".into()),
            ..at(3, "PermissionRequest", Some("a1"))
        };
        let mut s = run(&[
            at(1, "UserPromptSubmit", None),
            start(2, "a1", "general-purpose"),
            start(2, "a2", "general-purpose"),
            wait,
        ]);
        assert_eq!(s.verdict(), Verdict::NeedsYou);
        assert_eq!(s.waiting().map(|a| a.id.as_str()), Some("a1"));
        assert_eq!(agent(&s, "a1").waiting, Some((3, "$ curl -s x".into())));
        s.apply(&Event {
            tool: Some("Read".into()),
            ..at(5, "PreToolUse", Some("a2"))
        });
        assert_eq!(s.verdict(), Verdict::NeedsYou);
        s.apply(&Event {
            tool: Some("Bash".into()),
            ok: Some(true),
            ..at(7, "PostToolUse", Some("a1"))
        });
        assert_eq!(s.verdict(), Verdict::OnTrack);
    }

    #[test]
    fn parallel_calls_match_by_tool_use_id() {
        let call = |e: &str, id: &str, ok: Option<bool>| Event {
            tool: Some("Read".into()),
            id: Some(id.into()),
            act: Some(format!("reading {id}")),
            ok,
            msg: (ok == Some(false)).then(|| format!("{id} failed")),
            ..at(1, e, None)
        };
        let s = run(&[
            call("PreToolUse", "a", None),
            call("PreToolUse", "b", None),
            call("PreToolUse", "c", None),
            call("PostToolUseFailure", "a", Some(false)),
            call("PostToolUse", "b", Some(true)),
        ]);
        let r: Vec<_> = s.agents[0]
            .recent
            .iter()
            .map(|r| (r.act.as_str(), r.ok, r.msg.as_deref()))
            .collect();
        assert_eq!(
            r,
            [
                ("reading a", Some(false), Some("a failed")),
                ("reading b", Some(true), None),
                ("reading c", None, None),
            ]
        );
    }

    #[test]
    fn lanes_bucket_by_12s() {
        let p = |t| Event {
            tool: Some("Read".into()),
            ..at(t, "PreToolUse", None)
        };
        let s = run(&[p(0), p(5_000), p(13_000)]);
        assert_eq!(s.agents[0].lane, [(0, 2), (1, 1)]);
        let s = run(&(0..60).map(|i| p(i * 12_000)).collect::<Vec<_>>());
        assert_eq!(s.agents[0].lane.len(), 50);
    }

    #[test]
    fn files_count_edits_and_lines() {
        let e = Event {
            tool: Some("Edit".into()),
            act: Some("editing src/a.rs".into()),
            ok: Some(true),
            lines: Some((3, 2)),
            ..at(1, "PostToolUse", None)
        };
        let s = run(&[e.clone(), e]);
        assert_eq!(
            s.agents[0].files["src/a.rs"],
            FileRec {
                edits: 2,
                added: 6,
                removed: 4
            }
        );
    }

    #[test]
    fn task_owner_and_blocked_kept() {
        let s = run(&[Event {
            task: Some(TaskRef {
                id: "4".into(),
                owner: Some("impl".into()),
                blocked: vec!["3".into()],
                ..Default::default()
            }),
            ..at(1, "TaskCreated", None)
        }]);
        assert_eq!(s.tasks[0].owner.as_deref(), Some("impl"));
        assert_eq!(s.tasks[0].blocked, ["3"]);
    }

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
    #[test]
    fn background_names_replace() {
        let mut st = State::default();
        let mk = |a: Option<&str>, b: Vec<&str>| Event {
            a: a.map(Into::into),
            bgs: Some(b.iter().map(|s| s.to_string()).collect()),
            crons: Some(vec![]),
            ..ev("Stop")
        };
        st.apply(&mk(None, vec!["shell: a", "shell: b"]));
        assert_eq!(st.background_names, ["shell: a", "shell: b"]);
        st.apply(&mk(None, vec!["shell: c"]));
        assert_eq!(st.background_names, ["shell: c"]);
        st.apply(&mk(Some("sub1"), vec![]));
        assert_eq!(st.background_names, ["shell: c"]);
        st.apply(&mk(None, vec![]));
        assert!(st.background_names.is_empty());
        st.background_names = vec!["shell: d".into()];
        st.crons = vec!["*/5 x".into()];
        st.apply(&ev("SessionEnd"));
        assert!(st.background_names.is_empty() && st.crons.is_empty());
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
                ..Default::default()
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
        assert_eq!(s.verdict(), Verdict::Stuck);
        assert_eq!(
            s.stuck_reason(3).as_deref(),
            Some("same call 3× · reading a.rs")
        );
        assert_eq!(s.verdict_with(4), Verdict::OnTrack);
        let edit = call("main", "Edit", 9, "editing a.rs");
        let s = run(&[prompt("go"), r.clone(), edit, r.clone(), r.clone()]);
        assert_eq!(
            s.verdict(),
            Verdict::OnTrack,
            "an edit between repeats is progress"
        );
        let t = call("main", "TaskUpdate", 1, "updating plan");
        assert_eq!(
            run(&[prompt("go"), t.clone(), t.clone(), t]).verdict(),
            Verdict::OnTrack
        );
        let par: Vec<Event> = ["a1", "a2", "a3"]
            .iter()
            .map(|a| call(a, "Read", 7, "reading a.rs"))
            .collect();
        let mut evs = vec![prompt("go")];
        evs.extend(par);
        assert_eq!(
            run(&evs).verdict(),
            Verdict::OnTrack,
            "parallel agents don't add up"
        );
        let mut s = run(&[prompt("go"), r.clone(), r.clone(), r]);
        s.apply(&stop(0));
        assert_ne!(s.verdict(), Verdict::Stuck);
    }

    #[test]
    fn stuck_on_repeated_failure() {
        let fail = Event {
            sig: Some("Bash:cargo build".into()),
            ..tool("PostToolUseFailure", "Bash:cargo build", Some(false))
        };
        let mut s = run(&[prompt("go"), fail.clone(), fail.clone()]);
        assert_eq!(s.verdict(), Verdict::Retrying);
        s.apply(&fail);
        assert_eq!(s.verdict(), Verdict::Stuck);
        assert_eq!(
            s.stuck_reason(3).as_deref(),
            Some("cargo build failed 3× in a row")
        );
        s.apply(&tool("PostToolUse", "Bash:cargo build", Some(true)));
        assert_eq!(s.verdict(), Verdict::OnTrack);
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
        assert_eq!(s.verdict(), Verdict::NeedsYou);
        s.apply(&tool("PreToolUse", "Bash:ls", None));
        assert_eq!(s.verdict(), Verdict::OnTrack);
        let other = Event {
            kind: Some("auth_success".into()),
            ..ev("Notification")
        };
        s.apply(&other);
        assert_eq!(s.verdict(), Verdict::OnTrack);
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
    fn subagent_stop_failure_ends_that_agent_only() {
        let sf = Event {
            a: Some("aafffe06".into()),
            kind: Some("authentication_failed".into()),
            msg: Some("Login reached its time limit".into()),
            ..ev("StopFailure")
        };
        let s = run(&[prompt("go"), sf]);
        let a = s.agents.iter().find(|a| a.id == "aafffe06").unwrap();
        assert!(a.done && a.ended_t.is_some());
        assert_eq!(s.running_agents(1_000), 0);
        assert!(s.turn_active && !s.turn_failed);
        assert_ne!(s.verdict(), Verdict::Blocked);
        assert_eq!(s.errors[0].agent, "aafffe06");
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
        assert_eq!(s.verdict(), Verdict::Blocked);
        assert_eq!(s.open_errors(), 1);
        s.apply(&prompt("again"));
        assert_eq!(s.verdict(), Verdict::OnTrack);
        s.apply(&stop(0));
        assert!(s.errors[0].resolved_t.is_some());
    }

    #[test]
    fn retry_then_fix() {
        let fail = tool("PostToolUseFailure", "Bash:cargo build", Some(false));
        let mut s = run(&[prompt("go"), fail.clone(), fail]);
        assert_eq!(s.verdict(), Verdict::Retrying);
        assert_eq!((s.errors.len(), s.errors[0].retries), (1, 1));
        s.apply(&tool("PostToolUse", "Bash:cargo build", Some(true)));
        assert_eq!(s.verdict(), Verdict::OnTrack);
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
        assert_eq!(s.verdict(), Verdict::OnTrack);
        assert_eq!(s.open_errors(), 1);
        s.apply(&stop(0));
        assert_eq!(s.verdict(), Verdict::Idle);
    }

    #[test]
    fn ending_on_a_failure_blocks() {
        let s = run(&[
            prompt("go"),
            tool("PostToolUseFailure", "Bash:make", Some(false)),
            stop(0),
        ]);
        assert_eq!(s.verdict(), Verdict::Blocked);
    }

    #[test]
    fn interrupts_are_not_errors() {
        let mut f = tool("PostToolUseFailure", "Bash:sleep", Some(false));
        f.kind = Some("interrupt".into());
        let s = run(&[prompt("go"), f]);
        assert!(s.errors.is_empty());
        assert_eq!(s.verdict(), Verdict::OnTrack);
    }

    #[test]
    fn background_agents_keep_turn_active() {
        let s = run(&[prompt("go"), task("1", Some("a"), None), stop(1)]);
        assert_eq!(s.verdict(), Verdict::OnTrack);
    }

    #[test]
    fn done_vs_idle() {
        let s = run(&[
            prompt("go"),
            task("1", Some("a"), Some("completed")),
            stop(0),
        ]);
        assert_eq!(s.verdict(), Verdict::Done);
        let s = run(&[
            prompt("go"),
            task("1", Some("a"), Some("completed")),
            task("2", Some("b"), None),
            stop(0),
        ]);
        assert_eq!(s.verdict(), Verdict::Idle);
        let s = run(&[prompt("go"), stop(0)]);
        assert_eq!(s.verdict(), Verdict::Idle);
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
        assert_eq!(s.verdict(), Verdict::Done);
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
    fn turn_log_and_compactions() {
        let mut s = State::default();
        let at = |t, e: &str, msg: Option<&str>| Event {
            t,
            e: e.into(),
            msg: msg.map(Into::into),
            ..Default::default()
        };
        s.apply(&at(10, "UserPromptSubmit", Some("fix it")));
        s.apply(&at(11, "UserPromptSubmit", Some("<task-notification>x")));
        s.apply(&tool("PostToolUse", "Bash:a", Some(true)));
        s.apply(&tool("PostToolUseFailure", "Bash:b", Some(false)));
        s.apply(&at(20, "Stop", None));
        s.apply(&tool("PostToolUse", "Bash:c", Some(true)));
        s.apply(&at(30, "UserPromptSubmit", Some("again")));
        s.apply(&at(40, "StopFailure", None));
        let r: Vec<_> = s
            .turn_log
            .iter()
            .map(|r| (r.start, r.end, r.calls, r.failed, r.ok))
            .collect();
        assert_eq!(r, [(10, Some(20), 2, 1, true), (30, Some(40), 0, 0, false)]);
        assert_eq!(s.turn_log[0].prompt, "fix it");
        for i in 0..60 {
            s.apply(&at(100 + i, "UserPromptSubmit", Some("p")));
            s.apply(&Event {
                kind: Some("auto".into()),
                ..at(100 + i, "PreCompact", None)
            });
        }
        assert_eq!((s.turn_log.len(), s.compactions.len()), (50, 20));
        assert_eq!(s.compactions[0].1, "auto");
    }

    #[test]
    fn next_prompt_and_session_end_close_open_turn() {
        let at = |t, e: &str, msg: Option<&str>| Event {
            t,
            e: e.into(),
            msg: msg.map(Into::into),
            ..Default::default()
        };
        let mut s = State::default();
        s.apply(&at(10, "UserPromptSubmit", Some("a")));
        s.apply(&at(11, "UserPromptSubmit", Some("<task-notification>x")));
        assert_eq!(s.turn_log[0].end, None, "synthetic prompt keeps it open");
        s.apply(&at(
            12,
            "UserPromptSubmit",
            Some("<agent-message from=\"w\">hi"),
        ));
        assert_eq!(s.turn_log[0].end, None, "agent message keeps it open");
        assert_eq!((s.turn_log.len(), s.turns, s.prompts.len()), (1, 1, 1));
        s.apply(&at(30, "UserPromptSubmit", Some("b")));
        assert_eq!((s.turn_log[0].end, s.turn_log[0].ok), (Some(30), true));
        assert_eq!(s.turn_log[1].end, None);
        s.apply(&at(50, "SessionEnd", None));
        assert_eq!(s.turn_log[1].end, Some(50));
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
                ..Default::default()
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

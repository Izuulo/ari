# M1 Statusline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `ari statusline` prints ARI's compact row beside the user's existing statusline; `ari setup`/`uninstall` wire it in and out.

**Architecture:** `config` (TOML, defaults on any error) → `state` (pure reducer + verdict + incremental cache) → `usage` (statusline payload view) → `bar` (pure render) → `chain` (run prior statuslines) → `statusline` (glue). `setup` edits `settings.json` preserving key order.

**Tech Stack:** Rust, serde_json (`preserve_order`), toml, insta.

**Spec:** `docs/superpowers/specs/2026-11-05-m1-statusline-design.md`

## Global Constraints

- Statusline render p95 < 50 ms excluding chained commands; chained command timeout 300 ms default.
- A failing/slow chained command never breaks ARI's row; ARI never alters chained output bytes (one trailing newline stripped).
- Missing `rate_limits` → `—`, never `0%`.
- Never color alone; `NO_COLOR` disables ANSI.
- Setup always backs up `settings.json`, shows a diff, and asks unless `--yes`.

## Review Focus

- Malformed or empty statusline stdin → chained commands still run, ARI prints a minimal `ARI ◌ IDLE` row. Test in Task 7.
- `events.jsonl` with a half-written last line → that line is not consumed until complete. Test in Task 3.
- Settings file with comments-free but unusual content (no `statusLine`, extra keys, nested objects) → only `statusLine` changes, order kept. Test in Task 8.
- Chained command that spawns a long-lived grandchild → ARI still returns on time. Test in Task 6.
- Session dir missing (ARI installed mid-session, no hooks fired yet) → bar renders from usage alone. Test in Task 7.

---

### Task 1: Config

**Files:** Create `src/config.rs`; Modify `Cargo.toml`, `src/lib.rs`, `src/hook.rs`.

**Produces:** `pub struct Config { position: Position, hide: Vec<String>, face, bell, ascii: bool, max_width: usize, retention_days: u64, chain: Chain { commands: Vec<String>, timeout_ms: u64 }, thresholds: Thresholds { ctx_warn, ctx_crit, stuck_repeats: u8 }, prices: BTreeMap<String, Price { input, output, cache_read, cache_write: f64 }> }` with PRD defaults; `pub fn load() -> Config` (missing/invalid → default); `pub fn path() -> Option<PathBuf>`; `pub fn save(&Config) -> io::Result<()>`.

- [ ] Tests: empty string → defaults (`position = Top`, `retention_days = 7`, `timeout_ms = 300`, `ctx_warn = 60`, `ctx_crit = 80`); partial TOML overrides only named keys; garbage → defaults; round-trip save/parse.
- [ ] Hook prune uses `config::load().retention_days` (SessionStart only). Commit.

### Task 2: Order-preserving JSON with stable hash

**Files:** Modify `Cargo.toml` (`serde_json` feature `preserve_order`), `src/event.rs`.

- [ ] `fnv1a` input becomes `canonical(&Value) -> Vec<u8>` (keys sorted recursively) so `same_input_same_hash` keeps passing with `preserve_order`. Update snapshots (`h` values change once). Commit.

### Task 3: Reducer

**Files:** Create `src/state.rs`.

**Produces:**
- `pub struct State { turn_active: bool, turn_failed: bool, turns: u32, label: Option<String>, needs_you: Option<String>, tasks: Vec<Task>, agents: Vec<Agent>, errors: Vec<ErrorRec>, recent: VecDeque<(bool, String)>, last_main_fail: Option<String>, compacted_at: Option<u64>, last_t: u64, ended: bool }` (`Serialize + Deserialize + Default`)
- `pub struct Task { id, subject: String, status: String }`; `pub struct Agent { id, kind: String, started, last_t: u64, calls: u32, act: Option<String>, done: bool, result: Option<String>, transcript: Option<String>, ended_t: Option<u64> }`; `pub struct ErrorRec { sig, tool, msg: String, kind: Option<String>, agent: String, first_t, last_t: u64, retries: u32, resolved_t: Option<u64>, resolved_by: Option<String>, turn: u32 }`
- `impl State { pub fn apply(&mut self, ev: &Event); pub fn verdict(&self, now: u64) -> Verdict; pub fn tasks_done(&self) -> (usize, usize); pub fn now_task(&self) -> Option<&Task>; pub fn running_agents(&self, now: u64) -> usize; pub fn open_errors(&self) -> usize; pub fn agent_status(&self, a: &Agent, now: u64) -> AgentStatus }`
- `pub enum Verdict { NeedsYou, Blocked, Stuck, Retrying, OnTrack, Done, Idle }` with `label() -> &str` (`"NEEDS YOU"` …) and `glyph() -> char` (PRD §7.1 table). `Stuck` is never produced in M1.
- `pub enum AgentStatus { Running, Done, Stale }`; `pub const STALE_MS: u64 = 300_000`.
- `pub fn load_session(dir: &Path) -> State` — incremental via `state.json` `{v: 1, offset, state}`; reads only complete lines; saves temp + rename; unknown `v` → rebuild.

Rules exactly as the spec's "State tracks" and "Verdict" sections. `NEEDS YOU` kinds: `permission_prompt`, `idle_prompt`, `agent_needs_input`, `elicitation_dialog`, `elicitation_url_dialog`.

- [ ] Tests (each builds events with a small helper `ev(e, f)`): permission notification → `NeedsYou`, later PreToolUse → `OnTrack`; StopFailure → `Blocked`; failure then same-sig success → resolved, `retries` counts repeated failures, verdict `OnTrack`; failure followed by 10 unrelated successes → `OnTrack` (error goes cold but stays unresolved); Stop right after an unresolved main failure → `Blocked`; Stop `bg: 1` → still `OnTrack`; Stop `bg: 0` with all tasks completed → `Done`, with one pending → `Idle`; interrupt failure → no error; `<task-notification>` prompt → `turns` unchanged, label unchanged; TaskUpdate `deleted` removes; TodoWrite replaces list; subagent tool event without SubagentStart creates the agent; running agent with `last_t` 6 min ago → `Stale`.
- [ ] Test `load_session`: write 3 lines + half a 4th → state reflects 3; append the rest → 4; offset persisted; corrupt `state.json` → rebuild.
- [ ] Fixture test: verdict after each event of `fixtures/basic` → insta snapshot (`t-offset verdict`). Commit per group (tasks/agents, errors, verdict, cache).

### Task 4: Usage view + atomic write

**Files:** Create `src/usage.rs`.

**Produces:** `pub struct Usage { session_id, transcript_path, model: Option<String>, ctx_pct: Option<f64>, ctx_size: Option<u64>, ctx_used: Option<u64>, five_h: Option<Limit>, seven_d: Option<Limit>, cost_usd, duration_ms, api_duration_ms, lines_added, lines_removed: Option<…> }`, `pub struct Limit { pct: f64, resets_at: u64 }`; `pub fn from_value(&Value) -> Usage`; `pub fn write_atomic(path, bytes) -> io::Result<()>` (temp file in same dir + rename).

- [ ] Tests: docs' full example payload → all fields; payload without `rate_limits` → `five_h == None`; `current_usage: null` OK. Commit.

### Task 5: Compact bar

**Files:** Create `src/bar.rs`.

**Produces:** `pub fn render(st: &State, u: &Usage, cfg: &Config, now_ms: u64, width: usize, color: bool) -> String`; `pub fn visible_width(s: &str) -> usize` (ignores ANSI CSI sequences).

Segment ids for `hide`: `tasks agents errors ctx 5h 7d cost tokens` (verdict can't be hidden). Separator ` │ `. Countdown format `↺1h40m`, `↺4d`, `↺12m`.

- [ ] Tests (color off): the spec's example state renders `ARI ● ON TRACK │ ███████░░░ 4/7 │ ⠋ 2 agents │ ✗ 1 │ ctx ▰▰▰▱▱ 71% │ 5h 62% ↺1h40m │ 7d 34% ↺4d │ $2.69` (fixed `now` for spinner frame); no tasks → `no plan yet`; no `rate_limits` → `5h —`; width 30 → only verdict (+ whatever fits), never wider than 30; `hide = ["cost","7d"]` drops them; color on → contains `\x1b[`, and `visible_width` equals the no-color width. Commit.

### Task 6: Chain runner

**Files:** Create `src/chain.rs`.

**Produces:** `pub fn run_all(cmds: &[String], stdin: &[u8], timeout: Duration) -> Vec<String>` — outputs in config order, failures/timeouts omitted, `ari statusline` commands skipped.

Spawn all via `sh -c` with piped stdin/stdout, stderr null; write stdin from a thread; read stdout on a thread into a channel; poll `try_wait` until the shared deadline; kill on timeout and don't wait for the reader (a grandchild may hold the pipe).

- [ ] Tests: `printf 'a\nb'` → `"a\nb"`; `exit 1` → omitted; `sleep 2` with 300 ms → omitted and `run_all` returns in < 600 ms; `cat` echoes stdin; `(sleep 5 &) ; echo hi` → `"hi"` within budget; recursion guard. Commit.

### Task 7: `ari statusline`

**Files:** Create `src/statusline.rs`; Modify `src/main.rs`; Test `tests/statusline.rs`, `scripts/bench-statusline.sh`.

Flow per spec. Session dir from payload `session_id` (validated). If parse fails: run chain, print `ARI ◌ IDLE`. `position = bottom` puts ARI last.

- [ ] Integration tests with temp `XDG_STATE_HOME`/`XDG_CONFIG_HOME`: config chaining a ponytail-style badge script (`printf '\033[38;5;108m[PONYTAIL]\033[0m'`), a two-line script, a slow script and a failing one → stdout is exactly `<ARI row>\n\x1b[38;5;108m[PONYTAIL]\x1b[0m\nline1\nline2`; `usage.json` written; garbage stdin → still the badge plus an IDLE row; no session dir → renders.
- [ ] Bench script: 200 renders against the fixture session, p95 < 50 ms. Commit.

### Task 8: Setup / uninstall

**Files:** Create `src/setup.rs`, `commands/setup.md`; Modify `src/main.rs`; Test `tests/setup.rs`.

**Produces:** `pub fn settings_path() -> Option<PathBuf>`; `pub fn plan(settings: &Value, cfg: &Config) -> Option<(Value, Config, String /*diff*/)>` (None = already installed); `pub fn run_setup(args)`, `pub fn run_uninstall(args)`.

- [ ] Tests (temp `CLAUDE_CONFIG_DIR` + `XDG_CONFIG_HOME`): settings with an existing `statusLine {type, command, padding: 2}` → after `setup --yes`: command `ari statusline`, `padding` kept, `refreshInterval: 2`, other top-level keys unchanged and in the same order, backup file exists, config chain holds the old command, `hide` pre-filled; second setup → "already set up", no change; `uninstall --yes` → file parses equal to the original; no `statusLine` originally → uninstall removes the key; `--dry-run` writes nothing; missing settings file → created with only `statusLine`.
- [ ] `commands/setup.md` per spec. Commit.

### Task 9: README + manual ponytail check

- [ ] README: tagline, three-line install (`cargo install --git https://github.com/Izuulo/ari ari-cli` until releases exist; `/plugin marketplace add Izuulo/ari` + `/plugin install ari@ari`; `ari setup`), compact bar example, config reference, FAQ. Commit.
- [ ] Manual: `bash ~/.claude/ponytail-statusline.sh` output vs through `ari statusline` with it chained → byte-identical lines. Record in PR.

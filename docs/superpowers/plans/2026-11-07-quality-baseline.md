# Quality Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a session behaves worse than the user's own usual, the panel says so in numbers: `⚠ reads/edit 2.0 · your usual 6.1`, `⚠ errors/prompt 3.5 · your usual 1.2`, `⚠ retries/prompt 2.0 · your usual 0.4`.

**Architecture:** The reducer (`State::apply`) gains four counters (reads, edits, failed calls, retries). At the main agent's `Stop` and at `SessionEnd`, `ari hook` folds the session (`state::load_session`, incremental via `state.json`) and appends one small summary line (counts only, no content) to `<state root>/quality.jsonl`. At `SessionStart`, next to the existing `prune`, the hook compacts that file (last line per session, drop older than `retention_days`, keep the newest 200). `ari watch` loads the history when it picks a session, compares the live session with the median of the user's past sessions on the same model, and passes deviations to the panel's `session` section through `bar::Extras`.

**Tech Stack:** Rust 2024, serde/serde_json, ratatui 0.30, insta. No new crates.

**Spec:** `docs/ROADMAP.md` §7, item 7 ("Claude feels dumber today").

## Decisions this plan makes (record them in `docs/DECISIONS.md`, Task 5)

- **What is counted, all agents together.** reads = `Read`, `Grep`, `Glob` results (any outcome); edits = successful `Edit`, `Write`, `MultiEdit`, `NotebookEdit`; failed = every failed tool result that opens or retries an error (the same calls the errors section counts; interrupts excluded); retries = repeats of a still-unresolved error (`ErrorRec.retries` increments). Counting subagents too keeps the ratio stable when Claude delegates reading to Explore agents; counting only `main` would read "fewer reads" whenever it delegates.
- **"Per task" = per real prompt** (`State.turns`). The task tools are off by default, so prompts are the only unit every session has.
- **Baseline key: the model, global across repos.** The complaint is "the model got worse", so a switch from Opus to Haiku must not look like a regression. Per repo would need 5 qualifying sessions in *each* repo before anything shows, and read/edit habits differ less between repos than between models. Model = `usage::load(dir).model` (what the panel badge shows), else the app name (`claude`, `codex`, `dsh`).
- **Baseline = median** of each rate over the newest `BASELINE_SESSIONS` (30) past sessions with the same model and at least `MIN_EDITS` (5) edits; it counts once there are `MIN_SESSIONS` (5) of them. The live session is compared once it has `MIN_EDITS` edits too. Median, not pooled sums, so one giant session doesn't set the usual.
- **Only worse is shown.** reads/edit ≤ usual × `READS_LOW` (0.6); errors or retries per prompt ≥ usual × `RATE_HIGH` (1.5) **and** ≥ usual + `RATE_FLOOR` (1.0). The floor stops `0.1 → 0.2` from firing. The postmortem drop (6.6 → 2.0) is 0.3×, well past 0.6.
- **Where it shows: the panel's `session` section only.** Not the statusline row (no room, and it is a slow signal), not `ari recap` (a recap covers a window since a prompt or time; this is a whole-session comparison), not the hub zoom (zoom is per agent; these counts are per session). `hide_panel = ["session"]` hides it with the section.
- **Retention:** `quality.jsonl` follows `retention_days` like the session logs, so the default baseline is "your last 7 days". It holds counts, a session id, a time and a model name: no prompts, paths or output.

## Global Constraints

- ARI never sends anything to the model, never blocks or answers for the user, nothing leaves the machine. No network.
- `ari hook` always exits 0 and prints nothing; a failure to write or read `quality.jsonl` is silently ignored.
- Each `quality.jsonl` line is written with one `write_all` on an `O_APPEND` file and stays well under 4 KB.
- Default config and no history file = byte-identical panel output; existing snapshots must not change.
- No new crates, no new config keys.
- CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Snapshot updates: `INSTA_UPDATE=always cargo test`, then review every `.snap` diff.
- One commit per task step that says commit, conventional style with scope (`feat(state): …`, `docs(changelog): …`), ending with the attribution line. Work on branch `quality-baseline`.

## Review Focus

1. **The live session counted in its own baseline** (its earlier `Stop` lines are in the file): the current session id is always excluded. Test in Task 3.
2. **Q&A sessions with no edits, and a usual of 0 errors**: no `inf`/`NaN`, no row from 1 error against a usual of 0.0. Tests in Task 3.
3. **A broken or half-written `quality.jsonl` line** (crash mid-write, hand edit): skipped, the rest still loads, and compaction rewrites a clean file. Test in Task 2.
4. **`retention_days` lowered, or the file growing inside one very long session**: old lines go at the next `SessionStart`; past `MAX_BYTES` the file is compacted on append. Tests in Task 2.
5. **Hook latency at `Stop`** (README promises about 3 ms p95 for hooks): `load_session` is incremental, but a session with a stale `state.json` folds its whole log. Measured in Task 4, Step 5.

---

### Task 1: Quality counters in the reducer

**Files:**
- Modify: `src/state.rs` (`State`, `CACHE_VERSION`, `apply`'s `"PostToolUse" | "PostToolUseFailure"` arm, `tool_result`)

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq)] #[serde(default)] pub struct Quality { pub reads: u32, pub edits: u32, pub failed: u32, pub retries: u32 }`
  - `State.quality: Quality`
  - `const READS: [&str; 3] = ["Read", "Grep", "Glob"];`
  - `CACHE_VERSION` +1 from whatever `main` has (sibling §7 plans bump it too; cached states without the counters are rebuilt from the log).

- [ ] **Step 1: Write the failing test** `quality_counts_reads_edits_failures_retries` in `state.rs` tests, using the existing `ev`/`tool`/`run`/`prompt` helpers. Events: `prompt("go")`; `PostToolUse` with `tool` `Read` (sig `Read:/a`, ok), `Grep` (ok), `Glob` (ok) and a subagent `Read` (`a: Some("sub1")`, sig `Read:/b`, ok); `PostToolUse` `Edit` sig `Edit:/a` ok `Some(true)`; `PostToolUseFailure` `Edit` sig `Edit:/c` ok `Some(false)`; `tool("PostToolUseFailure", "cargo test", Some(false))` twice; one failure with `kind: Some("interrupt")`.
  ```rust
  assert_eq!(s.quality, Quality { reads: 4, edits: 1, failed: 3, retries: 1 });
  ```
- [ ] **Step 2: Run** `cargo test state::tests::quality_counts` — expect FAIL (no `quality` field).
- [ ] **Step 3: Implement.** In the Post arm, before `self.track_reads(ev, aid)`: `reads += 1` when `ev.tool` is in `READS`; `edits += 1` when it is in `FILE_EDITS` and `ev.ok == Some(true)`. In `tool_result`, after the interrupt early return and the `ok` early return: `failed += 1`; in the existing-error branch (where `e.retries += 1`) also `self.quality.retries += 1`. Bump `CACHE_VERSION` by one.
- [ ] **Step 4: Run** `cargo test` — PASS, no snapshot changes.
- [ ] **Step 5: Commit** `feat(state): count reads, edits, failed calls and retries per session`.

---

### Task 2: The summary history file

**Files:**
- Create: `src/quality.rs` (add `pub mod quality;` to `src/lib.rs`)
- Modify: `src/hook.rs` (make `append` `pub(crate)`)

**Interfaces:**
- Consumes: `State.quality`, `State.turns` (Task 1); `paths::write_atomic`; `hook::append`.
- Produces:
  - `pub const FILE: &str = "quality.jsonl";` (under `paths::state_root()`)
  - `pub const MAX_SESSIONS: usize = 200;` `pub const MAX_BYTES: u64 = 256 * 1024;`
  - `#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)] #[serde(default)] pub struct Summary { pub id: String, pub t: u64, pub model: String, pub turns: u32, pub reads: u32, pub edits: u32, pub failed: u32, pub retries: u32 }`
  - `pub fn Summary::of(id: &str, t: u64, model: &str, st: &State) -> Summary`
  - `pub fn record(root: &Path, s: &Summary)`: appends `serde_json` line + `\n` via `hook::append`; when the file is then larger than `MAX_BYTES`, `compact(root, u64::MAX, s.t)`. Errors ignored.
  - `pub fn load(root: &Path) -> Vec<Summary>`: unparsable lines skipped; the last line per `id` wins; sorted by `t`, oldest first; missing file → empty.
  - `pub fn compact(root: &Path, max_age_ms: u64, now: u64)`: `load`, drop `now - t > max_age_ms`, keep the newest `MAX_SESSIONS`, rewrite with `paths::write_atomic` (one line each). Missing file → nothing written. Comment: `// shortcut: a Stop line appended during the rewrite is lost; the session's next Stop writes it again.`

- [ ] **Step 1: Write the failing tests** in `quality.rs` (temp dir per test, as in `sessions.rs` tests):
  - `summary_of_state`: a `State` with `turns: 3` and `quality { reads: 12, edits: 4, failed: 2, retries: 1 }` → `Summary::of("s1", 9, "Opus", &st)` has those numbers, `id == "s1"`, `t == 9`, `model == "Opus"`.
  - `load_keeps_last_line_per_session_and_skips_garbage`: file with `s1 (t=1, reads 1)`, `not json`, `{"id":"s2","t":2}`, half a line `{"id":"s1","t":`, `s1 (t=3, reads 7)` → `load` returns ids `["s2", "s1"]` and `s1.reads == 7`.
  - `compact_drops_old_and_caps_count`: 205 sessions with `t = 1..=205`; `compact(root, u64::MAX, 205)` → 200 lines, oldest `t == 6`, newest `t == 205`; then `compact(root, 100, 300)` → only `t >= 200` remain (6 lines). Every line of the rewritten file parses.
  - `record_appends_and_compacts_past_max_bytes`: write a file just under `MAX_BYTES` of repeated lines for one id `"big"`, `record` another summary → file now has 2 lines (`big` deduped, new one).
  - `missing_file_is_empty_and_not_created`: `load` on an empty dir → empty; `compact` → no file created.
- [ ] **Step 2: Run** `cargo test quality::` — FAIL.
- [ ] **Step 3: Implement** the above in `src/quality.rs`.
- [ ] **Step 4: Run** `cargo test quality::` — PASS.
- [ ] **Step 5: Commit** `feat(quality): per-session summary history with retention and a size cap`.

---

### Task 3: Baseline and deviations

**Files:**
- Modify: `src/quality.rs`

**Interfaces:**
- Consumes: `Summary` (Task 2).
- Produces:
  - `pub const MIN_SESSIONS: usize = 5; pub const MIN_EDITS: u32 = 5; pub const BASELINE_SESSIONS: usize = 30;`
  - `pub const READS_LOW: f64 = 0.6; pub const RATE_HIGH: f64 = 1.5; pub const RATE_FLOOR: f64 = 1.0;`
  - `#[derive(Clone, Debug, PartialEq)] pub struct Deviation { pub what: &'static str, pub now: f64, pub usual: f64 }` with `what` one of `"reads/edit"`, `"errors/prompt"`, `"retries/prompt"`.
  - `pub fn Deviation::text(&self) -> String` → `format!("{} {:.1} · your usual {:.1}", what, now, usual)`.
  - `pub fn check(cur: &Summary, history: &[Summary]) -> Vec<Deviation>`: past = history entries with `id != cur.id`, `model == cur.model`, `edits >= MIN_EDITS`, newest `BASELINE_SESSIONS`. Empty when `past.len() < MIN_SESSIONS` or `cur.edits < MIN_EDITS`. Rates: reads/edits; failed/turns.max(1); retries/turns.max(1). Usual = median (mean of the two middle values for an even count). Order: reads, errors, retries.

- [ ] **Step 1: Write the failing tests** (helper `fn s(id: &str, model: &str, turns, reads, edits, failed, retries) -> Summary`):
  - `low_reads_per_edit_shows`: 5 past `Opus` sessions with reads/edit 6, 6, 6.2, 6.1, 6 (edits 10) and cur reads 20 / edits 10 → one deviation, `what == "reads/edit"`, `now == 2.0`, `usual == 6.0`, `text() == "reads/edit 2.0 · your usual 6.0"`.
  - `needs_min_sessions_and_min_edits`: 4 past → empty; 5 past but cur `edits == 4` → empty; a past session with `edits == 4` doesn't count toward the 5.
  - `excludes_itself_and_other_models`: 5 past `Opus` with the same id as `cur` → empty; 5 past `Haiku` → empty.
  - `zero_usual_errors_needs_floor` (every summary here has reads 60, edits 10, so reads/edit never deviates): past failed 0 / turns 5; cur failed 2 / turns 4 (0.5/prompt) → empty; cur failed 6 / turns 4 (1.5/prompt) → `errors/prompt` deviation with `usual == 0.0`.
  - `better_than_usual_is_silent`: cur with more reads and fewer errors than usual → empty.
  - `median_not_mean`: past reads/edit 1, 6, 6, 6, 60 → `usual == 6.0`.
- [ ] **Step 2: Run** `cargo test quality::` — FAIL.
- [ ] **Step 3: Implement** the constants, `Deviation`, `check`.
- [ ] **Step 4: Run** `cargo test quality::` — PASS.
- [ ] **Step 5: Commit** `feat(quality): compare a session with your own baseline`.

---

### Task 4: The hook writes summaries and compacts the file

**Files:**
- Modify: `src/hook.rs` (`handle`; top-of-file doc comment)
- Modify: `src/quality.rs` (one entry point for the hook)
- Test: `tests/hook.rs`

**Interfaces:**
- Consumes: `quality::{record, compact, Summary}` (Task 2), `state::load_session`, `usage::load`, `App::as_str`.
- Produces: `pub fn quality::on_stop(root: &Path, dir: &Path, id: &str, t: u64, app: App)`: `load_session(dir)`; returns when `st.turns == 0`; model = `usage::load(dir).model` else `app.as_str()`; `record(root, &Summary::of(id, t, &model, &st))`.

- [ ] **Step 1: Write the failing integration test** `stop_records_a_quality_summary` in `tests/hook.rs`, with the existing `hook`/`tmp` helpers and payloads shaped like `pre_tool`: for session `s1` send `UserPromptSubmit` (prompt `"fix it"`), a `PostToolUse` `Read` (`tool_input.file_path`), a `PostToolUse` `Edit` (`file_path`, `old_string`, `new_string`), then `Stop`. Assert every call is silent, `ari/quality.jsonl` has exactly 1 line, and it parses with `"id":"s1"`, `"turns":1`, `"reads":1`, `"edits":1`, `"model":"claude"`. Then a `Stop` carrying `agent_id` (a subagent's) → still 1 line. Then `SessionEnd` for `s1` → 2 lines.
- [ ] **Step 2: Write the failing test** `session_start_compacts_quality_history`: pre-write `ari/quality.jsonl` with a line `t = 1` (ancient) and two lines for id `s9` (recent `t`); send `SessionStart` for `s2` → the file has one line, for `s9`.
- [ ] **Step 3: Run** `cargo test --test hook` — FAIL.
- [ ] **Step 4: Implement.** In `handle`, after the event is appended: `if (ev.e == "Stop" && ev.a.is_none()) || ev.e == "SessionEnd" { quality::on_stop(&root, &dir, sid, t, app) }`. In the existing `SessionStart` branch, after `prune`: `quality::compact(&root, days * 86_400_000, t)`. Update the doc comment: "do nothing heavy, except folding the session once per turn at Stop".
- [ ] **Step 5: Measure the Stop hook.** Copy the largest dir under `~/.local/state/ari/sessions/` to a temp `XDG_STATE_HOME`, delete its `state.json` (worst case), and time 20 runs of `echo '{"session_id":"<id>","hook_event_name":"Stop","cwd":"/p"}' | ari hook`. Record p95 in the PR. Above 50 ms worst case: record on `SessionEnd` only and say so in the PR. If the usual case (fresh `state.json`) is above the FAQ's "about 3 ms (p95)", the FAQ line gets "(Stop, once per turn: about N ms)" in Task 5's docs step.
- [ ] **Step 6: Run** `cargo test` — PASS.
- [ ] **Step 7: Commit** `feat(hook): record a quality summary at Stop and SessionEnd`.

---

### Task 5: Show deviations in the panel, then docs and version

**Files:**
- Modify: `src/bar.rs` (`Extras`), `src/watch.rs` (`App`, `refresh`, `app_on` test helper), `src/panel.rs` (`session`)
- Modify: `README.md`, `CHANGELOG.md`, `docs/DECISIONS.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

**Interfaces:**
- Consumes: `quality::{load, check, Summary, Deviation}` (Tasks 2–3).
- Produces:
  - `Extras.quality: Vec<crate::quality::Deviation>` (panel only; default empty).
  - `App.history: Vec<quality::Summary>`: `quality::load(&self.root)` in `refresh` when `dir != self.dir` (a new session is picked), not per frame.
  - In `refresh`'s `if changed` block: `self.x.quality = quality::check(&Summary::of(<dir file name>, hook::now_ms(), &model, &self.st), &self.history)`, `model` = `self.usage.model` else `self.session_app.as_str()` (same rule as the hook).

- [ ] **Step 1: Write the failing panel test** `quality_rows_only_when_deviating` in `panel.rs` tests: `lines(...)` with `Extras { quality: vec![Deviation { what: "reads/edit", now: 2.0, usual: 6.1 }], .. }` at 80 wide → the `session` section contains `⚠ reads/edit 2.0 · your usual 6.1`; with `Extras::default()` → no `your usual`; with `hide_panel = ["session"]` → no `your usual`.
- [ ] **Step 2: Write the failing watch test** `quality_deviation_reaches_extras`: write `<root>/quality.jsonl` with 5 summaries (`model "claude"`, turns 2, reads 60, edits 10, failed 0, retries 0); `app_on(root, "cur", events)` where events are one prompt, 6 successful `Edit` results and 2 `Read` results; `app.refresh()` → `app.x.quality[0].what == "reads/edit"`. Add `history: Vec::new()` to `app_on`.
- [ ] **Step 3: Run** `cargo test panel:: watch::` — FAIL.
- [ ] **Step 4: Implement.** `panel::session`: after the tokens rows, one row per deviation: `c.span(format!("⚠ {}", d.text()), Style::new().fg(Color::Yellow))` (`⚠` already maps to `!` in `--ascii`). Wire `Extras.quality` and `App.history` as above.
- [ ] **Step 5: Run** `cargo test` (no existing `.snap` may change), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — PASS.
- [ ] **Step 6: Run the real thing.** `cargo install --path . --root ~/.local --force` (per the local-install note: `rm` the old binary first if `cp` is killed), use Claude Code for a few sessions, check `~/.local/state/ari/quality.jsonl` gains one line per Stop and contains no prompt text or paths.
- [ ] **Step 7: Commit** `feat(panel): show reads per edit, errors and retries against your usual`.
- [ ] **Step 8: Docs.** README: one sentence in "What you get" after the git-section sentence: when a session reads less before editing, or fails and retries more, than your usual on that model, the session section says so (e.g. `⚠ reads/edit 2.0 · your usual 6.1`); it needs 5 earlier sessions with at least 5 edits. FAQ "Where does my data go?": add that `quality.jsonl` keeps per-session counts only (no prompts, paths or output), pruned with `retention_days`. DECISIONS.md: a new entry (next free D-number) with the bullets from "Decisions this plan makes". CHANGELOG `## Unreleased`: one line, **Your usual, as a baseline.** … Commit `docs: quality baseline`.
- [ ] **Step 9: Version bump.** Next patch after the version on `main` (e.g. `0.3.1` → `0.3.2`) in `Cargo.toml`, `.claude-plugin/plugin.json`, and `Cargo.lock` (`cargo build` refreshes it). Commit `chore: bump version to <x.y.z>`.
- [ ] **Step 10: Finish.** Full CI commands pass; PR with a bullet body.

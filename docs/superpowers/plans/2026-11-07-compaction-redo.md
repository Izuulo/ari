# Compaction Redo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mark compactions on the hub's zoom timeline and activity lanes, and warn "redoing work since compaction" when main repeats, after a compaction, edits or commands it had already done.

**Architecture:** The reducer keeps hashes of main's recent successful work calls (file edits and shell commands). `PreCompact` or `PostCompact` opens a redo window. Each main work call inside the window that repeats work from before it counts as a redo. The window closes once a whole turn has started after the compaction. The redo count is a warning beside the verdict, not a verdict. The panel's turns section already marks compactions (`⟲ 14:02 compacted (auto) · ctx 91% → 18%`); this plan adds the same marks to the hub.

**Tech Stack:** Rust 2024, serde, ratatui 0.30, insta. No new dependencies.

**Spec:** `docs/ROADMAP.md` §7, item 4 ("Mark compactions on the timeline; warn 'redoing work since compaction' when the same edits or commands repeat after one").

## Global Constraints

- Zero-token, local, watch-only: ARI sends nothing to the model, never blocks a tool or answers for the user, nothing leaves the machine. Hooks are unchanged (`PreCompact` and `PostCompact` are already registered by `ari setup`).
- No new crates. No new event fields: the input hash `Event.h`, `sig`, `tests` and `act` are enough.
- **Verdict ladder unchanged:** NEEDS YOU > BLOCKED > STUCK > RETRYING > ON TRACK > DONE > IDLE. A redo is a warning shown next to any verdict; it never changes the verdict. Repeating work isn't the same as making no progress, and STUCK already covers going in circles.
- **Sibling plans:** land after `2026-11-07-edit-churn.md` (item 3) and before `2026-11-07-test-tampering.md` (item 5). The logic doesn't depend on them; the order only settles merge points: `CACHE_VERSION` (+1 from `main`), `State` fields, the segments after the verdict in `bar::render`, and the rows under the verdict in `panel::header`. **Statusline order:** verdict │ redo (this plan) │ test changes (item 5) │ conflicts │ …. **Panel header order:** verdict row, stuck row, redo row, test-change rows (item 5), needs-you row, ….
- Constants in `src/state.rs`: `REDO_MIN: u32 = 3`, `WORK: usize = 200`, `REDO_TOOLS: [&str; 6] = ["Edit", "Write", "MultiEdit", "NotebookEdit", "Bash", "PowerShell"]`, and `REORIENT: [&str; 17] = ["ls", "cat", "pwd", "head", "tail", "wc", "grep", "rg", "find", "tree", "which", "echo", "git status", "git log", "git diff", "git show", "git branch"]` (shell commands that only look around; matched as whole words against the start of the command part of `sig`).
- Copy, verbatim: statusline `⟲ redo ×{n}`; panel `⟲ redoing work since compaction · {n}× · {last}`; hub note `⟲ main is redoing work since compaction ({n}×)`; zoom timeline row `⟲ compacted ({kind})`.
- CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Snapshot updates: `INSTA_UPDATE=always cargo test`, then review every `.snap` diff.
- One commit per task, `feat(scope):` / `docs:`, each ending with the attribution line. Branch `compaction-redo` from `main`.

## Review Focus

1. **Re-running the tests after a compaction** (`cargo test` again): normal, never a redo. Test in Task 1.
2. **Looking around after a compaction** (`git status`, `ls`, `Read`, `Grep`): normal, never a redo. Test in Task 1.
3. **A subagent repeating its own calls** while main compacted: not counted. Subagents have their own context. Test in Task 1.
4. **A command that failed before the compaction and is retried after:** a retry, not a redo, because only successful calls are remembered as work. Test in Task 1.
5. **The warning lingering:** it clears by itself once a full turn has started after the compaction. A second compaction restarts the count. Test in Task 1.

---

### Task 1: Remember work, count redos

**Files:**
- Modify: `src/state.rs` (constants; `State`; `apply` arms `UserPromptSubmit`, `PostToolUse | PostToolUseFailure`, `PreCompact`, new `PostCompact`; `CACHE_VERSION` +1)

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)] #[serde(default)] pub struct Redo { pub since: u64, pub n: u32, pub last: String }` (`last` is the latest redone call's `act`).
  - `State.work: VecDeque<(u64, u64)>`: (time, input hash) of main's successful work calls, capped at `WORK`.
  - `State.redo: Option<Redo>`: the open redo window.
  - `fn track_work(&mut self, ev: &Event)` (called from the `PostToolUse | PostToolUseFailure` arm). It skips subagent events (`ev.a.is_some()`), tools not in `REDO_TOOLS`, test runs (`ev.tests.is_some()`), interrupts, and `Bash`/`PowerShell` whose `sig` command starts with a `REORIENT` entry. A call inside the window whose `h` matches `work` from before `since` counts: `n += 1`, `last = act`. Successful calls are then pushed to `work`.
  - `PreCompact` and `PostCompact` both set `redo = Some(Redo { since: ev.t, ..Default::default() })`. `compactions` is still pushed only on `PreCompact`, as today.
  - On a real (non-synthetic) `UserPromptSubmit`, before pushing to `prompts`: if `prompts.back()` is later than `redo.since`, set `redo = None`.
  - `pub fn redoing(&self) -> Option<&Redo>`: `redo` when `n >= REDO_MIN`.

- [ ] **Step 1: Write the failing tests** in `state.rs` tests, with a helper `post(t: u64, agent: Option<&str>, tool: &str, sig: &str, h: u64, ok: bool) -> Event` (`PostToolUse` or `PostToolUseFailure` by `ok`; `act = "$ {sig}"` as in the existing `tool` helper):
  - `redo_after_compaction_counts_repeats`: `prompt` (t = 1 000); successful `Edit` h=1, `Bash:npm install` h=2, `Bash:cargo build` h=3 at t = 1 001..=1 003; `PreCompact` at t = 1 050; the same three at t = 1 051..=1 053 → `redoing().map(|r| (r.n, r.last.as_str())) == Some((3, "$ Bash:cargo build"))`; `verdict() == OnTrack`. With two repeats → `redoing() == None`.
  - `redo_ignores_tests_reads_subagents_and_new_work`: before and after the compaction, the same `Bash:cargo test` with `tests: Some((5, 0))`, `Bash:git status`, `Read`, a repeat by agent `a1`, and a new hash → `redo.unwrap().n == 0`.
  - `failed_redo_attempt_counts`: successful `Edit` h=1; compaction; `PostToolUseFailure` `Edit` h=1 → `n == 1`.
  - `pre_compaction_failure_is_not_work`: failing `Bash` h=5; compaction; the same succeeds → `n == 0`.
  - `redo_window_closes_and_restarts`: compaction mid-turn → next real prompt keeps `redo`, the one after clears it; a second compaction resets `n` to 0 with the new `since`; `PostCompact` with no `PreCompact` opens a window too; a `<task-notification>` prompt never closes it.
  - Every event in these tests gets its own increasing `t`. The `ev` helper defaults to 1 000, and the window compares times.
- [ ] **Step 2: Run** `cargo test state::` — FAIL.
- [ ] **Step 3: Implement** and bump `CACHE_VERSION`.
- [ ] **Step 4: Run** `cargo test` — PASS (`turn_log_and_compactions` unchanged).
- [ ] **Step 5: Commit** `feat(state): count work redone after a compaction`.

---

### Task 2: The redo warning in the statusline, panel and hub notes

**Files:**
- Modify: `src/bar.rs` (`render`: segment right after the verdict), `src/panel.rs` (`header`: row after the stuck row), `src/hub/agents.rs` (`notes`, before the compaction forecast)

**Interfaces:**
- Consumes: `State::redoing() -> Option<&Redo>` (Task 1).

- [ ] **Step 1: Write the failing tests:**
  - `bar.rs` `redo_segment`: a state with `redo = Some(Redo { since: 1, n: 3, last: "editing a.rs".into() })` → row contains `│ ⟲ redo ×3 │` directly after the verdict segment, in SGR `33` when colour is on; `n: 2` → no `⟲`.
  - `panel.rs` `redo_row_in_header`: `render` with that state → contains `⟲ redoing work since compaction · 3× · editing a.rs`, in yellow.
  - `hub/agents.rs` `notes_warn_on_redo`: `draw_with` the table → contains `⟲ main is redoing work since compaction (3×)`.
- [ ] **Step 2: Run** `cargo test bar:: panel:: hub::` — FAIL.
- [ ] **Step 3: Implement** in the three places (yellow / `warn()`).
- [ ] **Step 4: Run** `cargo test` — PASS, no snapshot changes.
- [ ] **Step 5: Commit** `feat(panel): warn when work is redone after a compaction`.

---

### Task 3: Compactions on the hub's timeline and lanes

**Files:**
- Modify: `src/hub/zoom.rs` (`timeline`: a third entry kind), `src/hub/feed.rs` (`lanes`)

**Interfaces:**
- Consumes: `State.compactions: VecDeque<(u64, String)>` (existing).
- Produces: no new API. In `timeline`, when `a.id == "main"`, each compaction becomes a row `{when}⟲ compacted ({kind})` in `warn()`, sorted in with calls and messages (sort key rank 2, after a call or message at the same time). In `lanes`, main's row draws `⟲` in `warn()` over the bucket `t / LANE_MS` of each compaction inside the visible window.

- [ ] **Step 1: Write the failing tests:**
  - `zoom.rs` `zoom_marks_compactions`: main with calls at 10 000 (`one.rs`) and 50 000 (`three.rs`), `compactions = [(30_000, "auto")]` → rows in order `one.rs` < `compacted (auto)` < `three.rs`. Zooming a subagent shows no `compacted`.
  - `feed.rs` `lanes_mark_compaction`: main active in the window, a compaction 2 buckets before `now` → main's lane row contains `⟲`; one older than the window → none.
- [ ] **Step 2: Run** `cargo test hub::` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `INSTA_UPDATE=always cargo test`, review `tests/snapshots/hub__*.snap` (only if their fixture has a compaction), then `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — PASS.
- [ ] **Step 5: Commit** `feat(hub): compactions on the zoom timeline and activity lanes`.

---

### Task 4: Docs, version, changelog

**Files:**
- Modify: `README.md`, `CHANGELOG.md` (`## Unreleased`), `docs/DECISIONS.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

- [ ] **Step 1: README.** Under "The verdict" table, add one line: `⟲ redo ×N next to the verdict: after a compaction, main repeated N edits or commands it had already done before it (re-running tests and looking around don't count).` In the hub paragraph, note that zoom and the activity lanes mark compactions with `⟲`.
- [ ] **Step 2: DECISIONS.** A new entry with the next free `D` number, "redo after compaction": main only; work = successful `REDO_TOOLS` calls by input hash, the last 200; test runs and `REORIENT` commands skipped; window from `PreCompact`/`PostCompact` until a whole turn has started after it; `REDO_MIN = 3`; a warning, not a verdict. Note that D20 still applies: no local recording has a compaction yet, so this is tested on synthetic events only.
- [ ] **Step 3: CHANGELOG** under `## Unreleased`: `- After a compaction, ARI warns "⟲ redo ×N" when Claude repeats edits or commands it had already done. The hub's zoom view and activity lanes mark each compaction.`
- [ ] **Step 4: Patch bump** (one patch above `main`) in `Cargo.toml` and `.claude-plugin/plugin.json`; `cargo build` updates `Cargo.lock`.
- [ ] **Step 5: Run** the three CI commands — PASS.
- [ ] **Step 6: Commit** `docs: redo after compaction` and `chore: bump to 0.3.x`. Open a PR with a bullet body.

---

## Self-review

- Spec coverage: compaction marks on the timeline (the panel's turns section already has them, with a test; hub zoom and lanes in Task 3), the redo warning for repeated edits and commands after a compaction (Tasks 1 and 2). `ari recap` is not touched.
- Types: `Redo { since, n, last }`, `redoing() -> Option<&Redo>` and `work: VecDeque<(u64, u64)>` are used the same way in every task.
- Left out: redo for subagents' own compactions, a `hide` id for the segment, and catching identical work done before the 200-call memory.
- Risk: `REDO_MIN = 3` and `REORIENT` are guesses until a recorded session includes a real compaction (D20).

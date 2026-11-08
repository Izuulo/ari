# Edit Churn Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** STUCK also fires when one agent edits the same file over and over with no passing test between, or keeps undoing its own edits, and every STUCK shows the tokens spent since it got stuck.

**Architecture:** The hook adds two hashes to each successful `Edit` event (old and new string, never the text). The reducer keeps a short per-session edit log and gains two STUCK rules after the existing two. `stuck_reason` grows into `stuck()`, which also says when the loop began. The statusline samples (`limits.jsonl`) gain the session's token total, so "tokens since T" is a subtraction.

**Tech Stack:** Rust 2024, serde, ratatui 0.30, insta. No new dependencies.

**Spec:** `docs/ROADMAP.md` §7, item 3 ("STUCK also on edit churn: the same file edited N times, or an edit that undoes an earlier one; show tokens burned while stuck").

## Global Constraints

- Zero-token, local, watch-only: ARI sends nothing to the model, never blocks a tool or answers for the user, nothing leaves the machine. Hooks stay synchronous, `timeout: 1`, and print nothing.
- No new crates.
- Events carry hashes and counts, never file contents (DECISIONS: "Events are normalized in the hook"). An event line stays under `event::MAX_LINE` (4096).
- **Verdict ladder unchanged:** NEEDS YOU > BLOCKED > STUCK > RETRYING > ON TRACK > DONE > IDLE. No new `Verdict` variant (`watch --all` ranks by `v as u8`). The new rules only add ways to reach STUCK, and like the old ones they apply only while the turn is active.
- **Sibling plans:** `2026-11-07-compaction-redo.md` (item 4) and `2026-11-07-test-tampering.md` (item 5) touch the same files. Land in order 3 → 4 → 5. This plan adds `State::turn_start()`, which item 5 uses. Each plan bumps `CACHE_VERSION` by one from whatever `main` has.
- Thresholds are constants in `src/state.rs`, not config: `CHURN_EDITS: usize = 12`, `UNDO_REPEATS: usize = 2`, `EDIT_LOG: usize = 60`.
- Copy, verbatim: churn reason `{file} edited {n}× with no passing test`; undo reason `{file} edited back and forth {n}×`; statusline `↻ STUCK · {tokens} tok`; panel suffix ` · {tokens} tok while stuck`. `{file}` is the file name (last path component); `{tokens}` is `bar::fmt_tokens`.
- CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Snapshot updates: `INSTA_UPDATE=always cargo test`, then review every `.snap` diff.
- One commit per task, `feat(scope):` / `fix:` / `test:` / `docs:`, each ending with the attribution line. Branch `edit-churn` from `main`.

## Review Focus

1. **Parallel subagents editing one file** (main ×6 and `a1` ×6 on `a.rs`): not STUCK; counts are per agent, as rule A already is. Test in Task 2.
2. **Ticking a Markdown plan** (12+ edits to `plan.md` in one turn): not STUCK. Test in Task 2.
3. **A long refactor across two turns** (8 edits, new prompt, 8 edits): not STUCK; only the current turn counts. Test in Task 2.
4. **One honest revert** (edit A→B, then B→A): not STUCK; it takes a second undo. Test in Task 2.
5. **No token samples** (Codex, dsh, or samples without `tok` from before the upgrade): STUCK still shows, just without the token suffix. Tests in Tasks 3 and 4.

---

### Task 1: Edit hashes on the event

**Files:**
- Modify: `src/event.rs` (`Event` struct; `normalize`'s `"PostToolUse"` arm next to `edit_lines`)

**Interfaces:**
- Produces: `Event.edit: Option<(u64, u64)>` with `#[serde(skip_serializing_if = "Option::is_none")]`, doc `/// Edit that succeeded: hashes of (old_string, new_string), to spot an edit that undoes another. Never the text.` Set only for tool `Edit` on `PostToolUse`, and only when both strings are present and differ, as `(fnv1a(old.as_bytes()), fnv1a(new.as_bytes()))`. Codex `apply_patch` is canonicalised to an `Edit` with only `file_path`. Without that check it would hash `("", "")`, which looks like an undo of itself.

- [x] **Step 1: Write the failing test** `edit_hashes_old_and_new` in `event.rs` tests: normalize a `PostToolUse` `Edit` payload with `old_string: "SECRET_OLD"`, `new_string: "SECRET_NEW"` → `ev.edit == Some((fnv1a(b"SECRET_OLD"), fnv1a(b"SECRET_NEW")))`; `to_line(&ev)` contains neither `SECRET_OLD` nor `SECRET_NEW`. The same payload as `PostToolUseFailure` → `edit == None`. A `Write` → `None`. A Codex `apply_patch` (canonicalised, no strings) → `None`. `old_string == new_string` → `None`.
- [x] **Step 2: Run** `cargo test event::` — FAIL (no field).
- [x] **Step 3: Implement** the field and set it in the `PostToolUse` arm.
- [x] **Step 4: Run** `INSTA_UPDATE=always cargo test`, review `tests/snapshots/*events*.snap`: only `"edit":[…]` added to `Edit` lines. Then `cargo test` — PASS.
- [x] **Step 5: Commit** `feat(event): hash an edit's old and new strings`.

---

### Task 2: Edit log and the two churn rules

**Files:**
- Modify: `src/state.rs` (constants; `State`; `Call`; `PreToolUse` and `PostToolUse` arms of `apply`; `stuck_reason`; `verdict_with`; `CACHE_VERSION` +1 from `main`, 12 if it is still 11)

**Interfaces:**
- Consumes: `Event.edit` (Task 1).
- Produces:
  - `#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)] #[serde(default)] pub struct EditRec { pub t: u64, pub agent: String, pub path: String, pub h: Option<(u64, u64)> }` (`path` from the event's `sig`, the absolute path).
  - `State.edit_log: VecDeque<EditRec>`: successful `FILE_EDITS` results, oldest first, capped at `EDIT_LOG`.
  - `Call.t: u64` (the `PreToolUse` time).
  - `#[derive(Clone, Debug, PartialEq)] pub struct Stuck { pub why: String, pub since: u64 }`
  - `pub fn stuck(&self, repeats: u8) -> Option<Stuck>`: rules in this order: B (existing, `since` = the error's `first_t`), A (existing, `since` = `t` of the first of the repeated calls), C undo, D churn.
  - `pub fn stuck_reason(&self, repeats: u8) -> Option<String>` stays, as `self.stuck(repeats).map(|s| s.why)`; `verdict_with` calls `stuck`.
  - `pub fn turn_start(&self) -> u64`: `self.prompts.back()` or 0.
  - Rule C (undo): within the current turn (`t >= turn_start()`), per (agent, path), an edit with `h == Some((o, n))` undoes an earlier edit with `h == Some((n, o))`. `UNDO_REPEATS` or more undos → `{file} edited back and forth {n}×`, `since` = the first undone edit's `t`.
  - Rule D (churn): per (agent, path), edits with `t >= turn_start()` and `t >` the latest green test run (`fail == 0 && pass > 0`, partial runs included), paths ending `.md` skipped. `CHURN_EDITS` or more → `{file} edited {n}× with no passing test`, `since` = the first counted edit's `t`.

- [x] **Step 1: Write the failing tests** in `state.rs` tests, with a helper `edit(t: u64, agent: &str, path: &str, h: Option<(u64, u64)>) -> Event` (a successful `PostToolUse` of `Edit`, `sig = "Edit:{path}"`, `act = "editing {path}"`):
  - `churn_same_file_without_passing_test`: `prompt` (t = 1 000, from the `ev` helper), then 12 edits of `/r/src/a.rs` at t = 1 010..=1 021 → `verdict() == Stuck`, `stuck(3) == Some(Stuck { why: "a.rs edited 12× with no passing test".into(), since: 1_010 })`. With 11 edits → `OnTrack`. (Every timed event in these tests comes after the prompt's t, because `turn_start()` is the prompt time.)
  - `passing_test_resets_churn`: 6 edits, a green run (`tests: Some((5, 0))`), 6 edits → `OnTrack`. 6 edits, a red run (`Some((4, 1))`), 6 edits → `Stuck`.
  - `churn_ignores_markdown_other_agents_and_old_turns`: 12 edits of `/r/plan.md` → `OnTrack`; 6 by main plus 6 by `a1` on the same path → `OnTrack`; 8 edits, `prompt`, 8 edits → `OnTrack`.
  - `undo_flip_flop_is_stuck`: edits with `h` (1,2), (2,1) → `OnTrack`; then (1,2) → `Stuck`, why `a.rs edited back and forth 2×`, `since` = the first edit's `t`. Edits with `h: None` never count as undos.
  - `churn_needs_an_active_turn`: 12 edits then `stop(0)` → not `Stuck`.
  - `stuck_since_per_rule`: rule B's `since` == the first failure's `t`; rule A's == the first repeated call's `t`.
  - Existing `stuck_on_identical_calls` and `stuck_on_repeated_failure` pass unchanged.
- [x] **Step 2: Run** `cargo test state::` — FAIL.
- [x] **Step 3: Implement.** Push `EditRec` in the `PostToolUse` arm when `ev.ok == Some(true)` and the tool is in `FILE_EDITS`. Set `Call.t`. Add `stuck`, `turn_start`, and the two rules. Bump `CACHE_VERSION` by one.
- [x] **Step 4: Run** `cargo test` — PASS. Review `tests/snapshots/fixtures__basic_verdicts.snap` and the demo: if a fixture now turns STUCK, check it really churns before accepting.
- [x] **Step 5: Calibrate on real sessions.** In a throwaway test (not committed), load every `~/.local/state/ari/sessions/*/` with `replay::load`, fold it, and print each session where `stuck()` returns a churn or undo reason. Read a few with `cargo run -q -- replay <dir>`. If ordinary work trips rule D often, raise `CHURN_EDITS` and record the number in Task 5's DECISIONS entry. Old recordings have no `edit` hashes, so only rule D can be checked this way.
- [x] **Step 6: Commit** `feat(state): STUCK on edit churn and undone edits`.

---

### Task 3: Token totals in the samples

**Files:**
- Modify: `src/burn.rs` (`Sample`, `record`, new `tokens_since`), `src/statusline.rs` (`own_row`)

**Interfaces:**
- Produces:
  - `Sample.tok: Option<u64>` (`skip_serializing_if = "Option::is_none"`): the session's token total (`TokenCache::total().total()`) when sampled.
  - `pub fn record(dir: &Path, u: &Usage, tok: Option<u64>, now: u64)`.
  - `pub fn tokens_since(samples: &[Sample], t: u64) -> Option<u64>`: newest `tok` minus the `tok` of the newest sample at or before `t`, else of the oldest sample with one. `None` when no sample has `tok`. Saturating. Comment: `// shortcut: stuck longer than the sample tail (~64 KB) undercounts; keep a longer tail if that matters.`

- [x] **Step 1: Write the failing tests** in `burn.rs` tests:
  - `tokens_since_uses_the_sample_before`: samples (t, tok) = (10, 100), (20, 150), (30, 400) → `tokens_since(&s, 20) == Some(250)`, `(&s, 25) == Some(250)`, `(&s, 5) == Some(300)`; samples without `tok` → `None`.
  - `record_dedupes_and_loads` updated: pass `Some(42)` and check `load(..)[0].tok == Some(42)`.
- [x] **Step 2: Run** `cargo test burn::` — FAIL.
- [x] **Step 3: Implement.** In `own_row`, call `burn::record` after `x.tokens = Some(tokens::update_debounced(..))`, passing `x.tokens.as_ref().map(|t| t.total().total())`. Then, only when `st.verdict_with(cfg.thresholds.stuck_repeats) == Verdict::Stuck`, set `x.samples = burn::recent(&dir)` (the statusline doesn't read samples otherwise).
- [x] **Step 4: Run** `cargo test` — PASS.
- [x] **Step 5: Commit** `feat(burn): samples record the session's token total`.

---

### Task 4: Show the tokens burned while stuck

**Files:**
- Modify: `src/bar.rs` (`render`, verdict segment), `src/panel.rs` (`header`, the stuck row)

**Interfaces:**
- Consumes: `State::stuck` (Task 2), `burn::tokens_since` and `Extras.samples` (Task 3).
- Produces: no new API. The hub's top row is `bar::render`, so it picks up the statusline text; `ari watch` already loads `x.samples`.

- [x] **Step 1: Write the failing tests:**
  - `bar.rs` `stuck_shows_tokens_burned`: a state stuck on rule B with `first_t = 20`, `Extras.samples` (t, tok) = (10, 100), (20, 150), (30, 400) → row starts `↻ STUCK · 250 tok │`; with no samples → starts `↻ STUCK │`.
  - `panel.rs` `stuck_row_shows_tokens`: the `stuck_reason_in_header` setup through `render_x` with those samples (error `first_t` set to their range) → contains `· 250 tok while stuck`; without samples → the row is unchanged from today.
- [x] **Step 2: Run** `cargo test bar:: panel::` — FAIL.
- [x] **Step 3: Implement.** Add the suffix only when the verdict is `Stuck` and `tokens_since` gives more than 0.
- [x] **Step 4: Run** `INSTA_UPDATE=always cargo test`, review: no existing snapshot should change. Then `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — PASS.
- [x] **Step 5: Commit** `feat(panel): tokens burned while stuck`.

---

### Task 5: Docs, version, changelog

**Files:**
- Modify: `README.md` (verdict table), `CHANGELOG.md` (`## Unreleased`), `docs/DECISIONS.md` (new entry), `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

- [x] **Step 1: README.** The `↻ STUCK` row becomes: `Going in circles: the same command failing again and again, the same call repeated with nothing changing between, one file edited 12 times with no passing test, or an edit undone and redone. Shows the tokens spent since it got stuck`.
- [x] **Step 2: DECISIONS.** A new entry with the next free `D` number, "edit churn": per agent, current turn only, Markdown skipped, a green test run resets the count; `CHURN_EDITS = 12` (or the calibrated value from Task 2), `UNDO_REPEATS = 2` (one revert is normal); undo needs the `edit` hashes, so sessions recorded before this change only get the churn rule; tokens come from statusline samples, so Codex and dsh show none.
- [x] **Step 3: CHANGELOG** under `## Unreleased`: `- STUCK also fires when Claude edits one file 12 times without a passing test, or keeps undoing its own edit. STUCK shows the tokens spent since it began, in the statusline and the panel.`
- [x] **Step 4: Patch bump** (for example `0.3.0` → `0.3.1`, one patch above `main`) in `Cargo.toml` and `.claude-plugin/plugin.json`; `cargo build` updates `Cargo.lock`.
- [x] **Step 5: Run** the three CI commands — PASS.
- [x] **Step 6: Commit** `docs: edit churn and tokens while stuck` and `chore: bump to 0.3.x`. Open a PR with a bullet body.

---

## Self-review

- Spec coverage: same file N times without a passing test (Task 2, rule D), an edit that undoes an earlier one (Tasks 1 and 2, rule C), tokens burned while stuck (Tasks 3 and 4), statusline, panel and hub (Task 4; the hub reuses `bar::render`). `ari recap` is not touched: its verdict is computed after the session ends, when STUCK can't apply.
- Types: `Stuck { why, since }`, `EditRec { t, agent, path, h }`, `tokens_since(&[Sample], u64) -> Option<u64>`, `record(.., tok, now)` are used the same way in every task.
- Left out: undo detection for `Write` and `MultiEdit` (only `Edit` carries hashes). A new hide id for the token suffix.

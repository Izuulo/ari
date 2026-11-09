# Test Tampering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Flag a run that deleted test files, added skip/xfail/`.only`/`#[ignore]` markers, or edited a test file while the tests were failing. Mark a finished turn "unverified" when files changed after the last test run.

**Architecture:** The hook, which already sees the edit strings and the shell command, counts the skip markers an edit adds and lists the test files a shell command deletes. It writes only those counts and paths. The reducer turns them, plus test-file edits made while the latest run is red, into a short list of `TestFlag`s. `State::unverified()` compares the edit times ARI already keeps with the last test run. A new `State::verdict_note` replaces the two copies of the existing "DONE · tests failing" logic. It now also covers "unverified" on DONE and IDLE.

**Tech Stack:** Rust 2024, serde, ratatui 0.30, insta. No new dependencies.

**Spec:** `docs/ROADMAP.md` §7, item 5 ("Flag test files edited or deleted, or skip/xfail added, during the run; 'unverified' when files changed after the last test run").

## Global Constraints

- Zero-token, local, watch-only: ARI sends nothing to the model, never blocks a tool or answers for the user, nothing leaves the machine. Flags are for the person reading the panel. ARI never reverts or stops anything.
- No new crates. Events carry counts and paths, never file contents. An event line stays under `event::MAX_LINE` (4096). `rm` holds at most 5 paths, each truncated to 120 chars.
- **Verdict ladder unchanged:** NEEDS YOU > BLOCKED > STUCK > RETRYING > ON TRACK > DONE > IDLE. No new `Verdict` variant. Test flags never change the verdict. `verdict_note` adds a suffix to DONE or IDLE only: `tests failing` (DONE, as today) beats `unverified` (DONE or IDLE).
- **Sibling plans:** land last, after `2026-11-07-edit-churn.md` (item 3, which adds `State::turn_start()`; if it hasn't landed, add that one-liner here: `self.prompts.back().copied().unwrap_or(0)`) and `2026-11-07-compaction-redo.md` (item 4). `CACHE_VERSION` +1 from `main`. **Statusline order:** verdict (+ note) │ redo (item 4) │ test changes (this plan) │ conflicts │ …. **Panel header order:** verdict row, stuck row, redo row, test-change rows, needs-you row, ….
- Constants: `event::SKIP_MARKERS: [&str; 17] = ["#[ignore", "@pytest.mark.skip", "@pytest.mark.xfail", "@unittest.skip", "pytest.skip(", "pytest.xfail(", "it.skip(", "it.only(", "test.skip(", "test.only(", "describe.skip(", "describe.only(", "xit(", "xdescribe(", "t.Skip(", "@Disabled", "@Ignore"]`. A match counts only when the char before it is not `[A-Za-z0-9_.]`, so `bit.skip(` and `iter().skip(1)` don't count. `state::TEST_FLAGS: usize = 20`.
- Flag kinds, verbatim: `skip added`, `deleted`, `edited while failing`. Copy: statusline `⚠ {n} test change{s}`; panel row `⚠ {kind} · {file}` (at most 3 rows, then `  +{n} more`); verdict suffixes ` · tests failing` and ` · unverified`; recap row label `check`, Markdown item `**Check:**`.
- CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Snapshot updates: `INSTA_UPDATE=always cargo test`, then review every `.snap` diff.
- One commit per task, `feat(scope):` / `docs:`, each ending with the attribution line. Branch `test-tampering` from `main`.

## Review Focus

1. **Ordinary code that looks like a marker** (`xs.iter().skip(1)`, `bit.skip(`): never a skip. Test in Task 1.
2. **Removing a skip** (old string has `#[ignore]`, new doesn't): not flagged. Only markers added count. Test in Task 1.
3. **Normal TDD** (write a test, then run it red, then fix the code; or edit a test while the suite is green): not flagged. Only test edits made while the latest run is red are. Test in Task 2.
4. **A session that never runs tests:** never "unverified" (there's no last run to be after). Editing only Markdown after the last run isn't "unverified" either. Test in Task 2.
5. **Shell commands that mention `rm` or tests without deleting them** (`grep rm tests/`, `echo rm tests`), and `rm` of non-test files: not flagged. `git rm` and `rm -rf tests/` are. Test in Task 1.

---

### Task 1: Markers and deletions on the event

**Files:**
- Modify: `src/event.rs` (`Event`; `normalize`'s `PostToolUse` arm; new helpers)

**Interfaces:**
- Produces:
  - `pub fn is_test_path(p: &str) -> bool`: a path component equal to `tests`, `test`, `__tests__`, `spec` or `specs`; or a file name matching `test_*.py`, `*_test.py`, `*_test.go`, `*_spec.rb`, `*Test.java`, `*Tests.cs`, or containing `.test.` / `.spec.`.
  - `fn skips_added(tool: &str, input: &Value) -> Option<u32>`: `Edit`: markers in `new_string` minus markers in `old_string`; `MultiEdit`: the sum over `edits`; `Write`: markers in `content`. `Some` only when > 0. Comment on the `Write` case: `// shortcut: a Write can't see the old file, so existing markers count as added; read the file in PreToolUse if this misfires.`
  - `fn removed_tests(cmd: &str) -> Vec<String>`: the command split on `&&`, `;`, `|`, `\n`. For each part starting with `rm`, `git rm`, `unlink` or `Remove-Item`, the arguments that don't start with `-` and pass `is_test_path`. At most 5.
  - `Event.skips: Option<u32>` and `Event.rm: Option<Vec<String>>`, both `skip_serializing_if = "Option::is_none"`, set on `PostToolUse` only (`rm` only for `Bash`/`PowerShell`, `None` when empty).

- [x] **Step 1: Write the failing tests** in `event.rs` tests:
  - `test_paths`: true for `tests/a.rs`, `/r/tests/x.py`, `src/foo.test.ts`, `app/foo.spec.js`, `test_calc.py`, `calc_test.go`, `spec/models/user_spec.rb`, `web/__tests__/a.js`, `src/FooTest.java`; false for `src/state.rs`, `src/contest.rs`, `latest.py`, `attestation/x.rs`, `README.md`.
  - `skips_added_counts_new_markers_only`: `Edit` old `fn a() {}` → new `#[ignore]\nfn a() {}` gives `Some(1)`; the reverse gives `None`; new `xs.iter().skip(1)` gives `None`; new `it.only("x", ..)` gives `Some(1)`; new `@pytest.mark.xfail` gives `Some(1)`; a `MultiEdit` adding one `#[ignore]` in each of two edits gives `Some(2)`; a `Write` with `describe.skip(` gives `Some(1)`.
  - `rm_collects_deleted_test_paths`: `rm tests/test_calc.py src/a.py` gives `["tests/test_calc.py"]`; `cd x && git rm -f calc_test.go` gives `["calc_test.go"]`; `rm -rf tests/` gives `["tests/"]`; `grep rm tests/x` and `echo rm tests` give `[]`. Through `normalize`: a `PostToolUse` `Bash` with the first command has `rm == Some(vec!["tests/test_calc.py"])`; as `PostToolUseFailure` it has `rm == None`.
- [x] **Step 2: Run** `cargo test event::` — FAIL.
- [x] **Step 3: Implement.**
- [x] **Step 4: Run** `INSTA_UPDATE=always cargo test`, review the `tests/snapshots/fixtures__*_events.snap` diffs (new fields only where a fixture adds markers or deletes tests), then `cargo test` — PASS.
- [x] **Step 5: Commit** `feat(event): count added skip markers and deleted test files`.

---

### Task 2: Test flags, unverified, the verdict note

**Files:**
- Modify: `src/state.rs` (constants; `State`; the `PostToolUse` arm; new methods; `CACHE_VERSION` +1)

**Interfaces:**
- Consumes: `Event.skips`, `Event.rm`, `event::is_test_path` (Task 1); `State::turn_start()` (edit-churn plan).
- Produces:
  - `#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)] #[serde(default)] pub struct TestFlag { pub t: u64, pub path: String, pub kind: String }`
  - `State.test_flags: VecDeque<TestFlag>`, oldest first, capped at `TEST_FLAGS`. A repeat of the same (path, kind) only updates `t`.
  - On a successful `PostToolUse`: `skips` > 0 adds `skip added` for the edited path (from `sig`). Each `rm` path adds `deleted`. A `FILE_EDITS` edit of an `is_test_path` path adds `edited while failing` while `tests_failing()`.
  - `pub fn turn_flags(&self) -> Vec<&TestFlag>`: flags with `t >= turn_start()`.
  - `pub fn unverified(&self) -> bool`: there is a last test run and some path in `edits` not ending `.md` was edited after it.
  - `pub fn verdict_note(&self, v: Verdict) -> Option<&'static str>`: `Some("tests failing")` when `v == Done && tests_failing()`; else `Some("unverified")` when `v` is `Done` or `Idle` and `unverified()`; else `None`.

- [x] **Step 1: Write the failing tests** in `state.rs` tests (reuse the `tool`, `prompt`, `stop` and `task` helpers; a test run is a `Bash` result with `tests: Some((pass, fail))`; a file edit is a successful `PostToolUse` of `Edit` with `sig = "Edit:{path}"`; give every event its own increasing `t`, because the `ev` helper defaults to 1 000 and both `turn_flags` and `unverified` compare times):
  - `skip_and_delete_are_flagged`: an edit with `skips: Some(1)` of `/r/src/a.rs` and a `Bash` with `rm: Some(vec!["tests/b.py".into()])` → `turn_flags()` kinds/paths `[("skip added", "/r/src/a.rs"), ("deleted", "tests/b.py")]`; verdict unchanged.
  - `test_edit_flagged_only_while_failing`: editing `/r/tests/a.py` before any run → no flag; after a red run (`Some((4, 1))`) → `edited while failing`; after a green run → no new flag.
  - `flags_dedupe_cap_and_turn`: the same skip twice is one flag; 25 distinct flags keep 20; after a new `prompt`, `turn_flags()` is empty but `test_flags` keeps them.
  - `unverified_after_edit_past_last_run`: no run + edit → `false`; run then edit `/r/src/a.rs` → `true`; run then edit `/r/plan.md` only → `false`; run, edit, run → `false`.
  - `verdict_note_order`: DONE with a red last run and a later edit → `Some("tests failing")`; DONE green with a later edit → `Some("unverified")`; IDLE with a later edit → `Some("unverified")`; ON TRACK with a later edit → `None`.
- [x] **Step 2: Run** `cargo test state::` — FAIL.
- [x] **Step 3: Implement** and bump `CACHE_VERSION`.
- [x] **Step 4: Run** `cargo test` — PASS.
- [x] **Step 5: Commit** `feat(state): flag test tampering and unverified changes`.

---

### Task 3: Show it in the statusline, panel and recap

**Files:**
- Modify: `src/bar.rs` (`render`: verdict segment and a segment after it), `src/panel.rs` (`header`; `recap_sections`), `src/recap.rs` (`Recap`, `build`, `markdown`)

**Interfaces:**
- Consumes: `verdict_note`, `turn_flags`, `test_flags` (Task 2).
- Produces: `Recap.test_flags: Vec<String>`: `"{kind} · {file}"` for flags with `t >= from`. The verdict segment, panel verdict row and `Recap.verdict` all read `{glyph} {LABEL} · {note}` when `verdict_note` is `Some`. The style is the existing "tests failing" one (SGR `1;33`, yellow bold), replacing the inline `v == Verdict::Done && st.tests_failing()` checks at `bar.rs` (`render`) and `panel.rs` (`header`). The hub's top row and the `watch --all` list are built from `bar::render`, so they follow.

- [x] **Step 1: Write the failing tests:**
  - `bar.rs` `unverified_and_test_changes`: IDLE with a run then a later edit → row starts `◌ IDLE · unverified │`; existing DONE · tests failing assertions still pass; with two `turn_flags` → contains `│ ⚠ 2 test changes │` right after the verdict segment; one → `⚠ 1 test change`.
  - `panel.rs` `test_flags_in_header`: four turn flags → three `⚠ {kind} · {file}` rows and `+1 more`; `test_history_row_and_done_failing` still passes.
  - `recap.rs` (or `tests/recap.rs`, wherever `build` is tested on a temp dir) `recap_lists_test_flags`: a session with a skip added after the window start → `test_flags == ["skip added · a.rs"]`; `markdown` contains `- **Check:** skip added · a.rs`; `recap_lines` contains a `check` row. A session ending unverified → `verdict` ends `· unverified`.
- [x] **Step 2: Run** `cargo test bar:: panel:: recap` — FAIL.
- [x] **Step 3: Implement.**
- [x] **Step 4: Run** `INSTA_UPDATE=always cargo test`, review snapshots: only sessions that end with edits after their last test run change (to `· unverified`); then `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — PASS.
- [x] **Step 5: Commit** `feat(panel): show test changes and unverified work`.

---

### Task 4: Docs, version, changelog

**Files:**
- Modify: `README.md`, `CHANGELOG.md` (`## Unreleased`), `docs/DECISIONS.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

- [x] **Step 1: README.** Under "The verdict" table: `DONE or IDLE · unverified: files changed after the last test run. ⚠ N test changes: this turn deleted a test file, added a skip/xfail/.only/#[ignore], or edited a test while tests were failing.` Add the `check` row to the `ari recap` example.
- [x] **Step 2: DECISIONS.** A new entry with the next free `D` number, "test tampering": which edits count (markers added, deletions, test edits only while red), the test-path and marker lists, `Write` counting markers already in the file (shortcut), Codex `apply_patch` deletes not seen (it's canonicalised to an `Edit`), "unverified" needs at least one test run and ignores Markdown, and none of it changes the verdict.
- [x] **Step 3: CHANGELOG** under `## Unreleased`: `- ARI flags a turn that deleted a test file, added a skip/xfail/.only/#[ignore], or edited a test while tests were failing ("⚠ N test changes", listed in the panel and in ari recap). DONE and IDLE read "unverified" when files changed after the last test run.`
- [x] **Step 4: Patch bump** (one patch above `main`) in `Cargo.toml` and `.claude-plugin/plugin.json`; `cargo build` updates `Cargo.lock`.
- [x] **Step 5: Run** the three CI commands — PASS.
- [x] **Step 6: Commit** `docs: test tampering and unverified` and `chore: bump to 0.3.x`. Open a PR with a bullet body.

---

## Self-review

- Spec coverage: test files deleted (Tasks 1 and 2, `rm`), skip/xfail/`.only`/`#[ignore]` added (Tasks 1 and 2, `skips`), test files edited (Task 2, narrowed to "while failing"), "unverified" after the last run (Task 2 `unverified`, Task 3 note), shown in the statusline, panel, hub (via `bar::render`) and recap (Task 3).
- Types: `TestFlag { t, path, kind }`, `verdict_note(Verdict) -> Option<&'static str>`, `turn_flags() -> Vec<&TestFlag>` and `Recap.test_flags: Vec<String>` are used the same way in every task.
- Left out: flagging every test-file edit (too noisy in TDD), deletes through `git mv`/`find -delete`/Codex patches, and a `hide` id for the new segment.

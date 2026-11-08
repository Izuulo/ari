# Recap Review Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `ari recap` (terminal, `--md`, and the `r` view in `ari watch`) groups the changed files by the task or prompt that caused them, marks the files that need a careful look (tests, config, lockfiles, CI, migrations, files outside the repo), and suggests a review order.

**Architecture:** No state or hook change is needed: `recap::build` already replays the session's events, and at each edit event the replayed `State` knows the in-progress task (`State::now_task()`) and the loop can remember the last real prompt. `build` folds the events once by hand instead of `fold(&events)`, attributing every `Edit`/`Write`/`MultiEdit`/`NotebookEdit` `PostToolUse` in the window to a cause. A pure `look(rel, inside)` classifies paths; a pure `review_order(groups)` merges and sorts. The flat `Recap.files` stays as it is (other code and tests use it); the `files` section and the Markdown `Files` bullet now render the groups.

**Tech Stack:** Rust 2024, ratatui, insta. No new dependencies.

**Spec:** `docs/ROADMAP.md` §7 item 8 (row 8: "`ari recap` groups files by the prompt or task that caused them, marks files to look at hard (tests, config, lockfiles), suggests a review order").

## Global Constraints

- Zero tokens, local only: built from ARI's own event log, no model, no network. `git` is still only called for `commits`.
- No new crates. No change to `event.rs`, `hook.rs`, `state.rs` or the events format.
- Cause labels, exact copy: `task: {subject}` (task in progress at the edit), `“{prompt}”` (the prompt, whitespace already collapsed, cut to 50 chars with `event::truncate`), `before your first prompt` (no real prompt seen yet).
- Look labels, exact copy and precedence (first match wins): `outside repo`, `migration`, `CI`, `lockfile`, `test`, `config`.
- Review order: look rank in the order above (`outside repo` first, unflagged last), then lines changed (`added + removed`) descending, then path.
- Caps, as named constants in `recap.rs`: `GROUPS_SHOWN = 6` (newest groups kept, `and N more` first), `FILES_PER_GROUP = 8`, `REVIEW_SHOWN = 10`. The same caps apply to `--md`.
- CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Snapshot updates: `INSTA_UPDATE=always cargo test`, then review every `.snap` diff.
- One commit per task, `feat(recap):` / `test:` / `docs:` style, ending with the attribution line. Work on branch `recap-review`.
- Patch version bump in `Cargo.toml`, `.claude-plugin/plugin.json` and `Cargo.lock` (next patch after `main`'s version), one `CHANGELOG.md` "## Unreleased" bullet.

## Review Focus

1. **The same prompt text twice** (`continue`, `continue`): two groups, keyed by the prompt's time, not its text. Test `same_prompt_twice_is_two_groups` in Task 2.
2. **A window that starts mid-turn** (`--since 10m` after the prompt was sent): the edits keep that prompt as cause, not `before your first prompt`. Test `window_mid_turn_keeps_its_prompt` in Task 2.
3. **No cwd known** (no `meta.json` cwd, no `usage.json`): nothing is flagged `outside repo` just because the cwd is empty; paths show as recorded (tilde'd). Test `unknown_cwd_flags_nothing_outside` in Task 2.
4. **A long session** (20 prompts, 50 files): terminal output and `--md` respect `GROUPS_SHOWN`, `FILES_PER_GROUP`, `REVIEW_SHOWN` with `and N more` rows; no line wider than the terminal. Test `long_recap_is_capped` in Task 3.
5. **Synthetic prompts** (`<task-notification>…`, `<agent-message…`) between edits: never become a cause; the edits stay with the prompt you typed. Test `synthetic_prompts_are_not_causes` in Task 2.

Known limits (README, Task 4): files changed by shell commands (`sed -i`, `mv`, a formatter, codegen) aren't seen, only file-tool edits; with several tasks in progress at once, edits go to the first one in the list; a background subagent that edits after your next prompt is credited to that newer prompt.

---

### Task 1: Classify paths and order the review

**Files:**
- Modify: `src/recap.rs` (new types and pure functions; tests in its test module)

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, Default, PartialEq)] pub struct Changed { pub path: String, pub edits: u32, pub added: u32, pub removed: u32, pub look: Option<&'static str> }` (`path` as `Recap.files` shows it: relative to the cwd, else tilde'd).
  - `#[derive(Clone, Debug, Default, PartialEq)] pub struct Group { pub cause: String, pub files: Vec<Changed> }`
  - `pub fn look(rel: &str, inside: bool) -> Option<&'static str>`
  - `pub fn review_order(groups: &[Group]) -> Vec<Changed>`: one entry per path (edits/added/removed summed across groups), sorted per Global Constraints.
- Pinned constants (private):
  - `TEST_DIRS: [&str; 8] = ["test", "tests", "__tests__", "spec", "e2e", "fixtures", "snapshots", "__snapshots__"]`; a file is `test` when any directory component is in it, or its name starts with `test_`, or contains `_test.`, `.test.`, `.spec.` or `_spec.`, or is `conftest.py`.
  - `MIGRATION_DIRS: [&str; 3] = ["migrations", "migrate", "alembic"]`; also any file name containing `migration`.
  - `CI_PREFIXES: [&str; 4] = [".github/workflows/", ".github/actions/", ".circleci/", ".buildkite/"]`, `CI_FILES: [&str; 4] = [".gitlab-ci.yml", "Jenkinsfile", "azure-pipelines.yml", ".travis.yml"]`.
  - Lockfile: file name ends with `.lock`, `-lock.json` or `-lock.yaml`, or is `go.sum` or `bun.lockb`.
  - `CONFIG_EXT: [&str; 6] = ["toml", "yaml", "yml", "ini", "cfg", "conf"]`, `CONFIG_FILES: [&str; 12] = ["package.json", "tsconfig.json", "Makefile", "CMakeLists.txt", "go.mod", "Gemfile", "requirements.txt", "setup.py", "Dockerfile", ".gitignore", "CLAUDE.md", "AGENTS.md"]`; also any name starting with `.env` and anything under `.claude/`.

- [ ] **Step 1: Write the failing tests** in `recap.rs`:
  - `look_marks_files_to_check`:
    ```rust
    assert_eq!(look("~/notes.md", false), Some("outside repo"));
    assert_eq!(look("db/migrations/001_users.sql", true), Some("migration"));
    assert_eq!(look(".github/workflows/ci.yml", true), Some("CI"));
    assert_eq!(look("Cargo.lock", true), Some("lockfile"));
    assert_eq!(look("web/package-lock.json", true), Some("lockfile"));
    assert_eq!(look("tests/cli.rs", true), Some("test"));
    assert_eq!(look("test_calc.py", true), Some("test"));
    assert_eq!(look("src/auth.test.ts", true), Some("test"));
    assert_eq!(look("src/snapshots/a.snap", true), Some("test"));
    assert_eq!(look("Cargo.toml", true), Some("config"));
    assert_eq!(look(".env.local", true), Some("config"));
    assert_eq!(look(".claude/settings.json", true), Some("config"));
    assert_eq!(look("src/main.rs", true), None);
    assert_eq!(look("docs/testing.md", true), None); // "testing" is not a test dir
    ```
  - `review_order_riskiest_first`: groups `[{cause "a", files [src/big.rs +40 −10, tests/t.rs +5, src/small.rs +1]}, {cause "b", files [src/small.rs +1, Cargo.toml +1, ~/x.txt +1 (look outside repo), .github/workflows/ci.yml +2]}]` (set `look` via `look()`) → paths in order `~/x.txt`, `.github/workflows/ci.yml`, `Cargo.toml`, `tests/t.rs`, `src/big.rs`, `src/small.rs`; `src/small.rs` has `edits == 2`, `added == 2`.
- [ ] **Step 2: Run** `cargo test recap::` — FAIL.
- [ ] **Step 3: Implement** the types, constants, `look` and `review_order`.
- [ ] **Step 4: Run** `cargo test recap::` — PASS.
- [ ] **Step 5: Commit** `feat(recap): flag files to look at and order a review`.

---

### Task 2: Attribute each edit to its task or prompt

**Files:**
- Modify: `src/recap.rs` (`Recap`, `build`)

**Interfaces:**
- Consumes: `Changed`, `Group`, `look` (Task 1); `State::now_task()`, `event::is_synthetic`.
- Produces: `Recap.groups: Vec<Group>`, groups in order of their first edit in the window, files in a group in order of their first edit. `build` replaces `let after = fold(&events)` with one loop over `events.iter().enumerate()` that applies each event to a `State` (that state is `after`) and, for index `>= split_at` and a successful file edit (`PostToolUse`, tool in `Edit`/`Write`/`MultiEdit`/`NotebookEdit`, path from `sig` as today), picks the cause before applying it:
  1. `st.now_task()` → key `task:{id}`, label `task: {subject}`;
  2. else the last non-synthetic `UserPromptSubmit` seen so far (anywhere in the log, not just the window) → key `prompt:{t}`, label `“{msg cut to 50}”`;
  3. else key `none`, label `before your first prompt`.
  
  `Changed.added/removed` sum `ev.lines`; `look(rel, inside)` where `inside` = the path was under a non-empty cwd. Extract the existing relative-path code into `fn rel_path(path: &str, cwd: &str) -> (String, bool)` and use it for both `files` and `groups`; `files` must stay byte-for-byte what it was.

- [ ] **Step 1: Write the failing tests** (`fixture_dir` for the multi fixture; `session_from` plus `std::fs::write(d.join("meta.json"), r#"{"cwd":"/p"}"#)` for hand-made sessions):
  - `files_grouped_by_task` (multi fixture, `Since::Start`):
    ```rust
    let g = &build(&d, Since::Start, END).groups;
    assert_eq!(g.iter().map(|g| g.cause.as_str()).collect::<Vec<_>>(), ["task: scaffold", "task: build"]);
    assert_eq!(g[0].files.iter().map(|f| (f.path.as_str(), f.added, f.removed, f.look)).collect::<Vec<_>>(),
        [("calc.py", 2, 0, None), ("test_calc.py", 4, 0, Some("test"))]);
    assert_eq!((g[1].files[0].path.as_str(), g[1].files[0].added, g[1].files[0].removed), ("calc.py", 1, 1));
    ```
    and `whole_session`'s existing `r.files` assertion still passes unchanged.
  - `files_grouped_by_prompt_without_tasks`: prompt `fix login` (t=1_000), `Edit` `/p/src/login.rs`, prompt `add tests` (t=5_000), `Write` `/p/tests/login.rs`, `Edit` `/elsewhere/x.txt` → causes `["“fix login”", "“add tests”"]`; second group's files `tests/login.rs` (`test`) and `/elsewhere/x.txt` (`outside repo`).
  - `same_prompt_twice_is_two_groups` (Review Focus 1): prompts `continue` at 1_000 and 5_000, an edit after each → two groups, both `“continue”`.
  - `window_mid_turn_keeps_its_prompt` (Review Focus 2): prompt `refactor` at 1_000, edit at 50_000; `build(&d, Since::Ago(20_000), 60_000)` → one group `“refactor”`.
  - `edits_before_any_prompt`: an edit with no prompt in the log → one group `before your first prompt`.
  - `synthetic_prompts_are_not_causes` (Review Focus 5): prompt `ship it` at 1_000, prompt `<task-notification>done</task-notification>` at 2_000, edit at 3_000 → cause `“ship it”`.
  - `unknown_cwd_flags_nothing_outside` (Review Focus 3): no `meta.json`, edit `/p/src/a.rs` → its `look` is `None` and `path` is `/p/src/a.rs`.
- [ ] **Step 2: Run** `cargo test recap::` — FAIL.
- [ ] **Step 3: Implement** `rel_path`, the single fold loop and `Recap.groups` in `build`.
- [ ] **Step 4: Run** `cargo test recap::` — PASS, including every existing recap test.
- [ ] **Step 5: Commit** `feat(recap): group changed files by the task or prompt that caused them`.

---

### Task 3: Show groups and review order (terminal, `--md`, `r` view)

**Files:**
- Modify: `src/panel.rs` (`recap_sections`' `files` section; new `review` section; `sample_recap` in tests), `src/recap.rs` (`markdown`, `files_line` removed if unused)

**Interfaces:**
- Consumes: `Recap.groups` (Task 2), `review_order` (Task 1), `GROUPS_SHOWN`, `FILES_PER_GROUP`, `REVIEW_SHOWN`.
- Produces (rows are exact copy; `{look}` is yellow, `›` and counts are dim):
  - `files` section, when `groups` is non-empty: first row `{n} edited · {k} to look at` (`· {k} to look at` only when `k > 0`, `n`/`k` counted over `review_order`); if more than `GROUPS_SHOWN` groups, `and {m} earlier` next; then per shown group `› {cause}` and its file rows `  {path}` + ` ×{edits}` when > 1 + `  +{added}` when > 0 + ` −{removed}` when > 0 + ` · {look}` when flagged; more than `FILES_PER_GROUP` files → `  and {m} more`. Falls back to today's flat rows when `groups` is empty (keeps old recaps and hand-built `Recap`s working).
  - `review` section, after `files`, only when there are at least 2 files: rows `{i}. {path}` + ` · {look}`; more than `REVIEW_SHOWN` → `and {m} more`.
  - `markdown`: the `Files` bullet becomes `- **Files:**` with one nested bullet per shown group, `  - {cause}: {path} ×n (look), …` (same caps, `and N more`); new bullet `- **Review order:** 1. {path} ({look}), 2. {path}, …` (at most `REVIEW_SHOWN`, then `and N more`).

- [ ] **Step 1: Write the failing tests:**
  - `recap.rs` `whole_session` (update): text contains `│ 2 edited · 1 to look at` (distinct paths), then `│ › task: scaffold`, `│   test_calc.py  +4 · test`, `├─ review `, `│ 1. test_calc.py · test`, `│ 2. calc.py`. Markdown contains `- **Files:**\n  - task: scaffold: calc.py, test_calc.py (test)\n  - task: build: calc.py\n` and `- **Review order:** 1. test_calc.py (test), 2. calc.py`. Remove the old `- **Files:** calc.py ×2, test_calc.py` assertion.
  - `recap.rs` `long_recap_is_capped` (Review Focus 4): 20 prompts, each followed by edits to 3 different files under `/p/src/` (`meta.json` cwd `/p`) → terminal text at 80 wide has exactly 6 `› ` rows, contains `and 14 earlier`, the `review` section has 10 numbered rows and `and 50 more`, every line ≤ 80 wide; `markdown` has 6 nested group bullets and ends the review bullet with `and 50 more`.
  - `panel.rs` `sample_recap`: add `groups: vec![Group { cause: "“fix the token refresh”".into(), files: vec![Changed { path: "src/a.rs".into(), edits: 2, added: 30, removed: 4, look: None }, Changed { path: "tests/a.rs".into(), edits: 1, added: 12, removed: 0, look: Some("test") }] }]`; `recap_box` and `recap_view` snapshots get regenerated (Step 4).
- [ ] **Step 2: Run** `cargo test recap:: panel::` — FAIL.
- [ ] **Step 3: Implement** the section rows in `recap_sections`, `markdown`'s two bullets.
- [ ] **Step 4: Run** `INSTA_UPDATE=always cargo test`; review the `recap_box` and `recap_view` `.snap` diffs: only the `files` section changes and a `review` section appears; the 40-wide `recap_view` still fits. Then `cargo test` — PASS.
- [ ] **Step 5: Commit** `feat(recap): show files by cause and a review order`.

---

### Task 4: Docs, version, real check

**Files:**
- Modify: `README.md` (the `ari recap` block and paragraph under "What you get"), `CHANGELOG.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

- [ ] **Step 1: README.** Replace the `files` lines in the recap example with real output from the multi fixture (`ari recap --session` on a copy, or the `whole_session` text) showing `› task: …` groups and the `review` section. Add two sentences after the block: files are grouped by the task in progress when they were edited, else by your prompt; flagged files (tests, config, lockfiles, CI, migrations, outside the repo) come first in the review order. Add the known limits from Review Focus in one sentence.
- [ ] **Step 2: CHANGELOG** under `## Unreleased`: `- \`ari recap\` groups changed files by the task or prompt that caused them, marks the ones to look at hard (tests, config, lockfiles, CI, migrations, outside the repo) and suggests a review order. \`--md\` and the \`r\` view in \`ari watch\` too.`
- [ ] **Step 3: Version.** Next patch version in `Cargo.toml` and `.claude-plugin/plugin.json`; `cargo check` so `Cargo.lock` follows.
- [ ] **Step 4: Run** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — all pass.
- [ ] **Step 5: Real check.** `cargo build --release`, then in this repo `./target/release/ari recap` and `./target/release/ari recap --md` on a real session: groups read sensibly, `CHANGELOG.md`/`Cargo.toml` show as `config`, snapshot files as `test`.
- [ ] **Step 6: Commit** `docs: recap groups and review order` and `chore: bump version`. Open the PR with a bullet body.

---

## Self-review

- Spec coverage: grouping by prompt or task (Task 2), marks for tests, config, lockfiles plus CI, migrations, outside repo (Task 1), review order (Tasks 1, 3), `--md` (Task 3). The "smallest state change" turned out to be none: the replay in `build` already has prompts and tasks at each edit.
- Types: `Changed { path, edits, added, removed, look }`, `Group { cause, files }`, `look(&str, bool) -> Option<&'static str>`, `review_order(&[Group]) -> Vec<Changed>` used with the same names in Tasks 2 and 3.
- Overlap: the risky-commands plan also adds a section to `recap_sections`; whichever lands second rebases (independent sections, no shared code).

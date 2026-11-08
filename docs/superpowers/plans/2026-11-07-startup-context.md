# Startup Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show how much context was already used before Claude's first reply (system prompt, tools, MCP, CLAUDE.md and memory, plus the first prompt), e.g. `at start 31k (16%)` under the panel's ctx meter and in the hub zoom's context box, marked `▲` when it is a fifth of the window or more.

**Architecture:** The transcript ledger (`tokens::Ledger`) already reads every assistant entry's `usage`. It gains `start`: the input side (input + cache write + cache read) of the first request with a non-zero input side, never overwritten afterwards (unlike `fill["base"]`, which resets after a compaction). One shared formatter in `panel.rs` turns `(start, window)` into the text and a "large" flag; the panel's `usage` section and the zoom's `context` box both call it. Nothing new is read: same files, same incremental reader.

**Tech Stack:** Rust 2024, serde_json, ratatui 0.30, insta. No new crates.

**Spec:** `docs/ROADMAP.md` §7, item 9 ("MCP and CLAUDE.md eat context up front").

## What ARI can and cannot split out

- **Can:** the total. The first assistant entry's `usage` in the main transcript is the first API request's input side: system prompt + tool definitions (built-in and MCP) + CLAUDE.md/memory/skills/git status attachments + the first user message. Checked 2026-11-07 on six local sessions: about 46k each (`input_tokens + cache_creation_input_tokens + cache_read_input_tokens`). The bundled `fixtures/multi/transcript.jsonl` gives 27,592.
- **Includes the first prompt.** ARI can't subtract it; a large pasted first prompt inflates the number. The copy says "at start", not "system".
- **Cannot:** the per-part split `/context` shows (system prompt / system tools / MCP tools / memory files / messages). Claude Code computes it in-process; neither the documented transcript format nor the statusline payload carries it. Local transcripts also hold an undocumented `api-request-shape` record (system text blocks and tool schemas, in characters, not tokens); with deferred tool loading it lists no `mcp__*` tools at all, and CLAUDE.md arrives as an attachment, not in it. It is not used: undocumented, not tokens, and incomplete.
- **Codex and dsh:** Codex's ledger comes from the rollout log's running totals (`codex::ledger`), which has no first request, so `start` stays 0 and nothing shows. dsh has no transcript.
- **Subagents:** each subagent's ledger gets its own `start` (its system prompt + tools + task prompt), so the zoom shows it for any agent with a ledger.

## Global Constraints

- ARI never sends anything to the model, never blocks or answers for the user, nothing leaves the machine.
- Reuse `tokens::read_ledger` / `Ledger::apply`; no second transcript reader.
- `START_LARGE_PCT = 20.0` (percent of the window), a named constant in `src/panel.rs`. Window = `Usage.ctx_size` for the panel and `a.ctx_size.or(i.u.ctx_size)` for the zoom, else `200_000`, the same fallback as `hub::ctx_of`.
- Copy, verbatim: normal `at start 31k (16%)`; large `▲ at start 46k (23%) · system, tools, MCP, CLAUDE.md`. Token format `bar::fmt_tokens`, percent `{:.0}%`. `▲` already maps to `^` in `--ascii`.
- No row when `start == 0`. No config key and no statusline segment.
- CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Snapshot updates: `INSTA_UPDATE=always cargo test`, then review every `.snap` diff; only the new `at start` line may appear.
- One commit per task step that says commit, conventional style with scope (`feat(tokens): …`), ending with the attribution line. Work on branch `startup-context`.

## Review Focus

1. **Caches written by the previous version** (`tokens.json` with `v: 4`, no `start`, `offset` at the end of the transcript): without a version bump `start` would stay 0 forever for running sessions. `CACHE_VERSION` 4 → 5 forces one full re-read. Test in Task 1.
2. **First assistant entry with an empty usage** (`<synthetic>` after an interrupt or API error, all zeros): `start` must come from the first non-zero request, not lock at 0. Test in Task 1.
3. **Compaction, `/compact` or the context shrinking**: `fill["base"]` resets, `start` must not. Test in Task 1.
4. **One response spread over several transcript entries with the same message id** (D14): counted once, so `start` is that request, not a later one. Covered by the existing dedupe; asserted in Task 1.
5. **A narrow panel or short zoom box**: the large copy is long. The panel truncates rows to the column (existing `fit`); the zoom line is clipped to the box width and skipped when the box has no free row. Tests in Tasks 2 and 3.

---

### Task 1: `Ledger.start`

**Files:**
- Modify: `src/tokens.rs` (`Ledger`, `Ledger::apply`, `CACHE_VERSION`, tests)

**Interfaces:**
- Produces:
  - `Ledger.start: u64`: input side of the first counted request with side > 0; 0 when unknown.
  - `pub(crate) const CACHE_VERSION: u32 = 5;`

- [ ] **Step 1: Write the failing tests** in `tokens.rs` tests, with the existing `asst` helper:
  - `start_is_the_first_request_and_survives_shrink`: `asst("m1", 100, 900, &[])`, `asst("m2", 100, 1100, &[])`, `asst("m3", 100, 400, &[])` → `l.start == 1000`, and `fill(&l) == [("base", 500)]` still (base reset, start not).
  - `start_skips_empty_usage`: `asst("m0", 0, 0, &[])` then `asst("m1", 100, 900, &[])` → `start == 1000`.
  - `start_counts_a_repeated_entry_once`: `asst("m1", 100, 900, &[])` twice, then `asst("m2", 100, 1500, &[])` → `start == 1000`.
  - In `update_reads_main_and_agents`: add `assert_eq!(c.main.start, 27_592);`.
  - `old_cache_version_rereads`: write `tokens.json` with `{"v":4,"main":{"offset":<transcript len>}}` into a temp dir, `update(&dir, &State::default(), main.to_str())` with `fixtures/multi/transcript.jsonl` → `c.main.start == 27_592`.
- [ ] **Step 2: Run** `cargo test tokens::` — FAIL.
- [ ] **Step 3: Implement.** In `Ledger::apply`, right after `let side = …`: `if self.start == 0 && side > 0 { self.start = side; }`. Bump `CACHE_VERSION` to 5. `Ledger::add` leaves `start` alone (it sums session totals only).
- [ ] **Step 4: Run** `cargo test` — PASS.
- [ ] **Step 5: Commit** `feat(tokens): remember the context used by the first request`.

---

### Task 2: Panel row under the ctx meter

**Files:**
- Modify: `src/panel.rs` (`usage`, new helper and constant, tests)

**Interfaces:**
- Consumes: `Ledger.start` (Task 1), `Extras.tokens`, `Usage.ctx_size`.
- Produces:
  - `pub const START_LARGE_PCT: f64 = 20.0;`
  - `pub(crate) fn start_note(start: u64, window: u64) -> Option<(String, bool)>`: `None` when `start == 0`; else `(text, large)`, `large = pct >= START_LARGE_PCT`, text per the copy in Global Constraints (`window.max(1)` as divisor).

- [ ] **Step 1: Write the failing tests** in `panel.rs` tests:
  - `start_note_copy`: `start_note(31_000, 200_000) == Some(("at start 31k (16%)".into(), false))`; `start_note(46_000, 200_000) == Some(("▲ at start 46k (23%) · system, tools, MCP, CLAUDE.md".into(), true))`; `start_note(40_000, 200_000).unwrap().1` (exactly 20%) is `true`; `start_note(46_000, 1_000_000).unwrap() == ("at start 46k (5%)".into(), false)`; `start_note(0, 200_000) == None`.
  - `ctx_at_start_row`: `multi()` state and usage with `ctx_pct: Some(40.0)`, `ctx_size: Some(200_000)`, `Extras { tokens: Some(TokenCache { main: Ledger { start: 46_000, .. }, .. }), .. }`, rendered with `lines(...)` at 80×60 → contains `▲ at start 46k (23%)`, and that line comes after the line containing `ctx `. Same with `start: 0` → no `at start`. At 40 wide every line still fits (the render helper's width assert).
- [ ] **Step 2: Run** `cargo test panel::` — FAIL.
- [ ] **Step 3: Implement.** `start_note`; in `usage`, right after `rows.extend(meter("ctx", …))`: when `x.tokens` has `main.start > 0`, push `c.span(format!("    {text}"), style)`: `Style::new().fg(Color::Yellow)` when large, else `c.dim(...)` like the `⟲ compacted` row. Window `u.ctx_size.unwrap_or(200_000)`.
- [ ] **Step 4: Run** `INSTA_UPDATE=always cargo test`, review `.snap` diffs: only an added `at start` line, in snapshots whose `Extras` read a transcript (the demo). Then `cargo test` — PASS.
- [ ] **Step 5: Commit** `feat(panel): context used at start under the ctx meter`.

---

### Task 3: Hub zoom line

**Files:**
- Modify: `src/hub/zoom.rs` (`context`, tests)

**Interfaces:**
- Consumes: `panel::start_note` (Task 2), `hub::ledger`.

- [ ] **Step 1: Write the failing tests** in `zoom.rs` tests, with the existing `with_main`/`kid`/`show` helpers:
  - `zoom_shows_start_context`: main ledger `Ledger { ctx: 60_000, start: 46_000, .. }` in `Extras.tokens.main`, default usage (window 200k) → `show(&st, &x, 0, 0)` has a line containing `▲ at start 46k (23%)`.
  - `zoom_start_for_a_subagent`: ledger for `a1` with `start: 12_000`, `ctx: 30_000` → the zoom on `a1` contains `at start 12k (6%)` and no `▲`.
  - `zoom_no_start_no_line`: the existing `zoom_fill_is_labelled_estimate` ledger (`start` 0) → no `at start`.
- [ ] **Step 2: Run** `cargo test hub::zoom::` — FAIL.
- [ ] **Step 3: Implement.** In `context`, after the note at `inner.y + 1`: when `ledger(i, a)` has `start > 0` and `inner.y + 2 < inner.bottom().saturating_sub(2)` (the estimate footer's rows), `put(buf, x, inner.y + 2, &text, w, style)` with `Style::new().fg(Color::Yellow)` when large, else `dim()`. Window = the `size` already computed at the top of `context`.
- [ ] **Step 4: Run** `INSTA_UPDATE=always cargo test`, review: only the new line in zoom snapshots. Then `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — PASS.
- [ ] **Step 5: Run the real thing.** `cargo install --path . --root ~/.local --force` (`rm ~/.local/bin/ari` first if the copy is killed), start a fresh Claude Code session, send one short prompt, check `ari watch` shows `at start …` under ctx and that the number matches `jq` on the transcript's first assistant `usage` (input + cache write + cache read). Then `x`, `⏎` on main: the same line in the context box.
- [ ] **Step 6: Commit** `feat(hub): context used at start in the zoom`.

---

### Task 4: Docs and version

**Files:**
- Modify: `README.md`, `CHANGELOG.md`, `docs/DECISIONS.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

- [ ] **Step 1: Docs.** README "What you get": one sentence after the badges sentence: under the ctx bar, `at start 31k (16%)` is the context already used before Claude's first reply (system prompt, tools, MCP servers, CLAUDE.md and your first message), marked `▲` from 20% of the window. README hub paragraph: the zoom's context box shows it too. DECISIONS.md, next free D-number: the bullets from "What ARI can and cannot split out" in two lines, plus the 20% threshold. CHANGELOG `## Unreleased`: one line, **Context at start.** … Commit `docs: context used at start`.
- [ ] **Step 2: Version bump.** Next patch after the version on `main` (e.g. `0.3.1` → `0.3.2`) in `Cargo.toml`, `.claude-plugin/plugin.json`, and `Cargo.lock` (`cargo build` refreshes it). Commit `chore: bump version to <x.y.z>`.
- [ ] **Step 3: Finish.** Full CI commands pass; PR with a bullet body.

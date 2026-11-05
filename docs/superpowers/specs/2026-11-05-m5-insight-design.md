# M5: Insight — design

**Milestone done when:** `ari recap` summarizes a session from fixtures, and time split, test history, file conflicts, compaction forecast and mistake cost are fixture-tested and shown in the panel.

## Intent

Other Claude Code tools list sessions (Agent View, agenthud) or count tokens (ccusage, Claude-Code-Usage-Monitor). None of them says what a session *accomplished*, where its time went, or what is about to go wrong. ARI already holds the raw material in `events.jsonl`, `usage.json`, `limits.jsonl` and the transcripts. M5 turns that into answers, still at zero tokens: no LLM call anywhere, unlike agenthud's digest.

Researched 2026-11-05; the gap list and sources are in the conversation that produced this spec and in `docs/ROADMAP.md`.

## Goals

1. **`ari recap`:** a plain-text (or Markdown) summary of what a session completed: tasks done, errors hit and fixed, tests before → after, files edited, commits, time split, cost. Works for the whole session or a window (`--since 30m`, `--since prompt`).
2. **Time split:** where wall-clock time went: Claude (API), tools, waiting on you, idle.
3. **Test history:** passed *and* failed counts per run, a trend, and "DONE but tests failing".
4. **File conflicts:** warn when two live sessions edited the same file recently.
5. **Compaction forecast:** "compacts in ~5 turns", from context growth per turn.
6. **Mistake cost:** time and calls spent on failed tool calls, retries and redundant re-reads.
7. **Model per agent:** which model each agent is running on, shown next to what it's doing.

## Non-goals

- No LLM-written summaries; recap is assembled from data.
- No transcript viewer, session launcher or web UI. Bigger tools do those; they pull ARI away from a small zero-token panel.
- **Other harnesses (Codex, opencode, …) and per-provider branding are M6**, designed separately. M5 keeps that door open: model names are shown as given, never hardcoded to Claude.
- No per-test-name tracking (flaky detection) yet; counts only.
- Hook path stays as is: one parse, one append. New work happens in `statusline`, `watch` and `recap`.

## Design

### Recap (`src/recap.rs`, `ari recap`)

```
ari recap [--session <id>] [--since <30m|2h|prompt|start>] [--md]
```

- Session selection: the same as `ari watch` (follow cwd, or `--session`).
- Window: `start` (default) = whole session; `prompt` = since the last `UserPromptSubmit`; a duration = since now − d.
- **Method:** replay `events.jsonl` up to the window start → `before: State`; full replay → `after: State`. Recap is the diff. Reuses `replay::load` and `State::apply`; nothing new is stored by hooks.
- **Sections**, each omitted when empty:
  - header: project dir (`~/ari`), session title, window, duration, verdict now
  - `done`: tasks that became `completed` in the window (subjects)
  - `still open`: in-progress and pending tasks
  - `errors`: N hit, M fixed (with `fixed by`), K still open (signatures)
  - `tests`: last run before the window → last run in it (`58 → 71 passed, 0 failed`)
  - `files`: edited files, most-edited first, top 8 + "and N more" (from Edit/Write/MultiEdit/NotebookEdit `sig` paths, relative to the session cwd)
  - `commits`: `git log --oneline --since=<window start>` in the session cwd, max 10; skipped if not a repo or git is missing (1 s timeout)
  - `time`: time split for the window (below)
  - `cost`: cost and tokens in the window = last − first `limits.jsonl` sample in it (needs `cost` added to samples); whole session uses `usage.json`
  - `lines`: `+156 −23` (whole session only; the payload has no per-window lines)
- `--md` prints the same content as Markdown bullets for pasting into a PR.
- Panel: key `r` toggles a recap view (window `prompt`) in place of the sections; `r` again or `esc` returns.

### Time split (`State`)

Folded in the reducer from existing events:

- **tools:** Σ `dur` of `PostToolUse` / `PostToolUseFailure`, main agent only (subagents run in parallel and would double-count).
- **waiting on you:** from a `Notification` of kind `permission_prompt` or `elicitation_*` until the next event of that agent.
- **idle:** from `Stop` until the next `UserPromptSubmit`.
- **Claude:** `api_duration_ms` from `usage.json` when known; else wall time − the other three.
- Panel session section: `time  42m · claude 61% · tools 22% · you 9% · idle 8%`. Recap uses the window's numbers. The window needs per-event folding, so the split counters live in `State`; the reducer is replayed for the window.

### Test history (`event.rs`, `State`)

- Extend the parser to failed counts: `N failed` on the same summary lines (cargo, pytest, jest, vitest). `Event.pass` becomes `Event.tests: Option<(u32 /*pass*/, u32 /*fail*/)>`.
- `State.test_runs: VecDeque<TestRun { t, pass, fail }>`, cap 20; `tests_passed` is replaced by the last run.
- Bar: `✓ 71 tests`, or `✗ 3/74 tests` in red when the last run failed.
- Panel session section: `tests  ✓ 71 · ▁▃▅▆▇▇ last 6 runs` (sparkline of pass count; red cell for runs with failures).
- **Verdict:** `DONE` whose last test run in the session failed shows `DONE · tests failing` and uses the warning color. It is still DONE (the turn ended), not BLOCKED.
- Bump `CACHE_VERSION` to 3.

### File conflicts (`src/conflicts.rs`)

- Per session, the set of files edited in the last 30 minutes: `State.edits: BTreeMap<path, last_t>` from the `sig` of Edit-family events (absolute paths), pruned to 30 min on read.
- `conflicts(me, live_sessions) -> Vec<(path, other_session_label)>`: paths in my set also in another live session's set.
- Computed by `ari watch` every refresh, and by `ari statusline` at most every 5 s (result cached in `conflicts.json` in the session dir, like `tokens.json`).
- Bar: `⚠ src/main.rs also in tankobon` (first conflict, `+N` if more) right after the verdict. Panel: an `⚠ conflict` line under the header. `--all`: a `⚠` marker on both rows.

### Compaction forecast (`burn.rs`)

- `limits.jsonl` samples gain `ctx` (percent) and `cost` (USD), both `Option`, serde default; old lines still load.
- Turns come from `UserPromptSubmit` times. Growth per turn = median of ctx deltas across the last 5 turns, skipping any turn that contains a `PreCompact`/`PostCompact`.
- Auto-compact point: `thresholds.compact_pct` in config, default 90. The builder checks this against `PreCompact` events in local recordings and records the observed value and source in DECISIONS.
- Shown when ≤ 10 turns remain: panel usage section `ctx … 71% · compacts in ~4 turns`; bar `ctx ▰▰▰▰▱ 71% ~4t`.

### Mistake cost (`State`)

- Per error signature (already in `ErrorRec`): add `time_ms` (Σ `dur` of its failures).
- Redundant reads: a `Read` of a path already read by the same agent with no Edit-family event on that path between. Counted in `State.reread: u32`.
- Panel errors section footer: `cost of mistakes: 14 failed calls · 3m10s · 9 re-reads`. Recap includes the same line. No token attribution: it would need per-call transcript matching, and an estimate is worse than none.

### Model per agent (`tokens.rs`, panel)

- `Ledger` already counts `by_model`; add `last_model: Option<String>` set from each counted assistant entry's `message.model`.
- Short name for display: strip a `claude-` prefix and a trailing date (`claude-opus-5-5-20261001` → `opus-5-5`); other names pass through unchanged.
- Panel agent rows: `⠹ main  opus-5-5 · editing src/bar.rs · 3.3M tok`; subagents likewise. Compact bar unchanged (the payload's model is already in the panel header area).
- Recap: `models: opus-5-5 (main), haiku-5-5 (2 agents)`.

## Testing

- Recap: fixture sessions → exact text for `start`, `prompt` and duration windows; `--md`; no git repo; empty window ("nothing happened since …").
- Time split: hand-built event sequences with known durations for each bucket; subagent tool time not double-counted.
- Tests: parser cases for pass/fail per runner; history cap; DONE + failing verdict text; old `state.json` (v2) rebuilds.
- Conflicts: two temp session dirs editing the same file → both report; edit older than 30 min → none; ended session ignored.
- Forecast: steady growth → turns estimate; a compaction inside the window is skipped; old `limits.jsonl` lines without `ctx` load.
- Mistakes: retries add time; read → read flagged; read → edit → read not flagged.
- Models: fixture transcripts with two models → each agent row shows its own; name shortening cases.
- Statusline p95 stays < 50 ms (bench, as M3).

## Decided

- One milestone, branch `m5-insight`, one commit per task (git workflow rule).
- Recap default window is the whole session ("what was completed"); `--since prompt` is the "while you were away" view.
- No LLM, no new dependencies.

## Left to the builder

Exact wording and colors; section order inside recap; sparkline glyphs in ASCII mode (map through `ascii.rs`); the conflict window (30 min) and forecast cut-off (10 turns) may move if fixtures suggest better values. Record any change in DECISIONS.

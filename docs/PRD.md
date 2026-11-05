# ARI: All Relevant Information

**A zero-token status panel for Claude Code that answers "how far along are we, and is anything wrong?" so you never have to interrupt Claude to ask.**

Author: Finn · Status: Draft v1 · Build target: Claude Code

> **Note to Claude Code:** Claude Code's hook events and statusline JSON fields change between versions. Before implementing any collector, verify every event name and payload field in this PRD against the current docs (https://code.claude.com/docs/en/hooks and https://code.claude.com/docs/en/statusline). Where this PRD and the docs disagree, the docs win; note the discrepancy in `docs/DECISIONS.md`.

---

## 1. Problem

During long Claude Code runs (multi-step tasks, parallel subagents) the user can't easily tell:

- How far through the work Claude is
- What each subagent is doing right now
- Whether something failed, and whether it got fixed
- Whether Claude is stuck in a loop or waiting on a permission prompt
- How close they are to context compaction or rate limits

So users type "how's it going?" or "what are you doing?", which interrupts the run, burns tokens, and pollutes context. Existing statusline mods show raw numbers (model, cost, context %). None interpret what is happening.

## 2. Solution

ARI watches Claude Code passively through hooks and the statusline, builds a live picture of the session, and shows it two ways:

1. **Compact bar** in Claude Code's statusline: one line, always visible.
2. **Full panel** (`ari watch`) in a separate terminal pane: a TUI dashboard with agents, errors, usage, and a health verdict.

ARI never talks to Claude. It adds **zero tokens** to the session.

### Selling points (these drive the README)

1. **Interpretation, not just numbers.** A one-word health verdict (`ON TRACK`, `NEEDS YOU`, `STUCK`...), stuck-loop detection, and errors tracked through to resolution.
2. **Deeper info.** Per-subagent current action, tool calls, and token spend; cache savings in dollars; rate-limit burn-rate projection.
3. **Charming, clean presentation.** Box-drawing UI, braille spinners, segmented bars, and a tiny status face. It should feel crafted, not cluttered.
4. **Plays nice.** Chains existing statuslines (e.g. ponytail) instead of replacing them. Injects nothing into context.

## 3. Goals and non-goals

### Goals
- Answer "how far along?" and "any errors?" at a glance, with zero interaction
- Install to first render in under 2 minutes
- Zero added tokens; negligible hook overhead
- Coexist with other plugins and statuslines
- Be demo-able without running Claude (`ari demo`) for README GIFs and trying it out

### Non-goals (v1)
- Controlling or steering Claude (no blocking hooks, no context injection)
- Time ETAs for tasks (unreliable; show counts instead)
- Web UI, cloud sync, or any telemetry
- Milestones declared by Claude or other plugins; v1 derives progress only from Claude Code's own task list
- Windows (target v1.1; don't make design choices that block it)

## 4. Users

- **Primary:** Claude Code users who run long or agentic sessions and watch from a second pane or monitor.
- **Secondary:** Users with existing statusline setups who want more without losing what they have.

---

## 5. Architecture

```
 Claude Code
   │  hooks (stdin JSON)              statusline (stdin JSON, frequent)
   ▼                                  ▼
 ari hook <event>                   ari statusline
   │ append 1 line                    │ 1. run chained commands, pass stdin through
   ▼                                  │ 2. write usage snapshot (atomic)
 events.jsonl  ◄──────────────────────┤ 3. reduce state, print compact bar
   │                                  ▼
   └──────────► reducer ◄────── usage.json
                  │
                  ▼
             ari watch (TUI panel)
```

- **Single Rust binary `ari`** with subcommands. Rust for startup speed (hooks run synchronously in Claude's loop) and `ratatui` for the panel.
- **Hooks only append.** All interpretation happens in the reducer, which runs inside `ari statusline` and `ari watch`. Hooks never parse transcripts or do heavy work.
- **State location:** `~/.local/state/ari/sessions/<session_id>/` (respect `XDG_STATE_HOME`; use the platform equivalent on macOS). Never write into the user's project directory, so nothing has to be gitignored.
  - `events.jsonl`: append-only event log
  - `usage.json`: latest statusline payload, written via temp file + rename
  - `meta.json`: cwd, start time, last activity
- **Concurrency:** parallel subagents mean concurrent hook writes. Each event is one line written with a single `O_APPEND` write, kept under 4 KB. Truncate long fields.
- **Incremental reduce:** the reducer caches state plus a byte offset into `events.jsonl`, so each statusline render only reads new lines.
- **Retention:** prune session dirs older than 7 days (configurable) on `SessionStart`.

## 6. Data sources

### 6.1 Hooks (registered by the plugin's `hooks/hooks.json`)

| Event | Used for |
|---|---|
| `SessionStart` | Create session dir, prune old sessions |
| `UserPromptSubmit` | Mark turn start; clear "needs you" |
| `PreToolUse` | "Now doing" text per agent; loop detection |
| `PostToolUse` | Success/failure per call; task-list updates (TaskCreate/TaskUpdate, TodoWrite) |
| `PostToolUseFailure` (verify it exists) | Failed tool calls |
| `SubagentStart` / `SubagentStop` | Agent lifecycle, agent_id, agent_type, transcript path, final message |
| `Notification` | Permission prompts and idle waits → `NEEDS YOU` |
| `PreCompact` | Compaction marker |
| `Stop` / `StopFailure` | Turn end; error ending a turn |
| `SessionEnd` | Close session |

**Hook rules (non-negotiable):**
- Always exit 0. Never print to stdout. (Stdout from `SessionStart` and `UserPromptSubmit` is added to Claude's context.) Never return decisions or JSON.
- p95 runtime under 15 ms. Set a short `timeout` in hooks.json.
- If the `ari` binary is missing, the hook command must fail silently (e.g. `command -v ari >/dev/null && ari hook pre_tool || true`).

**Agent attribution:** check whether tool-event payloads inside subagents carry `agent_id`. If yes, attribute directly. If not, fall back to tailing each subagent's transcript (path from `SubagentStart`/`SubagentStop`) inside `ari watch`, never inside hooks.

**Robustness:** handle orphan events (a stop with no start, a start with no stop). Mark an agent `stale` after 5 minutes of silence rather than leaving it "running" forever.

### 6.2 Statusline payload

Read from stdin on every statusline render:
- `context_window.used_percentage`, `context_window_size`, `current_usage` (input, output, cache creation, cache read)
- `rate_limits.five_hour` / `seven_day`: `used_percentage`, `resets_at` (Pro/Max only; **may be absent**: show `—`, never 0%)
- `cost.total_cost_usd`, `cost.total_duration_ms`, `cost.total_api_duration_ms`, lines added/removed
- `model.display_name`, `session_id`, `transcript_path`

### 6.3 Transcripts

Session totals come from summing `usage` on every assistant message in the transcript JSONL. Do not use the statusline token fields for totals: they became per-response (not cumulative) in v2.1.132. Same method per subagent transcript for per-agent tokens. Parse incrementally with cached offsets, in `ari watch` and lazily in `ari statusline` (debounced, max once per 2 s).

---

## 7. Features

### 7.1 Health verdict

One state, evaluated in this priority order:

| Verdict | Glyph | Rule |
|---|---|---|
| `NEEDS YOU` | `◉` | Pending permission/idle notification not yet cleared by a later tool use or prompt |
| `BLOCKED` | `✗` | `StopFailure` on the latest turn, or the turn ended with unresolved errors |
| `STUCK` | `↻` | Loop detected (7.4) |
| `RETRYING` | `↻` | Open errors, still working |
| `ON TRACK` | `●` | Active, no open errors |
| `DONE` | `✓` | Turn stopped and all tasks completed |
| `IDLE` | `◌` | Turn stopped, tasks incomplete or none |

`NEEDS YOU` is the loudest state: color flip, and an optional terminal bell (`bell = true`).

### 7.2 Task progress (milestone %)

- Source: Claude Code's task list only, captured from task-tool calls in `PostToolUse`. Store the latest full list.
- Show `completed / total` with a bar, plus the `in_progress` item as `▸ now: ...`.
- Milestone label: the session's first user prompt, truncated, unless the task list provides a better title.
- No task list yet: hide the bar and show `no plan yet`. Never fake progress.

### 7.3 Errors with resolution

- Record failures: failed tool results (e.g. non-zero Bash exit), failure hooks, `StopFailure`.
- Each error gets a **signature**: tool + normalized key (Bash: first two command tokens, e.g. `cargo build`; file tools: path).
- An error is **resolved** when a later call with the same signature succeeds. Show `↻ retried N× · fixed`.
- Panel shows the last 5 errors with age and a one-line message (first meaningful line of stderr).

### 7.4 Stuck detection

Flag `STUCK` when any of these hold:
- The same (tool, input hash) appears 3+ times in the last 10 calls
- 3+ consecutive failures with the same signature
- An edit → run → fail cycle repeats 3+ times on the same file/command pair

Show why: `↻ STUCK · cargo build failed 4× in a row`.

### 7.5 Agents

Per agent (main plus subagents, as a tree):
- Spinner while active (braille frames); `✓` done; `✗` failed; `◌` stale. A spinner that freezes is itself a signal.
- Type/name, current action (`editing src/auth/session.rs`, `grep refresh_token`), elapsed time, tool-call count, tokens
- On finish: duration plus a one-line result from the final message

### 7.6 Usage

- **Context:** bar with green → yellow (≥60%) → red (≥80%) thresholds, plus `142k/200k`. `PreCompact` adds `⟲ compacted 12:20`.
- **Rate limits:** 5h and 7d bars with reset countdown (`↺ 1h40m`).
- **Burn-rate projection:** linear fit of 5h `used_percentage` over the last 15 minutes of samples. Show `▲ at this pace: 5h limit in ~55m` **only** when projected exhaustion comes before `resets_at`. Otherwise show nothing.

### 7.7 Session stats

- Total tokens and output tokens (from transcripts), turn count
- Cache hit % = cache_read / (input + cache_creation + cache_read), per turn and per session
- **Cache savings estimate** in dollars, from a per-model price table in config (cache reads priced at a fraction of input). Label it `≈`.
- **Cache-drop alert:** flag when the per-turn hit rate falls sharply (e.g. from above 70% to below 30%), usually caused by compaction or a changed prompt
- Active vs wall time: `total_api_duration_ms` vs `total_duration_ms`, as a percentage
- Cost per completed task

### 7.8 Statusline chaining

- `ari setup` reads the existing `statusLine.command` from `~/.claude/settings.json`, saves it to ARI's config as a chained command, and sets `statusLine` to `ari statusline`. It backs up settings.json first and shows a diff before writing.
- At render time, ARI reads stdin once, runs each chained command with the same stdin and environment, cwd unchanged, and a 300 ms timeout. It prints their output untouched, then its own row (`position = "top" | "bottom"`).
- A chained command that errors or times out is skipped silently. It must never break ARI's row.
- `ari uninstall` restores the original command exactly.
- `hide = [...]` drops compact segments already shown by a chained statusline. Setup pre-fills `["cost", "tokens", "time", "cache"]` when it finds a chained command. The full panel always shows everything.
- Must be tested against ponytail specifically.

---

## 8. UI spec

### 8.1 Compact bar

```
ARI ● ON TRACK │ ███████░░░ 4/7 │ ⠼ 2 agents │ ✗ 0 │ ctx ▰▰▰▰▱ 71% │ 5h 62% ↺1h40m
```

Segments in priority order: verdict, tasks, agents, errors, ctx, 5h, 7d, cost, tokens. When width is tight (use `COLUMNS` or `max_width` config), drop segments from the end. The verdict is never dropped.

### 8.2 Full panel (`ari watch`)

```
╭─ ARI (•_•) ────────────────────────── 12:41 ─╮
│ ● ON TRACK          milestone: auth refactor │
│ ███████████░░░░░░░░░ 4/7 tasks               │
│ ▸ now: hook up token refresh                 │
├─ agents ─────────────────────────────────────┤
│ ⠹ main      editing src/auth/session.rs      │
│ ├─⠼ explore grep refresh_token · 14 calls    │
│ └─✓ tests   done 38s · 12 passed             │
├─ errors ─────────────────────────────────────┤
│ ✗ 2m  cargo build · E0382 use of moved value │
│       ↻ retried 2× · fixed by main           │
├─ usage ──────────────────────────────────────┤
│ ctx  ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▱▱▱▱▱▱  71%  142k/200k    │
│ 5h   ▰▰▰▰▰▰▰▰▰▰▰▰▱▱▱▱▱▱▱▱  62%  ↺ 1h40m      │
│ 7d   ▰▰▰▰▰▰▰▱▱▱▱▱▱▱▱▱▱▱▱▱  34%  ↺ 4d         │
│ ▲ at this pace: 5h limit in ~55m             │
├─ session ────────────────────────────────────┤
│ tokens  6.2M  ·  26.8k out  ·  2 turns       │
│ cache   ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▱▱▱ 86% hit         │
│         saved ≈ $11.40 vs uncached           │
│ time    9m54s active / 14m wall · 71%        │
│ cost    $2.69 · $0.67/task done              │
╰──────────────────────────────────────────────╯
```

- **Face** in the header reflects the verdict: `(•_•)` on track, `(◉_◉)` needs you, `(>_<)` errors/stuck, `(^_^)` done, `(-_-)zz` idle. Toggle with `face = false`.
- **Responsive:** sections collapse in priority order on short terminals (session first, then usage detail). It must render cleanly at 40 columns.
- **Session picker:** `ari watch` follows the most recently active session for the current directory. `ari watch --session <id>` or `ari watch --all` shows a compact list of every live session.
- **Keys:** `q` quit, `e` expand errors, `a` expand agents, `tab` cycle sessions. Keep it minimal.
- **Accessibility:** `--ascii` fallback (`[####----]`, `|/-\` spinner, `+-|` boxes) for terminals without Unicode; respect `NO_COLOR`; never rely on color alone (glyphs carry meaning).
- Render at most 10 fps; idle at 1 fps.

---

## 9. CLI

| Command | Purpose |
|---|---|
| `ari setup` | Wire statusline (with chaining), verify hooks, print next steps |
| `ari uninstall` | Restore original statusline; optionally delete state |
| `ari watch` | Full TUI panel |
| `ari statusline` | Statusline entry point (called by Claude Code) |
| `ari hook <event>` | Hook entry point (called by Claude Code) |
| `ari doctor` | Check binary on PATH, hooks registered, statusline wired, chain health, last event age |
| `ari demo` | Replay a bundled fixture session in the panel. No Claude needed. Used for README GIFs and first impressions. |
| `ari replay <dir>` | Replay a recorded session (testing/debugging) |

Plugin slash commands: `/ari:setup` and `/ari:doctor`, which instruct Claude to run the matching CLI command.

## 10. Config

`~/.config/ari/config.toml`, all optional:

```toml
position = "top"            # ARI row above or below chained statuslines
hide = ["cost", "tokens"]   # compact-bar segments to omit
face = true
bell = false
ascii = false
max_width = 0               # 0 = auto
retention_days = 7

[chain]
commands = ["<previous statusLine command>"]
timeout_ms = 300

[thresholds]
ctx_warn = 60
ctx_crit = 80
stuck_repeats = 3

[prices]  # per million tokens, used for cache-savings estimate only
# model_id = { input = 0.0, output = 0.0, cache_read = 0.0, cache_write = 0.0 }
```

## 11. Distribution

- **Claude Code plugin** in this repo: `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `hooks/hooks.json`, `commands/`. Users install with `/plugin marketplace add finneritter/ari` then `/plugin install ari@ari`.
- **Binary** via `cargo-dist` (dist): GitHub Releases, a shell installer one-liner, Homebrew tap, and `cargo install ari-cli` (check crate-name availability).
- Install flow must be exactly: install binary → install plugin → `ari setup`. The README shows it as three copy-paste lines.
- **Open decision:** whether to bundle prebuilt binaries inside the plugin so it works with no separate install. Evaluate the size and update tradeoff and record it in `docs/DECISIONS.md`.

## 12. Performance and quality bars

| Metric | Target |
|---|---|
| Hook p95 runtime | < 15 ms |
| Statusline render p95 (excluding chained commands) | < 50 ms |
| Tokens added to session | 0 |
| Panel CPU when idle | < 1% |
| Install to first render | < 2 min |

**Testing:**
- Record real sessions into `fixtures/` (events + usage + transcripts, scrubbed of private content). The reducer is a pure function tested against fixtures with snapshot tests (`insta`).
- Unit tests for verdict rules, error signatures/resolution, loop detection, and burn-rate math.
- Integration test: a fake chained statusline (slow, failing, multi-line) to prove chaining isolation.
- CI on macOS and Linux: fmt, clippy, tests.

## 13. Milestones

| # | Scope | Done when |
|---|---|---|
| M0 | Repo skeleton, `ari hook` writing events, `ari replay`, fixtures | A real session is captured and replayable |
| M1 | `ari statusline` compact bar + chaining + `ari setup`/`uninstall` | Works alongside ponytail with no visual or behavioral change to ponytail |
| M2 | `ari watch` panel: verdict, tasks, agents, errors, usage | Panel answers "how far / any errors" on a real multi-agent run |
| M3 | Intelligence: stuck detection, error resolution, burn rate, cache stats, per-agent tokens | Covered by fixture tests |
| M4 | Launch polish: `ari demo`, `--ascii`, `ari doctor`, README, GIFs, dist releases | Stranger can install and see value in under 2 min |

**Ship M1 publicly as soon as it works.** An early working release with a good README beats a perfect late one.

## 14. Launch plan (the point of the project)

### README (the most important file in the repo)
1. Name + one-line tagline: *"Stop asking Claude how it's going."*
2. **Hero GIF at the very top**, recorded from `ari demo` with `vhs` (tape file committed in `docs/demo.tape` so it's reproducible). Show a subagent finishing, an error getting fixed, and the verdict changing.
3. Three-line install.
4. "What you get" with a screenshot of the full panel and the compact bar.
5. "Why ARI" as a short feature table: verdict, stuck detection, error resolution, per-agent tokens, cache savings, chaining, zero tokens.
6. Config reference, FAQ ("Does it slow Claude down?" "Does it cost tokens?" "Will it break my statusline?").
7. License (MIT), contributing guide.

### Repo hygiene
- GitHub description + topics: `claude-code`, `claude-code-plugin`, `statusline`, `tui`, `ratatui`, `developer-tools`
- Social preview image (the panel screenshot)
- Tagged releases with changelogs
- Issue templates; a few `good first issue` items (themes, new segments)

### Where to post (after M1, again after M4)
- r/ClaudeAI and r/ClaudeCode with the GIF
- Show HN: "ARI: a zero-token status panel for Claude Code"
- X/Bluesky with the GIF
- PRs to awesome-claude-code style lists
- Claude Code plugin marketplaces/directories

## 15. Open questions

1. Bundle binaries in the plugin, or require a separate install? (11)
2. Do subagent tool events include `agent_id`? (6.1) This decides direct attribution vs transcript tailing.
3. Default compact-bar position relative to chained statuslines: top or bottom?
4. Should `ari watch` offer a tmux auto-split helper (`ari watch --split`)?

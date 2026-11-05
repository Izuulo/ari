# M1: Compact bar, chaining, setup — design

**Milestone done when:** ARI's row shows in Claude Code's statusline alongside ponytail, with no visual or behavioral change to ponytail. Ship publicly as soon as it works (PRD §13).

## Intent

The first thing a user sees. One line that answers "how far along, anything wrong?", added to whatever statusline they already have, never replacing or breaking it.

## Goals

- **Reducer** (`src/state.rs`): pure, incremental fold of events into session state. Verdict (PRD §7.1, minus `STUCK`, which arrives in M3), tasks (§7.2), agents (§7.5, counts and current action), errors with signatures and resolution (§7.3, moved here from M3, see DECISIONS).
- **`ari statusline`**: read stdin once → run chained commands → write `usage.json` atomically → reduce → print chained output untouched plus ARI's row (`position` top/bottom).
- **Compact bar** (§8.1): segments in order verdict, tasks, agents, errors, ctx, 5h, 7d, cost; drop from the end to fit `max_width` or `COLUMNS`; verdict never dropped; `hide` respected; `NO_COLOR` respected.
- **Config** (`~/.config/ari/config.toml`, §10): every key optional.
- **`ari setup` / `ari uninstall`** (§7.8) plus `/ari:setup` slash command.
- Statusline render p95 < 50 ms excluding chained commands.

## Non-goals (M1)

Panel (M2). Stuck detection, burn rate, cache stats, transcript token totals (M3). The `tokens` segment ships in M3. `--ascii` and `doctor` are M4.

## Design

### State and reducer

`State::apply(&mut self, ev: &Event)` is the only mutation; the verdict is `State::verdict(&self, now_ms) -> Verdict`, computed, never stored. Cached as `state.json` = `{v, offset, state}` in the session dir. Each render reads `events.jsonl` from `offset` up to the last `\n` (a hook may be mid-write), applies, and saves via temp + rename. A cache with an unknown `v` is discarded and rebuilt from byte 0.

State tracks:
- **Turn:** `active` from a real `UserPromptSubmit` until `Stop` with `bg == 0`. `Stop` with `bg > 0` keeps it active (DECISIONS D12). `StopFailure` ends the turn as failed. Prompts starting with `<task-notification>` resume activity but don't count as turns (D15). `turns` count. `label` = first real prompt.
- **Needs you:** set by `Notification` with kind in D5's list; cleared by any later tool event, prompt, or `Stop`.
- **Tasks:** ordered list by first sight, keyed by id, from `task` events (`deleted` status removes). A `TodoWrite` `todos` list replaces the whole list.
- **Agents:** `main` plus one per `agent_id`, created on first sight (orphan-safe). Per agent: type, started, last event time, calls (count of `PreToolUse`), current action (last `act`), done flag, result (`SubagentStop` msg), transcript path. Running agents silent for 5 min render as stale.
- **Errors:** each failure (`ok == false`, not `kind == interrupt`) gets the event's `sig`. A failure whose `sig` matches an unresolved error bumps its retry count and message; otherwise it's a new error. A later success with the same `sig` resolves it (records resolver agent). `StopFailure` adds error `StopFailure:<kind>`, resolved by the next normal `Stop`.
- **Recent results:** a ring of the last 10 tool results (ok/fail + sig), for the verdict.

### Verdict (M1)

In order: `NEEDS YOU` → `BLOCKED` (turn ended by `StopFailure`, or the turn ended right after a failure that is still unresolved, i.e. the last main-agent tool result was that failure) → `RETRYING` (active, with an unresolved error among the last 10 tool results) → `ON TRACK` (active) → `DONE` (ended, ≥1 task, all completed) → `IDLE`.

**My call (recorded as D16):** the PRD says "turn ended with unresolved errors" → `BLOCKED` and "open errors" → `RETRYING`. Taken literally, one harmless failed `grep` early in a turn would keep the whole turn `RETRYING`, then mark it `BLOCKED`. ARI therefore only lets *recent* unresolved errors (last 10 results, or the turn's final result) drive the verdict. Older ones still count in the panel's error list.

### Compact bar

`ARI ● ON TRACK │ ███████░░░ 4/7 │ ⠼ 2 agents │ ✗ 1 │ ctx ▰▰▰▱▱ 71% │ 5h 62% ↺1h40m │ 7d 34% ↺4d │ $2.69`

- tasks: 10-cell `█░` bar + `done/total`; no tasks → `no plan yet`.
- agents: braille spinner frame from wall clock + `N agents` (running, non-stale subagents); omitted when 0.
- errors: `✗ N` unresolved errors in the current turn.
- ctx: 5 cells `▰▱` + `%`; green < `ctx_warn` ≤ yellow < `ctx_crit` ≤ red.
- 5h / 7d: `%` + `↺` countdown to `resets_at` (epoch seconds); absent → `5h —`, never `0%`.
- cost: `$x.xx` from `cost.total_cost_usd`.
- Glyph always paired with the word: never color alone. `NEEDS YOU` uses reverse video. `NO_COLOR` (any value) → no ANSI.
- Width: `max_width` if > 0, else `COLUMNS`, else 120. Visible width = chars of the text without ANSI.

### Chaining

Each `chain.commands` entry runs via `sh -c` with the same stdin bytes, inherited env and cwd, concurrently, with `chain.timeout_ms` (300). Output is printed byte-for-byte, minus one trailing newline. Non-zero exit, timeout, or spawn failure → skipped silently. A command containing `ari statusline` is skipped (no recursion).

### Setup / uninstall

- Settings path: `$CLAUDE_CONFIG_DIR/settings.json`, else `~/.claude/settings.json`.
- `ari setup [--yes] [--dry-run]`: if `statusLine` already runs `ari statusline` → say so, exit 0. Otherwise save the full original `statusLine` object (or `null`) to `<config>/original-statusline.json`; if it had a `command`, put it in `chain.commands` and pre-fill `hide = ["cost", "tokens", "time", "cache"]`. New `statusLine` keeps the original's other keys (`padding`, …), sets `type: "command"`, `command: "ari statusline"`, and adds `refreshInterval: 2` only if absent (D9). Print a `-`/`+` diff of `statusLine` and the config; ask `y/N` unless `--yes`; `--dry-run` prints and exits. Back up settings to `settings.json.ari-backup-<epoch>` before writing. Writes keep the file's key order (`serde_json` `preserve_order`).
- `ari uninstall [--yes] [--purge]`: restore the saved object exactly (or remove `statusLine` if it was `null`); `--purge` also deletes ARI's state and config dirs.
- `/ari:setup`: `commands/setup.md` tells Claude to run `ari setup --dry-run`, show it, and run `ari setup --yes` only after the user agrees.

### Config

`toml` crate. Missing file, bad TOML or unknown keys never break the statusline: fall back to defaults. `retention_days` is read by the hook on `SessionStart` only.

## Testing

- Reducer: unit tests per verdict rule; replay `fixtures/basic` through it and snapshot the verdict timeline.
- Chaining integration test: a fake chain of a ponytail-style ANSI badge, a multi-line script, a `sleep 2` script, and an `exit 1` script. Output contains the first two byte-for-byte, nothing from the last two, and returns in well under 1 s.
- Setup: run against a temp `CLAUDE_CONFIG_DIR`; setup then uninstall leaves `settings.json` semantically identical to the original.
- Manual: real ponytail `ponytail-statusline.sh` output direct vs through ARI, byte-compare.

## Left to the builder

Exact colors, spinner frames, diff formatting.

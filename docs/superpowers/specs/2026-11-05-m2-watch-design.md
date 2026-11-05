# M2: `ari watch` panel — design

**Milestone done when:** the panel answers "how far along / any errors?" on a real multi-agent run.

## Intent

The second-pane view for people who watch long agentic runs: everything the compact bar says, plus *which* agent is doing *what*, which errors happened and whether they got fixed, and how close the limits are. Glanceable, crafted, never cluttered (PRD §2, §8.2).

## Goals

- `ari watch` full-screen TUI (ratatui + crossterm), layout per PRD §8.2: header with face and clock; verdict + milestone; task bar + `▸ now`; agents tree; errors with resolution; usage bars; session time and cost.
- Session selection: follow the most recently active session whose cwd is the current directory; `--session <id>`; `--all` lists every live session; `tab` cycles sessions.
- Keys: `q` quit, `e` expand errors, `a` expand agents, `tab` cycle. Nothing else.
- Responsive: collapse sections on short terminals (session first, then usage, then errors are trimmed); renders cleanly at 40 columns; never wraps.
- At most 10 fps while something animates; 1 fps when idle; idle CPU < 1%.
- `bell = true` rings once on each transition into `NEEDS YOU`.
- `ari watch --once` prints the panel to stdout once (for tests, screenshots, scripts).

## Non-goals (M2)

Token totals, cache stats, burn rate, `STUCK` (M3 — the panel leaves their rows out until then). `--ascii` (M4). `--split` tmux helper (open question 4: not doing it; `tmux split-window ari watch` already works).

## Design

### Modules

- `sessions.rs`: `list(root) -> Vec<SessionInfo { id, dir, cwd, last_activity_ms, ended }>`. `cwd` from `meta.json`, else `usage.json` `workspace.current_dir`/`cwd`. Live = not ended and active within 30 min. `pick_for_cwd`, sorted most-recent first.
- `panel.rs`: pure `lines(st, usage, cfg, view, now, width, height) -> Vec<Line<'static>>`. All layout lives here, so it is snapshot-testable as plain text.
- `watch.rs`: terminal setup/teardown (alternate screen, raw mode, restore on panic), the loop (poll keys with a timeout; reload state when `events.jsonl`/`usage.json` mtimes change; re-pick the followed session every 2 s), bell.

### Hook change

`meta.json` is written (create-only) on the first event of a session, not only on `SessionStart`, so sessions that began before ARI was installed still get a cwd. One failed `O_EXCL` open per hook; the hook stays well under budget.

### Layout (width W ≥ 40, inner width W − 4)

```
╭─ ARI (•_•) ─────────────────── 12:41 ─╮
│ ● ON TRACK     milestone: auth refactor│
│ ███████████░░░░░░░░░ 4/7 tasks         │
│ ▸ now: hook up token refresh           │
├─ agents ───────────────────────────────┤
│ ⠹ main     editing src/auth/session.rs │
│ ├─⠼ Explore grep refresh_token · 14 calls · 1m12s
│ └─✓ Plan   done 38s · found 3 call sites│
├─ errors ───────────────────────────────┤
│ ✗ 2m  cargo build · E0382 use of moved │
│       ↻ retried 2× · fixed by main     │
├─ usage ────────────────────────────────┤
│ ctx ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▱▱▱▱▱▱ 71% 142k/200k  │
│ 5h  ▰▰▰▰▰▰▰▰▰▰▰▰▱▱▱▱▱▱▱▱ 62% ↺ 1h40m    │
│ 7d  —                                  │
├─ session ──────────────────────────────┤
│ time  9m54s active / 14m wall · 71%    │
│ cost  $2.69 · $0.67/task done          │
╰────────────────────────────────────────╯
```

- **Face** per verdict: `(•_•)` on track, `(◉_◉)` needs you, `(>_<)` blocked/retrying/stuck, `(^_^)` done, `(-_-)zz` idle; `face = false` hides it.
- **NEEDS YOU** adds a line with the notification text, reverse video. **No plan** shows `no plan yet`, never a fake bar.
- **Agents:** main first, then subagents in start order as a tree. Running: spinner, kind, current action, call count, elapsed. Done: `✓`, duration, first line of the final message. Stale (5 min silent): `◌ stale 6m`. Collapsed view = running + the 3 most recent done; `a` shows all.
- **Errors:** newest first, max 5 (`e`: 15). Line 1: `✗`/`✓`, age, signature in words (`cargo build`, `edit main.rs`), first meaningful message line. Line 2 when retried or resolved: `↻ retried N× · fixed by <agent>` / `· still failing`. Empty: `no errors`.
- **Usage:** 20-cell bars (shrink to fit narrow widths), threshold colors, `—` for anything missing, `⟲ compacted HH:MM` after a compaction.
- **Session:** active vs wall from `cost.total_api_duration_ms` / `total_duration_ms`; cost and cost per completed task.
- **Collapse order when height is short:** drop session, then usage, then trim errors to 1, then agents to running only. The header, verdict and task lines always stay.
- Long text truncates with `…`; nothing ever wraps.
- `NO_COLOR` → no styles; glyphs and words always carry the meaning.

### `--all`

One line per live session, most recent first: project dir name, then the compact bar (`bar::render`). `tab` and the list are the whole picker — no selection UI.

## Testing

- `panel::lines` snapshots (plain text): the basic fixture's final state at 46 and 40 columns; a hand-built multi-agent state with errors; NEEDS YOU; a 12-row terminal (collapse).
- `sessions` unit tests: cwd from meta vs usage, most-recent pick, live filter.
- Real run: a `claude -p` session with two parallel subagents and a fixed error, watched with `ari watch --once`; output recorded in the PR.

## Left to the builder

Exact colors, spacing inside rows, spinner speed.

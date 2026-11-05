# Changelog

## Unreleased

- Sessions without hook events say `no hooks yet` (or `WORKING` while their transcript is moving) instead of a misleading `IDLE`.
- The panel uses Claude Code's session title as the milestone.
- **`ari recap`**: what a session got done (tasks, errors fixed, tests before → after, files, commits, time, cost, models), for the whole session, `--since prompt` or `--since 30m`; `--md` for a PR. `r` in `ari watch` shows it since your last prompt.
- The compact row drops the `ARI` prefix and shows the latest test run (`✓ 70 tests`, or `✗ 2/69 tests` in red), read from cargo, pytest, jest and vitest summaries. A finished session whose last test run failed says `DONE · tests failing`.
- The panel shows a trend of recent test runs, where the time went (claude, tools, waiting on you, idle), the cost of mistakes (failed calls, their time, redundant re-reads), and each agent's model.
- Compaction forecast: `ctx 71% ~4t` and a panel line when context will auto-compact within 10 turns (`thresholds.compact_pct`, default 90).
- File conflicts: `⚠ a.rs also in tankobon #bbbb` when another live session edited the same file in the last 30 minutes.
- The panel hides finished subagents that did nothing or finished over 10 minutes ago (`a` shows them).
- The panel header shows the folder it follows beside the clock (`cd ~/ari · 14:29`).
- `ari watch --all` is a table: verdict, `project · session title`, what it's doing (or what it's asking, for NEEDS YOU), context, cost and last activity, in aligned columns. Sessions that need you sort to the top; the account's 5h/7d limits and total cost show once in the header instead of on every row.
- Running commands show with a live timer (`$ cargo test 1m15s`) in the row and on each agent's panel line; background tasks are counted.
- Release builds can publish a Homebrew formula (needs the tap repo and token).
- `docs/ROADMAP.md` lists what to build next.
- The statusline row leaves out `no plan yet` when there's no task list, so the row is shorter; the panel still says it.
- Fix: with several sessions in one folder, `ari watch` could follow an idle one. Activity now comes from the hook log and transcript, not the statusline's constant `usage.json` writes.
- The panel's session section shows the branch's open pull request and its review state (`pr #14 · changes requested`), or a GitLab merge request (`mr !3`).

## 0.1.0 - 2026-11-05

First release.

- **Compact statusline row**: health verdict (`NEEDS YOU`, `BLOCKED`, `STUCK`, `RETRYING`, `ON TRACK`, `DONE`, `IDLE`), task progress from Claude's own task list, running subagents, unresolved errors, context, 5h/7d rate limits with reset countdowns, cost and tokens. Chains your existing statusline untouched.
- **`ari watch`**: full panel with agent tree, errors and who fixed them, usage bars, tokens per agent, cache hit and savings, 5h burn-rate warning. Follows the session in the current directory; `--all`, `--session`, `--once`, `--ascii`.
- **Interpretation**: errors are tracked by signature and marked fixed when the same command later succeeds; stuck-loop detection; cache-drop alert.
- **`ari demo`**: replay a bundled real session; no Claude needed.
- **`ari setup` / `ari uninstall`**: backup, diff, confirm; uninstall restores your settings byte for byte.
- **`ari doctor`**: checks binary, plugin, statusline, chained commands and hooks, with a fix for each problem.
- Claude Code plugin with hooks for 15 events; `/ari:setup`, `/ari:doctor`.
- Zero tokens added to your session. Hooks p95 ≈ 3 ms; statusline p95 ≈ 3 ms.

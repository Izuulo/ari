# Changelog

## Unreleased

- Sessions without hook events say `no hooks yet` (or `WORKING` while their transcript is moving) instead of a misleading `IDLE`.
- The panel uses Claude Code's session title as the milestone.
- The compact row drops the `ARI` prefix and shows the latest test run's pass count (`✓ 70 tests`), read from `N passed` in cargo, pytest, jest and vitest output.
- The panel header shows the folder it follows beside the clock (`cd ~/ari · 14:29`).
- `ari watch --all` labels each row `project · session title` and colors the verdicts.
- Running commands show with a live timer (`$ cargo test 1m15s`) in the row and on each agent's panel line; background tasks are counted.
- Release builds can publish a Homebrew formula (needs the tap repo and token).
- `docs/ROADMAP.md` lists what to build next.

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

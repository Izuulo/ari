# Changelog

## Unreleased

- Clearer panel (from a UI audit): error rows name the real command (not a leading `cd`) and show `exit N` when the output names no problem; fixed errors fold into one count line (`e` lists them, `passed on retry`); the session section is three lines, cost first; unknown meters are left out; the pace warning leads the usage section; the session title sits after the verdict (no `milestone:`); the header shows the folder without `cd`; permission modes read `auto mode`, `plan mode`…; the face is off by default; task bars use the same `▰▱` cells as usage; stale agents show `?`; untracked sessions say `– not tracked`; agent summaries drop Markdown and stop after a sentence; the session list puts what a session is doing on its own line when narrow; the live panel lists its keys.

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
- Plan files: with no task list (the task tools are off by default), the newest Markdown file the session wrote with `- [ ]` checkboxes shows as its plan: `m6-richness ███░░ 3/15` in the statusline and the panel.
- The statusline row leaves out `no plan yet` when there's no task list, so the row is shorter; the panel still says it.
- Fix: with several sessions in one folder, `ari watch` could follow an idle one. Activity now comes from the hook log and transcript, not the statusline's constant `usage.json` writes.
- The panel's session section shows the branch's open pull request and its review state (`pr #14 · changes requested`), or a GitLab merge request (`mr !3`).
- Filtered test runs (`3 filtered out`, `N deselected`) no longer set the headline count or the trend unless they failed.
- The time split adds up: idle time starts after an API error and after a Stop with background tasks, and leftover time shows as `other N%` when it is 5% or more.
- Less panel noise: no `no plan yet` row, `cost of mistakes` drops `· 0s`, and long agent kinds are cut so columns align.
- Badges row under the verdict: model, effort, fast, permission mode (`PLAN`, `AUTO`, `EDITS`, `DON'T ASK`, `BYPASS`), `⎇ worktree`, `PR #14 open` (`MR !3` on GitLab) and `+120 −8`. It also shows for sessions without hooks.
- The cache row says more: `· cold in 4m`, `· cold`, `· last miss: <cause>`.
- Background tasks (`↳ shell: npm run dev`) and scheduled jobs (`◷ */5 * * * * · check deploy`) show under the main agent.
- Resume line: `↻ resumed after 3h · cache cold · ~$0.40 to rebuild` until the next Stop.
- `turns` section: one row per turn (time, duration, calls, failures, cost, prompt) with compactions (`⟲ 14:20 compacted (auto) · ctx 91% → 18%`). It fills spare height and is trimmed first.
- Trends: a context sparkline, cost per hour and a cost sparkline.
- Away summary: Claude Code's one-line session summary shows when a session is idle or done, and in quiet `--all` rows.
- The panel uses two columns at 120 columns or wider.
- `ari watch --all` is interactive: `↑`/`↓` or `k`/`j` select, `enter` opens that session's panel, `esc` goes back, `q` quits.
- `ari doctor` reports hooks that failed in the last 24 hours (any plugin's), grouped.
- Long agent lists: the fold row names what's hidden (`✓ 27 more done · 24 general-purpose · …`), and `a` groups finished agents per kind and model.

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

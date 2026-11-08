# Changelog

## Unreleased

- `ari watch` wide panel: turns move under agents on the left, where they get the room; errors move to the right column between session and git and shrink first when space runs out.
- Roadmap §7: eleven fixes for common Claude Code complaints (alerts, weekly forecast, edit churn, compaction redo, test tampering, risky commands, quality baseline, recap review order, startup context, `ari find`, spend alerts), each with an implementation plan in `docs/superpowers/plans/`.
- `ari hub --all` drops the `you` node: each session's main sits on the graph's top row with its live subagents under it as a tree of boxes (state, name, message count, context, what it's doing), and the header counts the agents still open. Each session gets one card instead of a window per agent: its main's calls on the left, a tree of its active subagents on the right, one small box each with what it's doing, its context and its message count. `↑`/`↓` highlights a box and `⏎` zooms into it.
- **Git section in the panel.** `ari watch` shows the session repo's branch, how far it is ahead of or behind upstream, how many files changed, the last 3 commits and the last 3 GitHub PRs (open, draft, merged or closed), each with its age. It refreshes every 30 s in the background. PRs need the `gh` CLI, logged in. Hide it with `hide_panel = ["git"]` or in settings → PANEL.
- `a` in the hub switches between the watched session and every live session (`ari hub --all`), and back.
- `ari hub --all`: each session's box, tile and zoom view name the repo (or folder) it works in.
- Every hub box is coloured by its state. Main and each session in `ari hub --all` take their verdict's colour, as in the statusline (green on track, yellow stuck or retrying, red blocked, cyan done, dim idle, purple needs you). Subagents: cyan running, yellow retrying, grey finished or stale.
- The hub graph counts messages between the boxes it shows. In `ari hub --all`, a session talking to its own subagents no longer floods the purple `also:` line, and subagents messaging across two sessions draw an arc between those sessions.
- Hub boxes that need you are drawn in the needs-you colour (purple, or your theme's highlight): main waiting on a prompt, and in `ari hub --all` any session where an agent needs you. Agents that need you sort first on the graph, so they never fold into `+N more`.
- `c` in `ari watch` and the hub clears finished subagents off the screen; ones that finish later still show. The `auto_clear` setting (settings → PANEL, or `auto_clear = true` in config.toml) hides them as soon as they finish. Nothing is deleted from the logs.
- **Codex and DeepSeek Harness sessions.** They show in `ari watch`, `ari watch --all`, the hub and `ari recap` with activity, tool calls, errors, tests and subagents. Codex also shows context fill, tokens and 5h/7d limits, read from its rollout log; it has no cost. dsh has no token, context or cost numbers.
- `ari setup` wires every app it finds (Claude Code, Codex, dsh) and `ari uninstall` removes exactly what it added. `ari doctor` shows one line per app. Codex hooks need a one-time `/hooks` approval inside Codex. For dsh, setup mounts the Claude Code hook bridge in `$DSH_HOME/cordis.patch.yml`.
- `ari hook --agent <app>` records which app ran a session; a missing flag means Claude Code.
- `ari demo --codex` and `ari demo --deepseek` replay the bundled session as that app would report it, in its colours and with only the numbers it gives ARI, in the panel and the hub (`x`).
- New themes `claude` (orange and grey), `codex`, `deepseek` and `auto` (follows the watched session's app). The default theme is unchanged.
- Multi-session screens label non-Claude sessions with their app, show account limits per app, and count dollar figures for Claude Code sessions only.
- Fix: after `/clear`, `ari watch` and `ari recap` kept showing the ended session until the new one's first prompt. They now follow the new session straight away.
- Known limits: Codex doesn't give hooks a command's exit code, so a failing non-test command isn't shown as an error (a failing test run is). dsh support is tested from sample data, not a real session.

## 0.3.0 - 2026-11-06

- **`ari hub --all`** (or `x` in `ari watch --all`): one hub over every live session. Each session is a node under you with its agents folded in, messages between sessions draw as arcs, any agent waiting on you turns it `NEEDS YOU`, and each session's context is measured against its own window.
- Errors from different scripts of one runner (`npm run build` vs `npm run lint`, `python -m pytest` vs `python -m mypy`) are tracked apart: one passing no longer marks the other fixed, and three different failures no longer read as STUCK.
- `ari setup` no longer writes `time` and `cache` into `hide`; they aren't segments, and `ari doctor` flagged them.
- `retention_days = 0` counts as 1 day instead of deleting every other session's logs, live ones included.
- `ari uninstall` also removes the chained statusline and the `hide` list that `ari setup` added to `config.toml` (your own changes stay; a broken file is left alone).

## 0.2.0 - 2026-11-06

- **Settings**: `s` in `ari watch` opens a settings screen with a live preview: five colour themes (`default`, `soft`, `vivid`, `colorblind`, `mono`) plus per-colour overrides, separate on/off switches for every statusline segment (`hide`) and every panel section (`hide_panel`, the task checklist is one), a live preview of both, alerts and log retention. Changes save to `config.toml` at once; hand edits to the file apply live. A config.toml ARI can't read is reported (settings screen, `ari doctor`) and never overwritten.
- **The hub**: `x` in `ari watch` (or `ari hub`) takes the whole terminal. It shows a graph of spawns and messages between agents, a table with each agent's own context window, a watch window per running agent, the message log and activity lanes. `⏎` zooms into one agent (timeline, what filled its context, files, tasks, errors) and `g` shows the graph full size with the task list. Needs 120×30; `--ascii` works too.
- A new `PermissionRequest` hook records which agent is waiting on you and for what; any waiting agent makes the verdict `NEEDS YOU`, even while others keep working.
- The hook also records subagent names and spawn prompts, `SendMessage` recipients and text, and lines in and out per file edit.
- `ari demo` replays a new recorded session: the same test fails three times (`STUCK`), Claude waits on a permission prompt (`NEEDS YOU`), then fixes it and finishes with two subagents. The replay slows down while something is wrong, and no longer shows a made-up 5h pace warning.

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

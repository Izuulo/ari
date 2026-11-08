# Roadmap: what ARI could show next

Ideas for turning ARI into the hub for every Claude Code session you run. Each one names where the data comes from. **Have it** means Claude Code already sends it to ARI, so it's cheap to add; **new** means ARI has to collect something it doesn't today.

## 1. The hub (`ari watch --all`)

| Idea | Why | Data |
|---|---|---|
| Group by repo, with a per-repo summary row | Several sessions in one repo read as one project | Have it (repo name) |
| How long each session has been waiting on you | "NEEDS YOU · 6m" says how urgent it is | Have it (notification time) |
| Account-wide totals: cost today, tokens, 5h and 7d use across sessions | Rate limits are per account, not per session | Have it |
| Hide or archive finished sessions | Keep the list short | Have it |

## 2. More about each session

| Idea | Why | Data |
|---|---|---|
| Git branch and uncommitted file count | Which branch Claude is changing | New: run `git` in the statusline, debounced |
| What a permission prompt is asking for | Decide without switching windows | Have it (notification message) |
| Spend limit (gateway users) | Budget alongside rate limits | Have it (`rate_limits.spend_limit`) |

## 3. Smarter interpretation

| Idea | Why | Data |
|---|---|---|
| Projected cost to finish the task list | Is this run worth it | Have it |
| Slow-command alerts: a command running far longer than its previous runs | Catch hung builds early | Have it (per-signature durations) |
| Repeated permission prompts for the same tool | Suggest an allow rule | Have it |
| Subagent tree with nesting and per-branch cost | Deep agent fan-outs | Partly (spawn depth is in transcripts) |

## 4. Other harnesses and providers (M7)

| Idea | Why | Data |
|---|---|---|
| Flaky-test detection by test name | A test that flips between pass and fail | New: per-test parsing |

Shipped in M7: Codex and DeepSeek Harness sessions in `ari watch --all` and the hub, and per-app themes (`claude`, `codex`, `deepseek`, `auto`). The per-model branding idea (Opus, GPT, DeepSeek) is not built.

Shipped from the hub list: sort by urgency, account limits once in the header, and what a NEEDS YOU session is asking.

Shipped from this list in M5: test pass/fail counts, files touched (in `ari recap`), done-but-tests-failing. Since: pull request number and review state.

## 5. Later plans

| Idea | Why | Data |
|---|---|---|
| In-app pane via a plugin mod | The panel beside the chat without a second terminal | New: Claude Code's mod API |
| Subagent rows via `subagentStatusLine` | Per-agent status inside Claude Code's own agent list | Have it (agent events); new: the setting |
| Hooks `PermissionDenied`, `Pre/PostModelSwitch` | Denials and model changes instead of guessing (`PermissionRequest` shipped with the hub) | New: register the hooks |
| Hub: split-pane teammates | Agent-team teammates in tmux/iTerm2 panes are separate sessions; join them into the lead's hub | New: read `~/.claude/teams/{team}/config.json` |
| Hub: per-agent compaction forecast | Only main has one today | Have it (per-agent context from transcripts) |

## 6. Platform and polish

| Idea | Why | Data |
|---|---|---|
| Option to run hooks async | Zero wait even on slow machines; trades exact event order | DECISIONS D8 |
| `refreshInterval` setting in config | Fewer statusline runs with many open sessions | — |
| Light theme, custom colors | Readability on light terminals | — |
| Windows support | PRD target for 1.1 | — |
| Homebrew formula | Easier install on macOS | Needs the tap repo and token (PR #8 merged) |
| crates.io release | `cargo install ari-cli` | Needs a crates.io token |

## 7. Fixes for what people complain about

From a survey of Claude Code GitHub issues, Reddit and blog posts (2026-11-07). Ranked: most common complaint and cheapest fix first. All of them stay zero-token, local and watch-only. Each has a plan in `docs/superpowers/plans/2026-11-07-*.md`.

| # | Complaint | Idea | Data |
|---|---|---|---|
| 1 | "I didn't know it was waiting on me" | Desktop notification (and optional ntfy URL) when any session flips to NEEDS YOU, STUCK, BLOCKED or DONE | Have it (verdict); new: notifier (osascript / notify-send) |
| 2 | The weekly limit hits by surprise | Extend the 5h burn forecast (`burn.rs`) to the 7d limit: "7d runs out Thu ~14:00 at this pace" | Have it (`rate_limits.seven_day`) |
| 3 | Loops burn tokens overnight | STUCK also on edit churn: the same file edited N times, or an edit that undoes an earlier one; show tokens burned while stuck | Have it (edit events, token ledger) |
| 4 | Compaction loses or redoes work | Mark compactions on the timeline; warn "redoing work since compaction" when the same edits or commands repeat after one | Have it (`PreCompact`/`PostCompact`) |
| 5 | "Done" when it isn't | Flag test files edited or deleted, or skip/xfail added, during the run; "unverified" when files changed after the last test run | Have it (edits, test signatures) |
| 6 | Destructive commands | Flag risky commands that ran (`rm -rf`, `git reset --hard`, force push, deletes outside the repo) in the panel and in `ari recap` | Have it (Bash tool input) |
| 7 | "Claude feels dumber today" | Quality signals against your own baseline: reads per edit, errors per task, retries | New: per-session summary kept across sessions |
| 8 | Too many diffs to review | `ari recap` groups files by the prompt or task that caused them, marks files to look at hard (tests, config, lockfiles), suggests a review order | Have it (edits, prompts, tasks) |
| 9 | MCP and CLAUDE.md eat context up front | Show the context already used before the first prompt (system, MCP, memory) | Have it (first usage in the transcript) |
| 10 | "Where did my session go?" | `ari find <text>`: search every transcript, print the folder and the `claude --resume` command | Have it (`~/.claude/projects`) |
| 11 | Surprise API bills | `$` threshold per session and per day: statusline turns red and a notification fires | Have it (cost); new: config keys |

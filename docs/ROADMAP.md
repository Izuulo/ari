# Roadmap: what ARI could show next

Ideas for turning ARI into the hub for every Claude Code session you run. Each one names where the data comes from. **Have it** means Claude Code already sends it to ARI, so it's cheap to add; **new** means ARI has to collect something it doesn't today.

## 1. The hub (`ari watch --all`)

| Idea | Why | Data |
|---|---|---|
| Group by repo, with a per-repo summary row | Several sessions in one repo read as one project | Have it (repo name) |
| Desktop notification when any session flips to NEEDS YOU or BLOCKED | Don't have to watch the pane at all | Have it; new: notifier (osascript / notify-send) |
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
| Sessions from other coding agents (starting with the one you use most; to decide) in `ari watch --all` | One panel for every agent you run | New: each harness's session logs or hooks |
| Per-harness and per-provider branding: the tool running the session (Claude Code, Codex) and the model doing the work (Opus, GPT, DeepSeek) | See at a glance what is running where | Model names already shown as given |
| Flaky-test detection by test name | A test that flips between pass and fail | New: per-test parsing |

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

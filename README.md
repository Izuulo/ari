# ARI

**Stop asking Claude how it's going.**

ARI (All Relevant Information) is a zero-token status panel for Claude Code. It watches your session through hooks and the statusline, and tells you at a glance how far along Claude is and whether anything is wrong, without interrupting the run or adding a single token to it.

```
ARI ● ON TRACK │ ██████░░░░ 4/7 │ ⠼ 2 agents │ ✗ 0 │ ctx ▰▰▰▰▱ 71% │ 5h 62% ↺1h40m
```

## Install

```sh
cargo install --git https://github.com/Izuulo/ari ari-cli   # the `ari` binary
claude plugin install ari --marketplace Izuulo/ari           # hooks that feed it
ari setup                                                    # add ARI's row to your statusline
```

Prefer the in-app flow? `/plugin marketplace add Izuulo/ari`, then `/plugin install ari@ari`, then `/ari:setup`.

`ari setup` shows you exactly what it will change and asks first. It keeps your current statusline and runs it alongside ARI's row; `ari uninstall` puts it back byte for byte.

## What the row tells you

| Verdict | Means |
|---|---|
| `◉ NEEDS YOU` | Claude is waiting on a permission prompt or your input |
| `✗ BLOCKED` | The turn ended on an API error, or right after a failure it never fixed |
| `↻ RETRYING` | Something just failed and Claude is still working on it |
| `● ON TRACK` | Working, nothing failing |
| `✓ DONE` | Turn finished and every task is complete |
| `◌ IDLE` | Turn finished; tasks incomplete or none |

Then: task progress from Claude's own task list (`no plan yet` when there isn't one; ARI never fakes progress), running subagents, unresolved errors this turn, context use, 5-hour and 7-day rate limits with reset countdowns (`—` when your plan doesn't report them), and cost. Segments drop from the right when the terminal is narrow; the verdict always stays.

A failed command counts as fixed when the same command (or a write to the same file) later succeeds.

## Config

`~/.config/ari/config.toml`, every key optional:

```toml
position = "top"            # ARI's row above or below your chained statusline
hide = ["cost"]             # segments to drop: tasks agents errors ctx 5h 7d cost
max_width = 0               # 0 = terminal width
retention_days = 7          # session logs older than this are deleted

[chain]
commands = []               # your previous statusline commands (ari setup fills this)
timeout_ms = 300            # a chained command slower than this is skipped

[thresholds]
ctx_warn = 60               # context % turns yellow
ctx_crit = 80               # and red
```

`NO_COLOR` is respected. Glyphs carry the meaning, so nothing relies on color alone.

## FAQ

**Does it cost tokens?** No. Hooks only append to a local log and print nothing; the statusline isn't sent to the model.

**Does it slow Claude down?** Hooks take about 3 ms (p95) and never block. The statusline renders in about 3 ms.

**Will it break my statusline?** No. Your command runs with the same input and its output is printed untouched. If it fails or takes longer than 300 ms, only it is skipped.

**Where does data go?** Nowhere. Everything stays in `~/.local/state/ari/` and is pruned after 7 days. No network, no telemetry.

## Coming next

`ari watch`: a full TUI panel with the agent tree, error history and usage detail; stuck-loop detection; cache savings and rate-limit burn rate. See [docs/PRD.md](docs/PRD.md).

## License

MIT

# M0: Capture — design

**Milestone done when:** a real Claude Code session is captured by `ari hook` and replayable with `ari replay`.

## Intent

Everything ARI shows later is derived from events that hooks append. M0 builds that foundation: a Rust binary whose `hook` subcommand turns each Claude Code hook payload into one compact line in a per-session log, fast and silent, and a `replay` subcommand that prints a recorded session as a readable timeline. Plus the repo skeleton (crate, plugin manifest, hooks.json, CI) and one real fixture session.

## Goals

- `ari hook` handles every event in PRD §6.1 plus `TaskCreated`, `TaskCompleted`, `PostCompact` (DECISIONS D3).
- p95 < 15 ms, always exit 0, never prints to stdout or stderr, never blocks (PRD §6.1 rules).
- Concurrent hook processes (parallel subagents) never corrupt the log.
- A fixture recorded from a real `claude` run, scrubbed of home paths, is committed and drives snapshot tests.
- `ari replay <dir>` prints a timeline anyone can read.

## Non-goals (M0)

Reducer, verdict, statusline, panel, config file, `ari setup`. Those are M1+.

## Constraints

- Single crate `ari-cli`, binary `ari`. Dependencies: `serde`, `serde_json`; dev: `insta`. No arg-parsing crate.
- State root: `$XDG_STATE_HOME/ari` or `~/.local/state/ari` (DECISIONS). Session dir: `<root>/sessions/<session_id>/`.
- Never write into the project directory.

## Design

### Event line (`events.jsonl`)

One JSON object per line, built by `normalize(raw, t_ms)`. A flat struct; absent fields are omitted:

| Field | Meaning |
|---|---|
| `t` | epoch ms, stamped by the hook (payloads carry no timestamp) |
| `e` | `hook_event_name` verbatim |
| `a`, `at` | `agent_id`, `agent_type` (absent = main agent) |
| `tool`, `id` | `tool_name`, `tool_use_id` |
| `act` | human "now doing" text, e.g. `editing src/auth.rs`, `$ cargo build`, `grep refresh_token` |
| `sig` | error signature: `Bash:<first two command tokens>`, `<Tool>:<path>` for file tools, else `<Tool>` |
| `h` | FNV-1a 64 hash of the canonical `tool_input` JSON (loop detection, M3) |
| `ok` | `true` on `PostToolUse`, `false` on `PostToolUseFailure` |
| `msg` | one meaningful line: failure message, subagent/stop final message, notification text, prompt text |
| `kind` | `notification_type`, StopFailure `error`, SessionStart `source`, SessionEnd `reason`, PreCompact `trigger`, or `exit N` for Bash failures |
| `task` | `{id, subject?, status?}` from `TaskCreated`, `TaskCompleted`, `PostToolUse` of `TaskCreate`/`TaskUpdate` |
| `todos` | full `[{c, s}]` list from `PostToolUse` of `TodoWrite` (max 40 items, 80 chars each) |
| `bg` | `Stop`: number of `background_tasks` with status `running` |
| `path` | transcript path (`SessionStart`: main; `SubagentStop`: `agent_transcript_path`) |
| `dur` | `duration_ms` |

Rules: strings truncated on char boundaries with `…` (`act` 80, `msg` 160, prompt 200). Serialized line must be < 4096 bytes; if still over, drop `todos` then `msg`; if still over, write `{t, e}` only. Paths inside `cwd` are shown relative.

Failure `msg`: skip blank lines and the `Exit code N` line; take the first line containing `error`, `failed`, `panicked` (case-insensitive), else the first remaining line.

### `ari hook`

Read all stdin → parse JSON → validate `session_id` (`[A-Za-z0-9_-]{1,128}`, else drop) → `create_dir_all(session dir)` → on `SessionStart` write `meta.json` (`cwd`, `started`, `transcript`) and prune session dirs whose `events.jsonl` mtime is older than 7 days → append the line with one `write_all` on an `O_APPEND` file. Last activity = `events.jsonl` mtime (no per-event meta rewrite). All of it runs inside `catch_unwind`; the process exits 0 whatever happens. A trailing event-name arg is ignored (DECISIONS D10).

`ARI_RAW_LOG=<file>`: also append `{"t":ms,"p":<raw payload>}` — the recording format for fixtures.

### `ari replay <dir|file>`

Reads `events.jsonl` (or `raw.jsonl`, normalizing on the fly) and prints one line per event: offset from first event, agent label (`main` or `agent_type`), glyph + event, detail. Exits non-zero with a message if the path has no log.

### Plugin + repo skeleton

`.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `hooks/hooks.json` registering every event with `command -v ari >/dev/null 2>&1 && ari hook || true`, `timeout: 1`. GitHub Actions CI on macOS + Linux: fmt, clippy `-D warnings`, test.

### Fixture

`fixtures/basic/`: `raw.jsonl` (recorded with `ARI_RAW_LOG`), the main and subagent transcripts, all scrubbed by `scripts/scrub.sh` (home → `/home/user`, project → `/home/user/proj`). The session: create 3 tasks, write a broken script, fail, fix, succeed, spawn an Explore subagent in the background, finish. Snapshot test: normalize every raw line → `insta` snapshot of the resulting events.

## Discovered in the spike (added to DECISIONS)

`Stop` fires while a background subagent is still running (`background_tasks` non-empty); the turn is not over. Hence `bg` on Stop events.

## Left to the builder

Module layout, exact `act` wording per tool beyond the examples, replay formatting.

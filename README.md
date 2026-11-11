<div align="center">

# ARI

### Stop asking Claude how it's going.

**All Relevant Information**: a zero-token status panel for Claude Code, Codex and DeepSeek Harness.

[![CI](https://github.com/Izuulo/ari/actions/workflows/ci.yml/badge.svg)](https://github.com/Izuulo/ari/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Izuulo/ari?color=blue)](https://github.com/Izuulo/ari/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey)
![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)

[Install](#install) · [What you get](#what-you-get) · [The hub](#every-agent-at-once-the-hub) · [Other agents](#other-agents-codex-and-deepseek-harness) · [Commands](#commands) · [Config](#config) · [FAQ](#faq)

<img src="docs/demo.gif" alt="ARI replaying a Claude Code session: the same test fails three times and the verdict turns to STUCK, Claude waits on a permission prompt (NEEDS YOU), the fix lands and it is back ON TRACK, two Explore subagents finish, and it ends on DONE" width="760">

</div>

ARI watches your session through hooks and the statusline and answers *how far along are we, and is anything wrong?* at a glance, so you never have to interrupt a long run to ask. Nothing is ever sent to the model, and nothing leaves your machine unless you set `notify.ntfy`.

## Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Izuulo/ari/releases/latest/download/ari-cli-installer.sh | sh
claude plugin install ari --marketplace Izuulo/ari
ari setup
```

1. Installs the `ari` binary to `~/.local/bin` (macOS and Linux). With Homebrew: `brew tap Izuulo/tap && brew trust --formula izuulo/tap/ari && brew install Izuulo/tap/ari` (newer Homebrew asks you to trust third-party taps). With Rust: `cargo install --git https://github.com/Izuulo/ari ari-cli`.
2. Adds the plugin whose hooks feed ARI. In-app: `/plugin marketplace add Izuulo/ari`, then `/plugin install ari@ari`.
3. Adds ARI's row to your statusline. It shows the change, asks first, and keeps your current statusline running alongside. If Codex or DeepSeek Harness is installed, it wires those up too (see [Other agents](#other-agents-codex-and-deepseek-harness)).

Start a new Claude Code session (or `/reload-plugins`) and the row appears. Something missing? `ari doctor` says what and how to fix it. Just want to look? `ari demo` replays a real session with no Claude needed; `ari demo --codex` and `ari demo --deepseek` show the same session as those apps report it.

## What you get

**A row in your statusline**, always visible:

```
● ON TRACK │ ▰▰▰▰▰▰▱▱▱▱ 4/7 │ ⠼ 2 agents │ ✗ 0 │ ✓ 70 tests │ ctx ▰▰▰▰▱ 71% │ 5h 62% ↺1h40m │ 7d 34% ↺4d │ $2.69 │ 6.2M tok
```

When the week's pace would run out the 7d limit before it resets, the 7d segment shows when (`▲Thu 14:00`) and the panel says `▲ 7d runs out Thu ~14:00 at this pace`. The pace is the last 6 h across all your Claude Code sessions.

**A full panel** in a second pane with `ari watch` (`tmux split-window ari watch` works fine). It follows the latest session in the current directory:

<p align="center"><img src="docs/panel.png" alt="The ARI panel mid-run: STUCK because python3 test_calc.py failed 3 times in a row, 1 of 4 tasks, the AssertionError under errors, usage bars, and session cost and tokens" width="620"></p>

**Notifications** when a session needs you: one desktop notification (and, if you set `notify.ntfy`, one push) each time a session flips to NEEDS YOU, STUCK, BLOCKED or DONE. They fire once per flip, from the hooks, so they work with no `ari watch` open. On macOS you may be asked once to allow notifications from Script Editor. Turn them off with `desktop = false` in `[notify]` or in settings; clear `ntfy` to stop pushes.

Keys (listed under the live panel): `q` quit, `e` fixed errors too, `a` all agents, `c` clear finished agents, `r` recap since your last prompt (`esc` closes it), `tab` next session, `x` the hub (below), `v` the next view (full, Calm, Constellation; see Simple views below). `ari watch --all` lists every live session: `↑`/`↓` or `k`/`j` select, `enter` opens that session's panel, `esc` goes back to the list from an opened session and quits from the list. A stale agent (no activity for a while, maybe stuck) shows `?`; a session ARI has no hook events for shows `– not tracked`. Under the verdict a badges row shows model, effort, fast mode, permission mode, worktree, pull request and lines changed. Under the ctx bar, `at start 31k (16%)` is the context already used before Claude's first reply (system prompt, tools, MCP servers, CLAUDE.md and your first message), marked `▲` from 20% of the window. The calls section lists main's recent tool calls, newest first (age, tool, target, ✓/✗ and duration); turns sit below it and shrink first when space is short. The git section shows branch, changed files, the last commits and the last GitHub PRs (PRs need `gh`, logged in). When a session reads less before editing, or fails and retries more, than your usual on that model, the session section says so (e.g. `⚠ reads/edit 2.0 · your usual 6.1`); it needs 5 earlier sessions with at least 5 edits. Subagents that went quiet for 5 min without finishing can be hidden with `hide_stale` (settings → PANEL). `--ascii` for terminals without Unicode.

### Every agent at once: the hub

Press `x` in `ari watch` (or run `ari hub`) and the panel takes the whole terminal. It shows a graph of who spawned whom and who is messaging whom, a table of every agent with its own context window, and a watch window per running agent with its last tool calls, what it's doing now and the last message it got. Below those: the messages between agents and an activity lane per agent. `ari hub --all` (or `x` in `ari watch --all`) puts every live session's agents in one hub: each session's main is a node on the graph's top row with its live subagents hanging under it as a tree of boxes, and each session gets one card, its main's calls beside a tree of its active subagents, each in a small box with what it's doing and its context. `↑`/`↓` walks the boxes and `⏎` zooms into one. Here it is replaying `ari demo`:

<p align="center"><img src="docs/hub.gif" alt="The ARI hub replaying the demo session: main gets STUCK on a failing test, waits on a permission prompt, fixes it, spawns two Explore subagents that each get a watch window, and the verdict turns to DONE" width="100%"></p>

<table>
<tr>
<td width="50%"><img src="docs/hub-zoom.png" alt="Zoomed into the main agent: its timeline with the three failed test runs, context window, edited files and the error it fixed"></td>
<td width="50%"><img src="docs/hub-graph.png" alt="The graph screen: main and its two Explore subagents, with the task board below"></td>
</tr>
<tr>
<td align="center"><sub><code>⏎</code> zoom: one agent's timeline, context, files and errors</sub></td>
<td align="center"><sub><code>g</code> graph: who spawned whom, with the task board</sub></td>
</tr>
</table>

`↑`/`↓` select an agent, `⏎` zooms into it (its timeline, what filled its context, files, tasks and errors), `g` opens the graph full size with the task list under it, `z` shows finished agents' windows, even with `auto_clear` on (one or two windows fill the row, with a preview of their zoom view beside them), `c` clears finished agents off the screen, `a` switches to every live session and back, `b` swaps the messages and activity panes for git (recent commits and PRs, and which agent's edits are still uncommitted; in a zoom, the agent's files are marked ● uncommitted or ✓ committed), `esc` goes back one level, `v` cycles the hub's view (below) and `x` returns to the panel. An agent waiting on a permission prompt shows the exact call and how long it has waited. ARI never answers it for you. The hub needs a 120×30 window (160×45 for everything); smaller, the panel says so and keeps going. "What filled it" is an estimate: each tool result is credited with the jump in input tokens on the next call. The zoom's context box also shows how much was used at start. Main's zoom timeline and activity lane mark each compaction with `⟲`.

### Simple views: Calm and Constellation

`v` cycles the panel, the hub and `ari hub --all` through three views: full (the default), **Calm** and **Constellation**. Calm keeps the layout but says what each agent is doing in plain English (`running the tests...`, `taking a break`), with no paths, commands, costs or token counts; Calm's hub is the graph alone, and `--all` shows one card per session. Constellation draws each session as a star system: main at the centre, helpers orbiting, messages flying between them as comets; `--all` puts every session on one sky. Alerts stay loud in both: an agent waiting on you or a fresh error blinks across the top row. Each screen remembers its own view in `[view]` (`watch`, `hub`, `hub_all`) in `config.toml`; pressing `v` rewrites that file the way the settings screen does. `--once` and pipes always print the full view. `ari demo` starts in Constellation and never saves the view; `ari demo --team` replays a session with helpers and messages.

### What a session got done: `ari recap`

```
recap     ~/proj · auth refactor · since start
verdict   ✓ DONE
done      4 tasks
            ✓ scaffold
            ✓ build
errors    1 hit · 1 fixed · 0 open
check     ⚠ skip added · test_calc.py
tests     58 passed → 71 passed
files     2 edited · 1 to look at
            › task: scaffold
              calc.py  +2
              test_calc.py  +4 · test
            › task: build
              calc.py  +1 −1
review    1. test_calc.py · test
          2. calc.py
commits   1
            e885f9c feat: retry the token refresh
time      21m · claude 64% · tools 22% · idle 14%
cost      $2.69 · +156 −23 lines
models    opus-5-5 (main), haiku-5-5 (2 agents)
```

Changed files are grouped by the task in progress when they were edited, else by the prompt you typed. Files to look at hard (outside the repo, migrations, CI, lockfiles, tests, config) are flagged and come first in the review order, then the biggest changes. Only file-tool edits count (not `sed -i`, `mv`, formatters or codegen); with several tasks in progress at once, edits go to the first; a background subagent that edits after your next prompt is credited to that prompt.

`--since prompt` covers only what happened since your last message; `--since 30m` the last half hour. `--md` prints Markdown bullets to paste into a PR. Built from ARI's own records, so it costs no tokens.

**Risky commands.** ARI flags Bash commands that ran and could lose work: a delete outside the repo, a recursive delete, `git reset --hard` and other ways of discarding uncommitted work, a force push, a deleted branch, an SQL drop or truncate, and a raw disk write. The latest shows under the verdict in `ari watch` (`⚠ ran git push --force · rewrites remote history · 2m`), and `ari recap` lists every one in a `risky` section. ARI never blocks a command; a command you denied at the prompt never ran and isn't flagged. Matching is by text, so it can't see:

- commands hidden in scripts or task runners (`./clean.sh`, `make clean`, `npm run reset`);
- variables and substitutions it can't resolve (`rm -rf $DIR`, `rm -rf $(cat list)`, backticks), aliases and shell functions;
- other languages doing the deleting (`python -c "shutil.rmtree(...)"`, `node -e ...`);
- file tools and MCP tools: a `Write` that overwrites a file, an `Edit` that empties one, a database MCP's drop;
- symlinks: paths are resolved as text, never on disk;
- PowerShell (`Remove-Item -Recurse`) and shell calls whose command isn't a single string.

Known false alarms: SQL keywords in a heredoc that only writes a `.sql` file, and `rm -r` on a build folder, which is flagged even when harmless.

### The verdict

| Verdict | Means |
|---|---|
| `◉ NEEDS YOU` | Waiting on a permission prompt or your input |
| `✗ BLOCKED` | The turn ended on an API error, or right after a failure it never fixed |
| `↻ STUCK` | Going in circles: the same command failing again and again, the same call repeated with nothing changing between, one file edited 12 times with no passing test, or an edit undone and redone. Shows the tokens spent since it got stuck |
| `↻ RETRYING` | Something just failed and Claude is still working on it |
| `● ON TRACK` | Working, nothing failing |
| `✓ DONE` | Turn finished cleanly, with every task complete or no task list |
| `◌ IDLE` | No turn yet, or the turn stopped with tasks still open |

`⟲ redo ×N` next to the verdict: after a compaction, main repeated N edits or commands it had already done before it (re-running tests and looking around don't count).

`DONE · unverified` or `IDLE · unverified`: files changed after the last test run. `⚠ N test changes`: this turn deleted a test file, added a skip/xfail/`.only`/`#[ignore]`, or edited a test while tests were failing. The panel lists each one.

## Other agents: Codex and DeepSeek Harness

ARI also follows Codex and DeepSeek Harness (`dsh`) sessions. They appear in `ari watch`, `ari watch --all`, the hub and `ari recap`. `ari setup` wires every app it finds (Claude Code, Codex, dsh), `ari uninstall` removes exactly what it added, and `ari doctor` shows one line per app.

| | Codex | DeepSeek Harness |
|---|---|---|
| Activity, tool calls, tests, subagents | yes | yes |
| Errors | test failures only | yes |
| Context fill, tokens, 5h/7d limits | yes, from Codex's rollout log | no (dsh doesn't expose its session log to hooks) |
| Cost | no | no |
| ARI row in the app's footer | no: run `ari watch` in a pane | no: run `ari watch` in a pane |

- **Codex:** after `ari setup`, run `/hooks` inside Codex once and approve ARI's hooks. Known limit: Codex doesn't give hooks a command's exit code, so a failing non-test command isn't shown as an error; a failing test run is.
- **dsh:** setup mounts dsh's Claude Code hook bridge (`@deepseek-ai/dsh-hooks-claude-code`) in `$DSH_HOME/cordis.patch.yml` (default `~/.dsh`), pointing at ARI's own hooks file. If you already use the bridge with your own hooks file, setup adds ARI's entries to that file instead. dsh support is tested from sample data shaped after dsh's documented payloads, not a recorded session.
- **Multi-session screens** label non-Claude sessions with their app, show account limits per app, and count dollar figures for Claude Code sessions only.
- **Themes:** each app has its own: `claude` (orange and grey), `codex` and `deepseek`. Set `theme = "auto"` and the panel and hub take the colours of the app running the watched session. Pick one with `s` in `ari watch` or in `config.toml`; the default theme is unchanged.
- **See it first:** `ari demo --codex` and `ari demo --deepseek` replay the bundled session as that app would report it, with its label, its colours and only the numbers it gives ARI. Press `x` for the hub.

## Why ARI

| | |
|---|---|
| **A verdict, not just numbers** | One word tells you whether to look up from what you're doing |
| **Stuck detection** | Notices loops and repeated failures and says which command |
| **Errors tracked to resolution** | A failure is marked fixed, with who fixed it, when the same command later succeeds |
| **Honest DONE** | Flags deleted or skipped tests, files changed after the last test run, and work redone after a compaction |
| **Per-agent view** | What each subagent is doing right now, its tool calls and token spend |
| **The hub** | Who spawned whom, who messages whom, and each agent's own context window, full screen |
| **Simple views** | `v` swaps the numbers for plain English (Calm) or an animated star system per session (Constellation); alerts stay loud |
| **Permission waits** | Which agent is waiting on you, for what call and how long; ARI never answers for you |
| **Notifications** | A desktop notification (or ntfy push) once when a session needs you, gets stuck, blocks or finishes |
| **Cache savings** | Hit rate, roughly what caching saved, and an alert when the cache suddenly stops hitting |
| **Rate-limit foresight** | Warns when you'll hit the 5-hour or the weekly limit before it resets |
| **Plays nice** | Chains your existing statusline instead of replacing it; `ari uninstall` restores it byte for byte |
| **Zero tokens** | Nothing is ever injected into Claude's context |

Progress comes only from Claude Code's own task list. When there isn't one, a Markdown plan the session wrote with `- [ ]` checkboxes stands in (its checked boxes are the progress); otherwise the statusline leaves progress out and the panel says `no plan yet`, rather than guess. On newer models the task tools are off unless you start Claude Code with `CLAUDE_CODE_ENABLE_TODO_TOOLS=1`.

## Commands

| Command | What it does |
|---|---|
| `ari setup [--yes] [--dry-run]` | Add ARI's row to your statusline, chaining the one you have |
| `ari uninstall [--yes] [--purge]` | Restore your original statusline byte for byte |
| `ari watch [--all] [--session <id>] [--once] [--ascii] [--hub]` | Full panel for this directory's session, or every live session |
| `ari hub [--session <id> \| --all] [--ascii]` | The hub: every agent, its messages and context |
| `ari recap [--session <id>] [--since start\|prompt\|30m] [--md]` | What a session got done |
| `ari demo [--team \| --codex \| --deepseek] [--speed <x>] [--ascii] [--once]` | Replay a bundled real session, no Claude needed, starting in the Constellation view (`v` cycles views, nothing is saved); `--team` replays a session with helpers and messages; `--codex` and `--deepseek` show the stuck session as that app reports it |
| `ari doctor` | Check binary, plugin, statusline and hooks, with a fix for each problem |
| `ari replay <dir>` | Print a recorded session as a timeline |

`ari hook` and `ari statusline` are the entry points Claude Code calls; you never run them yourself. In Claude Code, `/ari:setup` and `/ari:doctor` do the same as their CLI twins.

## Config

Press `s` in `ari watch` (panel, hub or `--all` list) for the settings screen: theme, colours, what the statusline and the panel show, alerts. Every change saves to `~/.config/ari/config.toml` and shows at once, with a live preview of your statusline row and panel beside the list when the window is 110 wide. Edits you make to the file by hand apply live too, and `ari doctor` points out anything it can't read.

Every key is optional:

```toml
position = "top"            # ARI's row above or below your chained statusline
theme = "default"           # default · soft · vivid · colorblind · mono · claude · codex · deepseek · auto
hide = ["cost"]             # statusline segments to drop: conflicts now tasks agents errors tests ctx 5h 7d cost tokens
hide_panel = ["turns"]      # panel sections to drop: tasks (the checklist bar) agents errors usage session calls turns git
face = false                # the little (•_•) in the panel header
bell = false                # ring the terminal bell when Claude needs you (ari watch)
auto_clear = false          # hide subagents in ari watch as soon as they finish
hide_stale = false          # hide subagents in ari watch that went quiet for 5 min without finishing
ascii = false               # ASCII-only glyphs everywhere
max_width = 0               # compact row width; 0 = terminal width
retention_days = 7          # session logs older than this are deleted (at least 1)

[view]                      # how each surface draws itself: full · calm · constellation
watch = "full"              # ari watch (the panel)
hub = "full"                # ari hub
hub_all = "full"            # ari hub --all

[notify]
desktop = true              # a desktop notification when a session flips
on = ["needs_you", "stuck", "blocked", "done"]   # also: idle
ntfy = ""                   # optional, e.g. "https://ntfy.sh/your-topic"; the only thing ARI ever sends off your machine

[chain]
commands = []               # your previous statusline commands (ari setup fills this)
timeout_ms = 300            # a chained command slower than this is skipped

[thresholds]
ctx_warn = 60               # context % turns yellow
ctx_crit = 80               # and red
stuck_repeats = 3           # repeats before STUCK
compact_pct = 90            # context % where auto-compaction happens, for the forecast

[prices.claude-opus-5-5]    # optional, $ per million tokens, for the cache-savings estimate
input = 0.0
cache_read = 0.0

[colors]                    # override one colour of the theme: a name, "#rrggbb" or 0–255
good = "#5fd787"            # roles: good warn bad accent highlight info muted bar
```

| Theme | Looks like |
|---|---|
| `default` | the terminal's own green, yellow, red, cyan |
| `soft` | muted 256-colour pastels |
| `vivid` | the bright variants, for dark low-contrast terminals |
| `colorblind` | blue for good, orange for bad, never red against green |
| `mono` | no colour; dim and bold only |
| `claude` | Claude Code's orange and grey |
| `codex` | Codex's colours |
| `deepseek` | DeepSeek's colours |
| `auto` | follows the app of the watched session; multi-session screens use `default` |

Without `[prices]`, savings are estimated from the session's own cost. `NO_COLOR` is respected; glyphs always carry the meaning, so nothing relies on color alone.

## FAQ

**Does it cost tokens?** No. Hooks only append to a local log and print nothing; the statusline is never sent to the model.

**Does it slow Claude down?** Hooks take about 3 ms (p95; Stop, once per turn: about 4 ms) and never block; the statusline renders in about 3–6 ms.

**Will it break my statusline?** No. Your command runs with the same input and its output is printed untouched. If it fails or takes longer than 300 ms, only it is skipped. `ari uninstall` puts your settings back exactly.

**Where does my data go?** Nowhere, unless you set `notify.ntfy`: then the notification text goes to that URL. Everything else stays in `~/.local/state/ari/` and is pruned after 7 days. `quality.jsonl` there keeps per-session counts only (reads, edits, failed calls, retries, prompts and the model name; no prompts, paths or output), pruned with `retention_days`. No telemetry.

**Windows?** Not yet; planned for 1.1.

## Contributing

Issues and PRs welcome; see [CONTRIBUTING.md](CONTRIBUTING.md). Design notes live in [docs/](docs/): the [PRD](docs/PRD.md) and [DECISIONS.md](docs/DECISIONS.md) (where ARI deviates from the PRD and why).

## License

[MIT](LICENSE)

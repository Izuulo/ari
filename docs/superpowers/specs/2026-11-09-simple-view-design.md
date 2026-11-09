# Simple views: Calm and Constellation

## Why

On a busy session, watch and the hub fill with bash commands, paths, token counts and costs. That's what the owner needs when they're debugging. But it buries what makes ARI nice to look at: agents working, helpers spawning, and messages flying between them. The owner wants to show ARI to friends without it looking intimidating, and sometimes just wants a calmer screen for themself. They asked for more ASCII animation.

The full detail view stays exactly as it is, one key away.

## Goals

1. **Two new views.** **Calm** keeps today's layout but puts plain-English verbs in place of commands and drops the numbers. **Constellation** shows each session as an animated star system. Both exist for watch (the panel), the hub, and hub --all.
2. **`v` cycles views:** full → Calm → Constellation → full. It works in watch, the hub and hub --all.
3. **Each screen remembers its own view.** Watch, the hub and hub --all each keep their last view across restarts, independently.
4. **Alerts stay loud.** An agent waiting for approval, the session needing you, or a fresh error is impossible to miss in both simple views.
5. **`ari demo` supports both views and starts in Constellation.** A new `--team` option replays a recording with messages, so the owner can show ARI off without a live agent running.

## Non-goals

- The Workshop direction (agents as little characters). The owner looked at it and turned it down.
- A `--view` flag. `v` plus memory covers the need. *(call)*
- Simple versions of the `watch --all` session list, zoom, or the settings screen. Those screens stay as they are. *(call)*
- Changes to the Claude Code statusline row.
- Any change to what ARI reads or stores. The simple views are drawing only.

## Anti-goals (a failure even if it works)

- Any change to the full view. Someone who never presses `v` sees exactly today's ARI.
- A simple view that hides an approval request or an error. Calm still means you notice what needs you.
- Animation that makes ARI noticeably heavy on CPU, or that flickers.
- Constellation turning into a screensaver that ignores reality. Everything it draws (bodies, comets, colours) comes from real state.

## Reference

Animated mockups the owner approved (directions A and B):
`.superpowers/brainstorm/91386-1794264061/content/directions.html` (gitignored; open it in a browser). The ASCII in this doc is from those mockups.

## How it works (decided)

### Switching and memory

- `v` cycles **full → Calm → Constellation → full** on watch's panel, the hub (`Screen::Hub`) and the combined hub (`ari hub --all`, and `x` from the `watch --all` list). `v` isn't bound anywhere today.
- The current view per screen is saved in `config.toml` the moment it changes:
  ```toml
  [view]
  watch = "calm"           # full | calm | constellation, default full
  hub = "full"
  hub_all = "constellation"
  ```
  Save it the way the settings screen already saves (edit `App.file`, then `config::save_to`), so other fields and the hot-reload path keep working. Missing or unknown values mean `full`.
- **The hub and hub --all are one screen in code.** `HubView.all` is toggled by `a`, and `Mode::All` + `x` opens the same combined drawing. `a` stays live in every view. The view shown is always the remembered one for whichever of the two is now on screen: `hub` when `all` is off, `hub_all` when it's on. So `a` from Calm hub can land on Constellation hub --all, which is the intended effect of remembering per screen. *(call)*
- **The view lives in `Config` and is read every frame.** Every open window follows `config.toml` through the existing `reload_config`, so pressing `v` in one watch window also changes the others. That's how every other setting already behaves, and it's the least code. *(call)*
- **`--once` and non-terminal output always print the full view,** whatever is remembered. That includes `ari demo --once`, so scripts and pipes never get a star field. *(call)*
- **`v` on the hub's Graph or Zoom screens** goes back to `Screen::Hub` and advances the view. *(call)*
- The settings screen (`s`) gets three items, "watch view", "hub view" and "hub --all view", that cycle the same three values. *(call: it's how every other remembered option is reached)*
- `--ascii` still applies. Glyphs that are new to ARI (`✦`, `✺`, `◌`, `☼`) get ASCII fallbacks in `ascii.rs`, keeping the existing one-column-in, one-column-out rule. Glyphs that already have a fallback (`◉`→`@`, `●`→`*`) keep it, so the full view doesn't change.
- In the simple views, stale agents show `z` (snoozing) where the full view uses `?`. *(call)*

### Plain-English verbs (shared by both views)

One function maps an agent's current activity to a short phrase. The hub's state glyph logic already decides working, done, stale, failed and waiting. This only adds the words.

| Activity | Phrase |
|---|---|
| Bash running tests (ARI already detects test runs) | running the tests |
| Bash (other) | running a command |
| Read | reading |
| Grep, Glob | searching the code |
| Edit, Write, MultiEdit, apply_patch | writing code |
| Task / Agent (spawning a subagent) | sending out a helper |
| SendMessage | messaging `<name>` |
| WebFetch, WebSearch | browsing the web |
| TodoWrite, plan tools | planning |
| Skill | using a skill |
| other tool | working |
| main between turns | waiting for you |
| finished | finished · all green (when that agent's own last test command passed; test runs aren't tracked per agent, so read it from the agent's recent Bash calls), else finished |
| stale | taking a break |
| last call failed | hit a snag |
| waiting for approval | needs you |

Phrases have no paths, commands or numbers. While working, the phrase ends in animated dots (`.` `..` `...`, cycling about every 300 ms). *(call: the exact wording; the builder may tune it)*

### Calm

**Watch:**
```
╭─ ARI ──────────────────────────────────────╮
│  ● on track · auth refactor                │
│  ▰▰▰▰▰▰▰▰▰▰▱▱▱▱▱▱▱▱ 4 of 7 done            │
│                                            │
│  ⠴ main     running the tests.             │
│  ⠇ Explore  searching the code...          │
│  ✓ tests    finished · all green           │
│  z Plan     taking a break                 │
│                                            │
│  memory ██████████░░░░ 71% full            │
│  v constellation                           │
╰────────────────────────────────────────────╯
```
- Verdict, label, task progress with a shimmer (a bright cell sweeps along the filled part every few seconds), one line per agent, and context as "memory … % full".
- No cost, time, 5h/7d limits, errors list or files. Errors show up as the alert banner instead (below).
- Finished and stale agents follow the existing `auto_clear` / `hide_stale` settings.
- The footer hint is `v constellation`.

**Hub:** the graph alone, full width and full height. The agents table, zoom pane, context, files, messages log, activity lanes and tasks board are all hidden.
```
            ╭─ ⠸ main ─────────╮
            │ running tests    │
            ╰────────┬─────────╯
   ┌─────────────────┴──────────────────┐
╭─ ⠦ Explore ─╮   ╭─ ✓ tests ──╮   ╭─ z Plan ────╮
│ searching   │   │ all green  │   │ resting     │
╰──────┬──────╯   ╰────────────╯   ╰──────┬──────╯
       ╰┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄●┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄╯
              Explore → Plan  “found it in auth.rs”
 3 helpers · 1 message flying          v constellation
```
- Nodes show the verb in place of the command line. Reuse the existing graph layout; only node contents and the message animation change.
- **Messages glide.** For a few seconds after (the existing in-flight window is 2 s; the builder may stretch it to 3–4 s for the simple views only) a `SendMessage`, a `●` travels along that edge from sender to receiver. Under the graph, a caption shows `from → to  “first ~40 chars of the message”`. Several in-flight messages each get their own dot, and the caption shows the newest.
- Keys: `↑↓` select a node, `⏎` opens today's full zoom for it (`esc` comes back to Calm), and `q`, `s`, `x` and `v` work as usual. `g` and `z` do nothing. *(call)*

**Hub --all:** a grid of session cards in place of the merged graph.
```
╭───────────────────────────╮ ╭───────────────────────────╮
│ ● ~/ARI                   │ │ ● ~/blog                  │
│ building the hub          │ │ writing a post            │
│                           │ │                           │
│ ⠋ ⠙ · ✓                   │ │ ⠸                         │
╰───────────────────────────╯ ╰───────────────────────────╯
```
- One card per live session: a pulsing status dot in the verdict colour, the session name, the main agent's verb (or the needs-you ask), and one glyph per agent.
- Cards flow in as many columns as fit (minimum card width around 28), newest activity first.
- Header: `N sessions · M agents working`.

### Constellation

**Watch:**
```
╭─ ARI ──────────────────────────────────────╮
│      ·      ··········· ·                  │
│   ✓ tests                 ·····            │
│  ··                ⠸          ··           │
│  ·              ( main )       ·  *        │
│  ··                           ⠦ Explore    │
│     ·····               ·····              │
│ ·            ··········      ◌ Plan        │
│  main is running the tests    3 helpers out│
╰────────────────────────────────────────────╯
```
- Main sits at the centre as `( main )` with its spinner above it. Subagents orbit on a dotted ellipse, spaced evenly, each drawn as glyph plus name in its state colour. One orbit takes about 20 s.
- The ellipse scales with the pane. A faint twinkling star field (`·` and `*`) fills empty cells, deterministic from position and time.
- The bottom line reads `main is <verb>` plus `N helpers out`.
- Comets: on a `SendMessage`, a `✦` with a short `·` trail flies from the sender's current position to the receiver's over about 3 s.
- More than 8 helpers: draw the 8 most recently active and show `+N more` in the bottom line. *(call)*

**Hub:** the same scene with bigger radii, plus each satellite's verb under its name, and the comet caption (`Explore → Review  “green now”`) shown while a comet is in flight. Keys: `q`, `s`, `x` and `v`. *(call: nothing to select in a sky)*

**Hub --all:** a galaxy. Each live session is a small system: a `✺` sun in its verdict colour, its agents orbiting as `●`, and its name underneath. Systems are spread evenly over the pane, and each spins at a slightly different speed so the sky doesn't move in lockstep. Cross-session messages (already matched by the hub --all merge) fly as comets between systems.

**Too small:** if the pane can't fit the orbit (under about 30×8 for watch), draw Calm instead. Hub and hub --all keep their existing too-small message.

### Alerts (both simple views)

- **A full-width banner on the top row:** `! <agent> needs you: <ask>`, red, blinking (on 500 ms, off 500 ms) for as long as the wait lasts. If several are waiting, show the first and add `(+N)`.
- **New errors:** the same banner style, `✗ <agent> hit a snag`, for 5 s after each new error. ARI already tracks errors with timestamps.
- **In Constellation,** the body that needs you turns red and pulses (`◉`/`○`). In hub --all, the whole system's sun blinks `!`.
- **In Calm hub --all,** the card's dot blinks red and its line shows the ask.
- The existing `bell` setting fires exactly as it does today, so no new sound logic is needed.

### Animation and redraw

- Everything animated is a pure function of `(state, now_ms, area)`, like `bar::spinner`. That keeps it testable.
- Constellation always redraws at the existing `FAST` rate (100 ms). Calm uses `FAST` only while something animates (spinners, dots, a message in flight, a blinking banner), as watch already does.
- Draw with ratatui into the buffer like the other panes. No new dependencies.

### Demo

- `ari demo` starts on the panel in Constellation, and `v` cycles views as normal.
- **`ari demo --team`** replays `fixtures/team` (already used by `tests/hub.rs`: several helpers and 15 `SendMessage` calls) in place of `fixtures/stuck`, so comets and gliding messages have something to show. Plain `ari demo` keeps the "stuck" story unchanged. The owner chose this.
  - `--team` is Claude-only: combined with `--codex` or `--deepseek`, it exits with a one-line usage error. *(call)*
  - How `pace` handles the team recording (it has no Stuck stretch) is left to the builder.
- Demo never writes `config.toml`. Its view lives in memory only. *(call: a demo shouldn't change your real settings)*

## Testing

- An insta snapshot per view per screen (watch, hub, hub --all × Calm, Constellation) at a fixed `now_ms`, using the existing test fixtures. Add one snapshot with a waiting agent to check the banner.
- Unit tests for the verb table, the `v` cycle and its config round-trip (including a missing `[view]` table meaning full), and the ASCII fallbacks.
- All existing full-view snapshots stay unchanged. That is anti-goal 1's check.

## Left to the builder

- Module layout. *(suggestion: `src/simple/` with `verbs.rs`, `calm.rs` and `sky.rs`, called from the panel and hub draw paths when the view isn't `full`)*
- Exact radii, speeds, star density and colours within the theme's roles (`good`, `warn`, `bad`, `accent`, `muted`).
- How Calm picks which message edge to animate when the graph collapsed it.
- Version bump: minor (0.4.0). This is a headline feature.

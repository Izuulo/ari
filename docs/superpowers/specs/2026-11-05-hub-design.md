# Hub: ARI's expanded mode

Approved mockup: https://claude.ai/artifact/HoFqkqMBRz2r1c2YyCvJRi (168×49, three screens). The owner accepted every suggestion in its "Decisions for you" section (Q1–Q6, below).

## Goal

`ari watch` gets a full-terminal mode, the hub. It shows each agent of the session in its own watch window, and a graph of who spawned whom and who is messaging whom. It also shows each agent's own context window. ARI stays zero-token and read-only.

## Entry and keys (Q1)

- `x` inside `ari watch` toggles the hub. `ari hub [--session <id>] [--ascii]` starts in it. `ari demo` supports `x` too.
- In the hub: `↑`/`↓` (`k`/`j`) select an agent, `⏎` zooms into it, `g` opens the graph screen, `h` returns to the hub screen, `z` shows or hides finished agents' watch windows. `pgup`/`pgdn` scroll the zoom timeline, `esc` goes back one level (zoom/graph → hub → panel), `x` returns to the panel, `q` quits.
- The panel's key hint gains `x hub`.

## Screens

**Hub** (the mockup's first view), top to bottom:
1. Top bar: `ARI hub`, project, label, clock.
2. Strip: the same row as the statusline (`bar::render`, no color codes), verdict first. When an agent is waiting on you, its name follows the verdict.
3. Graph (≈60% width) | agents table (the rest).
4. Watch windows, one per running agent: 4 across (full) or 2 across (compact).
5. Messages (≈60%) | activity lanes (the rest). In compact, messages fill the full width and lanes are hidden.
6. Key row.

**Graph screen**: the graph uses the full width at about 60% of the height, with the tasks board below (pending | in progress | done, showing owner and blocked-by).

**Zoom**: on the left, one agent's timeline (tool calls with result and duration, failure detail lines, messages in and out, spawn prompt, current action). On the right: context window, messages, files, tasks & errors.

## Graph rules

- Two tiers. The lead (main) is at the top and its direct children are in a row below. An agent spawned by a child is not drawn as a node; the child's node title shows `+N sub`.
- Spawn lines are light. A finished agent's node and line are dim.
- Messages between two children: a heavy arc below the row, labelled `⇄ N · "last line"`. At most 2 arcs are drawn. Further pairs are listed in one line: `also: a⇄b 3 · c⇄d 1`.
- Messages between the lead and a child ride that child's spawn line, drawn heavy in the message color with `⇄ N` next to the drop. (This differs from the mockup, which used a separate route; the spawn line generalises to any child.)
- A message less than 1.6 s old draws a `●` travelling its edge. While one is in flight the watch loop runs at 10 fps.
- Too many children to fit at the 16-column minimum node width: the last node becomes `+N more`. Running agents come first.

## Data (all from hooks and transcripts)

| What | Source |
|---|---|
| Spawn parent, name, prompt | `PreToolUse` of `Agent` (input `name`, `subagent_type`, `prompt`) is held as a pending spawn. A new `agent_id` claims the oldest pending spawn of its type, or else the oldest pending spawn. |
| Messages | `PreToolUse` of `SendMessage` (input `to`, `message`). The sender is the event's `agent_id` (or main). `to` resolves by name; `team-lead`/`main`/`lead` mean main; an unknown name is kept as written. |
| Per-agent recent calls | Ring buffer of the last 40 calls per agent: tool, action, ok, duration, failure line. |
| Activity lanes | Per agent, event counts in 12 s buckets, last 50 buckets. |
| Files per agent | `PostToolUse` of Edit/Write/MultiEdit: edits, lines added, lines removed (counted in the hook from the input strings). |
| Waiting on you, per agent | New `PermissionRequest` hook. It sets that agent's wait (time, action). Any later event from the same agent clears it, and `UserPromptSubmit` clears all waits. Any wait makes the verdict `NEEDS YOU`. |
| Context per agent | The latest request's input side (input + cache write + cache read) from each agent's own transcript. Percent of the main window size (`usage.ctx_size`, else 200k). |
| What filled it (Q5: shown, labelled "estimate") | Each request's growth in input side, split equally among the tools whose results arrived since the previous request. Growth with no tool results counts as `messages`. The first request, or any shrink (compaction), resets to `base`. |
| Tasks board | Existing task tracking, plus `owner` and blocked-by from `TaskUpdate` when present. |

Per-agent cost is not available (the statusline cost is session-wide), so tables and windows show tokens, not dollars.

## Sizes (Q3)

- Full: at least 160×45.
- Compact: at least 120×30. Two watch windows across, no arc labels, no lanes.
- Smaller: the hub doesn't draw. The panel shows with its hint replaced by `hub needs 120×30 · this window is W×H · x panel`, and the hub appears as soon as the window is big enough.

## Read-only (Q2)

The hub never sends anything to Claude Code. A waiting agent shows its exact pending call and `approve or deny it in the Claude Code window`.

## Plain subagents (Q4)

The graph always shows. Without `SendMessage` it is a spawn tree with dim finished nodes and `↩ result` on returned agents' lines. With no agents at all it shows the lead node and `no agents yet`.

## Paging (Q6)

The watch windows show the page that contains the selected agent. When agents overflow, the last window's title carries `+N more ›`.

## Out of scope

- Agent-team teammates in split panes (separate Claude Code sessions; `ari watch --all` already lists them).
- Mailbox and task-directory files under `~/.claude/teams` and `~/.claude/tasks`.
- Per-agent compaction forecasts.
- Approving prompts.

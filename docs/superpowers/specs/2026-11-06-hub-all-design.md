# Hub --all: every session's agents in one hub

The user skipped the design questions, so the calls below are mine (marked **call**).

## Goal

`ari hub --all` opens one hub over every live session: each session is a node, its agents hang under it, and messages between sessions draw as arcs. ARI stays zero-token and read-only.

## Approach (call)

Don't write a new screen. Merge the live sessions into one synthetic `State` and feed it to the existing hub renderers (`hub::lines`). The graph's two tiers then line up for free: the root is **you**, the sessions are the first tier, and each session's own subagents fold into that node's `+N sub`. Table, tiles, zoom, messages, lanes and the tasks board all work unchanged.

## Merge rules

| Field | Merged value |
|---|---|
| Root agent | id `main`, name `you`. `Agent::label` shows `name` for `main` when it's set (sessions' mains never get a name, so nothing changes for them). |
| Session key | the first 8 chars of the session id |
| Session's `main` | id `<key>`, name = `session_name`, else `project`, else `<key>`; kind `session`; parent `main` |
| Other agents | id `<key>/<id>`; parent mapped the same way (none → `<key>`) |
| Session with no agents yet (transcript only) | still gets its `<key>` node, so every live session shows up |
| Messages | `from`/`to` mapped. A `to` that isn't an agent id in its session is matched (case-insensitive) against the other sessions' names and becomes that session's key; otherwise it stays as written, and the graph already skips unknown names. **call**: guessing that cross-session `SendMessage` addresses a session by its name. Sorted by `t`. |
| Errors | `agent` mapped |
| Tasks | ids and `blocked` get the `<key>/` prefix; `owner` is kept |
| `turn_active`, `turn_failed` | true if any session's is |
| `needs_you` | the first session that has one, as `"<name>: <ask>"`. An agent's `waiting` carries over on its own, so the verdict turns NEEDS YOU. |
| `first_t` / `last_t` | min of nonzero / max |
| `label` | `"N sessions"` |
| Tokens | `agents[<key>]` = the session's `main` ledger, `agents[<key>/<id>]` = its ledgers; the root ledger is empty |
| Usage | `project = "all sessions"`, `cost_usd` = sum, `five_h`/`seven_d` from the most recently active session (account-wide), `ctx_size` none |
| Context window | New `Agent.ctx_size: Option<u64>` (serde default). The merge sets it on each session's node and subagents from that session's `u.ctx_size`. `ctx_of` and `zoom::context` use `a.ctx_size.or(i.u.ctx_size)` before the 200k fallback, so each session's % is right for its own model. |

Sessions included: the same live set as the `--all` list (`sessions::is_live`).

## Entry and keys

- `ari hub --all` starts in the combined hub. `ari hub --all --once` prints it at 168×49, like `ari hub --once`.
- In `ari watch --all`'s list, `x` opens the combined hub (today the list ignores `x`). `esc` on the hub screen, or `x`, goes back to the list.
- `⏎ zoom`, `g graph`, `z`, `↑↓` and `q` work as in the single-session hub.
- Too small (under 120×30): `hub needs 120×30 · this window is W×H · x list`, with `x` and `q` live. The hub appears once the window is big enough.

## Not doing (calls)

- Jumping from a session node into that session's own hub/panel (open it from the list instead).
- A session-specific key hint: the key row still says `x back to panel`.
- A subagent on a different model from its session still uses its session's window size (same as the single-session hub today).
- Any write action (approve, kill, message): ARI stays read-only.

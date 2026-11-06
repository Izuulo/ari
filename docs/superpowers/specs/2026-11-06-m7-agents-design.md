# M7: Codex and DeepSeek Harness sessions, app themes

## Why

The owner wants ARI to be a universal tool rather than a Claude Code-only one. A friend codes with DeepSeek through **DeepSeek Harness** (`dsh`) and wants to use ARI. **Codex** (OpenAI's CLI) is the other agent asked for by name. Each app should also have its own built-in colour theme, with Claude orange and grey, that colours the bottom bar too.

## Goals

1. **Codex sessions in ARI.** A Codex session shows up in `ari watch`, `ari watch --all`, the hub and `ari recap`, with the same activity, tool calls, errors, tests and subagents that ARI shows for Claude Code. It also shows context fill and token totals, read from Codex's session log.
2. **DeepSeek Harness sessions in ARI.** A `dsh` session (TUI, web or headless) shows up in the same places, with activity, tool calls, errors, tests and subagents. Token and context numbers are not available (see Constraints).
3. **Each session knows its app.** ARI records which app ran a session (Claude Code, Codex or DeepSeek Harness) and shows it in the panel header and the `--all` and hub rows. Model names from OpenAI and DeepSeek read cleanly (`gpt-5.5-codex`, `deepseek-v4-pro`), as Claude ones do now.
4. **App themes.** Three new built-in themes, `claude`, `codex` and `deepseek`, sit in the settings screen beside the five existing ones. A fourth new value, `auto`, picks the theme of the app running the watched session. Themes colour the panel, the hub and the Claude Code statusline row.
5. **One setup command.** `ari setup` wires up every supported app it finds on the machine, and `ari uninstall` reverses it. `ari doctor` reports, per app, whether it is wired up.

## Non-goals

- **An ARI row inside Codex's or dsh's own footer.** As far as we can tell, neither app lets an outside program draw into its status bar. Codex's `/statusline` only picks built-in items. For these apps, ARI lives in its own pane (`ari watch`) and the hub.
- **Token, context or cost numbers for dsh.** dsh deliberately sends hooks an empty `transcript_path` and stores sessions zstd-compressed under `~/.dsh`. Its design notes say this is intentional. These numbers stay hidden for dsh sessions rather than being reverse-engineered.
- **Other agents** (OpenCode, Aider, Deep Code CLI, the community `deepseek-harness` pip package). Later, and only if asked for.
- **DeepSeek models used *inside* Claude Code** through DeepSeek's Anthropic-compatible API. These already work as Claude Code sessions. The only change for them is goal 3's clean model names.
- **Codex plan display** (its `update_plan` tool), cost for Codex, and Windows.

## Anti-goals (a failure even if it works)

- Any change in what a Claude Code user sees today when they don't pick a new theme. The existing default stays the default.
- A hook that slows down or blocks Codex or dsh. `ari hook` keeps its rules: always exit 0, never print, one append, nothing heavy.
- Friends having to edit Codex or dsh config files by hand.
- Showing `0` or an empty bar where ARI just has no data. Missing numbers are left out, the way the no-hooks state already works.

## What we know about the two apps

Researched 2026-11-06. **The builder should re-check these before relying on them**, and record real payloads with `ARI_RAW_LOG` as fixtures.

**Codex** ([hooks docs](https://learn.chatgpt.com/docs/hooks))
- Hooks are on by default. They're read from `~/.codex/hooks.json` or the `[hooks]` table in `~/.codex/config.toml`, and have the same shape as Claude Code's `hooks.json`.
- Events: `SessionStart`, `SessionEnd`, `SubagentStart`/`SubagentStop`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`/`PostCompact`, `UserPromptSubmit`, `Stop`, `Interrupt`, `PreModelRequest`/`PostModelRequest`.
- Stdin JSON carries `session_id`, `transcript_path`, `cwd`, `hook_event_name` and `model`. Tool events add `tool_name`, `tool_use_id`, `tool_input` and `tool_response`. `PostModelRequest` carries `token_usage`.
- Hooks that aren't managed must be approved once by the user with `/hooks` in Codex. Setup must say so.
- `transcript_path` points at Codex's session log (a JSONL "rollout" file). It is the source for context fill and token totals.
- Tool names differ from Claude Code's (for example a shell tool and `apply_patch` instead of `Bash` and `Edit`). The builder records the real ones.
- Codex is installed on the owner's machine, so real sessions can be recorded.

**DeepSeek Harness (`dsh`)** ([site](https://deepseek.com/en/harness/), [repo](https://github.com/deepseek-ai/deepseek-harness), `@deepseek-ai/dsh`, MIT, preview)
- A TypeScript agent built on Cordis plugins. It has `dsh tui`, `dsh web`, `dsh headless`, a desktop app and `dsh plugin add`. Its home directory is `$DSH_HOME`, defaulting to `~/.dsh`.
- It ships a **Claude Code hook bridge** plugin, `dsh-hooks-claude-code`. The bridge runs unmodified Claude Code shell hooks from a `hooks.json` given by a `configPath`, on seven events: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStart` and `SubagentStop`. Payloads have Claude Code's shape, and `transcript_path` is always `''`. (Repo note: `.agents/notes/archived/feature/2026-06-30-hook-bridges.md`.)
- The `configPath` is resolved once at boot and applies to every session. Per-project hook discovery is not supported yet.
- dsh is not installed on the owner's machine, and a real session needs a DeepSeek API key.

## How it works (decided)

- **The hook command names its app.** Codex and dsh call `ari hook --agent codex` and `ari hook --agent dsh`. Claude Code keeps calling plain `ari hook`, and a missing flag means Claude Code, so existing installs and logs stay valid. The app goes into the session's `meta.json` (`"agent"`), next to `cwd` and `transcript`. *(Builder's call, made in this design: a flag is simpler and more reliable than guessing from payload fields. dsh sends Claude Code-shaped payloads, so they can't be told apart.)*
- **Events go through the existing normalizer.** Codex and dsh events go through `event::normalize` into the same `events.jsonl`. Where an app's tool names differ, each is mapped onto the Claude Code tool it acts like (shell → Bash, `apply_patch` → Edit with its file paths, and so on). Everything downstream (errors, tests, STUCK, conflicts, hub, recap) then works without per-app branches.
- **Codex numbers come from its session log.** For a Codex session, ARI reads the rollout file named in `meta.json` to get context used out of the context window, and token totals. This stands in for the `usage.json` that Claude Code's statusline provides. Rate limits are shown if the log carries them. Cost is not shown.
- **App themes keep warnings readable.** An app theme changes the accent, highlight, info, muted and bar roles. Good, warn and bad stay green, yellow and red so their meaning doesn't change. `[colors]` overrides still apply on top. Proposed colours (the builder may tune them; owner to check):
  - `claude`: orange accent (around `#D97757`) on greys.
  - `codex`: near-white accent on cool greys.
  - `deepseek`: DeepSeek blue accent (around `#4D6BFE`) on blue-greys.
- **`auto`** uses the app theme of the session being watched. Where one screen shows many sessions (`--all`, hub `--all`), the frame keeps the `default` palette, and each row's app label is drawn in its own app's accent colour. The Claude Code statusline row under `auto` is always `claude`. The default theme stays `default`. *(Builder's call, made in this design: the owner didn't ask to change the default look.)*
- **Mixed-app screens never blend accounts.** Account limits are per app. Where `--all` and hub `--all` currently show one "most recent session" 5h/7d limit, they show one labelled limit per app that has them (`Claude 5h …`, `Codex 5h …`). Token totals add across apps. Cache savings and anything priced in dollars count only Claude Code sessions, since the prices are Claude prices. *(Builder's call, made in this design: showing one account's limits under another's label would mislead.)*
- **Setup wires each app it finds.** `ari setup` keeps doing what it does for Claude Code. Then:
  - **Codex:** if `codex` is on PATH or `~/.codex` exists, it merges ARI's hook entries into `~/.codex/hooks.json`, keeping anything already there, after showing the change and asking first (as it does for the statusline). It then tells the user to approve the hooks once with `/hooks` in Codex.
  - **dsh:** if `dsh` is on PATH or `~/.dsh` exists, it writes an ARI-owned Claude Code-style hooks file (`hook --agent dsh`). It then turns on `dsh-hooks-claude-code` with `configPath` pointing at that file, using whatever mechanism dsh documents (the home-level config patch is the likely one). The builder must confirm that mechanism in `docs/config-catalog.md` and `docs/cli-help.md` in the dsh repo.
  - **If the dsh bridge is already in use** with the user's own `configPath`, setup merges ARI's entries into that file instead of taking over the one `configPath` slot. Uninstall then removes only ARI's entries. If that file already has an `ari hook` entry (for example, the bridge points at `~/.claude/settings.json`), setup changes that entry to `--agent dsh` only if the file is used for dsh alone. Otherwise it leaves the file alone and has `ari doctor` say so, rather than logging sessions twice. *(Builder's call, made in this design: never silently turn off a user's own hooks.)*
  - **Claude Code is detected like the others** (`claude` on PATH or `~/.claude` exists). A dsh-only machine gets no stray `~/.claude`. *(Builder's call, made in this design.)*
  - `ari uninstall` removes exactly what setup added, for each app. `ari doctor` gets one line per app: found or not, wired or not, and what to do.

## What's left to the builder

- The exact Codex tool names and the mapping table. Record real sessions; fixtures go under `fixtures/codex/`.
- Where in the Codex rollout file the context and token numbers live, and how often to re-read it (reuse the debounced token update).
- How to turn on the dsh bridge and give it its `configPath`.
- How to test dsh. Unit tests run from hand-built fixtures that follow the bridge's documented payload shape. Recording a real session needs a DeepSeek API key or the friend's machine.
- How the app label looks in the panel header and in the `--all`/hub rows, within the existing layout.
- README: an "Other agents" section with what works per app and the one-time Codex `/hooks` approval. Add D-entries to `docs/DECISIONS.md`, and move the M7 lines out of `docs/ROADMAP.md`.

## Done means

- A recorded real Codex session replays in `ari watch`, showing its activity, errors, context fill, the Codex label and the `codex` theme under `auto`.
- A dsh fixture session replays with its activity and the DeepSeek label, with no token or context numbers shown.
- The existing Claude Code snapshot tests pass unchanged.
- `ari setup`, `ari uninstall` and `ari doctor` handle a machine with any mix of the three apps.

# M6: Richness — design

**Milestone done when:** the three panel problems are fixed (filtered test runs, time split, noise), and the panel shows the data Claude Code already sends but ARI drops: header badges, prompt cache state, background tasks and crons by name, resume cost, a turn timeline, trends, Claude's away summary, plus a two-column layout for wide panes, an interactive `ari watch --all` and hook errors in `ari doctor`. All fixture-tested, zero tokens.

Other harnesses and provider branding move from M6 to **M7**.

## Intent

ARI receives far more than it shows. Researched 2026-11-05 against Claude Code v2.1.318: real hook payloads (`fixtures/*/raw.jsonl`), real `usage.json` files and real transcripts. Most of this milestone is reading fields that already arrive. The statusline row does not grow: everything new goes in the panel or `--all`.

## Problems to fix (found in the live panel)

1. **Filtered test runs count as full runs.** `cargo test no_task_list` recorded `✓ 1`, so the trend dropped from 130 to 1. Cargo prints `N filtered out`, pytest `N deselected`. A run with either (N > 0) is *partial*: it never sets the headline count or the trend, unless it failed.
2. **Time split doesn't add up** (`claude 16% · tools 3% · you 2% · idle 16%` = 37%). Cause, from this session's events: a turn ending in `StopFailure` never starts idle time (55 min lost), and a `Stop` with background tasks running doesn't either. Both now start idle. Whatever is still unaccounted shows as `other N%` when ≥ 5%, so the parts always add to ~100%.
3. **Noise:** `cost of mistakes: … · 0s` drops the time when under 1 s; the panel's `no plan yet` row goes (the statusline already dropped it); agent names longer than the 12-cell name column (`general-purpose`) are cut to fit, so rows stay aligned.

## Data (verified unless marked)

| Source | Field | Shown as |
|---|---|---|
| hooks (most events) | `permission_mode` | badge: `PLAN`, `AUTO`, `EDITS`, `DON'T ASK`, `BYPASS` (red); nothing for `default` |
| statusline | `effort.level`, `fast_mode`, `workspace.git_worktree` / `worktree.name`, `pr.*`, `cost.total_lines_*` | badges row |
| statusline | `prompt_cache.warm`, `.expires_at`, `.last_miss_at`, `.last_miss_cause.causes[]` | cache row: `· cold in 4m`, `· cold`, `· last miss: ttl_expired_1h` |
| Stop / SubagentStop | `background_tasks[]` (`type`, `description`, `status`), `session_crons[]` (`schedule`, `prompt`; **shape unverified**, no real example) | rows under main in agents |
| SessionStart (resume) | `seconds_since_last_response`, `prompt_cache_likely_expired`, `estimated_cache_write_usd` (**unverified**, v2.1.251+) | `↻ resumed after 3h · cache cold · ~$0.40 to rebuild` until the next Stop |
| hooks | UserPromptSubmit → Stop/StopFailure, main tool calls | turn timeline |
| hooks + `limits.jsonl` samples | compaction time, ctx before/after | `⟲ 14:20 compacted (auto) · ctx 91% → 18%` |
| `limits.jsonl` samples | ctx and cost over time | sparklines; cost per hour |
| transcript | `system` / `away_summary` `content` | quoted line when idle or done; `--all` detail for quiet rows |
| transcripts | `attachment` `hook_non_blocking_error` (`hookName`, `stderr`) | `ari doctor` check |

Not taken: subagent stats from the Agent tool's `tool_response` (ARI already gets tokens, calls and model per subagent from its transcript); `turn_duration` and `compact_boundary` from the transcript (hooks give the same with no timestamp parsing); OpenTelemetry (needs the user's environment changed).

## Display

- **Badges row** under the verdict: `Opus 5.5 · high · fast · AUTO · ⎇ feature-x · PR #14 approved · +156 −23`. Parts are omitted when absent. The PR moves here from the session section.
- **Turns section** (new, last): newest first, one row per turn `14:02  4m12s  23 calls  ✗ 2  $0.41  fix the panel…`, compactions interleaved. Shown with ≥ 2 turns. It is trimmed first when the pane is short, so it fills whatever height is left.
- **Trends:** ctx row ends with a sparkline of the last 12 samples; cost row `$3.48 · $2.10/h ▁▂▃▅▆`. One sparkline helper, shared with the tests trend.
- **Two columns** at ≥ 120 columns: the header spans the width; agents and errors on the left; usage, session and turns on the right.
- **Interactive `--all`:** `↑`/`↓` (or `k`/`j`) select, `enter` opens that session's panel, `esc` goes back to the list. The selected row is reversed. A dim key hint is the last line.
- **Long agent lists** (added at the user's request): the default view's `… N more done` row names what's hidden (`✓ 27 more done · 24 general-purpose · 3 Explore · 9.1M tok (a)`); the expanded view (`a`) groups done agents into one row per kind and model (`✓ 20× general-purpose · sonnet-5-5 · 5.9M tok · 18m`) and folds no-call helpers into one row, so it stays a few rows instead of dozens.
- **Plan files (optional):** with no task list, a Markdown file the session edited that has `- [ ]` checkboxes counts as the plan (`plan m6-richness.md 3/14`). Sessions without the task tools then still show progress.
- **Glyphs** are single-width and have `--ascii` equivalents: `⎇`, `↳` (background), `◷` (cron), sparkline blocks `▁`–`▇`.

## Non-goals (later plans, each its own spec)

- **In-app pane:** ARI's panel docked inside Claude Code via a plugin mod (`Pane` / AbovePrompt band).
- **Subagent rows:** an `ari subagent-statusline` command, installed by the plugin through `subagentStatusLine`.
- **New hooks:** `PermissionRequest` (what is asked, plus the suggested allow rule), `PermissionDenied`, `Pre/PostModelSwitch` (silent fallbacks, switch cost).

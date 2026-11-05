# M3: Intelligence — design

**Milestone done when:** stuck detection, error resolution, burn rate, cache stats and per-agent tokens are covered by fixture tests.

## Intent

Numbers are what every other statusline shows. M3 is where ARI interprets them: it notices Claude going in circles, says how much caching saved, warns before the 5-hour limit runs out, and shows which agent is spending the tokens (PRD §7.4, §7.6, §7.7).

Error resolution already shipped in M1 (DECISIONS: moved because the verdict needed it).

## Goals

1. **Stuck detection** → `↻ STUCK · <why>`, between BLOCKED and RETRYING in priority, only while the turn is active.
2. **Transcript tokens:** session totals, output tokens, per-agent tokens, per-request cache hit, read incrementally from the main and subagent transcripts.
3. **Cache:** session hit %, savings in dollars (`≈`), cache-drop alert.
4. **Burn rate:** linear fit of the 5h `used_percentage` over the last 15 minutes; a warning only when the limit would run out before it resets.
5. **Surfaces:** `tokens` compact segment; panel session section gains tokens, cache and savings rows; agent rows gain `· 12k tok`; usage section gains the burn-rate line; header gains the stuck reason.

## Design

### Stuck (`state.rs`)

`stuck_reason(&self, repeats: u8) -> Option<String>`; `verdict_with(now, repeats)` uses it, and `verdict(now)` = `verdict_with(now, 3)`. Bar and panel pass `cfg.thresholds.stuck_repeats`.

- **Rule A, repeated identical calls:** within one agent's last 10 `PreToolUse` calls, the same `(tool, h)` appears ≥ `repeats` times with no file-modifying call (Edit, Write, MultiEdit, NotebookEdit) between the first and last of them. Task-plan tools and ToolSearch are ignored. Reason: `same call 3× · reading src/x.rs`.
- **Rule B, failing in a row:** an unresolved error whose `retries + 1 ≥ repeats` and whose last failure is among the last 10 tool results. Reason: `cargo build failed 4× in a row`.
- **PRD rule 3 (edit → run → fail ×3)** is implied by B: the same command failing 3 times with no success between is exactly B, whatever happened in between. Recorded as D17.

State keeps a ring of the last 30 calls `(agent, tool, h, act)`; per-agent filtering happens at verdict time.

### Tokens (`tokens.rs`)

`TokenCache { v, main: Ledger, agents: BTreeMap<String, Ledger> }` cached in `tokens.json` (session dir). `Ledger { offset, input, cache_write, cache_read, output, requests, last_id, last_hit, prev_hit, by_model: BTreeMap<String, ModelTokens> }`.

- Only `type == "assistant"` entries count; entries repeating the previous `message.id` are skipped (one API response spans several entries, D14). Partial last lines wait for the next read.
- Hit rate per request = `cache_read / (input + cache_write + cache_read)`; skipped for requests with < 1000 input-side tokens (noise).
- Subagent transcript path: `SubagentStop`'s `agent_transcript_path` if known, else `<main transcript without .jsonl>/subagents/agent-<id>.jsonl` (observed layout).
- Main transcript path: `usage.json` `transcript_path`, else `meta.json` `transcript`.
- `update(dir, st) -> TokenCache` reads only new bytes of each file. `ari watch` calls it when a transcript's mtime changes; `ari statusline` at most once per 2 s (skip if `tokens.json` is younger than 2 s).

### Cache stats and savings

- Session hit % = Σcache_read / Σ(input + cache_write + cache_read) over main and agents.
- **Drop alert** when the previous request's hit was ≥ 70% and the latest is < 30%: `⚠ cache hit fell 86% → 12% · compaction or a changed prompt`.
- **Savings ≈ Σ cache_read × (input_price − cache_read_price) / 1e6**, per model.
  - With `[prices.<model>]` in config: those prices.
  - Without them (the default): derive the effective input price `p` from the statusline's `total_cost_usd` and Anthropic's fixed ratios (output 5p, cache write 1.25p, cache read 0.1p): `p = cost / (input + 1.25·cache_write + 0.1·cache_read + 5·output)`, savings = `0.9 · p · cache_read`. No hardcoded prices to go stale. Recorded as D18. Shown only when cost is known.

### Burn rate (`burn.rs`)

- `ari statusline` appends `{t, pct, resets_at}` to `limits.jsonl` (session dir) when the 5h percentage changed or the last sample is ≥ 30 s old. It reads only the file's tail to decide.
- `projection(samples, now) -> Option<u64 /* ms until 100% */>`: least-squares slope over samples from the last 15 min; needs ≥ 3 samples spanning ≥ 2 min and a positive slope; samples from an earlier `resets_at` window are ignored.
- Shown (panel, usage section) as `▲ at this pace: 5h limit in ~55m` only when `now + eta < resets_at`.

### Surfaces

- Compact `tokens` segment: `6.2M tok`, after cost.
- Panel session section: `tokens  6.2M · 26.8k out · 2 turns`, `cache   ▰▰▰…▱ 86% hit`, `        saved ≈ $11.40 vs uncached`, the drop alert when active, then time and cost.
- Agent rows: `· 12k tok` when known.
- Header: STUCK reason line under the verdict.

## Testing

- Stuck: unit tests for each rule, including the edit-between exemption and parallel agents not adding up.
- Tokens: `fixtures/basic` and `fixtures/multi` transcripts → exact totals checked against a Python one-liner over the same files; dedupe test (duplicate ids counted once); partial line test.
- Savings: price-table path and derived path.
- Burn rate: rising, flat and too-few-sample series; a reset boundary; the "only before reset" rule.
- Snapshots: multi fixture panel with tokens.

## Left to the builder

Formatting of large numbers, alert wording.

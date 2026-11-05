# M3 Intelligence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** STUCK verdict, transcript token totals per agent, cache hit/savings/drop alert, 5h burn-rate warning — all fixture-tested.

**Architecture:** Stuck lives in the reducer (`state.rs`). Tokens (`tokens.rs`) and burn rate (`burn.rs`) are separate incremental readers with their own cache files, so the hook path stays untouched. `bar` and `panel` take an extra `&Extras { tokens, burn_eta }` argument.

**Tech Stack:** Rust; no new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-05-m3-intelligence-design.md`

## Global Constraints

- Hooks unchanged; statusline p95 stays < 50 ms (token refresh at most every 2 s, incremental).
- Burn-rate line only when projected exhaustion precedes `resets_at`; otherwise nothing.
- Savings always labelled `≈`; hidden when cost is unknown.
- Duplicate `message.id` entries count once.

## Review Focus

- Transcript missing or unreadable (path moved, permissions) → tokens absent, nothing else breaks. Test in Task 2.
- Transcript truncated/rotated (shorter than the cached offset) → ledger rebuilt from 0. Test in Task 2.
- Rate-limit window resets mid-series → old-window samples ignored. Test in Task 4.
- Many parallel agents with identical Read calls → not STUCK (per-agent windows). Test in Task 1.
- `limits.jsonl` growing forever → statusline only reads its tail; prune keeps the last 2 h on append beyond 1 MB. Test in Task 4.

---

### Task 1: Stuck detection

**Files:** Modify `src/state.rs`, `src/bar.rs`, `src/panel.rs`.

**Produces:** `State.calls: VecDeque<Call { agent, tool: String, h: u64, act: String }>` (cap 30); `pub fn stuck_reason(&self, repeats: u8) -> Option<String>`; `pub fn verdict_with(&self, now: u64, repeats: u8) -> Verdict`. Bump `CACHE_VERSION` to 2.

- [ ] Tests: 3× identical Read (same h) → `Stuck`, reason `same call 3× · reading a.rs`; Read, Edit, Read, Read → not stuck (edit between); 3× identical TaskUpdate → not stuck; three agents each reading the same file once → not stuck; Bash `cargo build` failing 3× (retries 2) → `Stuck`, reason `cargo build failed 3× in a row`; then success → `OnTrack`; turn ended → not `Stuck`; `repeats = 4` raises the bar. Panel header shows the reason; bar shows `↻ STUCK`. Commit.

### Task 2: Transcript tokens

**Files:** Create `src/tokens.rs`.

**Produces:** `pub struct Ledger { offset: u64, input, cache_write, cache_read, output: u64, requests: u32, last_id: String, last_hit: Option<f64>, prev_hit: Option<f64>, by_model: BTreeMap<String, ModelTokens { input, cache_write, cache_read, output: u64 }> }` with `total()`, `hit() -> Option<f64>`; `pub fn read_ledger(path: &Path, l: &mut Ledger)` (incremental); `pub struct TokenCache { v, main: Ledger, agents: BTreeMap<String, Ledger> }` with `total() -> Ledger` (sum) and `drop_alert() -> Option<(f64, f64)>`; `pub fn update(dir: &Path, st: &State, transcript: Option<&str>) -> TokenCache`; `pub fn cached(dir) -> TokenCache`; `pub fn subagent_path(main: &str, agent_id: &str) -> String`.

- [ ] Tests: basic fixture main transcript totals equal the Python reference (`input 18, cache_write …`, computed in the step and pasted as literals); duplicate-id lines count once; half line then completion; offset beyond length → rebuild; missing file → zero ledger; drop alert on 0.9 → 0.1. Commit.

### Task 3: Savings

**Files:** Modify `src/tokens.rs`.

**Produces:** `pub fn savings(l: &Ledger, cost_usd: Option<f64>, prices: &BTreeMap<String, Price>) -> Option<f64>`.

- [ ] Tests: with a price table `{input 5, cache_read 0.5}` and 1 M cache-read tokens → `4.5`; derived path: tokens `{input 1M, output 0, cache_write 0, cache_read 0}` and cost `$5` → p = 5e-6 → 1 M cache reads would save `4.5`; no cost and no prices → `None`. Commit.

### Task 4: Burn rate

**Files:** Create `src/burn.rs`; Modify `src/statusline.rs`.

**Produces:** `pub struct Sample { t: u64, pct: f64, resets_at: u64 }`; `pub fn record(dir: &Path, u: &Usage, now: u64)`; `pub fn load(dir: &Path, now: u64) -> Vec<Sample>` (last 15 min, tail read); `pub fn projection(samples: &[Sample], now: u64) -> Option<u64>` (ms until 100%, only if before reset).

- [ ] Tests: 1%/min rising from 40% at now-10m → eta ≈ 50 min, reset in 3 h → `Some(~3_000_000)`; same with reset in 20 min → `None`; flat → `None`; 2 samples → `None`; samples from an older `resets_at` ignored; `record` skips unchanged pct within 30 s. Commit.

### Task 5: Wire into statusline, watch, bar, panel

**Files:** Modify `src/statusline.rs`, `src/watch.rs`, `src/bar.rs`, `src/panel.rs`, tests and snapshots.

**Produces:** `pub struct Extras { pub tokens: Option<TokenCache>, pub burn_eta: Option<u64> }` (in `bar.rs`, `Default`). `bar::render(.., &Extras, ..)`, `panel::lines(.., &Extras, ..)`.

- [ ] Bar: `tokens` segment `6.2M tok` after cost. Panel: session rows (tokens/out/turns, cache bar + hit, saved ≈, drop alert), agent `· 12k tok`, usage burn line. Statusline: `burn::record` every render; `tokens::update` when `tokens.json` older than 2 s else `tokens::cached`. Watch: `tokens::update` on transcript mtime change, `burn::load` every refresh.
- [ ] Snapshot: multi fixture panel at the end with tokens from its transcripts. Bench statusline stays < 50 ms. Commit.

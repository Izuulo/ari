# Spend Alerts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A dollar limit per session and per day in `config.toml`; past either, the statusline's cost segment turns red and one notification fires.

**Architecture:** Every Claude Code session dir already has `limits.jsonl`, where `ari statusline` records the session's running `cost.total_cost_usd` with a timestamp (`burn::Sample.cost`). Today's spend is the sum, over session dirs touched since local midnight, of each session's cost increases after midnight (a drop counts as the counter restarting from 0, so it never subtracts). That sum is cached in `<state_root>/today.json` for 30 s, since summing reads several files. The check runs in `ari statusline`, the only place cost arrives. "Fire once" is a `create_new` marker file whose name carries the limit, so two statuslines at once can't both fire and raising the limit re-arms it. Notifications go through the notifier from the notifications plan.

**Tech Stack:** Rust 2024, serde, toml, libc (`localtime_r`/`mktime`, already a dependency). No new crates.

**Spec:** `docs/ROADMAP.md` §7, item 11.

**Depends on:** `docs/superpowers/plans/2026-11-07-notifications.md`, merged first (or this branch cut from its branch). Uses its `notify::Note { title: String, body: String }`, `notify::title(dir: &Path) -> String`, `notify::send(note: &Note, n: &config::Notify)`, `config::Notify` (`desktop`, `ntfy`) and its `ALERTS` settings rows (`ITEMS` length 44 before this plan).

## Global Constraints

- Config: `[spend] session = 0.0`, `day = 0.0`, dollars; `0` means off; both off by default.
- Copy, verbatim. Statusline cost segment: `$5.12` red past the session limit; `$2.69 · $41.20 today` red past the day limit. Notifications: body `$5.12 spent in this session (limit $5.00)` with title `notify::title(dir)`; body `$41.20 spent today across sessions (limit $40.00)` with title `ARI`.
- "Today" is local time since midnight. Only Claude Code sessions have a cost; Codex and dsh add nothing. Sessions run without ARI's statusline (e.g. `claude -p`) aren't counted.
- Spend notifications use the notifier's channels (`notify.desktop`, `notify.ntfy`) but not `notify.on`: setting a limit is the opt-in. Red still shows with both channels off.
- ARI never sends anything to the model or blocks the user. Nothing leaves the machine except the opt-in ntfy URL.
- No new crates. CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`; snapshots via `INSTA_UPDATE=always cargo test`, review every `.snap` diff.
- Branch `spend-alerts`. Commits `feat(spend): …` / `docs(changelog): …`, ending with the attribution line. PR with a bullet body.
- Patch version bump in `Cargo.toml`, `.claude-plugin/plugin.json` and `Cargo.lock`, plus a line under `## Unreleased` in `CHANGELOG.md`.

## Review Focus

1. **A session's cost drops** (Claude Code restarted or resumed and `total_cost_usd` began again): today's total never goes down, and spend after the drop still counts. Test `cost_drop_never_subtracts` in Task 2.
2. **A session that started yesterday and runs past midnight**: only post-midnight spend counts; one whose pre-midnight samples were trimmed (`limits.jsonl` rotation) counts from its first kept sample, an undercount rather than a double count. Tests `only_spend_since_midnight` and `trimmed_old_session_counts_from_first_sample` in Task 2.
3. **Two statuslines of one session cross the limit in the same second, or the user raises the limit after an alert**: one notification; a new limit alerts again once. Test `alerts_once_per_limit` in Task 3.
4. **Hand-written `session = -5`, `day = nan`**: treated as off, `ari doctor` names them. Test in Task 1.
5. **The day changes while the cache is fresh**: a cache from before midnight is never used after it. Test `cache_resets_at_midnight` in Task 2.

---

### Task 1: Config keys and local midnight

**Files:**
- Modify: `src/config.rs` (new `Spend`, `Config.spend`, `problems`)
- Modify: `src/panel.rs` (new `local_midnight` beside `local_hhmm` at line ~100)

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)] #[serde(default)] pub struct Spend { pub session: f64, pub day: f64 }`; `Config.spend: Spend` (serialised as `[spend]`).
  - `pub fn Spend::session_limit(&self) -> Option<f64>` and `day_limit(&self) -> Option<f64>`: `Some(v)` only when `v.is_finite() && v > 0.0`.
  - `problems` gains `spend.session: not a dollar amount ≥ 0 (<value>)` and the same for `spend.day`, when `!(v >= 0.0) || !v.is_finite()`.
  - `pub fn local_midnight(ms: u64) -> u64` in `panel.rs`: `localtime_r`, zero `tm_hour/min/sec`, `libc::mktime`, × 1000; fallback `ms - ms % 86_400_000`.

- [ ] **Step 1: Write the failing tests**:
  - `config.rs` `spend_parses_and_limits`: `parse("")` → `spend == Spend { session: 0.0, day: 0.0 }`, both limits `None`; `parse("[spend]\nsession = 5.0\nday = 40\n")` → `Some(5.0)`, `Some(40.0)`; `session = -5.0` → `None` and `problems` has one line containing `spend.session`; `day = nan` → `None` and a `spend.day` problem.
  - `config.rs` `round_trip` extended with `spend.day = 40.0`.
  - `panel.rs` `midnight_is_local_start_of_day`: for `ms = NOW`, `m = local_midnight(ms)` → `m <= ms`, `ms - m < 86_400_000 + 3_600_000` (DST), `local_hhmm(m) == "00:00"`, `local_midnight(m) == m`.
- [ ] **Step 2: Run** `cargo test config:: panel::tests::midnight` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test` — PASS.
- [ ] **Step 5: Commit** `feat(spend): spend limit config keys`.

---

### Task 2: Today's spend across sessions

**Files:**
- Create: `src/spend.rs` (add `pub mod spend;` in `src/lib.rs`)
- Modify: `src/burn.rs` (add `pub fn all(dir: &Path) -> Vec<Sample>`: every line of `limits.jsonl`, at most 1 MiB by rotation; `recent`'s 64 KiB tail can miss the pre-midnight baseline)

**Interfaces:**
- Consumes: `burn::Sample { t, cost, .. }`, `panel::local_midnight` (Task 1), `paths::write_atomic`.
- Produces:
  - `pub const TODAY_CACHE_MS: u64 = 30_000;`
  - `pub fn since(samples: &[Sample], started: u64, midnight: u64) -> f64`, over the samples that carry a cost, in time order:
    - baseline = the last cost before `midnight`; with none, `0.0` when `started >= midnight`, else the first cost at or after midnight;
    - result = the sum over the costs at or after midnight of `c - prev` when `c >= prev`, else `c` (a drop means the counter started again from 0), `prev` starting at the baseline and moving to each `c`.
  - `pub fn today(root: &Path, now: u64) -> f64`: for each dir in `root/sessions` whose `limits.jsonl` mtime is at or after `local_midnight(now)`: `since(&samples, started, midnight)` with `samples = burn::all(dir)` and `started` = `meta.json`'s `started`, else the first sample's `t`; summed.
  - `#[derive(Serialize, Deserialize)] struct Cached { midnight: u64, t: u64, usd: f64 }` in `root/today.json`.
  - `pub fn cached_today(root: &Path, now: u64) -> f64`: the cached `usd` when `midnight == local_midnight(now)` and `now - t < TODAY_CACHE_MS`; else `today`, then write the cache with `write_atomic`.

- [ ] **Step 1: Write the failing tests** in `spend.rs` (samples built with `burn::Sample { t, cost: Some(c), ..Default::default() }`; `M` a fixed midnight):
  - `only_spend_since_midnight`: costs `$1` at M−1h, `$3` at M−1m, `$4` at M+1h, `$6` at M+2h, started M−2h → `since == 3.0`.
  - `started_today_counts_in_full`: costs `$0.5` at M+1h, `$2` at M+2h, started M+30m → `2.0`.
  - `trimmed_old_session_counts_from_first_sample`: costs `$10` at M+1h, `$12` at M+2h, started M−5h, nothing before midnight → `2.0`.
  - `cost_drop_never_subtracts`: `$3` at M−1m, `$5` at M+1h, `$0.5` at M+2h, `$1.5` at M+3h, started M−1h → `2.0 + 0.5 + 1.0 = 3.5`.
  - `no_cost_samples_is_zero`: ctx-only samples → `0.0`.
  - `today_sums_sessions_touched_today`: temp root with three session dirs (two with today's samples, one whose `limits.jsonl` mtime is set before midnight with `File::set_modified`) → sum of the two.
  - `cache_resets_at_midnight`: write `today.json` with `midnight = M − 86_400_000`, `t = now − 1_000`, `usd = 99.0` → `cached_today` returns the freshly computed value, not `99.0`; a cache with today's midnight and `t = now − 1_000` is returned as is.
- [ ] **Step 2: Run** `cargo test spend::` — FAIL.
- [ ] **Step 3: Implement** `burn::all` and `src/spend.rs` as above.
- [ ] **Step 4: Run** `cargo test spend:: burn::` — PASS.
- [ ] **Step 5: Commit** `feat(spend): today's spend across sessions`.

---

### Task 3: Red cost segment and one alert per limit

**Files:**
- Modify: `src/spend.rs` (alerts)
- Modify: `src/bar.rs` (`Extras`, the `cost` segment in `render` at line ~338)
- Modify: `src/statusline.rs` (`own_row`)

**Interfaces:**
- Consumes: Task 1 limits, Task 2 `cached_today`; `notify::{Note, title, send}` (notifications plan).
- Produces:
  - `Extras.spent_today: Option<f64>`: today's total, set only when a day limit is on.
  - `bar::render` cost segment, with `c = u.cost_usd`: `over_session = cfg.spend.session_limit().is_some_and(|l| c >= l)`; `over_day = cfg.spend.day_limit().zip(x.spent_today).is_some_and(|(l, d)| d >= l)`. Text `${c:.2}`, plus ` · ${d:.2} today` when `over_day`; painted `"1;31"` when either is over; unchanged otherwise.
  - `pub fn alerts(dir: &Path, root: &Path, cost: Option<f64>, today: Option<f64>, cfg: &Config, now: u64) -> Vec<Note>`, with `cents = (limit * 100.0).round() as u64` and `midnight = local_midnight(now)`:
    - session: `cost >= limit` and `File::create_new(dir.join(format!("spend-alert-{cents}")))` succeeds → `Note { title: notify::title(dir), body: "$5.12 spent in this session (limit $5.00)" }`;
    - day: `today >= limit` and `File::create_new(root.join(format!("spend-day-{midnight}-{cents}")))` succeeds → `Note { title: "ARI", body: "$41.20 spent today across sessions (limit $40.00)" }`; on success, delete other `spend-day-*` files in `root` whose `midnight` differs.
  - `statusline::own_row`, inside the session-dir closure after `burn::record`: `if cfg.spend.day_limit().is_some() { x.spent_today = Some(spend::cached_today(&root, now)) }`, then `for n in spend::alerts(&dir, &root, u.cost_usd, x.spent_today, cfg, now) { notify::send(&n, &cfg.notify) }` (reuse the closure's existing `state_root` lookup).

- [ ] **Step 1: Write the failing tests**:
  - `bar.rs` `cost_turns_red_past_a_limit` (existing fixture, `cost_usd = 2.69`, `color = true`): no limits → segment `$2.69` unpainted; `spend.session = 2.0` → contains `\x1b[1;31m$2.69\x1b[0m`; `spend.session = 5.0` → no `1;31`; `spend.day = 40.0` with `spent_today = Some(41.2)` → contains `\x1b[1;31m$2.69 · $41.20 today\x1b[0m`; with `spent_today = Some(10.0)` → plain `$2.69`, no `today`.
  - `spend.rs` `alerts_once_per_limit`: temp dir/root, `session = 5.0`, cost `5.12` → one Note, body `$5.12 spent in this session (limit $5.00)`; again → none; limit raised to `10.0`, cost `10.5` → one Note with `(limit $10.00)`; cost `4.0` under a fresh limit → none.
  - `spend.rs` `day_alert_once_and_old_markers_go`: `day = 40.0`, today `41.2` → one Note with title `ARI` and body `$41.20 spent today across sessions (limit $40.00)`; again → none; a stale `spend-day-1-4000` file in `root` is gone afterwards.
  - `spend.rs` `no_limits_no_alerts_no_files`: defaults → empty, no `spend-*` files created.
- [ ] **Step 2: Run** `cargo test bar:: spend::` — FAIL.
- [ ] **Step 3: Implement** the field, the segment, `alerts`, and the statusline wiring.
- [ ] **Step 4: Run** `cargo test` — PASS, no snapshot changes. `scripts/bench-statusline.sh` with `[spend] day = 1.0` in its temp config still exits 0 (p95 < 50 ms).
- [ ] **Step 5: Commit** `feat(spend): red cost and one alert past a spend limit`.

---

### Task 4: Settings rows, docs, bump

**Files:**
- Modify: `src/settings.rs` (`ITEMS`, `value`, `step`, `reset`, tests; `settings_screen_snapshot`)
- Modify: `README.md`, `CHANGELOG.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

**Interfaces:**
- Consumes: `Config.spend` (Task 1).
- Produces:
  - Two `ALERTS` rows after the notification rows: `("ALERTS", "spend.session", "session $ limit")`, `("ALERTS", "spend.day", "daily $ limit")`. `ITEMS` length 46.
  - `const SPEND_STEPS: [f64; 11] = [0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0];`
  - `value`: `0` (or any value with no limit) → `off`; whole dollars → `$5`; else `$7.50`.
  - `step`: `+1` → the first step above the value (stays at `1000` at the top); `-1` → the last step below it (stays at `0`). A hand-typed `7.5` steps to `10` or `5`. `reset` → `0.0`.
  - Doctor needs nothing new: `config_status` already reports `problems`.

- [ ] **Step 1: Write the failing tests** in `settings.rs`:
  - `items_cover_every_role_and_feature`: length 46.
  - `spend_steps`: default `off`; `+1` → `$1`, `+1` → `$2`; from `7.5` `+1` → `$10`, `-1` → `$5`; from `1000` `+1` stays `$1000`; `reset` → `off`.
  - `every_item_has_a_value_and_steps_back` passes unchanged for both rows.
- [ ] **Step 2: Run** `cargo test settings::` — FAIL. **Step 3: Implement.** **Step 4: Run** `INSTA_UPDATE=always cargo test settings::`, review the snapshot diff (two new rows only), then `cargo test` — PASS.
- [ ] **Step 5: Real check.** Install (`rm ~/.local/bin/ari`, then `cargo install --path . --root ~/.local --force`), set `[spend] session = 0.01` in `~/.config/ari/config.toml`, send one prompt: the cost turns red and one notification arrives; a second prompt brings none. Set `day` just under today's spend: `$… · $… today` in red, one notification. Remove both.
- [ ] **Step 6: Docs.** README Config block: `[spend]` with `session = 0.0  # $ per session; 0 = off` and `day = 0.0  # $ per day across sessions (local midnight); 0 = off`, plus one line: past either, the cost turns red and one notification fires (see `[notify]`); only Claude Code sessions report a cost. Why-ARI table: add `Spend limits` row. CHANGELOG `## Unreleased`: one **Spend limits** bullet. Bump the patch version in `Cargo.toml` and `.claude-plugin/plugin.json`; `cargo check` refreshes `Cargo.lock`.
- [ ] **Step 7: Run** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — PASS.
- [ ] **Step 8: Commit** `feat(spend): spend limits in settings`, `docs: spend limits`, `chore: bump version`; push, open the PR with a bullet body.

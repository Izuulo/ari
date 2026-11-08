# Weekly Limit Forecast Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Warn before the 7-day limit runs out: the panel says `▲ 7d runs out Thu ~14:00 at this pace` and the statusline's 7d segment gains `▲Thu 14:00`, shown only when that moment comes before the limit resets.

**Architecture:** `src/burn.rs` already fits a line through the 5h `used_percentage` over the last 15 min (`projection`). The 7d limit moves far slower and spans many sessions, while a session's `limits.jsonl` only holds that session and is trimmed to 2 h. So 7d samples go to one account-wide file, `<state_root>/week.jsonl`, written by `ari statusline` (Claude Code only) at most every 10 min unless the value moved, kept for 24 h, and fitted over the last 6 h. The least-squares fit is shared with the 5h projection by turning its body into one `eta` function with the window as a parameter. The ETA rides in `bar::Extras` like `burn_eta`.

**Tech Stack:** Rust 2024, serde_json, libc (`localtime_r`, already used by `panel::local_hhmm`). No new crates.

**Spec:** `docs/ROADMAP.md` §7, item 2.

## Global Constraints

- Copy, verbatim: panel row `▲ 7d runs out <Day> ~<HH:MM> at this pace` (day = `Mon`…`Sun`, local time); statusline suffix ` ▲<Day> <HH:MM>` on the 7d segment. `--ascii` maps `▲` to `^` through the existing `ascii::map`.
- Shown only when the projected time is before `seven_d.resets_at`, as for 5h. Hidden with the 7d segment (`hide = ["7d"]`) and with `hide_panel = ["usage"]`.
- Only Claude Code's own 7d limit: the file is fed by `ari statusline`; Codex sessions never show it.
- No config keys, so no settings or `ari doctor` change.
- 5h behaviour unchanged: the existing `burn` tests pass untouched.
- No new crates. CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`; snapshots via `INSTA_UPDATE=always cargo test`, review every `.snap` diff (none expected).
- Branch `weekly-forecast` off `main`. Commits `feat(burn): …` / `docs(changelog): …`, ending with the attribution line. PR with a bullet body.
- Patch version bump in `Cargo.toml`, `.claude-plugin/plugin.json` and `Cargo.lock`, plus a line under `## Unreleased` in `CHANGELOG.md`.

## Review Focus

1. **Two statuslines appending to `week.jsonl` at once** (lines slightly out of time order): no panic, same ETA as sorted input. Test `week_fit_survives_out_of_order_lines` in Task 1.
2. **The week resets mid-window** (samples at 90% with the old `resets_at`, then 2% with the new one): old samples ignored, no warning from a "falling" line. Test `week_ignores_previous_window` in Task 1.
3. **A quiet night, then work**: samples older than 6 h don't count; under 1 h of new samples, no forecast. Test `week_needs_an_hour_of_samples` in Task 1.
4. **A session whose payload has no `rate_limits`** (API-key users): nothing is written to `week.jsonl`, nothing shown. Test `record_week_skips_missing_limit` in Task 1.
5. **`ari watch` tests reading the developer's real state dir**: the watch path reads `week.jsonl` from `App.root` (a temp dir in tests), never from `paths::state_root()`. Test `watch_reads_week_from_its_root` in Task 2.

---

### Task 1: 7d samples and the weekly fit

**Files:**
- Modify: `src/burn.rs`

**Interfaces:**
- Consumes: `usage::Limit { pct: f64, resets_at: u64 }`, existing `Sample`, `tail`, `parse`, `paths::write_atomic`.
- Produces:
  - Constants: `WEEK_WINDOW_MS: u64 = 6 * 3_600_000`, `WEEK_MIN_SPAN_MS: u64 = 3_600_000`, `WEEK_GAP_MS: u64 = 10 * 60_000`, `WEEK_KEEP_MS: u64 = 24 * 3_600_000`, `WEEK_MAX_BYTES: u64 = 256 * 1024`, `const WEEK_FILE: &str = "week.jsonl"`.
  - `fn eta(samples: &[Sample], now: u64, window_ms: u64, min_span_ms: u64) -> Option<u64>`: today's `projection` body with `WINDOW_MS`/`MIN_SPAN_MS` replaced by the parameters; it sorts its window by `t` first and uses `saturating_sub` for the span.
  - `pub fn projection(samples: &[Sample], now: u64) -> Option<u64>` = `eta(samples, now, WINDOW_MS, MIN_SPAN_MS)` (signature unchanged).
  - `pub fn record_week(root: &Path, l: Option<Limit>, now: u64)`: `None` → return. Else append `Sample { t: now, pct: l.pct, resets_at: l.resets_at, ..Default::default() }` to `root/week.jsonl` unless the last line (tail 512 B) has the same `pct` and `resets_at` and is under `WEEK_GAP_MS` old. Past `WEEK_MAX_BYTES`, rewrite keeping samples younger than `WEEK_KEEP_MS` (`write_atomic`).
  - `pub fn load_week(root: &Path, now: u64) -> Vec<Sample>`: tail 64 KiB, samples within `WEEK_WINDOW_MS`, sorted by `t`.
  - `pub fn week_projection(samples: &[Sample], now: u64) -> Option<u64>` = `eta(samples, now, WEEK_WINDOW_MS, WEEK_MIN_SPAN_MS)`.

- [ ] **Step 1: Write the failing tests** in `burn.rs` tests (reuse `series`, which steps one sample per minute; add a `series_h(start_pct, per_hour, hours, resets_in_s)` with one sample per 10 min):
  - `week_pace_warns_before_reset`: 40% → 46% over 3 h (2%/h), resets in 3 days → `week_projection` ≈ 27 h (within 1 min); resets in 20 h → `None`.
  - `week_needs_an_hour_of_samples`: 50 min of rising samples → `None`; samples 7 h old only → `None`.
  - `week_ignores_previous_window`: old window at 90% rising with one `resets_at`, then a flat 2% hour with a new one → `None`.
  - `week_fit_survives_out_of_order_lines`: the rising series with two neighbours swapped → same ETA as sorted, no panic.
  - `record_week_dedupes_and_loads`: same pct at `NOW` and `NOW + 60_000` → one line; same pct at `NOW + WEEK_GAP_MS` → two; new pct at `NOW + 61_000` → appended; `load_week(root, NOW + WEEK_WINDOW_MS + 1)` drops the first.
  - `record_week_skips_missing_limit`: `record_week(root, None, NOW)` creates no file.
- [ ] **Step 2: Run** `cargo test burn::` — FAIL.
- [ ] **Step 3: Implement** the constants and functions; turn `projection` into a call to `eta`.
- [ ] **Step 4: Run** `cargo test burn::` — PASS, including the untouched 5h tests.
- [ ] **Step 5: Commit** `feat(burn): sample and forecast the 7-day limit`.

---

### Task 2: Show it in the statusline and the panel

**Files:**
- Modify: `src/bar.rs` (`Extras`, `render`'s 7d segment)
- Modify: `src/panel.rs` (new `local_weekday` beside `local_hhmm` at line ~100; `usage()` at line ~1197)
- Modify: `src/statusline.rs` (`own_row`)
- Modify: `src/watch.rs` (`refresh`, next to `self.x.burn_eta` at line ~182)

**Interfaces:**
- Consumes: Task 1's `record_week`, `load_week`, `week_projection`; `panel::local_hhmm(ms) -> String`.
- Produces:
  - `Extras.week_eta: Option<u64>`: ms until the 7d limit at the current pace, only when before it resets.
  - `pub fn local_weekday(ms: u64) -> &'static str` in `panel.rs`: `Sun`…`Sat` from `localtime_r`'s `tm_wday`; fallback `(ms / 86_400_000 + 4) % 7` (1970-01-01 was a Thursday).
  - `bar::render`: when `shown("7d")`, `app_limits` empty and `x.week_eta` is `Some(eta)`, the 7d segment gets ` ` + `paint(color, "1;33", "▲<Day> <HH:MM>")` for `now_ms + eta`.
  - `panel::usage`: after the 5h `▲ at this pace` row, when `x.week_eta` is `Some(eta)`, a row `▲ 7d runs out <Day> ~<HH:MM> at this pace` in the same yellow bold style.
  - `statusline::own_row`: inside the session-dir closure, after `burn::record`: `if let Some(root) = paths::state_root() { burn::record_week(&root, u.seven_d, now); x.week_eta = burn::week_projection(&burn::load_week(&root, now), now); }` (merge with the existing `state_root` call for conflicts).
  - `watch::refresh`: `self.x.week_eta = (self.session_app == App::Claude).then(|| burn::week_projection(&burn::load_week(&self.root, now), now)).flatten();` Watch never writes `week.jsonl`.

- [ ] **Step 1: Write the failing tests**:
  - `panel.rs` `weekday_names`: `local_weekday(0)` is one of `Wed`/`Thu` (time-zone dependent); for every `ms` in a week stepped by 6 h, the result is in `["Sun","Mon","Tue","Wed","Thu","Fri","Sat"]`; seven consecutive days give seven distinct names.
  - `bar.rs` `week_forecast_on_the_7d_segment`: with the existing fixture `st`/`u` and `Extras { week_eta: Some(30 * 3_600_000), .. }` → row contains `format!("7d 34% ↺4d ▲{} {}", local_weekday(NOW + eta), local_hhmm(NOW + eta))`; `week_eta: None` → row identical to today's expected string; `hide = ["7d"]` → no `▲`; `ascii = true` → contains `^` and no `▲`.
  - `panel.rs` `week_runs_out_row`: `lines(...)` with `week_eta: Some(eta)` → plain text contains `▲ 7d runs out ` + weekday + ` ~` + `local_hhmm(NOW + eta)` + ` at this pace`; without → no `7d runs out`.
  - `watch.rs` `watch_reads_week_from_its_root`: `app_on(root, "s1", events)`; write a rising `week.jsonl` under `root` (via `burn::record_week` at past times) and a `usage.json` with `rate_limits.seven_day` resetting in 3 days; `refresh` → `app.x.week_eta.is_some()`; a Codex session (`meta.json` `"agent":"codex"`) → `None`.
- [ ] **Step 2: Run** `cargo test bar:: panel:: watch::` — FAIL.
- [ ] **Step 3: Implement** the field, `local_weekday`, the segment suffix, the panel row, and the two wiring lines.
- [ ] **Step 4: Run** `cargo test` — PASS with no snapshot changes (fixtures have no `week.jsonl`). `scripts/bench-statusline.sh` still exits 0 (p95 < 50 ms).
- [ ] **Step 5: Real check.** `cargo install --path . --root ~/.local --force` (rm first per local-install memory). With a session open for an hour of work, `tail ~/.local/state/ari/week.jsonl` shows samples about every 10 min or on change; the forecast appears only when the pace would finish before the reset.
- [ ] **Step 6: Commit** `feat(burn): 7d forecast in the statusline and panel`.

---

### Task 3: Document and bump

**Files:**
- Modify: `README.md`, `CHANGELOG.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

- [ ] **Step 1: README.** Why-ARI "Rate-limit foresight" row: `Warns when you'll hit the 5-hour or the weekly limit before it resets`. "What you get": one sentence after the statusline example: when the week's pace would run out the 7d limit before it resets, the 7d segment shows when (`▲Thu 14:00`) and the panel says `▲ 7d runs out Thu ~14:00 at this pace`; the pace is the last 6 h across all your Claude Code sessions.
- [ ] **Step 2: Changelog and version.** One bullet under `## Unreleased` (**Weekly limit forecast**). Bump the patch version in `Cargo.toml` and `.claude-plugin/plugin.json`; `cargo check` refreshes `Cargo.lock`.
- [ ] **Step 3: Run** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — PASS.
- [ ] **Step 4: Commit** `docs: weekly limit forecast` and `chore: bump version`, push, open the PR with a bullet body.

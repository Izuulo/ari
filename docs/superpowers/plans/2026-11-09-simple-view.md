# Simple Views (Calm, Constellation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two simple views, Calm and Constellation, to watch, the hub and hub --all. `v` cycles full → Calm → Constellation, each screen remembers its view in `config.toml`, alerts stay loud, and `ari demo` starts in Constellation and gains `--team`.

**Architecture:** One new module, `src/simple/`, holds the verbs, the alert banner, Calm and Constellation. It draws into a ratatui `Buffer` with the hub's helpers (`put`, `frame`, `to_lines`, `glyph`). Two dispatch points pick the view: `panel::lines` for watch and demo, and `hub::lines` on `Screen::Hub`. Both read the view from `Config`, so `--once` only has to reset it to full. Calm hub reuses `graph::draw`; only node text, the packet time and the legend row change.

**Tech Stack:** Rust 2024, ratatui, serde/toml, insta snapshots. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-09-simple-view-design.md`. Read it first. Its ASCII sketches are the target look. The animated mockups are at `.superpowers/brainstorm/91386-1794264061/content/directions.html`, directions A and B.

## Global Constraints

- The full view is unchanged. Every existing snapshot in `src/snapshots`, `src/hub/snapshots` and `tests/snapshots` must pass untouched.
- No new dependencies. ARI stays read-only and uses zero tokens.
- `v` cycles **full → calm → constellation → full**. TOML values are `full | calm | constellation`, and a missing or unknown value is `full`.
- The config table is `[view]` with keys `watch`, `hub`, `hub_all`.
- `--once`, piped output and `ari demo --once` always print the full view.
- `--ascii`: one column in, one column out. Glyphs that already have a fallback keep it (`◉`→`@`, `●`→`*`, `◌`→`o`).
- Simple views never show paths, raw commands, costs or token counts.
- Animation is a pure function of `(state, now_ms, area)`. Redraw uses the existing `FAST` (100 ms) rate.
- Branch `simple-view`. Make small commits ending in `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Bump the version minor to `0.4.0` in the last task.
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` green before every commit.

## Review Focus

1. **Tiny or short windows** (30×10, 12×4, 5×2) in Calm and Constellation: no panic, and no line wider than the window. *(tests in Tasks 4 and 6)*
2. **A crowd in the sky** (20 helpers): at most 8 bodies drawn, `+12 more` in the bottom line, and no panic. *(test in Task 6)*
3. **A hand-typed bad value** (`[view] watch = "zen"`) reads as full and doesn't knock out the file's other settings. *(test in Task 1)*
4. **Long or wide names** (CJK session names, 60-char labels) are clipped inside their card or label and never spill past the frame. *(tests in Tasks 5 and 6)*
5. **An empty session** (no agents yet, no hooks) shows "no agents yet" in every simple view, and nothing panics. *(tests in Tasks 4, 5 and 6)*

---

### Task 1: View setting in config and the settings screen

**Files:**
- Modify: `src/config.rs` (new types, `Config.view`, `Default`)
- Modify: `src/settings.rs` (`ITEMS` +3 rows, `value`, `step`, `reset`)
- Modify: `README.md` `## Config` section (document `[view]`)

**Interfaces:**
- Produces:
  - `pub enum Look { Full, Calm, Constellation }`: `Copy`, `Default = Full`, serde lowercase, unknown string → `Full`. Try `#[serde(other)]` on `Full`; if serde rejects it, use a `deserialize_with` that maps unknown strings to `Full`.
  - `impl Look { pub fn next(self) -> Look; pub fn name(self) -> &'static str }`.
  - `pub enum Surface { Watch, Hub, HubAll }`.
  - `#[serde(default)] pub struct Views { pub watch: Look, pub hub: Look, pub hub_all: Look }` with `pub fn get(&self, s: Surface) -> Look` and `pub fn bump(&mut self, s: Surface)`.
  - `Config.view: Views` (TOML table `[view]`).
  - Settings keys `view.watch`, `view.hub` and `view.hub_all` under group `"LOOK"`, labelled `watch view`, `hub view`, `hub --all view`.

- [ ] **Step 1: Write the failing tests in `config.rs` tests**

```rust
#[test]
fn view_defaults_cycles_and_parses() {
    assert_eq!(Config::default().view, Views::default());
    assert_eq!(Look::Full.next(), Look::Calm);
    assert_eq!(Look::Calm.next(), Look::Constellation);
    assert_eq!(Look::Constellation.next(), Look::Full);
    let c = parse("[view]\nwatch = \"calm\"\nhub_all = \"constellation\"\n");
    assert_eq!((c.view.watch, c.view.hub, c.view.hub_all), (Look::Calm, Look::Full, Look::Constellation));
    let mut v = Views::default();
    v.bump(Surface::Hub);
    assert_eq!(v.get(Surface::Hub), Look::Calm);
    assert_eq!(v.get(Surface::Watch), Look::Full);
}

#[test]
fn unknown_view_is_full_and_keeps_other_settings() {
    let c = parse("bell = true\n[view]\nwatch = \"zen\"\n");
    assert_eq!(c.view.watch, Look::Full);
    assert!(c.bell);
}
```
Add a `view` round-trip line to the existing `round_trip` test.

- [ ] **Step 2: Write the failing test in `settings.rs` tests**

```rust
#[test]
fn view_rows_cycle_and_reset() {
    let mut c = Config::default();
    assert_eq!(value(&c, "view.hub"), "full");
    step(&mut c, "view.hub", 1);
    assert_eq!(value(&c, "view.hub"), "calm");
    step(&mut c, "view.hub", -1);
    step(&mut c, "view.hub", -1);
    assert_eq!(value(&c, "view.hub"), "constellation");
    reset(&mut c, "view.hub");
    assert_eq!(c.view.hub, Look::Full);
}
```
The existing `every_item_has_a_value_and_steps_back` test covers the new rows. Update its count and the `ITEMS` array length to 49.

- [ ] **Step 3: Run the tests and check they fail**

Run: `cargo test --lib config:: settings::`
Expected: compile errors (`Look`, `Views` not found).

- [ ] **Step 4: Implement the types, the field and the three settings rows**

Step back with `next()` twice. Put the rows right after `ascii`.

- [ ] **Step 5: Run `cargo test` and check it passes.** The settings snapshot will change, because there are three new rows. Accept it with `cargo insta review` after checking that the only diff is the three rows.

- [ ] **Step 6: Document `[view]` in the README `## Config` section,** using the spec's TOML block.

- [ ] **Step 7: Commit:** `feat(config): [view] setting for watch, hub, hub --all`

---

### Task 2: `v` key and saving the view

**Files:**
- Modify: `src/panel.rs` (`View.next_look: bool`)
- Modify: `src/watch.rs` (`key()`, new `surface()`, `App::apply_look`, called next to `apply_edit`)
- Test: `src/watch.rs` `key_tests`

**Interfaces:**
- Consumes: `config::{Look, Surface, Views}` from Task 1.
- Produces:
  - `panel::View.next_look: bool`, a queued cycle that the frame closure takes. It mirrors `View.edit`.
  - `pub fn surface(view: &View, all_mode: bool) -> Option<Surface>` in `watch.rs`. It returns `None` while settings is open or the `--all` list shows. It returns `HubAll` when `view.hub` has `all` set or `all_mode` is true. Otherwise it returns `Hub` when the hub is open and `Watch` when it isn't.

- [ ] **Step 1: Write the failing tests in `key_tests`**

```rust
#[test]
fn v_queues_a_view_change() {
    let mut v = View::default();
    key(&mut v, KeyCode::Char('v'), None, None);
    assert!(v.next_look, "panel");
    let mut v = hub_view(Screen::Zoom, 2);
    key(&mut v, KeyCode::Char('v'), None, Some(3));
    assert!(v.next_look);
    assert_eq!(v.hub.unwrap().screen, Screen::Hub, "v leaves graph/zoom for the hub");
    let mut v = View::default();
    key(&mut v, KeyCode::Char('v'), Some(4), None);
    assert!(!v.next_look, "the --all list has no simple view");
}

#[test]
fn surface_picks_the_screen() {
    assert_eq!(surface(&View::default(), false), Some(Surface::Watch));
    assert_eq!(surface(&View::default(), true), None, "--all list");
    let h = |all| View { hub: Some(HubView { all, ..Default::default() }), ..Default::default() };
    assert_eq!(surface(&h(false), false), Some(Surface::Hub));
    assert_eq!(surface(&h(true), false), Some(Surface::HubAll));
    assert_eq!(surface(&h(false), true), Some(Surface::HubAll), "x from the --all list");
    assert_eq!(surface(&View { settings: Some(0), ..Default::default() }, false), None);
}
```
Note: `surface(&View::default(), true)` means `Mode::All` with no session open and no hub, which is the list. With `view.open` set it means an opened session's panel, so it's `Watch`. Add that case too.

- [ ] **Step 2: Run the tests and check they fail:** `cargo test --lib key_tests`

- [ ] **Step 3: Implement**
  - In `key()`, inside the hub branch, `v` sets `h.screen = Screen::Hub`, `h.scroll = 0` and `view.next_look = true`. In the panel `match`, `(KeyCode::Char('v'), None)` sets `next_look`.
  - Add `surface()`.
  - Add `App::apply_look(&mut self, view: &mut View)`. It takes `next_look` and returns if `surface(view, self.mode == Mode::All)` is `None`. Then it does `reload_config()`, then `self.file.view.bump(s)`, saves with `config::save_to` the same way `apply_edit` does (including the `cfg_mtime` update), and calls `derive_cfg()`. If `cfg_error` is set, it bumps `self.cfg.view` only and doesn't save, so a broken file is never overwritten.
  - Call it in the frame closure right after `apply_edit`.
  - Add `("v", "calm")` to the full panel's key hint in `App::panel` and to `hub_screen`'s keys in `hub/mod.rs`, before `s`.

- [ ] **Step 4: Run `cargo test` and check it passes.** Accept snapshot diffs only where they show the new `v` hint.

- [ ] **Step 5: Commit:** `feat(watch): v cycles the view and remembers it per screen`

---

### Task 3: Verbs and the alert banner

**Files:**
- Create: `src/simple/mod.rs` (add `pub mod simple;` to `src/lib.rs`)

**Interfaces:**
- Consumes: `state::{State, Agent, AgentStatus}`, `event::runs_tests`, `hub::{needs_you}`.
- Produces:
  - `pub fn phrase(st: &State, a: &Agent, now: u64) -> String`: the spec's verb table, plus `thinking` for an agent in a turn but between tools. The current tool is `a.recent.back()` while `a.running_since.is_some()`. `runs_tests(&rec.act)` decides `running the tests`. A finished agent gets `finished · all green` when its last `runs_tests` Bash call in `a.recent` had `ok == Some(true)`. `SendMessage`'s target is the part of `act` after the tool name, as the hub shows it.
  - `pub fn dots(now: u64) -> &'static str`: `"."`, `".."`, `"..."` or `""`, stepping every 300 ms.
  - `pub struct Banner { pub text: String, pub on: bool }`
  - `pub fn banner(st: &State, now: u64) -> Option<Banner>`. In priority order:
    1. The first agent with `waiting` gives `! {label} needs you: {action}`, plus ` (+N)` when more are waiting.
    2. Otherwise `st.needs_you` gives `! needs you: {ask}`.
    3. Otherwise the error with the newest `last_t` within 5000 ms gives `✗ {agent} hit a snag`.

    `on` is `(now / 500) % 2 == 0`.
  - `pub fn draw_banner(buf: &mut Buffer, area: Rect, b: &Banner)`: the full-width top row in `hub::err()` bold. It leaves the row blank when `!on`.
  - `pub fn moving(look: Look, st: &State, now: u64) -> bool`: `look == Constellation`, or `look != Full` and `banner(...)` is `Some`.

- [ ] **Step 1: Write the failing tests** in `src/simple/mod.rs`. Build agents with `hub::tests::kid` and push `CallRec`s into `recent`. Assert:
  - Bash `cargo test` (running) → `running the tests`; Bash `ls` → `running a command`.
  - Read → `reading`; Grep → `searching the code`; Edit → `writing code`; WebFetch → `browsing the web`; an unknown tool → `working`.
  - Done with a passing `cargo test` → `finished · all green`; done without one → `finished`.
  - Stale (last_t older than `STALE_MS`) → `taking a break`; last call `ok: Some(false)` → `hit a snag`; `waiting` set → `needs you`.
  - main with `turn_active == false` → `waiting for you`.
  - `phrase` never contains `/`, `$` or a digit for any of the above.
  - Banner: two waiting agents → text starts `! ` and ends `(+1)`. An error 4 s old shows `hit a snag`, and at 6 s it's `None`. `on` flips between `now = 0` and `now = 500`.

- [ ] **Step 2: Run them and check they fail:** `cargo test --lib simple::`

- [ ] **Step 3: Implement the functions above.**

- [ ] **Step 4: Run them and check they pass.**

- [ ] **Step 5: Commit:** `feat(simple): plain-English verbs and the alert banner`

---

### Task 4: Calm watch

**Files:**
- Create: `src/simple/calm.rs`
- Modify: `src/panel.rs` `lines()` (dispatch), `src/watch.rs` (`--once` forces full, hint skipped in simple views, `animating`)
- Test: `src/simple/calm.rs`, snapshot `calm_panel`

**Interfaces:**
- Consumes: Task 3, `hub::{Input, to_lines, put, glyph}`.
- Produces: `pub fn panel(i: &hub::Input, w: u16, h: u16, color: bool) -> Vec<Line<'static>>`.

- [ ] **Step 1: Write the failing tests**
  - `calm_panel` insta snapshot, using the same state as `panel::tests::wide_140`'s fixture at width 60. Lift that test's state builder into a `pub(crate)` helper if it's private.
  - `calm_panel_hides_numbers`: the plain text has no `$`, no `tok` and no `/`, and contains `on track` and `4 of 7 done`.
  - `calm_panel_banner`: with an agent `waiting`, at `now = 0` the first inner row starts `! `.
  - `calm_panel_tiny`: for `(w, h)` in `[(30, 10), (12, 4), (5, 2)]`, every line's width is ≤ `w`.
  - `calm_panel_empty`: `State::default()` shows `no agents yet`.

- [ ] **Step 2: Run them and check they fail.**

- [ ] **Step 3: Implement `calm::panel`** to the spec's watch sketch:
  - Rows: verdict with the label, the task bar with its shimmer, one row per agent (glyph, name, `phrase` + `dots`), and memory.
  - The footer reads `v constellation`.
  - Stale agents show `z`. The existing `auto_clear` / `hide_stale` / `view.cleared` filtering applies (reuse the panel's agent filter, or lift it into a shared fn).
  - The height is `min(h, rows needed)`.
  - The shimmer cell is `(now / 140) % 28` across the filled cells (bright when < filled).

- [ ] **Step 4: Dispatch it.** At the top of `panel::lines`, when `!view.recap` and `cfg.view.watch == Look::Calm`, return `simple::calm::panel(...)` (ascii-mapped when `cfg.ascii`). In `watch.rs`:
  - The `--once`/pipe branch sets `app.cfg.view = Views::default()` before drawing.
  - `App::panel` skips its key hint when `cfg.view.watch != Look::Full`.
  - `animating()` adds `|| simple::moving(self.cfg.view.get(s), &self.st, now)` for the current `surface`.

- [ ] **Step 5: Run `cargo test` and check it passes.**

- [ ] **Step 6: Commit:** `feat(simple): Calm watch panel`

---

### Task 5: Calm hub and hub --all cards

**Files:**
- Modify: `src/simple/calm.rs` (`hub`, `cards`)
- Modify: `src/hub/mod.rs` (`look()`, dispatch in `lines()`, `chrome(.., bare: bool)`)
- Modify: `src/hub/graph.rs` (Calm node text, packet time, legend row)
- Test: `tests/hub.rs` (snapshots `calm_hub_team`, `calm_hub_all`)

**Interfaces:**
- Consumes: Tasks 1 and 3.
- Produces:
  - `pub(crate) fn look(i: &Input) -> Look`: `i.cfg.view.hub_all` if `merged(i)`, else `i.cfg.view.hub`.
  - `pub fn hub(buf: &mut Buffer, i: &Input, v: &HubView)`, `pub fn cards(buf: &mut Buffer, area: Rect, i: &Input)`.
  - `chrome` gains `bare: bool`. When it's true, the status row keeps only the verdict (no ctx, 7d or tok).
  - `panel::View.look: Look` is the look of the screen being drawn. The frame closures set it every frame (`watch.rs` from `cfg.view.get(surface(..))`, and `demo.rs` the same way), so `key()` can gate hub keys without seeing config. In Calm, `g`, `z`, `c` and `b` do nothing in the hub branch.

- [ ] **Step 1: Write the failing tests in `tests/hub.rs`**
  - `calm_hub_team`: `team()` with `Config { view: Views { hub: Look::Calm, ..}, ..}` at 168×49, `HubView::default()`, insta snapshot. The text has no `tok`, `ctx` or `$`.
  - `calm_hub_packet_caption`: at `now = msg.t + 1000` for the last message, the text contains `→` and the message's first 20 chars. At `msg.t + 3500` it doesn't.
  - `calm_hub_all`: merge two `bare(..)` parts with `hub_all: Look::Calm` at 168×49 and take a snapshot. Each session name appears once in a card.
  - `calm_cards_clip_wide_names`: one part named `"日本語プロジェクト".repeat(5)`. Every line's width is ≤ 168, and no card's right border `│` is missing.
  - `calm_hub_empty`: `State::default()` contains `no agents yet`.
  - `key_tests::calm_hub_ignores_graph_keys` (in `src/watch.rs`): with `view.look = Look::Calm`, `g` leaves `screen` at `Hub`, and `⏎` still opens `Zoom`.
  - Make the `show_u` helper take a `&Config` so these tests can pass one in. Update its callers.

- [ ] **Step 2: Run them and check they fail:** `cargo test --test hub calm_`

- [ ] **Step 3: Implement**
  - In `hub::lines`, when `v.screen == Screen::Hub && look(i) == Look::Calm`, call `simple::calm::hub` instead of `hub_screen`.
  - `calm::hub` calls `chrome(.., bare = true)` with keys `↑↓ select · ⏎ zoom · a · x · v constellation · s · q`. The `a` label follows `hub_screen`'s rule. Then:
    - merged: `cards` over `Rect::new(0, 2, w, h - 3)`.
    - otherwise: `graph::draw` over the same rect with `big = true`.
  - Draw the banner over row 2 when `simple::banner` is `Some`.
- [ ] **Step 4: Calm changes in `graph.rs`.** Gate everything on `super::look(i) == Look::Calm`.
  - The node's second line is `phrase` + `dots`, and its third line is empty (no model or ctx).
  - `PACKET_MS` becomes `fn packet_ms(i) -> u64` (1600 full, 3000 Calm).
  - The legend row becomes `from → to  “first 40 chars”` for the newest in-flight edge, else `N helpers · M messages flying`.
  - `Screen::Graph` and full mode don't change.
- [ ] **Step 5: Implement `cards`.**
  - One card per merged session node (`kind == "session"`), newest `last_t` first.
  - Columns: `(area.width / 30).max(1)`. Cards are 6 rows high.
  - Row 1: a dot in `verdict_style(v)` that pulses `●`/`◉` every 500 ms (blinking `!` in `err()` when the session needs you) and the name, clipped with `hub::clip`.
  - Row 2: the needs-you ask (in `err()`) or that node's `phrase`.
  - Row 4: `hub::glyph` per child agent.
  - Header: `N sessions · M agents working`. Cards that don't fit become `+N more`.
- [ ] **Step 6: Run `cargo test` and check it passes,** with full-view snapshots unchanged.
- [ ] **Step 7: Commit:** `feat(simple): Calm hub and hub --all cards`

---

### Task 6: Constellation

**Files:**
- Create: `src/simple/sky.rs`
- Modify: `src/panel.rs` and `src/hub/mod.rs` dispatch, `src/ascii.rs` (new glyphs)
- Test: `src/simple/sky.rs`, `tests/hub.rs` (snapshots `sky_hub_team`, `sky_galaxy`), `src/ascii.rs`

**Interfaces:**
- Consumes: Tasks 1, 3 and 5 (`look`, `chrome(.., bare)`).
- Produces:
  - `pub fn fits(w: u16, h: u16) -> bool` (`w >= 30 && h >= 8`).
  - `pub fn panel(i: &Input, w: u16, h: u16, color: bool) -> Vec<Line<'static>>`. It falls back to `calm::panel` when `!fits`.
  - `pub fn system(buf: &mut Buffer, area: Rect, i: &Input, big: bool)`: one session's sky. `big` adds verbs under the names.
  - `pub fn galaxy(buf: &mut Buffer, area: Rect, i: &Input)`: every merged session as a system.
  - `fn orbit(cx: f64, cy: f64, rx: f64, ry: f64, a: f64) -> (u16, u16)` and `fn star(x: u16, y: u16, now: u64) -> Option<char>` (deterministic). These are private.

Geometry (from the approved mockup; tune by eye):
- One session: centre the pane. `rx = (inner_w / 2 - 6)`, `ry = (inner_h / 2 - 2)`. The angle is `now as f64 / 20_000.0 * TAU + k * TAU / n`.
- Helpers are agents whose parent is `main`, sorted by `last_t` with the newest first, capped at 8.
- Main is `( main )` with its `glyph` one row above.
- Galaxy: lay systems out on a `ceil(sqrt(n))` grid, each with `rx = 6, ry = 2`. Speed is `1/20 s * (1 + 0.25 * k)`.
- Comets: messages with `now - t < 3000` interpolate a `✦` from sender to receiver, with 3 `·` trailing behind. Sender and receiver positions are taken at `now`. In the galaxy, map an agent to its system with `hub::top_of`.
- Stars are drawn first, in `faint()`, on cells still blank. Hash `(x * 37 + y * 17)` to pick about 1 cell in 12. A star shows `*` for 1/9 of the time and `·` for 4/9 of the time, phased by `now / 400`.
- The bottom line is `main is {phrase}` on the left and `{n} helpers out` on the right, plus ` · +N more` over 8.
- Needs you: that body is drawn in `err()` and alternates `◉`/`○` every 500 ms. A galaxy sun blinks `!`. The banner sits on the top inner row.

- [ ] **Step 1: Write the failing tests**
  - `sky_panel_moves`: `panel` at `now = 0` and `now = 5000` differ, and at `now = 0` it's the same across two calls (deterministic).
  - `sky_panel_tiny`: for `[(30, 10), (29, 10), (12, 4), (5, 2)]` every line is ≤ `w` wide. `(29, 10)` contains Calm's `on track`.
  - `sky_crowd`: 20 running helpers. At most 8 labels are drawn, and the text contains `+12 more`.
  - `sky_comet`: with a message at `t`, the text at `t + 1500` contains `✦`, and at `t + 3500` it doesn't.
  - `sky_needs_you`: an agent `waiting` puts `! ` on the first inner row at `now = 0`.
  - `sky_hub_team` (tests/hub.rs): `team()`, `hub: Look::Constellation`, 168×49, insta snapshot at a fixed `now`.
  - `sky_galaxy`: two merged parts with `hub_all: Look::Constellation`, 168×49, snapshot. Both names appear.
  - `sky_wide_names`: a CJK name clipped. Every line ≤ 168.
  - `sky_empty`: `State::default()` contains `no agents yet`.
  - `ascii::maps_sky_glyphs`: `map("✦ ✺ ○ ☼")` is `"* @ o @"`. The existing assertions stay unchanged.

- [ ] **Step 2: Run them and check they fail.**

- [ ] **Step 3: Implement `sky.rs`,** and add `'✦' => '*'`, `'✺' | '☼' => '@'` and `'○' => 'o'` to `ascii::glyph`.

- [ ] **Step 4: Dispatch it.**
  - `panel::lines`: `Look::Constellation` → `simple::sky::panel`.
  - `hub::lines` on `Screen::Hub`: `Look::Constellation` → `chrome(.., bare = true)` with keys `x · v details · s · q`, then `galaxy` if `merged(i)`, else `system(.., big = true)`.
  - In `watch.rs` `key()`'s hub branch, when `view.look == Look::Constellation` (from Task 5), `↑↓ ⏎ g z c b` also do nothing. Test it in `key_tests`: `↓` leaves `sel` at 0 and `⏎` leaves `screen` at `Hub`.
- [ ] **Step 5: Run `cargo test` and check it passes.**
- [ ] **Step 6: Commit:** `feat(simple): Constellation for watch, hub and hub --all`

---

### Task 7: Demo starts in Constellation, `--team`, version 0.4.0

**Files:**
- Modify: `src/demo.rs`, `Cargo.toml` (`version = "0.4.0"`), `Cargo.lock` (via `cargo build`), `README.md` (`ari demo --team` under `## Commands`)

**Interfaces:**
- Consumes: Tasks 1, 2 and 6, and `View.next_look`.
- Produces: `struct Rec { raw: &'static str, main: &'static str, agents: &'static [(&'static str, &'static str)], label: &'static str }` with `const STUCK: Rec` (today's consts) and `const TEAM: Rec` (`fixtures/team`, label `"team"`). `frame_for`, `frame_at`, `events` and `events_for` take `&Rec`.

- [ ] **Step 1: Write the failing tests in `demo.rs`**

```rust
#[test]
fn team_replay_has_messages_and_helpers() {
    let evs = replay::parse(TEAM.raw);
    let (st, ..) = frame_at(&TEAM, &evs, u64::MAX);
    assert!(st.messages.len() >= 15);
    assert!(st.agents.iter().filter(|a| a.id != "main").count() >= 2);
}

#[test]
fn demo_look_starts_in_the_sky_and_cycles_in_memory() {
    let mut cfg = config::Config::default();
    start_look(&mut cfg);
    assert_eq!(cfg.view.watch, Look::Constellation);
    let mut v = View { next_look: true, ..Default::default() };
    take_look(&mut v, &mut cfg);
    assert_eq!(cfg.view.watch, Look::Full);
    assert!(!v.next_look);
}
```
`fn start_look(cfg: &mut Config)` and `fn take_look(view: &mut View, cfg: &mut Config)` use `watch::surface(view, false)`. They never call `config::save`. Keep the existing tests passing with `&STUCK`.

- [ ] **Step 2: Run them and check they fail:** `cargo test --lib demo::`

- [ ] **Step 3: Implement**
  - Parse `--team` and the `Rec` choice. `--team` with `--codex` or `--deepseek` fails with `--team replays a Claude Code session; drop --codex/--deepseek`. Add `--team` to the unknown-option hint.
  - Call `start_look` after parsing, and `take_look` at the top of the TUI closure.
  - The `--once` path uses `View::default()` and resets `cfg.view` to full.
  - The closure's `animating` adds `simple::moving(cfg.view.get(..), &st, now)` so the sky keeps turning after the replay ends.
  - Check `pace` on the team recording by running `cargo run -- demo --team`. It must finish and must not stall.
- [ ] **Step 4: Bump the version to `0.4.0`, run `cargo build`, and document `--team` and `v` in the README.**
- [ ] **Step 5: Run the full verification:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`. Everything must pass.
- [ ] **Step 6: Try it by hand:** `cargo run -- demo`, `cargo run -- demo --team`, then `v` ×3 in the demo, and in `ari watch` and `ari hub`. Check that `~/.config/ari/config.toml` gains `[view]`, and that `ari watch --once` still prints the full panel.
- [ ] **Step 7: Commit:** `feat(demo): start in Constellation, --team replay; 0.4.0`

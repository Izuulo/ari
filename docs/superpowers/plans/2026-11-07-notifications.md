# Notifications Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A desktop notification (and, opt-in, an ntfy push) when any session flips to NEEDS YOU, STUCK, BLOCKED or DONE, exactly once per flip, whether zero, one or five `ari watch` panes are open.

**Architecture:** The flip is detected in `ari hook`, the one process that runs on every event of every session (Claude Code, Codex, dsh) whether or not anything is watching. After appending its event the hook brings the session's cached state up to date (`state::load_session`, incremental), takes the verdict, and compares it with `notify.json` in the session dir under an exclusive `flock`, so concurrent hooks of one session fire once. A new `notify` module owns the flip rule, the notification text and the notifier commands (`osascript` / `notify-send` / `curl` for ntfy), spawned detached with null stdio so the hook never waits on them. The notifier interface (`Note`, `title`, `send`) is reused by the spend-alerts plan.

**Tech Stack:** Rust 2024, serde, toml, libc (`flock`, already a dependency). No new crates.

**Spec:** `docs/ROADMAP.md` §7, item 1.

## Global Constraints

- ARI never sends anything to the model, never blocks or answers for the user. The hook still always exits 0 and never prints (`src/hook.rs` header rule).
- Nothing leaves the machine except the opt-in ntfy URL the user configures; `notify.ntfy` defaults to `""` (off).
- Desktop notifications default on; `notify.desktop = false` turns them off; `notify.on` picks verdicts. Default `on = ["needs_you", "stuck", "blocked", "done"]`; `idle` is accepted but off by default.
- Notifier processes get `Stdio::null()` for stdin, stdout and stderr and their own process group; the hook never waits on them. A missing notifier binary is silently skipped.
- User-controlled text (notification messages, commands, file names, project folder) only ever travels as a separate argv element; it is never interpolated into AppleScript, a shell string or a URL.
- With notifications fully off (`desktop = false`, `ntfy = ""`) the hook does no extra work beyond loading config.
- No new crates. CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Snapshot updates: `INSTA_UPDATE=always cargo test`, then review every `.snap` diff.
- Branch `notifications` off `main`. One commit per task step that says "Commit", style `feat(notify): …` / `docs(changelog): …`, ending with the attribution line. PR with a bullet body.
- Patch version bump in `Cargo.toml`, `.claude-plugin/plugin.json` and `Cargo.lock`, plus a line under `## Unreleased` in `CHANGELOG.md`.

## Review Focus

1. **Hostile text in a notification** (a Bash command or prompt message containing `"`, `\`, `$(…)`, a newline, or starting with `-` or `@`): shown verbatim, nothing executed, no curl option injected. Test `hostile_text_stays_one_argv_element` in Task 2.
2. **Two hooks of one session at once** (parallel tool calls both see STUCK): one notification, not two. Test `concurrent_checks_fire_once` in Task 3.
3. **Turn ends DONE, then Claude Code's `idle_prompt` arrives 60 s later** (NEEDS YOU): one notification per turn end, not two; but when the turn end itself wasn't notified (IDLE, not in `on`), the idle NEEDS YOU does notify. Tests in Task 3.
4. **Linux box without `notify-send`, or ntfy host unreachable**: hook still exits 0 within its usual time, prints nothing; `ari doctor` names the missing binary. Tests `spawn_ignores_missing_binary` (Task 2) and `notify_check_reports` (Task 4); latency measured in Task 5.
5. **Notifications off**: no `notify.json` is created and `state.json` isn't touched by the notify path. Test `off_does_nothing` in Task 3.

---

### Task 1: Config keys and verdict ids

**Files:**
- Modify: `src/config.rs` (new `Notify` struct, `Config.notify`, `NOTIFY_ON`, `problems`)
- Modify: `src/state.rs` (`impl Verdict`, next to `label` at line ~343)

**Interfaces:**
- Produces:
  - `pub const NOTIFY_ON: [&str; 5] = ["needs_you", "stuck", "blocked", "done", "idle"];` (the ids `notify.on` accepts, in settings order)
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(default)] pub struct Notify { pub desktop: bool, pub on: Vec<String>, pub ntfy: String }`, default `desktop: true`, `on: ["needs_you", "stuck", "blocked", "done"]`, `ntfy: ""`.
  - `Config.notify: Notify` (serialised as `[notify]`).
  - `pub fn Verdict::id(self) -> &'static str`: `needs_you`, `blocked`, `stuck`, `retrying`, `on_track`, `done`, `idle`.
  - `problems` gains: `notify.on: unknown verdict "<id>"` for an id not in `NOTIFY_ON`; `notify.ntfy: not an http(s) URL "<value>"` when non-empty and not starting with `https://` or `http://`, or containing whitespace.

- [x] **Step 1: Write the failing tests** in `config.rs` tests:
  - `notify_defaults_and_parse`: `parse("")` → `notify.desktop == true`, `notify.on == ["needs_you", "stuck", "blocked", "done"]`, `notify.ntfy == ""`; `parse("[notify]\ndesktop = false\non = [\"done\"]\nntfy = \"https://ntfy.sh/x\"\n")` → those values.
  - `round_trip` extended: set `notify.ntfy = "https://ntfy.sh/t"` and `notify.on = ["idle"]`; still round-trips.
  - `problems_name_bad_notify_values`: `on = ["nope", "done"]`, `ntfy = "-o /tmp/x"` → exactly 2 problems, containing `nope` and `-o /tmp/x`; `ntfy = "https://ntfy.sh/x"` → no problem.
  - In `state.rs` tests, `verdict_ids`: every `Verdict` maps to the id above, and each `NOTIFY_ON` id is the `id()` of some verdict.
- [x] **Step 2: Run** `cargo test config:: state::tests::verdict_ids` — expect FAIL (no `notify`, no `id`).
- [x] **Step 3: Implement** the struct, default, constant, `problems` lines and `Verdict::id`.
- [x] **Step 4: Run** `cargo test` — PASS.
- [x] **Step 5: Commit** `feat(notify): notify config keys and verdict ids`.

---

### Task 2: The notifier

**Files:**
- Create: `src/notify.rs` (add `pub mod notify;` in `src/lib.rs`)

**Interfaces:**
- Consumes: `config::Notify` (Task 1), `sessions::cwd_of(dir: &Path) -> Option<String>`, `event::truncate(s: &str, max_chars: usize) -> String`.
- Produces (the spend-alerts plan uses `Note`, `title` and `send`):
  - `#[derive(Clone, Debug, PartialEq)] pub struct Note { pub title: String, pub body: String }`
  - `pub const NOTE_MAX_CHARS: usize = 140;`
  - `pub fn title(dir: &Path) -> String`: `ARI: <last path component of cwd_of(dir)>`, else `ARI`.
  - `pub fn commands(note: &Note, n: &Notify, os: &str) -> Vec<Vec<String>>`: argv lists, pure. Body and title are passed through `clean` first.
    - `n.desktop && os == "macos"` → `["osascript", "-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run", title, body]`
    - `n.desktop && os == "linux"` → `["notify-send", "--app-name=ARI", "--", title, body]`
    - ntfy valid (same rule as `problems`) → `["curl", "-fsS", "-m", "10", "-H", "Title: <title>", "--data-raw", body, "--", url]`
    - other OS or invalid ntfy → that entry is left out.
  - `fn clean(s: &str) -> String`: control characters (incl. `\n`, `\r`) become a space, then `truncate(.., NOTE_MAX_CHARS)`.
  - `pub fn spawn_all(cmds: &[Vec<String>])`: each via `std::process::Command` with null stdin/stdout/stderr and `std::os::unix::process::CommandExt::process_group(0)`; `spawn()` errors ignored; never waits.
  - `pub fn send(note: &Note, n: &Notify)`: `spawn_all(&commands(note, n, std::env::consts::OS))`.

- [x] **Step 1: Write the failing tests** in `notify.rs`:
  - `macos_and_linux_commands`: for `Note { title: "ARI: proj", body: "◉ NEEDS YOU · Bash" }` with defaults: `commands(.., "macos")` is exactly the osascript argv above with the title and body as its last two elements; `"linux"` gives the notify-send argv; `"windows"` gives `[]`; `desktop = false` gives `[]`.
  - `ntfy_only_when_set_and_valid`: `ntfy = "https://ntfy.sh/t"` adds a curl argv ending `["--", "https://ntfy.sh/t"]` with `"Title: ARI: proj"`; `ntfy = "ftp://x"` or `"-K /etc/passwd"` adds nothing.
  - `hostile_text_stays_one_argv_element`: body `"\"; do shell script \"rm -rf ~\" --\n@/etc/passwd $(id)"` → appears as exactly one argv element in each command (newline turned into a space), and the three `-e` script lines are the constant ones.
  - `long_text_is_cut`: a 500-char body → `chars().count() <= NOTE_MAX_CHARS`.
  - `title_from_cwd`: a temp session dir with `meta.json` `{"cwd":"/x/tankobon"}` → `ARI: tankobon`; no meta → `ARI`.
  - `spawn_ignores_missing_binary`: `spawn_all(&[vec!["/nonexistent/ari-notify".into()]])` returns without panic.
- [x] **Step 2: Run** `cargo test notify::` — FAIL.
- [x] **Step 3: Implement** `src/notify.rs` as above.
- [x] **Step 4: Run** `cargo test notify::` — PASS.
- [x] **Step 5: Commit** `feat(notify): desktop and ntfy notifier, spawned detached`.

---

### Task 3: Detect the flip in the hook

**Files:**
- Modify: `src/notify.rs`
- Modify: `src/hook.rs` (`handle`: load config on every event, call the check after `append`)

**Interfaces:**
- Consumes: Task 1 and 2; `state::load_session(dir: &Path) -> State`, `State::verdict_with(u8)`, `State::stuck_reason(u8)`, `State.needs_you`, `State::waiting()`, `State.turn_active`, `State.turns`.
- Produces:
  - `#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)] #[serde(default)] pub struct Seen { pub v: String, pub fired: String, pub fired_t: u64, pub fired_turn: u32 }` — stored as `notify.json` in the session dir.
  - `pub const REPEAT_MS: u64 = 60_000;` the same verdict doesn't fire twice for one session within this.
  - `pub fn decide(prev: &Seen, v: Verdict, turn_active: bool, turns: u32, now: u64, on: &[String]) -> (Seen, bool)`: returns the new `Seen` (with `v = v.id()`) and whether to fire. Fire iff all of:
    1. `on` contains `v.id()`;
    2. `prev.v != v.id()` (a flip, not a repeat);
    3. not (`prev.fired == v.id()` and `now - prev.fired_t < REPEAT_MS`);
    4. not (`v == NeedsYou`, `!turn_active`, `prev.fired` is `done`/`idle`/`blocked` and `prev.fired_turn == turns`) — the turn end was already reported.
    On fire, `fired`, `fired_t`, `fired_turn` are updated; otherwise carried over.
  - `pub fn note(dir: &Path, st: &State, v: Verdict, repeats: u8) -> Note`: title `title(dir)`; body `"{glyph} {LABEL}"`, plus `" · {detail}"` where detail = `st.needs_you`, else the waiting agent's `label()` (NEEDS YOU), or `st.stuck_reason(repeats)` (STUCK); none for BLOCKED/DONE/IDLE.
  - `pub fn check(dir: &Path, cfg: &Config, now: u64) -> Option<Note>`: returns `None` at once when `!cfg.notify.desktop && cfg.notify.ntfy.is_empty()`. Otherwise opens (creating) `dir/notify.json`, `libc::flock(fd, LOCK_EX)`, reads `Seen` (default on empty/broken), `load_session`, `verdict_with(cfg.thresholds.stuck_repeats)`, `decide`, writes the new `Seen` in place (`set_len(0)`, seek 0, `write_all` — not `write_atomic`, which would swap the inode under the lock), unlocks by drop. Returns `Some(note)` on fire. It does not send.
  - `hook::handle`: after the `events.jsonl` append: `let cfg = config::load();` (move the existing `SessionStart` load to reuse it), then `if let Some(n) = notify::check(&dir, &cfg, t) { notify::send(&n, &cfg.notify) }`.

- [x] **Step 1: Write the failing `decide` tests** in `notify.rs`:
  - `fires_once_per_flip`: from `Seen::default()`, `OnTrack` → no fire; `Stuck` → fire; `Stuck` again → no fire; `OnTrack` then `Stuck` 2 min later → fire.
  - `not_in_on_list_never_fires`: `on = ["done"]`, `NeedsYou` → no fire, but `Seen.v == "needs_you"` (so a later flip still counts).
  - `same_verdict_within_repeat_ms_is_quiet`: `Stuck` fire at t, `Retrying` at t+10 s, `Stuck` at t+20 s → no fire; at t+REPEAT_MS+1 → fire.
  - `idle_prompt_after_done_does_not_double_notify`: `Done` (turn_active false, turns 3) fires; `NeedsYou` (turn_active false, turns 3) 60 s later → no fire; next turn `OnTrack` (turns 4) then `NeedsYou` with turn_active true → fire.
  - `idle_prompt_after_unnotified_idle_fires`: default `on`, `Idle` (turns 3) → no fire; `NeedsYou` (turn_active false, turns 3) → fire.
- [x] **Step 2: Write the failing `check` tests** (temp session dir; write `events.jsonl` lines with `event::to_line` as other tests do):
  - `off_does_nothing`: `desktop = false`, `ntfy = ""`, events that make the session STUCK → `None`; neither `notify.json` nor `state.json` exists afterwards.
  - `check_reports_stuck_once`: events making the verdict STUCK (three identical failing Bash calls, as in `state.rs`'s stuck tests) → first `check` is `Some(note)` with body starting `↻ STUCK · `; second `check` → `None`.
  - `concurrent_checks_fire_once`: same STUCK dir, 8 threads call `check` at once → exactly one `Some`.
- [x] **Step 3: Run** `cargo test notify::` — FAIL.
- [x] **Step 4: Implement** `Seen`, `decide`, `note`, `check`, and the two lines in `hook::handle`.
- [x] **Step 5: Run** `cargo test` — PASS.
- [x] **Step 6: Commit** `feat(notify): notify once when a session flips, from the hook`.

---

### Task 4: Settings rows and `ari doctor`

**Files:**
- Modify: `src/settings.rs` (`ITEMS`, `KEYS`, `value`, `step`, `reset`, tests; `settings_screen_snapshot`)
- Modify: `src/doctor.rs` (new `notify_status`, added in `checks()` right after `config_status`)

**Interfaces:**
- Consumes: `config::{Notify, NOTIFY_ON}` (Task 1), `setup::which(name: &str) -> Option<PathBuf>`.
- Produces:
  - Six `ALERTS` rows after `bell`: `("ALERTS", "notify.desktop", "desktop notifications")`, then `("ALERTS", "notify_on.<id>", "notify · <words>")` for each `NOTIFY_ON` id, words: `needs you`, `stuck`, `blocked`, `done`, `idle (turn ended)`. `ITEMS` becomes 44 long.
  - `notify_on.<id>` is on when `cfg.notify.on` contains the id; flipping rebuilds the list in `NOTIFY_ON` order (so a flip and a flip back equal the default); `reset` restores that id's default membership.
  - `KEYS` becomes `↑↓ move · ←→ change · d default · esc back · chain, prices, ntfy: edit the file`.
  - `fn notify_status(n: &config::Notify, os: &str, found: impl Fn(&str) -> bool) -> Check`, name `"notify"`:
    - both off → `info`, `off`;
    - desktop on, `os == "linux"`, `!found("notify-send")` → problem `notify-send not found`, fix `install libnotify (apt install libnotify-bin), or set notify.desktop = false`;
    - ntfy set, `!found("curl")` → problem `curl not found (needed for ntfy)`, fix `install curl, or clear notify.ntfy`;
    - else pass with detail the channels joined by ` + `: `desktop`, `ntfy <host>` (host only: the topic in the path works like a password).
  - `checks()` pushes `notify_status(&loaded_cfg.notify, std::env::consts::OS, |b| setup::which(b).is_some())`.

- [ ] **Step 1: Write the failing tests**:
  - `settings.rs` `items_cover_every_role_and_feature`: length 44; every `NOTIFY_ON` id has a `notify_on.<id>` row in `ALERTS`. Groups order unchanged.
  - `settings.rs` `notify_rows`: default → `notify.desktop` `on`, `notify_on.needs_you` `on`, `notify_on.idle` `off`; step `notify_on.needs_you` twice → `cfg.notify.on` equals the default list in the same order; `reset` after a change → `Config::default()`.
  - `every_item_has_a_value_and_steps_back` and `reset_restores_default` pass unchanged for the new rows.
  - `doctor.rs` `notify_check_reports`: the five cases above, incl. `ntfy = "https://ntfy.sh/secret-topic"` → detail `desktop + ntfy ntfy.sh` and does not contain `secret-topic`.
- [ ] **Step 2: Run** `cargo test settings:: doctor::` — FAIL.
- [ ] **Step 3: Implement** the rows, value/step/reset cases, `KEYS`, `notify_status` and its call.
- [ ] **Step 4: Run** `INSTA_UPDATE=always cargo test settings::`, review the `settings_screen_snapshot` diff (new rows and the key hint only), then `cargo test` — PASS.
- [ ] **Step 5: Commit** `feat(notify): notification settings and doctor check`.

---

### Task 5: Measure, document, bump

**Files:**
- Modify: `scripts/bench-hook.sh` (set `XDG_CONFIG_HOME` to the temp dir too, so the bench runs with default config: notifications on)
- Modify: `README.md`, `CHANGELOG.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

- [ ] **Step 1: Latency.** On `main`, run `scripts/bench-hook.sh 500` and note p50/p95; on this branch, run it again. Expected: exit 0 and p95 no more than 1 ms above `main`'s. If it is, skip the notify path for `PreToolUse` events (they can't end a turn or add a failure) and re-measure. Put both numbers in the PR body.
- [ ] **Step 2: Real run.** `cargo install --path . --root ~/.local --force` (per local-install memory: `rm ~/.local/bin/ari` first if the copy gets killed), start a Claude Code session, trigger a permission prompt: exactly one desktop notification `ARI: <repo>` / `◉ NEEDS YOU · …`, also with two `ari watch` panes open and with none. Let a turn finish with all tasks done: one `✓ DONE`, no second one 60 s later. Set `ntfy` to a throwaway `https://ntfy.sh/<random>` topic and confirm one push arrives; clear it again.
- [ ] **Step 3: Docs.** README Config block: a `[notify]` table (`desktop = true`, `on = ["needs_you", "stuck", "blocked", "done"]  # also: idle`, `ntfy = ""  # optional, e.g. "https://ntfy.sh/your-topic"; the only thing ARI ever sends off your machine`). README "What you get": one paragraph on notifications (once per flip, from the hooks, works with no `ari watch` open; macOS may ask once to allow notifications from Script Editor). README FAQ "Where does my data go?": add "unless you set `notify.ntfy`: then the notification text goes to that URL". Update the FAQ hook timing if Step 1 changed it. Why-ARI table: a `Notifications` row.
- [ ] **Step 4: Changelog and version.** `CHANGELOG.md` under `## Unreleased`: one **Notifications** bullet (what fires, `[notify]` keys, how to turn off, ntfy opt-in). Bump the patch version in `Cargo.toml` and `.claude-plugin/plugin.json`, then `cargo check` to refresh `Cargo.lock`.
- [ ] **Step 5: Run** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — PASS.
- [ ] **Step 6: Commit** `docs: notifications` and `chore: bump version` (two commits), push, open the PR with a bullet body.

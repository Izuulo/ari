# Audit Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the seven findings of the 2026-11-06 repo audit: four wrong results (error signatures, setup's hide list, `retention_days = 0`, uninstall leftovers) and three lean cleanups (one JSONL reader, one `usage.json` loader, no unused `now` in the verdict).

**Architecture:** No new modules. Each fix lives where the bug is: `event::signature`, `setup.rs`, `hook.rs`. The cleanups move one copy of shared code into the module that already owns that job (`paths.rs` for file I/O, `usage.rs` for the statusline payload) and delete the others.

**Tech Stack:** Rust 2024, serde_json, toml, insta. No new dependencies.

**Spec:** The audit report from this session. Each finding is quoted under its task as **Finding**. Line numbers are as of `main` at `9a13336`.

## Global Constraints

- No new crates. No new config keys.
- Hook rules stay: `ari hook` always exits 0, prints nothing, appends one line.
- Old event logs are not rewritten. Errors recorded with the old two-word signature stay as they are. Only new events get the new signature. No cache version bump is needed, because `sig` is stored in `events.jsonl`, not derived by the reducer.
- Existing configs that already hold `hide = ["cost", "tokens", "time", "cache"]` are not migrated on load. Task 4's uninstall cleanup handles them, and `ari doctor` already names the two bad entries.
- CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Only update snapshots on purpose (`INSTA_UPDATE=always cargo test`), then review every `.snap` diff. None are expected to change.
- Branch `audit-fixes`. One commit per task, `fix:` for Tasks 1–4 and `refactor:` for Tasks 5–7, each ending with the attribution line. Then open a PR with a bullet-list body. Do not merge without the user's go-ahead.
- `CHANGELOG.md`: add a `## Unreleased` section above `## 0.2.0` in Task 1. Tasks 2–4 each add one bullet to it. Tasks 5–7 add nothing (no user-visible change).

## Review Focus

1. **Runner word with flags before the script** (`uv run --frozen pytest`, `npm run build -- --watch`): the signature names the script (`uv run pytest`, `npm run build`), not a flag. Test in Task 1.
2. **Runner word with nothing after it** (`npm run` on its own, which lists scripts): signature stays `Bash:npm run` and nothing panics. Test in Task 1.
3. **A user who changed `hide` after setup** (e.g. `["cost"]`): uninstall leaves their choice alone and only removes what setup added. Test in Task 4.
4. **A broken `config.toml` at uninstall time**: settings are still restored, and the config file stays byte for byte the same (matches the 0.2.0 promise "never overwritten"). Test in Task 4.
5. **An events file truncated to shorter than the cached offset** (log deleted and restarted): the reader starts over from 0 and does not keep stale state. Test in Task 5.

---

### Task 1: Error signature keeps the script after a runner word

**Finding:** `event::signature` keys shell failures on the first two words, so `npm run build` and `npm run lint` share `Bash:npm run`. A failed build is marked fixed when lint passes, and three different failing scripts look like STUCK.

**Files:**
- Modify: `src/event.rs:402-421` (`signature` and its doc comment)
- Test: `src/event.rs` (tests module), `src/state.rs` (tests module)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Produces: `signature(tool, input)` unchanged in shape. For Bash/PowerShell it returns `"{tool}:{w1} {w2}"`, or `"{tool}:{w1} {w2} {script}"` when `w2` is one of `run`, `exec`, `-m`, `x`. `script` is the first later word that does not start with `-`.

- [ ] **Step 1: Write the failing tests**

In `src/event.rs` tests, next to `bash_pre_and_failure`:

```rust
#[test]
fn signature_keeps_the_script_after_a_runner() {
    let sig = |cmd: &str| signature("Bash", &json!({ "command": cmd }));
    assert_eq!(sig("npm run build"), "Bash:npm run build");
    assert_ne!(sig("npm run build"), sig("npm run lint"));
    assert_eq!(sig("npm run build -- --watch"), "Bash:npm run build");
    assert_eq!(sig("python -m pytest -x"), "Bash:python -m pytest");
    assert_eq!(sig("uv run --frozen pytest"), "Bash:uv run pytest");
    assert_eq!(sig("pnpm exec tsc"), "Bash:pnpm exec tsc");
    assert_eq!(sig("cd /p && npm run build 2>&1 | tail"), "Bash:npm run build");
    assert_eq!(sig("npm run"), "Bash:npm run");
    assert_eq!(sig("cargo test foo"), "Bash:cargo test", "non-runners keep two words");
}
```

In `src/state.rs` tests, an end-to-end case through `normalize`:

```rust
#[test]
fn a_passing_script_does_not_fix_a_different_failing_one() {
    use serde_json::json;
    let ev = |t, v: serde_json::Value| crate::event::normalize(&v, t).unwrap();
    let bash = |cmd: &str| json!({ "command": cmd });
    let mut st = State::default();
    st.apply(&ev(1, json!({"hook_event_name": "UserPromptSubmit", "prompt": "build"})));
    st.apply(&ev(2, json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash",
        "tool_input": bash("npm run build"), "error": "Exit code 1\nerror TS2304"})));
    st.apply(&ev(3, json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
        "tool_input": bash("npm run lint")})));
    assert_eq!(st.errors[0].resolved_t, None, "lint passing doesn't fix the build");
    // Three different scripts failing once each is not "the same command failing again".
    for (t, cmd) in [(5, "npm run test"), (6, "npm run lint")] {
        st.apply(&ev(t, json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash",
            "tool_input": bash(cmd), "error": "Exit code 1\nfailed"})));
    }
    assert_eq!(st.stuck_reason(3), None);
    st.apply(&ev(7, json!({"hook_event_name": "Stop"})));
    assert_eq!(st.open_errors(), 3);
}
```

(The verdict after this Stop is BLOCKED because the turn ended on an open failure. The test doesn't assert that, because that rule already works; only the signature was wrong.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test signature_keeps_the_script a_passing_script`
Expected: both FAIL. The first fails on `"Bash:npm run" != "Bash:npm run build"`. The second fails on `resolved_t` being `Some(3)`.

- [ ] **Step 3: Implement in `signature`**

Keep the `cd`-skipping split as it is. After taking `w1 w2`: if `w2` is in `["run", "exec", "-m", "x"]`, append the first following token that doesn't start with `-`, if there is one. Update the doc comment to: `Bash → first two words of the first command that isn't a cd, plus the script after a runner word (npm run build, python -m pytest)`.

- [ ] **Step 4: Run the whole suite**

Run: `cargo test`
Expected: all pass, and no snapshot changes. The fixtures use `python3 …` commands, which are not runners.

- [ ] **Step 5: Changelog and commit**

Add `## Unreleased` above `## 0.2.0` with this bullet:
`- Errors from different scripts of one runner (\`npm run build\` vs \`npm run lint\`, \`python -m pytest\` vs \`python -m mypy\`) are tracked apart: one passing no longer marks the other fixed, and three different failures no longer read as STUCK.`

```bash
git add src/event.rs src/state.rs CHANGELOG.md
git commit -m "fix: tell runner scripts apart in error signatures"
```

---

### Task 2: `ari setup` hides only segments that exist

**Finding:** `CHAINED_HIDE` includes `time` and `cache`, which are not in `config::SEGMENTS`, so `ari doctor` reports a config problem right after setup.

**Files:**
- Modify: `src/setup.rs:11`
- Test: `src/setup.rs` (`plan_chains_and_keeps_keys`), `tests/setup.rs:79`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Produces: `const CHAINED_HIDE: [&str; 2] = ["cost", "tokens"];` (Task 4 reads it).

- [ ] **Step 1: Write the failing test**

In `plan_chains_and_keeps_keys`, after `assert_eq!(p.config.hide, CHAINED_HIDE);`, add:

```rust
assert!(config::problems(&p.config).is_empty(), "{:?}", config::problems(&p.config));
```

- [ ] **Step 2: Run it and see it fail**

Run: `cargo test plan_chains_and_keeps_keys`
Expected: FAIL listing `hide: unknown segment "time"` and `"cache"`.

- [ ] **Step 3: Implement**

Set `CHAINED_HIDE` to `["cost", "tokens"]`. Update `tests/setup.rs:79` to `assert_eq!(config.hide, ["cost", "tokens"]);`.

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 5: Changelog and commit**

Bullet: `- \`ari setup\` no longer writes \`time\` and \`cache\` into \`hide\`; they aren't segments, and \`ari doctor\` flagged them.`

```bash
git add src/setup.rs tests/setup.rs CHANGELOG.md
git commit -m "fix: setup hides only real statusline segments"
```

---

### Task 3: `retention_days = 0` can't delete live sessions

**Finding:** The SessionStart prune uses `retention_days` as the max age. A hand-edited `0` deletes every other session folder, live ones included.

**Files:**
- Modify: `src/hook.rs:49`
- Test: `tests/hook.rs` (`hook` helper and a new test)
- Modify: `README.md:151`, `CHANGELOG.md`

**Interfaces:** none.

- [ ] **Step 1: Make the hook tests hermetic**

In `tests/hook.rs`, make `hook()` also set `.env("XDG_CONFIG_HOME", state.join("config"))`. Today the hook tests read the developer's real `~/.config/ari/config.toml`.

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn zero_retention_keeps_live_sessions() {
    let s = tmp("zero");
    std::fs::create_dir_all(s.join("config/ari")).unwrap();
    std::fs::write(s.join("config/ari/config.toml"), "retention_days = 0\n").unwrap();
    let live = s.join("ari/sessions/live");
    std::fs::create_dir_all(&live).unwrap();
    std::fs::write(live.join("events.jsonl"), "").unwrap();
    std::thread::sleep(Duration::from_millis(20)); // age > 0
    let start = serde_json::json!({"session_id": "s1", "hook_event_name": "SessionStart"});
    assert_silent(&hook(&s, start.to_string().as_bytes()));
    assert!(live.exists(), "a session active seconds ago must survive");
}
```

- [ ] **Step 3: Run it and see it fail**

Run: `cargo test --test hook zero_retention`
Expected: FAIL, because `live` was deleted.

- [ ] **Step 4: Implement**

`let days = config::load().retention_days.max(1);`. Change the README line 151 comment to `# session logs older than this are deleted (at least 1)`.

- [ ] **Step 5: Run tests, changelog, commit**

Run: `cargo test`. Expected: all pass.
Bullet: `- \`retention_days = 0\` counts as 1 day instead of deleting every other session's logs, live ones included.`

```bash
git add src/hook.rs tests/hook.rs README.md CHANGELOG.md
git commit -m "fix: retention_days of 0 no longer prunes live sessions"
```

---

### Task 4: `ari uninstall` removes what setup added to config.toml

**Finding:** Uninstall restores `settings.json` but leaves the chained command and the setup-written `hide` list in `config.toml`. After a later reinstall, ARI keeps running a deleted script and hiding cost and tokens.

**Files:**
- Modify: `src/setup.rs` (new `unchain`, `run_uninstall` at `:182-240`)
- Test: `src/setup.rs` tests, `tests/setup.rs`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `CHAINED_HIDE` from Task 2.
- Produces: `pub fn unchain(cfg: &Config, original: &Value) -> Option<Config>`. It returns the config with the original statusline's `command` removed from `chain.commands`, and with `hide` cleared when it equals `CHAINED_HIDE` or the pre-fix `["cost", "tokens", "time", "cache"]`. It returns `None` when that changes nothing.

- [ ] **Step 1: Write the failing unit test**

```rust
#[test]
fn unchain_drops_only_what_setup_added() {
    let s: Value = serde_json::from_str(
        r#"{"statusLine":{"type":"command","command":"bash p.sh"}}"#).unwrap();
    let p = plan(&s, &Config::default()).unwrap();
    let c = unchain(&p.config, &p.original).unwrap();
    assert!(c.chain.commands.is_empty() && c.hide.is_empty());
    let mut mine = p.config.clone();
    mine.hide = vec!["cost".into()];
    assert_eq!(unchain(&mine, &p.original).unwrap().hide, ["cost"]);
    let mut legacy = p.config.clone();
    legacy.hide = ["cost", "tokens", "time", "cache"].map(String::from).to_vec();
    assert!(unchain(&legacy, &p.original).unwrap().hide.is_empty());
    assert_eq!(unchain(&Config::default(), &Value::Null), None);
}
```

- [ ] **Step 2: Extend the integration tests**

In `setup_then_uninstall_restores_exactly`, after the final uninstall:

```rust
let config = ari_cli::config::parse(
    &std::fs::read_to_string(d.join("config/ari/config.toml")).unwrap());
assert!(config.chain.commands.is_empty() && config.hide.is_empty());
```

Add a new test `uninstall_leaves_a_broken_config_alone`: write `ORIGINAL` to settings, run `setup --yes`, overwrite `config/ari/config.toml` with `face = [[[`, then run `uninstall --yes`. Assert that settings equal `ORIGINAL` and the config file still reads `face = [[[`.

- [ ] **Step 3: Run them and see them fail**

Run: `cargo test unchain; cargo test --test setup`
Expected: `unchain` does not compile yet, and the roundtrip assert fails on a non-empty `chain.commands`.

- [ ] **Step 4: Implement**

Write `unchain` as specified. In `run_uninstall`, when `changes && !has("--purge")`: if `config::load_checked()` is `Ok(cfg)` and `unchain(&cfg, &original)` is `Some(new)`, print `config.toml  chain.commands / hide\n- <old values>\n+ <new values>` with the settings diff, before `confirm`. After the settings are restored, `config::save(&new)`. A failed save prints a warning and still exits 0, because settings are already restored. An `Err` from `load_checked` means the file is not touched.

- [ ] **Step 5: Run tests, changelog, commit**

Run: `cargo test`. Expected: all pass.
Bullet: `- \`ari uninstall\` also removes the chained statusline and the \`hide\` list that \`ari setup\` added to \`config.toml\` (your own changes stay; a broken file is left alone).`

```bash
git add src/setup.rs tests/setup.rs CHANGELOG.md
git commit -m "fix: uninstall removes setup's chain and hide from config.toml"
```

---

### Task 5: One incremental JSONL reader

**Finding:** `state::load_session` (`src/state.rs:1073-1099`) and `tokens::read_ledger` (`src/tokens.rs:211-234`) copy the same "read from the offset, keep complete lines, restart if shorter" logic.

**Files:**
- Modify: `src/paths.rs` (new function and tests), `src/state.rs:1062-1104`, `src/tokens.rs:211-235`

**Interfaces:**
- Produces: `pub fn new_lines(path: &Path, offset: u64) -> Option<(bool, Vec<u8>)>`. `bool` is `true` when the file is shorter than `offset`, which means the caller resets and the bytes are read from 0. The bytes are only complete lines, ending in `\n`. It returns `None` when the file is missing or unreadable, or when there is nothing new and no restart.

- [ ] **Step 1: Write the failing test** (in `src/paths.rs` tests)

```rust
#[test]
fn new_lines_reads_complete_lines_and_restarts() {
    let p = std::env::temp_dir().join(format!("ari-newlines-{}", std::process::id()));
    assert_eq!(new_lines(&p, 0), None, "missing file");
    std::fs::write(&p, "a\nb").unwrap();
    assert_eq!(new_lines(&p, 0), Some((false, b"a\n".to_vec())));
    assert_eq!(new_lines(&p, 2), None, "partial line waits");
    std::fs::write(&p, "a\nb\n").unwrap();
    assert_eq!(new_lines(&p, 2), Some((false, b"b\n".to_vec())));
    assert_eq!(new_lines(&p, 4), None, "unchanged");
    std::fs::write(&p, "x\n").unwrap();
    assert_eq!(new_lines(&p, 4), Some((true, b"x\n".to_vec())), "shorter: start over");
    std::fs::write(&p, "").unwrap();
    assert_eq!(new_lines(&p, 2), Some((true, vec![])), "emptied: still a restart");
    let _ = std::fs::remove_file(&p);
}
```

- [ ] **Step 2: Run it and see it fail**

Run: `cargo test new_lines_reads`
Expected: does not compile (`new_lines` not found).

- [ ] **Step 3: Implement `new_lines`, then switch both callers**

In `load_session`: on `Some((restart, bytes))`, reset `cache` to the default when `restart`, then set the offset to 0. Apply each line, add `bytes.len()` to the offset, and write the cache only when `bytes` is non-empty. On `None`, return `cache.state`. Do the same in `read_ledger`, resetting with `*l = Ledger::default()`. Delete the now-unused `Read`/`Seek` imports.

- [ ] **Step 4: Run tests**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, including the existing partial-line test in `tokens.rs` and all fixture snapshots.

- [ ] **Step 5: Commit**

```bash
git add src/paths.rs src/state.rs src/tokens.rs
git commit -m "refactor: one incremental JSONL reader for state and tokens"
```

---

### Task 6: One `usage.json` loader

**Finding:** `watch::load_usage` (`src/watch.rs:64-70`), `recap::build` (`src/recap.rs:75-79`) and `doctor` (`src/doctor.rs:295-296`) each parse `usage.json` themselves.

**Files:**
- Modify: `src/usage.rs` (new function and test), `src/watch.rs`, `src/recap.rs`, `src/doctor.rs`

**Interfaces:**
- Produces: `pub fn load(dir: &Path) -> Usage` in `usage.rs`. It reads `dir/usage.json` through `from_value`, and returns `Usage::default()` when the file is missing or broken.

- [ ] **Step 1: Write the failing test** (in `src/usage.rs` tests)

```rust
#[test]
fn load_reads_usage_json_or_defaults() {
    let d = std::env::temp_dir().join(format!("ari-usage-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(load(&d), Usage::default());
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("usage.json"), r#"{"session_id":"s1"}"#).unwrap();
    assert_eq!(load(&d).session_id.as_deref(), Some("s1"));
    std::fs::write(d.join("usage.json"), "not json").unwrap();
    assert_eq!(load(&d), Usage::default());
    let _ = std::fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run it and see it fail**

Run: `cargo test load_reads_usage_json`
Expected: does not compile.

- [ ] **Step 3: Implement and replace the copies**

Move the body of `watch::load_usage` into `usage::load` and delete `load_usage`. Its callers in `watch.rs` call `usage::load`. In `recap.rs:75-79`, use `crate::usage::load(dir)`. In `doctor.rs:295-296`, use `let u = crate::usage::load(&s.dir);`. That drops the `?` on a missing file: `transcript_for` already falls back to `meta.json`, so doctor now also checks sessions that have a hook transcript but no statusline payload yet.

- [ ] **Step 4: Run tests**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/usage.rs src/watch.rs src/recap.rs src/doctor.rs
git commit -m "refactor: one usage.json loader"
```

---

### Task 7: The verdict takes no unused time

**Finding:** `State::verdict(now)` and `verdict_with(_now, repeats)` ignore `now`, which suggests the verdict depends on time when it doesn't.

**Files:**
- Modify: `src/state.rs:873-875, 919`, plus every caller. As of the audit that is `src/bar.rs`, `src/demo.rs`, `src/hub/agents.rs`, `src/hub/mod.rs`, `src/panel.rs`, `src/recap.rs`, `src/watch.rs`, `tests/fixtures.rs`, and the tests in `src/state.rs`.

**Interfaces:**
- Produces: `pub fn verdict(&self) -> Verdict` and `pub fn verdict_with(&self, repeats: u8) -> Verdict`.

- [ ] **Step 1: Change both signatures**

Drop the parameter, then let the compiler list every call site: `cargo build --all-targets 2>&1 | grep -c "error\["`.

- [ ] **Step 2: Fix every caller**

Remove the argument. Where a local `now` existed only to feed the verdict, delete it. Clippy's `unused_variables` will name those.

- [ ] **Step 3: Run the full CI set**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all clean. No snapshot changes, because behavior is identical.

- [ ] **Step 4: Commit**

```bash
git add -A src tests
git commit -m "refactor: drop the verdict's unused now parameter"
```

---

### Finish

- [ ] `git push -u origin audit-fixes`, then `gh pr create` with a bullet body (one bullet per task) ending with the attribution line. Leave it unmerged until the user says so.
- [ ] Once it's merged: `cargo install --path . --root ~/.local --force`, then run `ari doctor` and confirm the config check passes.

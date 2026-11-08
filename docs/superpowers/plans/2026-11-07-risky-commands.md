# Risky Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Flag risky Bash commands that ran (recursive deletes, deletes outside the repo, `git reset --hard`, force pushes, forced cleans, discarded changes, deleted branches, `DROP TABLE`, raw disk writes) with one row in the `ari watch` panel and a `risky` section in `ari recap`. Watch-only: ARI never blocks a command.

**Architecture:** A new pure module `src/risky.rs` splits a shell command into simple commands (quote-aware), strips prefixes (`sudo`, `env`, `VAR=1`, `bash -c`), follows `cd` inside the chain, and matches a pinned rule list. The hook runs it on `PostToolUse` / `PostToolUseFailure` of `Bash` (only commands that ran; a denied command fires no Post event) and stores `(rule, command)` on the event, because the event otherwise keeps only the first 80 chars of the first line (`act`) and two words (`sig`). The reducer keeps the last 20 in `State.risky`; the panel shows the latest under the header like the conflicts row; `ari recap` lists every one in its window.

**Tech Stack:** Rust 2024, serde, ratatui, insta. No new dependencies.

**Spec:** `docs/ROADMAP.md` §7 item 6 (row 6 of the table: "Flag risky commands that ran (`rm -rf`, `git reset --hard`, force push, deletes outside the repo) in the panel and in `ari recap`").

## Global Constraints

- Zero tokens, local only, watch-only: nothing is sent to the model, nothing leaves the machine, no command is blocked, delayed or answered. The hook still always exits 0 and prints nothing.
- No new crates. No regex: hand-written matching only.
- The hook stays cheap: `risky::check` is pure, does no filesystem access (paths are resolved by text), and only runs for `Bash` on `PostToolUse` / `PostToolUseFailure`.
- Event lines stay under `event::MAX_LINE` (4096 bytes); the stored command is cut to 80 chars with `event::truncate`.
- Rule labels are exact copy, used everywhere: `delete outside repo`, `recursive delete`, `discards uncommitted work`, `rewrites remote history`, `deletes a branch`, `drops data`, `writes a raw disk`.
- Panel row copy: `⚠ ran {cmd} · {rule} · {age}` plus ` · +{n} more` when older ones exist. `--ascii` maps `⚠` to `!` (already in `ascii.rs`).
- CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Snapshot updates: `INSTA_UPDATE=always cargo test`, then review every `.snap` diff.
- One commit per task, `feat(scope):` / `test:` / `docs:` style, ending with the attribution line. Work on branch `risky-commands`.
- Patch version bump in `Cargo.toml`, `.claude-plugin/plugin.json` and `Cargo.lock` (next patch after `main`'s version), one `CHANGELOG.md` "## Unreleased" bullet.

## Review Focus

1. **Risky words that never run as a command** (`git commit -m "rm -rf old code"`, `echo "a; rm -rf /"`, `grep -rn "DROP TABLE" migrations/`): not flagged. Test `quoted_and_printed_text_is_not_flagged` in Task 1.
2. **Deletes that look local but land outside** (`rm -rf ../x`, `cd .. && rm -rf proj`, `rm -rf "$HOME/notes"`, `rm -rf ~`, `rm -rf /*`): flagged `delete outside repo`. Test `outside_targets_resolve_by_text` in Task 1.
3. **No cwd in the payload** (some events, other apps): the outside rule is skipped, the others still apply, no panic. Assertion in `outside_targets_resolve_by_text` (Task 1).
4. **A huge multi-line command** (a 10 KB heredoc script with a `DROP TABLE` on line 300): flagged, and the event line stays under 4096 bytes. Test `long_command_risk_fits_the_line` in Task 2.
5. **Old logs and caches** (an `events.jsonl` line without `risk`, a `state` cache at version 11): load fine; the cache is rebuilt. Test `old_line_without_risk_parses` in Task 2 and the `CACHE_VERSION` bump in Task 3.

What pattern matching cannot catch (goes verbatim into README and DECISIONS in Task 5):

- commands hidden in scripts or task runners (`./clean.sh`, `make clean`, `npm run reset`);
- variables and substitutions it can't resolve (`rm -rf $DIR`, `rm -rf $(cat list)`, backticks), aliases and shell functions;
- other languages doing the deleting (`python -c "shutil.rmtree(...)"`, `node -e ...`);
- file tools and MCP tools: a `Write` that overwrites a file, an `Edit` that empties one, a database MCP's drop;
- symlinks: paths are resolved as text, never on disk;
- PowerShell (`Remove-Item -Recurse`) and shell calls whose command isn't a single string;
- known false alarms: SQL keywords in a heredoc that only writes a `.sql` file; `rm -r` on a build folder is flagged even when harmless.
- Commands denied at the permission prompt never ran and are never flagged (by design).

---

### Task 1: `risky::check`, the pattern list

**Files:**
- Create: `src/risky.rs` (add `pub mod risky;` in `src/lib.rs`, alphabetical, after `replay`)

**Interfaces:**
- Produces:
  - Rule labels as `pub const`: `OUTSIDE = "delete outside repo"`, `RECURSIVE = "recursive delete"`, `DISCARD = "discards uncommitted work"`, `FORCE_PUSH = "rewrites remote history"`, `BRANCH = "deletes a branch"`, `DROP = "drops data"`, `DISK = "writes a raw disk"`.
  - `pub fn commands(cmd: &str) -> Vec<Vec<String>>`: simple commands, each as words with quotes removed. Splits on unquoted `;`, `&`, `|`, `(`, `)` and newlines (so `&&`, `||`, `|&` and subshells too). Single quotes are literal; double quotes allow `\"`; `\` outside quotes escapes the next char (`\;` stays a word). Empty commands are dropped.
  - `pub fn check(cmd: &str, cwd: &str, home: &str) -> Option<(&'static str, String)>`: the first risky simple command as `(rule, words joined by spaces cut to 80 chars)`, else `None`.
- Pinned constants (private):
  - `PREFIXES: [&str; 11] = ["sudo", "doas", "env", "command", "builtin", "exec", "nohup", "time", "nice", "timeout", "xargs"]`: dropped from the front, repeatedly, with the dash words after them; `-u`, `-g`, `-n`, `-I` also drop the next word; `timeout` also drops its duration word. Leading `NAME=value` words (first char a letter or `_`, `=` before any `/`) are dropped too.
  - `SHELLS: [&str; 4] = ["sh", "bash", "zsh", "dash"]`: `<shell> -c <script>` (any dash word containing `c`, e.g. `-lc`) recurses into `commands(script)`.
  - `TEMP_DIRS: [&str; 4] = ["/tmp/", "/private/tmp/", "/var/folders/", "/private/var/folders/"]`: exempt from `OUTSIDE` only.
  - `TEXT_TOOLS: [&str; 13] = ["echo", "printf", "grep", "egrep", "rg", "ag", "cat", "less", "head", "tail", "sed", "awk", "git"]`: skipped by the SQL rule.
  - `SQL: [&str; 4] = ["DROP TABLE", "DROP DATABASE", "DROP SCHEMA", "TRUNCATE TABLE"]` (not bare `TRUNCATE`: coreutils `truncate -s 0 f` would match), plus `DELETE FROM` with no `WHERE` after it.

**Rules**, checked per simple command in this order; the first hit wins:

| Rule | Program (after prefixes) | Matches |
|---|---|---|
| `OUTSIDE` | `rm`, `rmdir`, `unlink`, `shred` | any target (non-dash word, or every word after `--`) that resolves outside `cwd` and not under `TEMP_DIRS`; skipped when `cwd` is empty |
| `RECURSIVE` | `rm` | `-r`, `-R`, `--recursive`, or a short cluster with `r`/`R` (`-rf`, `-fR`) before `--` |
| `RECURSIVE` | `find` | `-delete`, or `-exec`/`-execdir` followed by `rm` |
| `DISCARD` | `git` | `reset --hard`; `checkout` with `.`, `-f`/`--force`, or any path after `--`; `restore` without `--staged` (or with `--worktree`/`-W`); `clean` with `-f`/`--force` or a cluster with `f`, unless `-n`/`--dry-run` |
| `FORCE_PUSH` | `git` | `push` with `--force`, `--force-with-lease[=…]`, `-f` or a short cluster with `f`, or a refspec starting with `+` |
| `BRANCH` | `git` | `branch` with `-D` or a short cluster with `D` |
| `DROP` | anything not in `TEXT_TOOLS` | the command's words joined and uppercased contain an `SQL` entry, or `DELETE FROM` with no `WHERE` after it |
| `DISK` | `dd` with an `of=/dev/` word; any program starting with `mkfs` | |

`git` global options before the subcommand are skipped: dash words, and `-C`/`-c` take the next word (`git -C ../x reset --hard` is `DISCARD`).

Path resolution (text only): `~`, `~/…`, `$HOME`, `${HOME}` → `home`; another leading `$` or a backtick → unknown (not `OUTSIDE`); absolute stays; relative joins the current cwd; then `.`/`..`/`//`/trailing `/` are normalized lexically. Outside means the result does not start with `{cwd}/` (so `cwd` itself and its parents count as outside). `cd DIR` / `pushd DIR` earlier in the same command move the current cwd for later commands (`cd` alone or `cd ~` → home; `cd -` → cwd becomes unknown and `OUTSIDE` is skipped for the rest).

- [ ] **Step 1: Write the failing tests** in `risky.rs`, with `let c = |cmd: &str| check(cmd, "/home/u/proj", "/home/u").map(|(r, _)| r);`:
  - `splits_on_unquoted_separators`:
    ```rust
    assert_eq!(commands("a 'b c' && d \"e;f\" | g\\;h"),
        vec![vec!["a", "b c"], vec!["d", "e;f"], vec!["g;h"]]);
    assert_eq!(commands("(cd x; rm -r y)\nls").len(), 3);
    assert!(commands("").is_empty());
    ```
  - `flags_each_rule`:
    ```rust
    assert_eq!(c("rm -rf build"), Some(RECURSIVE));
    assert_eq!(c("rm -r build"), Some(RECURSIVE));
    assert_eq!(c("rm -fR build"), Some(RECURSIVE));
    assert_eq!(c("find . -name '*.o' -delete"), Some(RECURSIVE));
    assert_eq!(c("find . -exec rm {} \\;"), Some(RECURSIVE));
    assert_eq!(c("git reset --hard HEAD~1"), Some(DISCARD));
    assert_eq!(c("git checkout -- ."), Some(DISCARD));
    assert_eq!(c("git checkout ."), Some(DISCARD));
    assert_eq!(c("git restore src/a.rs"), Some(DISCARD));
    assert_eq!(c("git clean -fd"), Some(DISCARD));
    assert_eq!(c("git clean -xdf"), Some(DISCARD));
    assert_eq!(c("git push --force"), Some(FORCE_PUSH));
    assert_eq!(c("git push -f origin main"), Some(FORCE_PUSH));
    assert_eq!(c("git push --force-with-lease"), Some(FORCE_PUSH));
    assert_eq!(c("git push origin +main"), Some(FORCE_PUSH));
    assert_eq!(c("git branch -D old"), Some(BRANCH));
    assert_eq!(c("psql -c \"drop table users\""), Some(DROP));
    assert_eq!(c("sqlite3 a.db 'DELETE FROM users'"), Some(DROP));
    assert_eq!(c("mysql -e 'truncate table logs'"), Some(DROP));
    assert_eq!(c("dd if=x.img of=/dev/disk2"), Some(DISK));
    assert_eq!(c("mkfs.ext4 /dev/sdb1"), Some(DISK));
    ```
  - `safe_commands_pass`:
    ```rust
    for ok in ["rm notes.txt", "rm -- -rf", "rm -f a.o", "git reset --soft HEAD~1",
        "git restore --staged a.rs", "git clean -n", "git push -u origin feat",
        "git push --follow-tags", "git branch -d merged", "git checkout main",
        "sqlite3 a.db 'DELETE FROM t WHERE id = 1'", "truncate -s 0 app.log", "cargo test", "ls -la"] {
        assert_eq!(c(ok), None, "{ok}");
    }
    ```
  - `prefixes_chains_and_shells`:
    ```rust
    assert_eq!(c("sudo rm -rf build"), Some(RECURSIVE));
    assert_eq!(c("sudo -u root rm -rf build"), Some(RECURSIVE));
    assert_eq!(c("FOO=1 BAR=2 rm -r build"), Some(RECURSIVE));
    assert_eq!(c("env -i git push -f"), Some(FORCE_PUSH));
    assert_eq!(c("timeout 5 git reset --hard"), Some(DISCARD));
    assert_eq!(c("cargo build && git push --force"), Some(FORCE_PUSH));
    assert_eq!(c("ls || rm -rf build"), Some(RECURSIVE));
    assert_eq!(c("bash -lc 'git reset --hard'"), Some(DISCARD));
    assert_eq!(c("find . | xargs rm -rf"), Some(RECURSIVE));
    assert_eq!(c("git -C ../other reset --hard"), Some(DISCARD));
    assert_eq!(check("cd x && rm -rf y", "/p", "/h").unwrap().1, "rm -rf y");
    ```
  - `quoted_and_printed_text_is_not_flagged` (Review Focus 1):
    ```rust
    for ok in ["git commit -m \"rm -rf old code\"", "echo \"a; rm -rf /\"",
        "grep -rn \"DROP TABLE\" migrations/", "echo 'git push --force'"] {
        assert_eq!(c(ok), None, "{ok}");
    }
    ```
  - `outside_targets_resolve_by_text` (Review Focus 2, 3):
    ```rust
    assert_eq!(c("rm -rf ../x"), Some(OUTSIDE));
    assert_eq!(c("cd .. && rm -rf proj"), Some(OUTSIDE));
    assert_eq!(c("rm -rf \"$HOME/notes\""), Some(OUTSIDE));
    assert_eq!(c("rm -rf ~"), Some(OUTSIDE));
    assert_eq!(c("rm -rf /*"), Some(OUTSIDE));
    assert_eq!(c("rm ../other/file.txt"), Some(OUTSIDE));
    assert_eq!(c("rm -rf ."), Some(OUTSIDE)); // the repo itself
    assert_eq!(c("rm -rf ./src/../build"), Some(RECURSIVE));
    assert_eq!(c("rm -rf /tmp/scratch"), Some(RECURSIVE)); // temp: only the recursive rule
    assert_eq!(c("rm /tmp/a.txt"), None);
    assert_eq!(c("rm -rf $DIR"), Some(RECURSIVE)); // unknown target: not OUTSIDE
    assert_eq!(c("cd - && rm a"), None);
    assert_eq!(check("rm -rf ../x", "", "/h").map(|r| r.0), Some(RECURSIVE)); // no cwd
    assert_eq!(check("rm ../x", "", "/h"), None);
    ```
  - `command_text_is_cut`: `check(&format!("rm -rf {}", "a".repeat(200)), "/p", "/h").unwrap().1.chars().count() == 80`.
- [ ] **Step 2: Run** `cargo test risky::` — expect FAIL (module missing).
- [ ] **Step 3: Implement** `commands`, `check` and private helpers (`strip_prefixes`, `resolve(word, cwd, home) -> Option<String>`, one `fn rule(words, cwd, home) -> Option<&'static str>`) in `src/risky.rs`. Module doc comment: what it flags, that it is watch-only, and the "cannot catch" list in one paragraph.
- [ ] **Step 4: Run** `cargo test risky::` — PASS; `cargo clippy --all-targets -- -D warnings` — clean.
- [ ] **Step 5: Commit** `feat(risky): match destructive shell commands`.

---

### Task 2: The hook records the risk on the event

**Files:**
- Modify: `src/event.rs` (`Event` struct; `normalize`, `PostToolUse` and `PostToolUseFailure` arms)

**Interfaces:**
- Consumes: `risky::check` (Task 1).
- Produces: `Event.risk: Option<(String, String)>` = `(rule, command)`, `#[serde(skip_serializing_if = "Option::is_none")]`, doc comment `Bash that ran: (rule, the risky command), see risky.rs.` Set only when `ev.tool == Some("Bash")` on `PostToolUse` or `PostToolUseFailure`, from `tool_input.command` (string only), the payload's `cwd` and `std::env::var("HOME")` (empty if unset).

- [ ] **Step 1: Write the failing tests** in `event.rs`'s test module:
  - `bash_that_ran_records_risk`: `n(json!({"hook_event_name":"PostToolUse","cwd":"/p","tool_name":"Bash","tool_input":{"command":"git push --force"},"tool_response":{"stdout":""}}))` → `risk == Some(("rewrites remote history".into(), "git push --force".into()))`; the same as `PostToolUseFailure` with `"error":"Exit code 1\nrejected"` → same `risk`.
  - `only_bash_that_ran`: `PreToolUse` of `git push --force` → `risk == None`; `PermissionRequest` of it → `None`; a `Write` whose content contains `rm -rf /` → `None`; `PostToolUse` of `cargo test` → `None`.
  - `long_command_risk_fits_the_line` (Review Focus 4): a `PostToolUse` Bash whose command is 300 lines of `echo x` then `psql <<'SQL'\nDROP TABLE users;\nSQL` → `risk.0 == "drops data"` and `to_line(&ev).len() < MAX_LINE`.
  - `old_line_without_risk_parses` (Review Focus 5): `serde_json::from_str::<Event>(r#"{"t":1,"e":"PostToolUse","tool":"Bash"}"#)` → `Ok` with `risk == None`; an event with `risk` round-trips through `to_line` and `serde_json::from_str`.
- [ ] **Step 2: Run** `cargo test event::` — FAIL.
- [ ] **Step 3: Implement** the field and the two `normalize` arms (one shared line after the `match`, guarded by event name and tool, is enough).
- [ ] **Step 4: Run** `cargo test` — PASS (no snapshot changes: no fixture runs a risky command).
- [ ] **Step 5: Commit** `feat(hook): record risky commands that ran`.

---

### Task 3: The reducer keeps them; the panel shows the latest

**Files:**
- Modify: `src/state.rs` (`State`, new `Risky`, `apply`'s `PostToolUse | PostToolUseFailure` arm, `CACHE_VERSION` +1: 11 → 12 unless another plan landed first)
- Modify: `src/panel.rs` (`lines`, the header section where `conflicts::summary` is inserted; new `fn risky_row`)

**Interfaces:**
- Consumes: `Event.risk` (Task 2).
- Produces:
  - `#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)] #[serde(default)] pub struct Risky { pub t: u64, pub agent: String, pub rule: String, pub cmd: String, pub ok: bool }` (`agent` is the agent id, `"main"` for the main agent; `ok` from `ev.ok`).
  - `State.risky: VecDeque<Risky>`, doc `Risky commands that ran, oldest first.`, capped at `const RISKY_KEEP: usize = 20`.
  - `fn risky_row(st: &State, c: &Ctx, now: u64) -> Option<Row>` in panel.rs: `⚠ ran {cmd} · {rule} · {age(now - t)}`, plus ` · +{n} more` for the older ones kept; yellow + bold like the conflicts row. Inserted at index `1.min(rows.len())` of the header rows, after the conflicts row and before badges are inserted (so the order under the verdict is badges, risky, conflicts).

- [ ] **Step 1: Write the failing tests:**
  - `state.rs` `risky_commands_are_kept`: apply 22 `PostToolUse` events with `risk: Some(("recursive delete".into(), format!("rm -r d{i}")))`, the last one with `a: Some("ag1")` and `ok: Some(false)` → `risky.len() == 20`, `risky.front().cmd == "rm -r d2"`, `risky.back()` has `agent == "ag1"`, `ok == false`; an event without `risk` adds nothing.
  - `panel.rs` `risky_row_under_the_verdict`: `multi()` state plus two applied risky events (`git reset --hard` at `NOW - 180_000`, then `rm -rf ../x` / `delete outside repo` at `NOW - 60_000`) → `render(...)` at 80×40 contains `⚠ ran rm -rf ../x · delete outside repo · 1m · +1 more`; with `ascii = true` in the config the same row starts with `! ran`; the plain `multi()` panel has no `⚠ ran`.
- [ ] **Step 2: Run** `cargo test state:: panel::` — FAIL.
- [ ] **Step 3: Implement** `Risky`, the field, the push in `apply`, `CACHE_VERSION` +1, and `risky_row` + its insertion.
- [ ] **Step 4: Run** `cargo test` — PASS with no snapshot changes (fixtures have no risky commands). If an insta snapshot changes, stop and find out why.
- [ ] **Step 5: Commit** `feat(panel): show the latest risky command under the verdict`.

---

### Task 4: `ari recap` lists them

**Files:**
- Modify: `src/recap.rs` (`Recap`, `build`, `markdown`), `src/panel.rs` (`recap_sections`, `sample_recap` in tests)

**Interfaces:**
- Consumes: `Event.risk` (Task 2), `state::Risky` (Task 3).
- Produces:
  - `Recap.risky: Vec<crate::state::Risky>`: every window event (`events[split_at..]`) with `risk`, oldest first, uncapped. `agent` holds the agent's label (`after.agents` entry's `label()`, `"main"` for the main agent) instead of its id.
  - `recap_sections`: a `risky` section right after `errors`, when non-empty. One row per command: `⚠ ` (yellow) + `{cmd} · {rule}`, then ` · {agent}` when not `main`, then ` · failed` when `!ok`.
  - `markdown`: `- **Risky:** {cmd} ({rule}); {cmd} ({rule})` after the `Failing` bullet.

- [ ] **Step 1: Write the failing tests** in `recap.rs` (use `session_from`):
  - `risky_commands_in_the_window`: events `PostToolUse` Bash with risk `git reset --hard`/`discards uncommitted work` at t=1_000, the same shape `rm -rf ../x`/`delete outside repo` with `a: Some("ag1")`, `at: Some("Explore".into())`, `ok: Some(false)` at t=60_000 → `build(&d, Since::Start, 70_000).risky.len() == 2`; `build(&d, Since::Ago(30_000), 70_000).risky` has one entry, `cmd == "rm -rf ../x"`, `ok == false`. The recap text (`recap_lines` at 60 wide) contains `├─ risky ` and `⚠ rm -rf ../x · delete outside repo · Explore · failed`; `markdown` contains `- **Risky:** rm -rf ../x (delete outside repo)`.
  - `no_risky_section_when_clean`: `whole_session` fixture recap text has no `├─ risky`.
- [ ] **Step 2: Run** `cargo test recap::` — FAIL.
- [ ] **Step 3: Implement** the field, the window scan in `build` (next to the `files` loop), the section and the bullet.
- [ ] **Step 4: Run** `INSTA_UPDATE=always cargo test`; `recap_box` / `recap_view` must not change (`sample_recap` has no risky). Then `cargo test` — PASS.
- [ ] **Step 5: Commit** `feat(recap): list risky commands that ran`.

---

### Task 5: Docs, version, real check

**Files:**
- Modify: `README.md`, `CHANGELOG.md`, `docs/DECISIONS.md`, `Cargo.toml`, `.claude-plugin/plugin.json`, `Cargo.lock`

- [ ] **Step 1: README.** Under "What you get", after the recap block, a short **Risky commands** paragraph: what is flagged (the seven rule labels in one sentence), that it shows under the verdict in `ari watch` and as a `risky` section in `ari recap`, that ARI never blocks anything, and the "cannot catch" list from Review Focus as bullets. Add a `risky` line to the recap example block (`risky     1 · git push --force`) only if it matches real output; otherwise leave the block alone.
- [ ] **Step 2: DECISIONS.** New `- **D<next free number>, risky commands:**` bullet (D25 if nothing else took it): flagged only from Post events (commands that ran; a denied one fires no Post event); paths resolved as text against the payload's `cwd`, so symlinks and variables aren't followed; temp dirs exempt from the outside rule only; the full command never lands on disk, only the matched simple command (80 chars).
- [ ] **Step 3: CHANGELOG** under `## Unreleased`: `- **Risky commands.** \`ari watch\` shows the latest risky Bash command that ran under the verdict (recursive deletes, deletes outside the repo, \`git reset --hard\`, force pushes, forced cleans, discarded changes, deleted branches, \`DROP TABLE\`, raw disk writes) and \`ari recap\` lists every one. Watch-only: nothing is blocked.`
- [ ] **Step 4: Version.** Next patch version in `Cargo.toml` and `.claude-plugin/plugin.json`; run `cargo check` so `Cargo.lock`'s `ari-cli` entry follows.
- [ ] **Step 5: Run** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` — all pass.
- [ ] **Step 6: Real check.** `cargo build --release`; pipe one recorded payload through the hook into a temp state dir: `d=$(mktemp -d); echo '{"hook_event_name":"PostToolUse","session_id":"risk1","cwd":"/tmp/p","tool_name":"Bash","tool_input":{"command":"cd .. && rm -rf p"},"tool_response":{}}' | XDG_STATE_HOME=$d ./target/release/ari hook; cat $d/ari/sessions/risk1/events.jsonl` shows `"risk":["delete outside repo","rm -rf p"]`; `XDG_STATE_HOME=$d ./target/release/ari recap --session risk1` shows the `risky` section.
- [ ] **Step 7: Commit** `docs: risky commands` and `chore: bump version`. Open the PR with a bullet body (per the repo's git workflow).

---

## Self-review

- Spec coverage: every named pattern (`rm -rf`/`rm -r`, `git reset --hard`, `git push --force`/`-f`, `git clean -fd`, `git checkout -- .`, deletes outside the repo or under `~`, `DROP TABLE`) has an assertion in Task 1; panel (Task 3) and recap (Task 4) both show them; watch-only stated in Global Constraints and docs.
- Tricky cases pinned: quoted args, `sudo`, `&&`/`||`/`;`/pipes, env prefixes, `bash -c`, `cd` in the chain, `git -C`, `--`.
- Types: `(&'static str, String)` from `check` → `Option<(String, String)>` on `Event` → `state::Risky { t, agent, rule, cmd, ok }` in State and Recap; names match across tasks.
- Proportion: one new pure module, one event field, one state field, one panel row, one recap section.

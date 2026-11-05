# M4: Launch polish — design

**Milestone done when:** a stranger can install ARI and see its value in under 2 minutes.

## Intent

Everything between "it works on my machine" and "someone on Hacker News tries it": something to look at before installing (`ari demo`, the GIF), a way to tell what's wrong when it doesn't show up (`ari doctor`), a fallback for plain terminals (`--ascii`), and binaries so nobody needs a Rust toolchain (PRD §9, §11, §14).

## Goals

1. **`ari demo`**: replays the bundled `fixtures/multi` session in the real panel at 1.5× speed, with a plausible usage curve (context, 5h, cost rising), then holds on the final frame until `q`. No Claude needed. `--speed <x>`, `--once` prints the final frame.
2. **`--ascii`** for `watch` and `demo`, and `ascii = true` in config for the statusline too: `#`/`-` bars, `|/-\` spinner, `+ - |` boxes, ASCII verdict glyphs. A pure character map applied after layout; every mapped glyph is one column, so layout is unchanged.
3. **`ari doctor`** and `/ari:doctor`: binary on PATH (and which one), plugin installed and enabled, statusline wired to ARI, each chained command's health (ok/time, failed, timed out), last hook event age. Each problem gets a one-line fix. Exit 1 if anything failed.
4. **README**: hero GIF at the top, recorded from `ari demo` with `vhs` (`docs/demo.tape` committed), three-line install, "Why ARI" table, FAQ, contributing.
5. **Releases with `dist`** (cargo-dist): GitHub Releases for macOS (arm64, x86_64) and Linux (x86_64, arm64), a shell installer one-liner, plus `cargo install ari-cli`. Homebrew tap and crates.io publishing are prepared but need the owner's tokens (see below).
6. **Repo hygiene:** CHANGELOG, issue templates, CONTRIBUTING, a few `good first issue` candidates listed in CONTRIBUTING.

## Non-goals

Windows (v1.1), themes, new segments.

## Design notes

- **Demo source:** `include_str!` the fixture's `raw.jsonl` so the binary is self-contained; events normalize at start, and offsets are rescaled by speed. The clock and all durations use the replay's virtual time, so the panel reads like a live run. Usage is synthetic and labeled in code as demo-only; tokens come from the bundled transcripts' final ledger, scaled by progress.
- **The TUI loop** in `watch.rs` becomes `run_tui(frame: impl FnMut(View, u16, u16) -> (Vec<Line>, bool /*animating*/))`, shared by `watch` and `demo`.
- **ASCII map:** `█▰→#`, `░▱→-`, `╭╮╰╯├┤└→+`, `─→-`, `│→|`, braille frames → `|/-\`, `●→*`, `◉→@`, `✗→x`, `↻→~`, `✓→v`, `◌→o`, `▸→>`, `↺→@`, `⟲→~`, `▲→^`, `⚠→!`, `…→.`, `·→-`, `×→x`, `≈→~`, `→→>`, `—→-`, `•→o`.
- **Doctor plugin check:** `~/.claude/plugins/installed_plugins.json` lists an `ari@…` entry, and `enabledPlugins` in settings doesn't set it to `false`. Falls back to "hooks: ari hook found in settings" for people who wired hooks by hand.
- **Bundled binaries in the plugin (PRD open question 1): no.** A plugin's `bin/` only reaches the Bash tool's PATH, not necessarily hooks; per-platform binaries would bloat every clone; and claude.ai/Cowork refuse to install plugins with a top-level `bin/`. Install stays: binary, plugin, `ari setup`. Recorded as D19.
- **Owner-only steps** (not automated, listed in the PR): pushing a `v0.1.0` tag to publish the release; creating `Izuulo/homebrew-tap` with a `HOMEBREW_TAP_TOKEN` secret; `cargo publish` with a crates.io token.

## Testing

- ASCII: map unit test; ASCII panel snapshot keeps the exact width of the Unicode one; `--ascii` bar has no non-ASCII chars.
- Demo: frame at 0, middle and end (`--once` path) snapshot; replay offsets scaled by speed.
- Doctor: run against a temp `CLAUDE_CONFIG_DIR`/`XDG_*` with each check passing and failing.
- Install path: fresh temp `HOME`, `cargo install --path .`, `ari setup --yes`, `ari doctor` → timed under 2 minutes, recorded in the PR.

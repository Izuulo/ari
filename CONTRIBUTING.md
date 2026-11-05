# Contributing

Thanks for helping. ARI is small on purpose; the best changes keep it that way.

## Build and test

```sh
cargo test                       # unit, integration and snapshot tests
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo run -- demo                # see the panel without Claude
```

Snapshots use [insta](https://insta.rs): after an intended layout change, run `INSTA_UPDATE=always cargo test` and review the `.snap` diff.

Performance budgets: `scripts/bench-hook.sh` (p95 < 15 ms) and `scripts/bench-statusline.sh` (p95 < 50 ms).

## Ground rules

- **Hooks stay dumb and fast.** `ari hook` appends one line and exits 0, silently, always. Interpretation belongs in the reducer (`src/state.rs`).
- **Zero tokens.** Nothing ARI does may print to a hook's stdout or otherwise reach Claude's context.
- **Never break the user's statusline.** Chained output is passed through byte for byte.
- **No faked progress.** If the data isn't there, show `—` or `no plan yet`.
- **Real fixtures.** New reducer behavior gets a test; recorded sessions come from `scripts/capture.sh` and are scrubbed with `scripts/scrub.sh`. Check the result for private content before committing.
- Where you deviate from `docs/PRD.md`, add a line to `docs/DECISIONS.md`.

## Good first issues

- A light theme for `ari watch` (colors live in `src/panel.rs`).
- A `model` segment for the compact row (`src/bar.rs`).
- Show `lines added/removed` in the panel's session section (already in `usage.json`).
- `ari watch --all`: let `enter` open the highlighted session.

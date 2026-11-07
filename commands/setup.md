---
description: Wire ARI into your Claude Code statusline, and into Codex and dsh if installed
allowed-tools: Bash(ari setup:*), Bash(command -v ari)
---

Set up ARI for the user: the Claude Code statusline, and hooks for Codex (`~/.codex/hooks.json`) and DeepSeek Harness (a dsh patch) when they are installed.

1. Run `command -v ari`. If it prints nothing, tell the user the `ari` binary isn't on PATH, show the install line from https://github.com/Izuulo/ari#install, and stop.
2. Run `ari setup --dry-run` and show the user its output exactly as printed. It lists the `statusLine` change to `~/.claude/settings.json`, the Codex and dsh changes, and any existing statusline command ARI will keep running (chained) above or below its own row.
3. Ask the user whether to apply it. Only if they say yes, run `ari setup --yes` and show its output.

Don't edit settings.json yourself, and don't run `ari setup --yes` without the user's yes.

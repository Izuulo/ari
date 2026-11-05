---
description: Wire ARI into your Claude Code statusline (keeps your current one)
allowed-tools: Bash(ari setup:*), Bash(command -v ari)
---

Set up ARI's statusline for the user.

1. Run `command -v ari`. If it prints nothing, tell the user the `ari` binary isn't on PATH, show the install line from https://github.com/Izuulo/ari#install, and stop.
2. Run `ari setup --dry-run` and show the user its output exactly as printed. It lists the `statusLine` change to `~/.claude/settings.json` and any existing statusline command ARI will keep running (chained) above or below its own row.
3. Ask the user whether to apply it. Only if they say yes, run `ari setup --yes` and show its output.

Don't edit settings.json yourself, and don't run `ari setup --yes` without the user's yes.

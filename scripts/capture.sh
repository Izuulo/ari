#!/usr/bin/env bash
# Record a real Claude Code session as an ARI fixture.
# usage: scripts/capture.sh <name> <prompt-file> [extra claude args...]
# Runs `claude -p` (Haiku) in a throwaway project with every hook wired to `ari hook`,
# then copies the raw hook log and transcripts into fixtures/<name>/ and scrubs them.
set -euo pipefail
name=$1 prompt=$2; shift 2
root=$(cd "$(dirname "$0")/.." && pwd)
cargo build --release -q --manifest-path "$root/Cargo.toml"
bin="$root/target/release/ari"
work=$(cd "$(mktemp -d)" && pwd -P)
proj="$work/proj" out="$root/fixtures/$name"
mkdir -p "$proj" "$out"
: > "$out/raw.jsonl"

python3 -I - "$bin" "$out/raw.jsonl" "$work/state" > "$work/settings.json" <<'PY'
import json, shlex, sys
bin, raw, state = sys.argv[1:]
cmd = f"ARI_RAW_LOG={shlex.quote(raw)} XDG_STATE_HOME={shlex.quote(state)} {shlex.quote(bin)} hook"
events = ("SessionStart UserPromptSubmit PreToolUse PostToolUse PostToolUseFailure SubagentStart "
          "SubagentStop Notification PreCompact PostCompact Stop StopFailure SessionEnd TaskCreated TaskCompleted").split()
print(json.dumps({"hooks": {e: [{"hooks": [{"type": "command", "command": cmd, "timeout": 2}]}] for e in events}}))
PY

(cd "$proj" && CLAUDE_CODE_ENABLE_TODO_TOOLS=1 claude -p "$(cat "$prompt")" \
  --model haiku --settings "$work/settings.json" --permission-mode dontAsk "$@")

transcript=$(python3 -I -c 'import json,sys
for l in open(sys.argv[1]):
    p = json.loads(l)["p"]
    if p.get("transcript_path"): print(p["transcript_path"]); break' "$out/raw.jsonl")
cp "$transcript" "$out/transcript.jsonl"
if [ -d "${transcript%.jsonl}/subagents" ]; then
  rm -rf "$out/subagents" && cp -R "${transcript%.jsonl}/subagents" "$out/subagents"
fi
cp "$prompt" "$out/prompt.txt" 2>/dev/null || true
"$root/scripts/scrub.sh" "$out" "$proj"
echo "fixture written to $out"

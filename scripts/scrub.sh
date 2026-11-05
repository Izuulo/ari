#!/usr/bin/env bash
# Replace private paths and the user name in a fixture dir, in place.
# usage: scripts/scrub.sh <fixture-dir> <project-path>
set -euo pipefail
dir=$1 proj=$2
export PROJ=$proj PROJ_ENC=${proj//\//-} HOME_ENC=${HOME//\//-} U=${USER:-$(id -un)}
find "$dir" -type f \( -name '*.jsonl' -o -name '*.json' \) -print0 | xargs -0 perl -pi -e '
  s/\Q$ENV{PROJ}\E/\/home\/user\/proj/g;
  s/\Q$ENV{PROJ_ENC}\E/-home-user-proj/g;
  s/\Q$ENV{HOME}\E/\/home\/user/g;
  s/\Q$ENV{HOME_ENC}\E/-home-user/g;
  s/\Q$ENV{U}\E/user/g;
'

# Transcripts carry the user's whole environment (hook output, skill and MCP listings,
# account records). Keep only what ARI reads: assistant usage per message.
find "$dir" -name '*.jsonl' ! -name raw.jsonl -print0 | xargs -0 -n1 python3 -I -c '
import json, sys
path = sys.argv[1]
keep = []
for line in open(path):
    d = json.loads(line)
    if d.get("type") != "assistant":
        continue
    m = d.get("message", {})
    u = m.get("usage", {})
    keep.append(json.dumps({
        "type": "assistant", "timestamp": d.get("timestamp"), "isSidechain": d.get("isSidechain"),
        "message": {"id": m.get("id"), "model": m.get("model"),
                    "content": [{"type": c.get("type")} for c in m.get("content", [])],
                    "usage": {k: u.get(k, 0) for k in ("input_tokens", "cache_creation_input_tokens",
                                                       "cache_read_input_tokens", "output_tokens")}},
    }))
open(path, "w").write("\n".join(keep) + "\n")
'
find "$dir" -name '*.meta.json' -delete

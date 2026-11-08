#!/usr/bin/env bash
# Measure `ari hook` wall time over N runs (default 200). Target: p95 < 15 ms.
# Timing includes process spawn from Python, so it slightly overstates the real cost.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cargo build --release -q --manifest-path "$root/Cargo.toml"
state=$(mktemp -d)
python3 -I - "$root/target/release/ari" "$state" "${1:-200}" <<'PY'
import json, subprocess, sys, time, os
bin, state, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
payload = json.dumps({"session_id": "bench", "hook_event_name": "PostToolUse", "cwd": "/p",
    "tool_name": "Edit", "tool_input": {"file_path": "/p/src/main.rs", "old_string": "a" * 2000, "new_string": "b" * 2000},
    "tool_response": {"filePath": "/p/src/main.rs"}, "tool_use_id": "toolu_bench"}).encode()
env = dict(os.environ, XDG_STATE_HOME=state, XDG_CONFIG_HOME=state)
times = []
for _ in range(n):
    t = time.perf_counter()
    subprocess.run([bin, "hook"], input=payload, env=env, check=True)
    times.append((time.perf_counter() - t) * 1000)
times.sort()
p = lambda q: times[min(len(times) - 1, int(q * len(times)))]
print(f"ari hook over {n} runs: p50 {p(0.5):.2f} ms  p95 {p(0.95):.2f} ms  max {times[-1]:.2f} ms")
sys.exit(0 if p(0.95) < 15 else 1)
PY
rm -rf "$state"

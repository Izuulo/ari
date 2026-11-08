#!/usr/bin/env bash
# Measure `ari statusline` render time (no chained commands) over N runs. Target: p95 < 50 ms.
# Replays the basic fixture into a session first, so the reducer has real events.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cargo build --release -q --manifest-path "$root/Cargo.toml"
tmp=$(mktemp -d)
python3 -I - "$root/target/release/ari" "$tmp" "$root/fixtures/basic/raw.jsonl" "${1:-200}" <<'PY'
import json, os, subprocess, sys, time
bin, tmp, raw, n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
env = dict(os.environ, XDG_STATE_HOME=f"{tmp}/state", XDG_CONFIG_HOME=f"{tmp}/config", COLUMNS="160")
os.makedirs(f"{tmp}/config/ari")
open(f"{tmp}/config/ari/config.toml", "w").write("[notify]\ndesktop = false\n")  # benches must not pop notifications
for line in open(raw):
    p = json.loads(line)["p"]
    p["session_id"] = "bench"
    subprocess.run([bin, "hook"], input=json.dumps(p).encode(), env=env, check=True)
payload = json.dumps({"session_id": "bench", "context_window": {"used_percentage": 40},
    "rate_limits": {"five_hour": {"used_percentage": 30, "resets_at": int(time.time()) + 3600}},
    "cost": {"total_cost_usd": 1.23}}).encode()
times = []
for _ in range(n):
    t = time.perf_counter()
    subprocess.run([bin, "statusline"], input=payload, env=env, check=True, stdout=subprocess.DEVNULL)
    times.append((time.perf_counter() - t) * 1000)
times.sort()
p = lambda q: times[min(len(times) - 1, int(q * len(times)))]
print(f"ari statusline over {n} runs: p50 {p(0.5):.2f} ms  p95 {p(0.95):.2f} ms  max {times[-1]:.2f} ms")
sys.exit(0 if p(0.95) < 50 else 1)
PY
rm -rf "$tmp"

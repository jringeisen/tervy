#!/usr/bin/env bash
# Cold-start benchmark (implementation-plan.md task 0.6).
# Launches Tervy N times, each run quitting right after its first frame,
# and reports p50/p95 of launch → first rendered frame.
#
# Usage: scripts/perf/startup.sh [release|debug] [runs]
set -euo pipefail
cd "$(dirname "$0")/../.."

profile="${1:-release}"
runs="${2:-10}"
if [[ "$profile" == "release" ]]; then
    cargo build --release -p tervy-app --quiet
else
    cargo build -p tervy-app --quiet
fi
bin="target/$profile/tervy"

results=()
for i in $(seq 1 "$runs"); do
    t0=$(python3 -c 'import time; print(time.time_ns())')
    line=$(TERVY_PERF_T0="$t0" TERVY_PERF_EXIT=1 "$bin" 2>&1 >/dev/null | grep '^tervy-perf' || true)
    if [[ -z "$line" ]]; then
        echo "run $i: no tervy-perf line (did the window open?)" >&2
        exit 1
    fi
    ms=$(sed -E 's/.*to_first_frame_ms=([0-9.]+).*/\1/' <<<"$line")
    results+=("$ms")
    echo "run $i: $line"
done

printf '%s\n' "${results[@]}" | python3 -c '
import sys, statistics
values = sorted(float(v) for v in sys.stdin.read().split())
p95 = values[min(len(values) - 1, round(0.95 * (len(values) - 1)))]
print(f"first frame ({len(values)} runs): p50={statistics.median(values):.1f} ms  p95={p95:.1f} ms  min={values[0]:.1f}  max={values[-1]:.1f}")
'

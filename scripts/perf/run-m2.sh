#!/usr/bin/env bash
# M2 regression measurements. Close other DevDesk windows before running.
set -euo pipefail
cd "$(dirname "$0")/../.."
output_dir="${1:-perf/local}"
mkdir -p "$output_dir"
cargo build --release -p devdesk-app
cargo build --release -p devdesk-term --example session_cleanup
DEVDESK_PERF_LAYOUT=terminal ./scripts/perf/startup.sh release 10 > "$output_dir/m2-startup.txt"
python3 scripts/perf/terminal.py echo --output "$output_dir/m2-echo.json"
python3 scripts/perf/many.py --output "$output_dir/m2-many.json"
# Keep this isolated: no builds or other benchmark windows during idle sampling.
python3 scripts/perf/idle.py --layout terminal --seconds 60 --output "$output_dir/m2-idle.json"

#!/usr/bin/env bash
# Synthetic fixtures only; no owner config, service commands or model calls.
set -euo pipefail
cd "$(dirname "$0")/../.."
output_dir="${1:-perf/local}"
mkdir -p "$output_dir" target/perf-helpers
cargo build --release -p devdesk-app
cargo build --release -p devdesk-term --example session_cleanup
swiftc -module-cache-path target/perf-helpers/module-cache scripts/perf/window-state.swift -o target/perf-helpers/window-state
DEVDESK_PERF_LAYOUT=terminal scripts/perf/startup.sh release 10 > "$output_dir/m7-startup.txt"
python3 scripts/perf/many.py --noisy 20 --output "$output_dir/m7-many.json"
python3 scripts/perf/terminal.py echo --output "$output_dir/m7-echo.json"
python3 scripts/perf/terminal.py flood --output "$output_dir/m7-flood.json"
# No builds or other probes while the idle measurements run.
python3 scripts/perf/idle.py --layout idle20 --seconds 60 --sample --output "$output_dir/m7-idle20.json"
python3 scripts/perf/idle.py --layout scrollback20 --seconds 20 --output "$output_dir/m7-scrollback20.json"
python3 scripts/perf/idle.py --layout terminal --seconds 60 --output "$output_dir/m7-idle.json"
python3 scripts/perf/idle.py --layout normal --seconds 20 --output "$output_dir/m7-empty.json"

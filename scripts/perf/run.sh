#!/usr/bin/env bash
# Reproduce the M1 release measurements; results stay local until reviewed.
set -euo pipefail
cd "$(dirname "$0")/../.."
output_dir="${1:-perf/local}"
mkdir -p "$output_dir"
cargo build --release -p devdesk-term --example memory_probe
cargo build --release -p devdesk-app
./scripts/perf/startup.sh release 10 > "$output_dir/startup.txt"
for scenario in echo flood seq claude codex; do
    python3 scripts/perf/terminal.py "$scenario" --output "$output_dir/$scenario.json"
done
# No build or other benchmark runs concurrently with the idle sample.
python3 scripts/perf/idle.py --layout terminal --seconds 60 --output "$output_dir/idle.json"
for scenario in plain styled combining links; do
    target/release/examples/memory_probe "$scenario" > "$output_dir/memory-$scenario.jsonl"
done

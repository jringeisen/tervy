#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo build --release -p tervy-app
cargo build --release -p tervy-term --example session_cleanup
mkdir -p target/perf-helpers
swiftc -module-cache-path target/perf-helpers/module-cache scripts/perf/window-state.swift -o target/perf-helpers/window-state
python3 scripts/perf/many.py --noisy 20 --output "${1:-perf/local/m7-many.json}"

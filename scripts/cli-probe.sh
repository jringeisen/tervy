#!/usr/bin/env bash
# Default is a initialization-only, zero-model-cost Claude hook merge probe.
# --real also runs the paid Claude fork/enforcement/resume probes (Appendix A).
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 scripts/cli_probe.py "$@"

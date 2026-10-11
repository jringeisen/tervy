#!/usr/bin/env bash
# The gate every change passes before review (implementation-plan.md task 0.4).
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> cargo fmt";    cargo fmt --all -- --check
echo "==> cargo clippy"; cargo clippy --workspace --all-targets -- -D warnings
echo "==> cargo test";   cargo test --workspace
echo "==> backup tests"; python3 -m unittest discover -s scripts -p 'test_*.py'
echo "==> cargo deny";   cargo deny check licenses
echo "All checks passed."

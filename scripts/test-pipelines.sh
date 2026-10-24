#!/usr/bin/env bash
# Real HTTP Git + runner + SSH acceptance. Python asserts response values and
# process samples, enforces a check floor, and returns nonzero on any failure.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -p ferry-server -p ferry-runner
exec python3 -I scripts/test-pipelines.py

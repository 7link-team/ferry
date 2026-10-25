#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --locked -p ferry-server -p ferry-runner
python3 -I scripts/test-deployments.py

#!/usr/bin/env bash
# Use for worker builds/tests so the pinned upstream PTY fix is always applied.
set -euo pipefail
cd "$(dirname "$0")"
worker_go=${ACTIONS_GO:-go}
worker_patch=$(mktemp -d)
trap 'rm -rf "$worker_patch"' EXIT
GOOS= GOARCH= CGO_ENABLED=0 "$worker_go" run ./internal/actpatch "$worker_patch"
cd "$worker_patch/worker"
"$worker_go" "$@"

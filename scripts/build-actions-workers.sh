#!/usr/bin/env bash
# Build the pinned Actions kernel for hosts managed by either server package.
set -euo pipefail
worker_output=${1:-target/debug/actions-workers}
mkdir -p "$worker_output"
worker_output=$(cd "$worker_output" && pwd)
cd "$(dirname "$0")/../workers/actions"
worker_go=${ACTIONS_GO:-go}
for worker_target in ${ACTIONS_WORKER_TARGETS:-linux/amd64 linux/arm64 darwin/amd64 darwin/arm64}; do
  case "$worker_target" in linux/amd64|linux/arm64|darwin/amd64|darwin/arm64) ;; *) echo "Unsupported worker target: $worker_target" >&2; exit 1;; esac
  worker_os=${worker_target%/*}
  worker_arch=${worker_target#*/}
  CGO_ENABLED=0 GOOS="$worker_os" GOARCH="$worker_arch" ACTIONS_GO="$worker_go" bash ./go.sh build -trimpath -ldflags='-s -w' -o "$worker_output/actions-worker-$worker_os-$worker_arch" .
done

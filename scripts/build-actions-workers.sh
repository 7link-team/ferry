#!/usr/bin/env bash
# Build the pinned Actions kernel for hosts managed by either server package.
set -euo pipefail
worker_output=${1:-target/debug/actions-workers}
mkdir -p "$worker_output"
worker_output=$(cd "$worker_output" && pwd)
cd "$(dirname "$0")/../workers/actions"
worker_go=${ACTIONS_GO:-go}
# Targets are independent (go.sh patches into its own temporary module), so
# they build concurrently; any failure fails the whole build.
worker_targets=${ACTIONS_WORKER_TARGETS:-linux/amd64 linux/arm64 darwin/amd64 darwin/arm64 windows/amd64 windows/arm64}
for worker_target in $worker_targets; do
  case "$worker_target" in linux/amd64|linux/arm64|darwin/amd64|darwin/arm64|windows/amd64|windows/arm64) ;; *) echo "Unsupported worker target: $worker_target" >&2; exit 1;; esac
done
worker_pids=()
for worker_target in $worker_targets; do
  worker_os=${worker_target%/*}
  worker_arch=${worker_target#*/}
  worker_suffix=
  if [ "$worker_os" = windows ]; then worker_suffix=.exe; fi
  CGO_ENABLED=0 GOOS="$worker_os" GOARCH="$worker_arch" ACTIONS_GO="$worker_go" bash ./go.sh build -trimpath -ldflags='-s -w' -o "$worker_output/actions-worker-$worker_os-$worker_arch$worker_suffix" . &
  worker_pids+=("$!")
done
worker_failed=0
for worker_pid in "${worker_pids[@]}"; do
  wait "$worker_pid" || worker_failed=1
done
exit "$worker_failed"

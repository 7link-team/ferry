# Actions worker

The service dispatches one job to this worker over `HostChannel`. The worker
uses the pinned `nektos/act v0.2.89` library for Actions expressions, steps,
`needs` outcomes/outputs, matrices, and action execution. It does not connect
to GitHub or Gitea as a registered runner.

Build with Go 1.25 or newer:

```sh
bash scripts/build-actions-workers.sh target/debug/actions-workers
```

The directory contains Linux/macOS workers for amd64/arm64. The service looks
beside its executable for `actions-workers`, or uses `ACTIONS_WORKER_DIR`.
Desktop packages supply their bundled resource directory. Neither installed
servers nor execution hosts need Go. Host execution needs Git and the runtimes
used by actions (notably Node for JavaScript actions); container execution needs
Docker and an image containing the required runtimes.

The request is a private JSON file passed with `--request`. It contains the
original workflow, selected job, resolved dependency results, platform/image
mapping, variable snapshot, repository work directory and timeout bound.
Stdout contains JSON log records followed by a `kind: result` record. Both the
message and structured fields are masked before output. Results carry outputs
and artifact paths, never the request secrets. SIGTERM/SIGINT cancel execution;
post steps and engine cleanup run before the result is returned.

Artifacts use the upstream v4 protocol with a per-worker capability check.
Bearer tokens are required for metadata requests; returned blob URLs carry a
separate random capability. The filesystem is confined using `os.Root`.
Archives are transferred back through `HostChannel` into the existing service
artifact store and restored before subsequent jobs, including jobs on other
hosts. Run archives remain immutable: use distinct names; overwrite/delete is
rejected. Retention follows the platform's build-history policy.

Tests:

```sh
cd workers/actions
bash go.sh test ./...
ACTIONS_TEST_REMOTE=1 bash go.sh test -run TestArtifactActions -count=1
ACTIONS_TEST_REMOTE=1 ACTIONS_TEST_DOCKER_IMAGE=<local-node-image> bash go.sh test -run TestArtifactActions -count=1
```

The remote test fetches `actions/upload-artifact@v4` and
`actions/download-artifact@v4`. The container variant additionally requires a
local Docker daemon and an image with Git, Node, CA certificates and `sh`.

Worker builds and tests use `go.sh` to apply the pinned act PTY synchronization
fix in a disposable source tree. The pinned module is downloaded on cache
misses; the shared Go module cache is never modified. A stable relative module
replacement keeps independently staged binaries reproducible.
The patches make the cross-thread stop flag atomic and enforce matrix
`fail-fast` using the engine’s existing job result and sequential executor.
Source matching fails closed if upstream changes. Run `bash go.sh test -race ./...`
to include the native execution race regression. Matrix branches currently
execute serially because the pinned engine shares mutable step models.

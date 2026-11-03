# Actions worker

The service dispatches one job to this worker over `HostChannel`. The worker
uses the pinned `nektos/act v0.2.89` library for Actions expressions, steps,
`needs` outcomes/outputs, matrices, and action execution. It does not connect
to GitHub or Gitea as a registered runner.

Build with Go 1.25 or newer:

```sh
bash scripts/build-actions-workers.sh target/debug/actions-workers
```

The directory contains Linux/macOS/Windows workers for amd64/arm64. Windows
filenames are `actions-worker-windows-amd64.exe` and
`actions-worker-windows-arm64.exe`; Linux/macOS names have no extension. The service looks
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
remaining `cancelled()`/`always()` steps, post steps and engine cleanup share a
30-second cancellation grace before the result is returned. A cleanup script
that hangs is killed when that grace expires. Artifact HTTP shutdown has a
separate maximum of five seconds. The launcher must allow these bounds before
forcibly terminating the worker.

Server requests additionally bind `execution_host`, `execution_slot`, and
`execution_capacity`. The worker holds a kernel file lock until execution and
cancellation cleanup finish. SQL lease expiry alone therefore cannot admit a
second worker into an occupied slot, including when capacity has been reduced.
Locks are shared across SSH and HostAgent invocations in the system shared data
directory (`/tmp` or Windows ProgramData); permission errors fail closed. A
successor waits up to 60 seconds and then reports occupied capacity. Lock files
must not be removed while workers may be running. Standalone requests without
these fields retain their existing behavior.

## Worker protocol

`Request` also accepts `actor`, `run_number` (int64), `run_attempt` (int), `ref`,
`sha`, `repository` (`owner/repository`) and `server_url`, alongside the existing
`event_name`, `event_path`, `inputs`, `vars`, `secrets` and resolved `needs`.
These populate the GitHub context; the event JSON remains the trigger payload.
`run_cancelled: true` means the run was cancelled before this job started. As
on GitHub, only the job-level `if` sees it (`cancelled() == true`,
`success() == false`, `always() == true`): a job it rejects (including the
default `success()`) reports `skipped`; a job it admits runs its steps
normally. `mode: "plan"` results also carry `max_parallel` (0 = unbounded) and
`fail_fast`, so a caller spreading instances applies the same strategy.

`mode: "plan"` evaluates the selected job's matrix using the supplied dependency
outputs, inputs and variables. It returns `instances: [{name, matrix}, ...]` in
declared axis/product order (then unmatched includes). It starts no steps,
action fetches, cache/artifact listener or Docker connection. An empty/omitted
`mode` executes normally. Supplying a non-null `matrix` object, including `{}`,
executes exactly that instance without fan-out. Omit `matrix` to execute all
instances inside this worker. Distributed dispatch must retain the original
workflow and dependency/input snapshots so strategy totals remain available.

Every instance owns its workflow model, steps, output expressions and action
cache namespace. `strategy.max-parallel` controls scheduling; fail-fast cancels
running and queued siblings only for failures without job-level tolerance.
`strategy.job-total`, `strategy.job-index` and `strategy.max-parallel` describe
the original strategy. Job `timeout-minutes` is evaluated per instance, defaults
to 360 minutes and has no additional policy ceiling. `timeout_seconds` is the
outer invocation bound and can therefore expire across several serial legs.

The final stdout record retains `kind: "result"`, `state`, `outputs`, `steps`
and `artifacts`, and adds `continue_on_error` and `artifact_retention`. The raw
failed state is preserved when tolerated: the server must use
`continue_on_error: true` to apply successful dependency/run semantics, rather
than treating the worker's nonzero exit alone as authoritative. `steps` includes
skipped steps. Matrix outputs merge in completion order; a later explicitly
empty output replaces an earlier value, while a reference to an un-emitted step
output contributes no key. Secret-bearing outputs are omitted before merging.

Artifacts use the upstream v4 protocol with a per-worker capability check.
Bearer tokens are required for metadata requests; returned blob URLs carry a
separate random capability. The filesystem is confined using `os.Root`.
Archives are transferred back through `HostChannel` into the existing service
artifact store and restored before subsequent jobs, including jobs on other
hosts. A duplicate create conflicts unless the action first calls
`DeleteArtifact`, as `upload-artifact@v4` does for `overwrite: true`. Deletes
use the confined filesystem root and remove retention metadata. The result's
`artifact_retention` maps archive-relative paths (for example
`41/proof/proof.zip`) to requested days; zero means platform policy default.
The v4 wire protocol supplies an expiry timestamp, from which requested whole
days are recovered. The server owns persistence, replacement/deletion of
previously published archives and expiry enforcement. `artifacts` is the final
archive manifest, including restored archives that still exist.

Host jobs (the selected platform is `-self-hosted`, without `container:`) always
run `actions/upload-artifact@v4` / `@v4.*` and
`actions/download-artifact@v4` / `@v4.*` through built-in worker helpers; these
two actions need no Node installation or remote action fetch. Other versions
and container jobs retain act's real actions. An exact-match act patch rewrites
steps at the step factory, including composite and reusable-workflow steps,
while keeping step conditions, IDs, environment, timeouts and outputs. Helpers
invoke this worker executable using explicit `sh` (Unix) or hidden `cmd`
(Windows). Inputs travel through step environment variables and use act's
normal expression evaluation, without interpolating user values into shell code.

The upload helper supports `name` (default `artifact`), multiline `path` entries
(files/directories/globs, including `**` and `!` exclusions),
`if-no-files-found` (`warn`/`error`/`ignore`), `retention-days`,
`compression-level` (0–9; default 6), `overwrite` and `include-hidden-files`.
Hidden files/directories are excluded by default unless explicitly selected.
ZIP names are relative to the common search root; executable mode bits survive
download on Unix. Outputs are `artifact-id`, `artifact-digest` (ZIP SHA-256)
and `artifact-url` (empty for this local backend).

The download helper supports `name`, `pattern`, `path` (default
`GITHUB_WORKSPACE`, with `~` expansion), and `merge-multiple`. Without a name,
artifacts go into separate artifact-name directories unless merging is enabled.
It emits `download-path`. Cross-run `github-token`, `repository`, and `run-id`
inputs fail explicitly because the local backend only serves the current run.
Both helpers use the same authenticated v4 protocol and signed blob URLs as the
Node actions; server-side confinement, retention, overwrite and run scoping
remain authoritative. Transfers and input errors fail the step and appear in
its masked log.

`actions/cache` uses act's `pkg/artifactcache` service, bound to `127.0.0.1` on
an ephemeral port. `cache_dir` selects persistent storage; the default is
`os.UserCacheDir()/ferry/actions-cache`. `ACTIONS_CACHE_URL` and a runtime token
are supplied even without artifacts. The service uses its private URL
capability and the v1 cache API supported by `cache@v4`; the v2 service flag is
disabled for this backend. Cache contents survive separate worker invocations.
Container reachability of this loopback service is not verified on this machine.

Remote actions and reusable workflows use act's go-git fetching. Private
repositories receive `secrets.GITHUB_TOKEN` as the Git authentication token;
the same value populates `github.token`. Fetching requires connectivity to the
Git server and failures propagate. No public network is required by default
tests: local actions, reusable workflows and an authenticated loopback Git
server exercise those paths. Opt-in remote tests fetch upstream action refs.

Tests:

```sh
cd workers/actions
# In the Windows worktree sandbox, keep tool/test state within the worktree.
worker_root=$(cd ../.. && pwd -W)
mkdir -p "$worker_root/.gocache" "$worker_root/.tmp"
export GOCACHE="$worker_root/.gocache"
export TMP="$worker_root/.tmp" TEMP="$worker_root/.tmp" GOTMPDIR="$worker_root/.tmp"
export GOMODCACHE="$worker_root/.tmp/gomodcache" GOPATH="$worker_root/.tmp/gopath"
export LOCALAPPDATA="$worker_root/.tmp/user-cache" XDG_CACHE_HOME="$worker_root/.tmp/user-cache"
bash ./go.sh test -count=1 ./...
GOOS=linux GOARCH=amd64 bash ./go.sh vet ./...
GOOS=darwin GOARCH=arm64 bash ./go.sh vet ./...
ACTIONS_TEST_REMOTE=1 bash go.sh test -run TestArtifactActions -count=1
ACTIONS_TEST_REMOTE=1 ACTIONS_TEST_DOCKER_IMAGE=<local-node-image> bash go.sh test -run TestArtifactActions -count=1
```

The remote test fetches checkout; artifact steps use the built-ins in host
mode. The container variant fetches the upstream artifact actions and requires a
local Docker daemon and an image with Git, Node, CA certificates and `sh`.

Worker builds and tests use `go.sh` to apply the pinned act PTY synchronization
fix in a disposable source tree. The pinned module is downloaded on cache
misses; the shared Go module cache is never modified. A stable relative module
replacement keeps independently staged binaries reproducible.
The patches make cross-thread PTY/cancellation flags atomic and expose the
strategy context to the worker. Reusable workflows enter act's planner, whose
matrix scheduling seam is patched to snapshot models, run isolated legs and
publish their results under a lock. That patch is larger than the host fixes;
local reusable-workflow tests cover concurrent output merging and tolerated
failures, and exact matching deliberately blocks an unreviewed engine upgrade.
Source matching fails closed if upstream changes. Run `bash go.sh test -race ./...`
to include the native execution race regression.

## Windows host mode

Pass a native absolute repository `workdir` and map the selected runner label
to `-self-hosted`, for example `{"self-hosted":"-self-hosted"}`. Paths and the
sibling action-cache directory may contain spaces. The launcher must start the
worker executable hidden as well; hiding its children cannot hide a console
allocated by the server/runner when it launches the worker. This worker does
not provision Git, Node, PowerShell 7 or Git Bash. The service account's `PATH`
must contain the tools required by the workflow, including Windows PowerShell's
`System32/WindowsPowerShell/v1.0` directory. Use the matching Windows `.exe`
from the build output; the request and stdout JSON protocol are unchanged.

For an unspecified `run:` shell, pinned act already searches for `pwsh`, then
falls back to `powershell` on Windows hosts. Stock Windows 10/11 therefore uses
Windows PowerShell 5.1. Explicit `shell: pwsh` requires PowerShell 7; it does not
fall back. Explicit `powershell`, `cmd`, and `bash` retain their runner semantics;
`bash` requires Git Bash on `PATH`. Unix shell selection is unchanged.
Windows host PowerShell invocations use `-NoLogo -NoProfile -NonInteractive
-ExecutionPolicy Unrestricted`, with act's existing terminating-error and
native-exit-code handling. The process-scoped execution policy permits scripts
on a stock install; it does not change the machine's policy. Git Bash script
paths are quoted on Windows, including when the action cache contains spaces.

The Windows patches are generated by `internal/actpatch/windows_host.go` with
exact source matching. They preserve `CREATE_NEW_PROCESS_GROUP` and add
`CREATE_NO_WINDOW` plus `HideWindow` in act's `container.getSysProcAttr`.
`HostEnvironment.exec` is the single process-creation point used by host
`Exec`/`ExecWithCmdLine`: run steps, composite run steps and JavaScript actions
(including pre/post hooks) all reach it. Both attribute assignments, including
the PTY setup path, use this helper; Windows PTYs are unsupported upstream.
The runtime worker itself has no `exec.Command`/`os.StartProcess` calls.
Git metadata/action fetching use go-git, rather than a Git CLI subprocess.
An audit of act and the worker's dependency graph also found launch helpers for
`gh auth token`, Docker credentials/SSH/compression, dotenv execution,
shellwords backtick expansion and actionlint external linters. These are not
called by the Windows host execution path (shellwords backticks default off).
Workflow/action code can explicitly launch its own processes; such code must
use its runtime's hidden-process option if it creates new consoles or windows.

Windows PowerShell 5.1's output encoding was a real masking problem: a Unicode
secret printed through its default code page reached the UTF-8 logger with a
corrupted prefix and an unmasked ASCII suffix. Host PowerShell scripts now set
console/native-pipeline output to UTF-8 and carry a UTF-8 BOM so 5.1 also reads
Unicode script literals correctly. No change to the worker's redaction rules
was needed. Configured secrets, dynamic masks and multiline secret lines are
still removed from job outputs and masked in structured logs.

Act already handles CRLF and UTF-8 BOM command files. It did not decode the
UTF-16 files produced by PowerShell 5.1's `>>`/`Out-File`. On Windows, command
files (`GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`, and action state) now decode BOM-marked
UTF-16 LE/BE before parsing or secret checks. UTF-8 with/without BOM remains
supported; Unix readers are unchanged. Use a consistent encoding within each
file; mixing UTF-8 and UTF-16 in a single append stream is not supported.

The native Windows tests exercise the actual PowerShell child and require
`GetConsoleWindow()` to return zero; they also restrict `PATH` to test the 5.1
fallback, check explicit missing-pwsh failure, run PowerShell/cmd/Git Bash with
paths containing spaces, and verify CRLF, UTF-16, UTF-8, multiline values,
environment propagation and secret masking. Existing Unix scripts/assertions
remain intact; Windows variants use native PowerShell syntax. Remote artifact
tests retain their existing opt-in requirements.

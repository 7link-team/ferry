# Actions pipelines implementation plan

Goal: use GitHub Actions YAML with an executable React Flow editor, right-hand job/step configuration, remote execution logs/cancellation, and environment/group/application/tag management. User approved implementation on 2026-10-27. Aliyun YAML conversion is deferred.

## Architecture and boundaries
- YAML is workflow source of truth; preserve unsupported fields and comments in visual edits. Layout and workspace resource bindings live outside YAML. Do not claim complete GitHub compatibility.
- Reuse an act execution kernel (original versus Gitea fork selected from fixed-version feasibility tests). No embedded Git hosting and no mandatory Gitea server. Existing Ferry host connections remain authoritative; every operation uses HostChannel.
- Jobs have execution targets, dependencies and independent workspaces. Steps within each job share its target/workspace. Preserve existing sequential JSON pipelines and histories.
- Reuse existing workspace applications and tags. Groups/environment assignments are workspace-scoped; no permission or variable boundary inferred solely from a UI tag. One pipeline binds multiple environments; execution requires selecting one, as confirmed by the user.
- UI extends ResourceForm DSL with an Actions editor; use existing shared list/forms for classification resources. Retain existing theme, permission checks, optimistic concurrency, activity audit and cross-workspace checks.

## Execution / testing
Execute directly on feat/actions-pipelines, reuse current build cache because local disk space is constrained. Durable state is maintained in root-level MEMORY.md; long-term decisions are recorded in ADR.md. No new ledger files. Finish with independent read-only code review.

### 1. Engine feasibility
- [x] Compare pinned original act and Gitea fork using same actual workflows: dependency/outputs, matrix, composite/JavaScript/container actions, logs, timeout and cancellation.
- [x] Record chosen version and concrete compatibility limits; inspect adapter API for job-level dispatch and context injection. No duplicate YAML expression evaluator.

### 2. Execution transport
- [x] Write failing live test proving logs arrive before exit and cancel/timeout actually stop command descendants.
- [x] Extend protocol and HostChannel with streaming execution, stdout/stderr frames, cancellation and completion; implement runner and SSH with bounded buffers.
- [x] Preserve existing exec API and old runner compatibility; test UTF-8/chunk boundaries, disconnect cleanup and secrets before persistence.

### 3. Actions worker and server orchestration
- [x] Add pinned execution worker adapter with structured events, dependency context, secrets, lifecycle and deterministic cleanup.
- [x] Add YAML validation and versioned pipeline definition metadata; add per-job execution state and snapshots; map runs-on to workspace executor bindings.
- [x] Run a real selected private repository through Ferry->runner/SSH->act; verify job dependencies/outputs, live logs, artifact handling, failures and cancellation.

### 4. Classification
- [x] Reuse applications and tags; add workspace pipeline groups and environment configuration with confirmed multi-environment runtime selection.
- [x] Add CRUD, membership/filtering, delete reference protection, transactional validation, audit and transfer coverage.

### 5. Visual editor
- [x] Add React Flow via shared DSL editor; job nodes/dependencies, job step rows, right configuration panel for executor/commands/environment/timeouts.
- [x] YAML edit/import/export and visual edit share one document; preserve unknown fields or explicitly restrict visual edits without losing source.
- [ ] Validate cycles/dangling dependencies and required configs; browser test edit/save/reopen/run, errors, mobile and keyboard access.

### 6. Acceptance
- [x] npm test/typecheck/build; cargo check/test/fmt; Go worker tests/build; backend real runner+SSH and browser through real action execution.
- [x] Check permissions/tenant isolation, old pipeline regressions, history/log recovery, timeout/cancel cleanup, and platform dependencies.
- [ ] Review original user requirements and documented compatibility boundaries; commit source only. After acceptance, publish the next alpha as explicitly authorized; verify latest remote version before selecting its tag.

## Verified implementation checkpoints (2026-10-27)
- Original act v0.2.89 selected after actual dependency/matrix/composite/JS/container comparison with the configured Gitea fork. Both can execute these fixtures; no universal compatibility ranking.
- Streaming runner and SSH, real cancellation/descendant checks, bounded log buffers, graceful cleanup; worker masks both message and structured fields.
- YAML envelope, immutable resolved-job snapshots, job states/encrypted outputs, cross-host dependency dispatch and artifact restore/archive are implemented.
- Real runner -> private Git -> build -> upload-artifact v4 -> SSH -> download-artifact v4 passed with the existing regression suite (153 checks), including matrix failure status.
- Real Chromium canvas/panels/YAML/save/reopen/execute/logs/mobile passed. Existing platform OAuth/private repository browser acceptance now verifies actual Actions output download and exact commit content.
- Worker artifact tests passed both native and local Docker image execution. Network Docker Hub pull was unavailable during one attempt; the container proof uses the locally built Linux fixture with Node/Git/CA/sh.
- Environment classification semantics are confirmed in the implementation section below.

## Initial compatibility boundaries
This is a self-hosted execution service, not a complete GitHub platform emulator.
The YAML source is retained; manual execution and existing platform scheduling
own run creation. Importing `on: push` does not install a provider webhook.
GitHub permissions do not mint or reduce the scope of a user-provided PAT;
repository authentication continues to use the selected workspace connection.
Static runs-on labels map to the bound host/image. Dynamic host selection,
reusable workflow jobs, concurrency policies, protected GitHub environments,
and Actions cache service are not part of this worker integration and must be
rejected explicitly rather than silently skipped. Composite and JS actions
are supported; Docker actions require container execution. Archive names are
immutable within a run, and retention follows existing platform history policy.

Matrix branches execute serially within each worker because the pinned engine
shares mutable step models. The build applies two exact-match upstream fixes
in a disposable module copy: atomic PTY stop synchronization and serial matrix
`fail-fast` (default true; explicit false continues later branches). The original
YAML is preserved. Native worker tests pass with Go race detection enabled.

## Recovery and worker review checkpoint
- Interrupted-run recovery now finalizes job rows with the run and steps in one
  transaction. Real forced restart passes on SQLite (154 checks), MySQL and
  PostgreSQL (152 each), including existing pipeline regression coverage.
- Actions transfer/layout/binding preservation and delete protection pass on
  SQLite (80 checks), MySQL and PostgreSQL (78 each).
- Independent worker review found and verified fixes for clean-cache builds,
  dynamic mask propagation through outputs, and random build metadata. Two
  independently staged worker binaries had identical SHA-256 hashes.
- Concurrent workers now use private Action checkout caches. Real parallel
  artifact worker processes pass; the Docker artifact suite also passes with
  race detection. Fresh remote downloads need a longer test budget than cached
  runs; no functional assertions were relaxed.
- Classification, complete product acceptance and desktop installer delivery
  remain unfinished. No release, tag or push was performed.

## Confirmed classification implementation (2026-10-27)
User confirmed one pipeline supports multiple environments, selected at trigger.
- Workspace pipeline contexts hold groups and environments; environment has a default host/group. Reuse existing host validation, resource listing, version guard, audit and workspace mutation lock.
- Pipeline group/application are optional single references; tags and environments are many-to-many references. Applications are existing deployments. Missing fields preserve old clients' associations.
- Bound environments require an explicit environment at trigger. Unbound legacy pipelines keep their current default execution behavior. Selected environment overrides pipeline default executor; explicit job bindings remain more specific. Environment variables override global/build/pipeline variables; ordinary and secret values reuse the existing encrypted vault and immutable run snapshots.
- Run records retain selected environment ID/name and actual resolved hosts. Reference deletion is rejected while definitions use resources; historical run snapshots do not depend on mutable environment names.
- Reuse shared ResourceForm/ResourceTable and existing trigger action dialog support. Add environment/group management alongside pipeline controls, existing tags/apps remain shared resources. Test cross-workspace IDs, stale writes, omission-vs-empty update, filters, imports, variable deletion, real execution on distinct targets, selected-environment snapshots and cron trigger parameters.
- New migration: context/link tables, pipeline group/application references, run environment snapshot columns. No deploy environment model, protected-environment policy, auto Git webhooks, or new app registry.

## Final acceptance and review fixes (2026-10-28)
- Whole-change independent review found multiline secret masking, transitive dependency status, and restricted-editor permission regressions. Focused tests reproduced each before fixes. Worker race suite passes; server now sends ancestor outcomes and redacts constituent secret lines. Optional classification and job executor controls respect available read permissions.
- Real API classification: 29 checks on SQLite, MySQL and PostgreSQL, including three-level chains, multiline secret logs, selected environment snapshots, cron reference protection and legacy defaults. Classification CSV roundtrip and resource templates: SQLite85/MySQL83/PostgreSQL83 checks.
- Full legacy/Actions runner and SSH regression:154 checks. Frontend73 tests and Rust131 tests pass (one existing live-only test separately exercised earlier). Actual browser verifies create/edit/save, YAML comments, keyboard movement, multi-environment selection and cancellation, real worker execution and mobile layout.
- Local macOS debug app contains four workers. Its bundled server passes lifecycle/setup/persistence/graceful shutdown and29 actual execution checks. This is local package validation; release platform builds and signatures are separately gated in CI.
- Updater signature harness fixed macOS inherited nonblocking sockets; valid signature/tamper/version-binding test passes. Upgrade test explicitly checks additive classification/Actions defaults while retaining equality of all existing resource fields; built-in permission migration includes the two new resources without changing custom roles.
- Release candidate version:0.1.0-alpha.11. Publication remains pending CI and final delivery verification.

- The alpha.10 tag was not published: CI found two stale acceptance fixtures. Backend reference validation now supplies the current version while retaining separate409 stale-write coverage;119 checks pass on each database. The build browser uses the new Git source form and edits a seeded legacy JSON pipeline, preserving its target reset/validation/live logs/variables/history/terminal assertions (4 groups pass). Actions creation remains covered by the separate canvas browser suite. Preserve the alpha.10 tag; publish the corrected alpha.11 candidate.

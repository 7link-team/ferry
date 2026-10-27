# Connected repositories in builds — implementation plan

Goal: a user with a configured platform connection selects a repository visible to that connection in the build form, then a real build host checks out that repository and builds its contents.

Architecture: extend the shared form DSL and reuse repository-auth list/browse and repositories.create_in. Pipeline saves materialize/reuse the selected repository atomically with the pipeline; no new connection registry or database schema. Add an explicit checkout step to the existing sequential executor, using the same temporary encrypted-credential delivery and cleanup as command steps.

Spec: user's explicit 2026-10-27 instruction and this design. Execute directly in the current workspace; previous source commit50caed6 is committed, only local .claude state remains modified. Scope is tightly coupled and does not benefit from separate implementers.

## Design / approaches
- Selected: select connected platform and its paginated visible repository list in the build form. Reuse exact existing workspace repository binding (provider/URL/credential/defaultbranch), otherwise create it within pipeline transaction with repositories.create permission. No manual intermediate repository creation step.
- Existing saved-repository dropdown remains for compatibility and genericGit; it alone does not satisfy the user's requested connection-based selection.
- Rejected: duplicate per-pipeline OAuth secrets or silently add checkout to existing pipelines. Existing command-based clones remain unchanged.
- New connected-source selection adds an explicit `{name:"拉取代码",checkout:true}` step to the definition. Existing saved sources can also add the step explicitly. Other build steps remain existing commands/templates. This is not a new graph editor.
- Checkout uses safe shell quoting of the run's repository snapshot, fetches requested commit when supplied or selects the remote branch, and detaches at the resolved commit. Clone into the run workdir only when empty; refuse destructive cleanup. Logs/state/timeout/cancel and credential cleanup reuse existing step executor. Failures stop later steps. No token in argv/URLs/logs.
- Pipeline source omitted on an old-client edit preserves existing repository binding behavior. Creation/update transaction rolls back newly materialized repositories on validation/stale failure. Imported pipelines still accept repository_id and old definitions unchanged.

## Task 1 — regression evidence and checkout step
Files: scripts/pipeline-git-http.py, scripts/test-pipelines.py, crates/server/src/pipelines.rs, crates/server/src/pipeline_runs.rs.
- [x] Extend owned smartHTTP Git fixture with authenticated provider user/repository-list endpoints; keep real Git HTTP Basic auth mandatory.
- [x] Add acceptance selecting provider connection, creating build from selected repository, checkout + running checked-in build.sh; run old binary and capture expected missing-source/checkout failure.
- [x] Add optional checkout flag to Step with command/checkout mutual exclusion, require repository when selected. Implement generated clone/checkout script through normal executor.
- [x] Real runner and SSH: verify checked-out commit, compiled artifact content, missing branch/invalid credential fails before build, existing tests still pass and credential files/logs/process args remain protected.

## Task 2 — atomic source binding / shared build form
Files: crates/server/src/pipelines.rs, crates/server/src/repositories.rs, src/components/PipelineRepository.tsx, src/components/ResourceForm.tsx, src/components/dsl.ts, src/resources.tsx; scripts/test-repository-auth-ui.mjs.
- [x] Add optional repository_selection payload; revalidate via existing repository logic, reuse exact binding or create in pipeline transaction; stale edit must not create orphan repositories.
- [x] Add shared-form pipeline-repository field. Modes: saved repository, connected platform's visible repositories, or existing no-source commands. Paginate; reset selection on connection change; preserve user-entered pipeline name/commands.
- [x] Add checkout step to newly selected source without duplicating it or replacing existing steps; display selection/validation errors and selected URL. Require intended permissions and refuse cross-workspace credentials.
- [x] Browser acceptance selects configured connection/repository, saves build, then API/live runner verifies same repository and actual execution. No repository/auth tabs.

## Review focus
- stale callbacks after connection change must not bind the previous platform's repository;
- credentials have expired/been revoked: refuse actual clone, never report success or run build;
- remote branch/commit contains shell-sensitive characters: quote as data;
- reusable workdir is nonempty: fail without deleting user files;
- old saved pipelines with manual Git steps: no implicit behavior change.

## Acceptance / delivery
- [x] npm test/typecheck/build, cargo check/test, realHTTP source binding/rollback/permissions, real Chromium selection, smartHTTP private Git checkout on runner+SSH.
- [x] Review original requested link→visible repository→pull→build chain, document tested vs real-provider-account limitations, commit source with no build artifacts. No release/tag needed to prove this source change; prioralpha9 public package remains unchanged.

## Acceptance record — 2026-10-27

- RED: old service ignored repository_selection and rejected checkout; old browser form had no connected-source selector.
- Actual protected Git with all five provider adapters: Runner and SSH checkout, checked-in build script, exact commit artifact, failed branch skips build, expired/invalid Git credential skips build, explicit commit, shell-sensitive branch data, nonempty directory preservation, source transaction rollback and stale/cross-workspace rejection. Existing cancellation, variables, redaction, argv, credential cleanup and graceful-shutdown checks retained: 142 passed, 0 failed.
- Frontend: 59 tests passed; production build/typecheck passed. Rust workspace: 118 passed, 1 existing ignored. New definition serialization/validation test passes with absent optional timeout omitted.
- Independent read-only review found uneditable source-name failures. Connected-source mode now offers only platforms (generic Git uses saved repositories); generated local display names are limited to 60 Unicode characters without changing the remote URL. No remaining Critical/Important review findings.
- Final embedded application browser acceptance passed: existing Gitee connection→repository selection→Runner protected Git→build.sh→exactcommitartifact, preserving draft commands/name, with a 75-character provider repository name. Original OAuth/COOP, PAT, reauthorization, generic Git, and mobile checks also passed.
- Real external provider accounts and installed WebView remain unverified. No release, tag, image, or public push performed.

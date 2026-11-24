# Task State

## Objectives / Confirmed scope
1. Make production Codeup workflow116765975084269568 (workspace111019986952126464) complete PDF + enterprise chat ferry delivery, then Monday09:00 workspace-time schedule. Preserve UTC persistence/workspace business timezone.
2. User follow-ups: fix collapsed execution-history height, enrich execution logs, correct batch-prompt help; independent Workspace information menu/form for name/avatar/timezone.
3. Complete native v0.10.14-alpha publication, preserving immutable tags and signing limitations.

## Current Phase / Current Step
TESTING / CLOUD: follow-ups pushed in4897b632da094d904f17cffa72e38b319d2ebd60; two CI acceptance adaptations locally verified and SSH-pushed as2be1d0b; fresh CI45322550654 pending. No production writes, schedule enabling or deployment occurred. .14 tag already exists at previous accepted source; new source is outside that tag.

## Production status / Blocker
- User repaired provider token. Latest actual execution118539555044327424 (#3) passed period+commits, then failed expression /nodes/3/params/prompt before analyze starts. Real sandbox reproduces batch $json being absent. Saved prompt uses JSON.stringify($json). Recommend plain instructions: remove leading '=' and interpolation; compose automatically appends every input JSON once. Explicit $input.all().map(item=>item.json) is also verified if needed. No new execution after#3 at last server check.
- Initial #2 failure118383005512237056 was proven401 INVALID_API_KEY in exact ASPEED Codex thread logs, not timezone/Runner authentication. Error preservation fix d27e621 already pushed and tested.
- Draft still v2 with Shanghai text/settings and old company-mobile target. Must use workspace clock and requested enterprise channel; verify actual PDF/delivery before schedule.
- No normal Ferry API login/MCP connection available. Browser tools removed after automatic URL-validation refusals. Never bypass with SQL writes, forged sessions or borrowed cookies. SSH readonly diagnostics authorized; SSH session38958 explicitly closed. Do not repeat passwords. User is editing/rerunning personally; no repeated request to merely reopen Chrome.

## Implemented source / Decisions
- Shared execution table flex/min-height chain restored; vertical scrolling keeps pagination reachable on short phones. Existing table/filter/pagination/loading/error behavior reused.
- Existing NodeLog gains native/API agent lifecycle metadata: input/output counts, sizes, host/model, duration/outcome/error identity, native token usage, CLI10s waiting progress. Start metadata flushed before dispatch. No raw prompt/tool-body/credential logging. Frontend translates known phase labels in six languages, preserves historical/native/malformed/future records.
- All6 agent prompt-help translations corrected: previous help explicitly recommended invalid $json.since for this batch node. Automatic input contract unchanged; no implicit current-item fallback or production graph rewrite.
- Workspace information is visible independently in maintenance, same workspace-settings URL. Shared form: name/avatar/timezone; switcher shows saved workspace avatar and refreshed name. Name reuses creation validation (trim/nonempty<=60Unicode), optional fields preserve old clients. Avatar uses existing validated account JSON/PNG and neutral initials/emoji/upload controls, no person gender controls. Scoped settings.update, optimistic version, transaction/audit and timezone schedule recomputation preserved.
- Add only nullable workspaces.avatar using existing long_text helper (MySQLLONGTEXT, PG/SQLiteTEXT), early additive migration before old complete-entity backfills, idempotent; down refuses destructive data loss. No production migration. Existing entity/fixtures updated only for new field. /me authorized workspace summaries include parsed avatar. Audit records avatar_changed, not image body.
- Generated platform-tools catalogue regenerated from source; workspace API docs describe fields/version/permissions/compatibility. No manual generated-file edits. ADR covers design/review/DDL/rollback.

## Actual verification / Evidence
PASS: latest affected frontend75tests (then58 rerun after menu assertions), other related resource/data-view checks19; final TypeScript typecheck; production Web build and server build; cargo check --workspace --all-targets; fmt/diff checks; generator --check320operations.
PASS: account/workspace8 tests; migrations40tests; final MySQL long_text refinement retested via workspace_avatar migration; CLI/API persisted lifecycle log test; four timeout/cancellation/schema/API-no-replay regressions. Real sandbox exact old-batch-error, plain-input-single-injection and explicit batch expression test passed.
PASS: complete real local workflow-editor browser suite including actual DB/sandbox execution, immutable versions, node forms, mobile editing. New history layout/pagination screenshots at1440/820/390/320. New Workspace information saved name+emoji+timezone, /me avatar readback, independent nav, zh/en/ms1440/820/320 layouts. Screens inspected: Chinese desktop and Malayphone form, shortphone pagination. No whole-page horizontal overflow. Local synthetic fixtures only; not a real weekly business report.
Evidence outside Git: LOCALAPPDATA/ferry-verification/workflow-116765975084269568 (logs); final browser screenshots and ui-acceptance.json in LOCALAPPDATA/Temp/workflow-editor-XLBnuS. Prior workflow-editor-IFAztB initial height fix. No browser/test process remains.
Self-review: scope/reuse/architecture/complexity/regression/commit hygiene/durable state PASS for local fixes. Production end-to-end acceptance NOT READY. Publication NOT READY.

## Publication status / Next actions
- Main source now4897b63; previous checkpoint3ad5ef8; accepted product d27e621 and launcher CI order94e6033. Main45226851695 + deployment45226851603 SUCCESS.
- v0.10.14-alpha TAG CREATED and SSH-pushed at d27e6219f9c69e626dd1dc4812be594e8e396cb8, exact accepted source. Never move it. Source manifests still .14; later feature changes need a later version before release.
- Exact-tag root45313356617 completed FAILURE, sole mobile/ios job138174186856 FAILED before first case. Image45313356564 completed SUCCESS; rerun attempt remains1. No .14 publication success claim.
- Actual27,152,377byte diagnostic ZIP verified: LOCALAPPDATA/ferry-verification/v0.10.14-alpha/tag-ios-diagnostics.zip. results.json: cases[], uiAcceptancePENDING, xcrun simctl launch ETIMEDOUT at60s; subsequent terminate and uninstall also time out. Not an app assertion failure. Latest main standalone mobile45181740341 had passed. Recommended original failed-job rerun on a fresh CI host; user question pending with job URL. Re-run menu may require whole root run completion; do not tell user to cancel running release jobs.
- No GitHub authenticated CLI/browser available to rerun. Public API/artifact downloads usable; no token/cookie bypass. Signing remains user-deferred: AndroiddebugAPK, unsignedIPA, no direct iPhone installation or cross-CI upgrade-signing claim.

## Next concrete actions
1. Read exact source4897b63 main CI45318798793 (queued at last check) and deployment acceptance45318798764 (in progress). Source/docs/tests committed and SSH-pushed; no tag movement. Correct any verified failure before a later release version/tag.
2. Read exact-tag release/image statuses; after user rerun, verify iOS/native/mobile/device artifacts, Release checksums/source and both image architectures. .12 evidence folder contains reusable verify/download scripts; .14 folder diagnostic downloader uses8verified ranges.
3. Production workflow needs user normal API authorization or their direct prompt edit/rerun. Then inspect analyze/PDF/notification, repair workspace-timezone/channel via normal authorized API, publish/schedule only after actual success. Do not claim local sample or tests equal real report delivery.

## Constraints / Last checkpoint
Direct execution, no new agents/worktrees. LF, no blind staging, no build/temporary artifacts in Git. Preserve users' explicit approval modes/budget/timezone decisions. Prior native logo/HTTPS/launcher changes and evidence remain in Git/ADR; earlier .12 verified31assets, .13failed immutable.
2026-11-24: all requested local UI/workspace/log fixes accepted; production workflow and .14 release remain independently unfinished.

## 2026-11-24 CI follow-up checkpoint
- Main45318798793: upgrade check rejects additive workspace avatar:null; existing test already normalizes additive account avatar and workspace timezone. Add an explicit avatar-is-null assertion before full historical resource equality, preserving every old-field assertion. No product/API change.
- Workspace browser assertion included new initials in whole-menu innerText. Now separately assert exact workspace name and exact owner role, retaining aria-checked and all downstream operations. Complete local test-workspace-admin-ui PASS.
- Deployment acceptance45318798764 SUCCESS. Main still running with these two failures; tag iOS rerun not started.
- Local upgrade first attempted with old-named release0101 binary: HTTP404 at legacy hosts route, proves unsuitable artifact, not a migration result. Exported immutable v0.1.0-alpha.1 source outside repo (no worktree) and compiling actual old server; next run old-source binary against current embedded server. PostgreSQL/MySQL gates remain cloud verification.
- Next: finish actual alpha1 build, run upgrade regression, checkpoint/commit exact two test adaptations, SSH-push, track new CI. Production workflow and release still NOT READY.

## CI adaptation verification
PASS: actual v0.1.0-alpha.1 exported source compiled without changes; complete SQLite upgrade using that binary and current server passed, preserving account/session, grants, credentials, repository, host, pipeline/run, backups, restart behavior, and asserting workspace avatar NULL. POSIX successful-run comparison explicitly skipped on Windows; cloud Linux and three-database verification still required.
PASS: full workspace-admin real Chromium acceptance (including exact workspace name/owner role with initials avatar, invite rotation/join, custom roles and member operations), LF/diff hygiene. Only two acceptance scripts changed; production code unchanged. Self-review scope/reuse/architecture/complexity/regression PASS for test adaptations. Overall production/report/release acceptance remains NOT READY.
Committed and SSH-pushed2be1d0b. Fresh Build/release45322550654 pending. Next: inspect exact-source cloud backends/e2e-build-ui and remaining gates; .14 iOS rerun still awaits user action with original request pending.

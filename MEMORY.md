# Task State
## Task / Objective
Complete user-authorized workspace-wide business timezone, workflow diagnostics and v0.10.8-alpha release with APK/unsigned IPA assets. Keep UTC backend/API/SQL instants; browser/UI uses confirmed workspace business zone. Original request also concerns the existing Codeup weekly workflow, whose live configuration/report delivery has NOT been changed or accepted during this task.
## Current Phase
REVIEW / RELEASE DELIVERY
## Current Step
Final full Rust workspace test PASS (server 1264 passed, 0 failed, 14 ignored), followed by cargo check --workspace PASS. Latest Runner suite PASS (112 + 5). Fresh embedded build, real HTTP 32 checks and node inspector acceptance all PASS. Final cargo fmt, generated catalog, mobile fixtures 7 and manifest tests 4 PASS; YAML parsed. Reviewing/staging task deliverables for SSH push, then exact-source CI gates before immutable v0.10.8-alpha tag. No release published yet.
## Confirmed Requirements
- One editable workspace IANA timezone. New UI creation suggests browser/system zone and requires an editable selection; detection failure requires a manual choice.
- User explicitly chose legacy transition: existing workspaces stay NULL/unconfirmed and preserve each historical behavior until administrator explicitly saves timezone. Never guess/backfill a zone or silently convert historical instants.
- Confirmed workspace controls business calendars, date inputs/display, statistics, schedules, Agent CLI and generic command/new-terminal process defaults. No host OS/global Runner pin or existing-terminal mutation. User commands can explicitly set their own TZ.
- Older API/template clients omitting the field remain explicitly unconfirmed; both new UI creation entry points require timezone. Confirmed timezone cannot be cleared. settings.update + fresh authorization/version/lock/audit protect scoped configuration.
- Model/API context remains per application conversation/Agent. No monetary admission caps; resource ceilings remain null/unlimited by default; preserve auth/protocol/liveness. General conversation policy stays maintainable in platform settings; clock JSON is technical runtime metadata.
## Implemented Work
- Additive nullable workspaces.timezone migration after Initial (before old complete-entity backfills). Automatic down refuses configuration loss. Shared scoped singleton GET/PATCH workspace-settings/1 and platform write path; /me exposes timezone; scheduler recompute atomic with save. Readonly body id/name never choose tenant.
- Workflow Start freezes zone; existing node snapshots freeze actual attempt clock; report and API/CLI Agent use persisted clock. Old Start without field stays legacy. Pre-activation public errors persist in encrypted context with lease fencing, authorized GET exposes only localized diagnostic, UI shows it.
- MCP help fixes broken batch $json example, documents items/upstream/selections and numeric JSON Pointer preview paths; explicit failure guidance tells model how to recover.
- Cron/insight schedules, insight filters and SQL calendar buckets, model/conversation clocks. Calendar CASE now uses occupied UTC days, not min..max empty span; metadata filter uses same cohort calendar. NULL retains original UTC SQL path and statement count.
- Frontend Context has no global mutable zone, covers HostWorkspace early branch and isolates platform pages. Date-only calendar labels remain distinct from instants. DST gap/overlap input rejects silent adjustment and allows explicit-offset ISO.
- Sparse heatmap preserves actual facts with compressed empty-week markers/full dates and extended signed-year keys. Bare workspace-settings route is id1 readonly singleton, so Save/Cancel remain usable.
- Additive supports_process_timezone capability and optional Exec/stream/PTY timezone, including cluster owner recheck. Child-only TZ; SSH positive acknowledgement required. with_timezone is a synchronous constructor boxing work BEFORE task-local scope (normal-stack overflow fixed; scope40bytes, dispatch_inner4936bytes, outer8bytes).
- Mobile reusable CI now produces versioned APK/unsigned IPA/simulator assets, validates identity/version/ABI/no sidecars/hashes, merges dist-mobile-* into Release before manifests/checksums/publish. Main native-input changes preflight standalone mobile CI; old v0.10.7 tag remains immutable.
## Verification
- Frontend final: npm test -- --maxWorkers=3 =>128files/971PASS/1existingSkip; typecheck PASS; latest npm production build PASS. Default concurrency hit existing PowerShell5s test timeout; unchanged isolated test and full3worker run PASS (no assertions/timeouts weakened).
- Isolated built-preview Playwright320/900/1440 + msMYPhone PASS: actual Create zone detection/edit, Save->readonly singleton->Edit->Cancel restored value, sparse1970/+100000 bounded DOM/gap keyboard tooltip/no pageerror/no page overflow. Root inspected representative settings/create/heatmap screenshots. Evidence C:/Users/FullStack/AppData/Local/Temp/ferry-timezone-qa/.
- Real source-fresh HTTP: scripts/test-workspace-admin.py with FERRY_TEST_WORKFLOW_TIMEZONE=1 =>32checksPASS (config/identity/authorization/version/frozen workflow after live zone change/pre-activation exact diagnostic/no invented node). Default28 checks run on all3 SQL backends in CI; optional4 workflow checks added to build-ui CI after sandbox/AppArmor setup.
- Initial full .8 Rust run1259serverPASS/1FAIL/14ignored; provider admission fixture ConnectionAcquireTimeout under heavy parallel load, same test isolated PASS. Final full run logs workspace-timezone-final-tests.log; DO NOT claim complete before reading final result.
- Runtime focused: server timezone12PASS; old Runner local+peer refusal PASS; Runner session17PASS/1existingignored; protocol additive frame test PASS; initial workspace check PASS. Final Windows inherited-TZ fix8/8PASS including real Node subprocess, Tokio/PTY removal, unchanged legacy and parent environment. Latest full Runner suite remains.
- Sparse query RED proved bounded four-fact -0001..+100000 processing then failed author name as expected; GREEN metadata fix is in frozen source and compiling in final full run. Do not use old a93a6262f8ef4ca6 RED executable as final proof.
- Mobile fixture7PASS; updater/runner manifest tests4PASS including mobile extras ignored. Native builds/publication remain NOT_RUN for .8. YAML/actionlint previously PASS; rerun after final pipeline edits if available.
- Old scripts/test-workflow-drafts.py fails identical revision-list/detail enrichment equality atline220 on unchanged .7 and .8; no assertions/API weakened. It is not the relevant new runtime acceptance; evidence baseline and new logs preserved.
## Active Agents / Ownership
- timezone_backend_audit: all fixes/focused tests complete, source frozen. No commits/worktrees.
- timezone_frontend_audit: implementation and final independent runtime review complete; frontend frozen.
- mobile_release_delivery: sparse query and metadata fix frozen; independent frontend/CI review no blockers.
## Remaining Work / Next Action
1. Read final full Rust result; inspect/fix any real failures (never weaken tests). Finish final Windows Runner fix and fresh Runner tests. Run current cargo fmt/check and generator validation.
2. Rebuild embedded server after final runtime changes; rerun actual HTTP32 acceptance, targeted source-fresh Runner/SSH/clock tests and meaningful existing UI runtime acceptance. Refresh durable evidence and self/final acceptance review.
3. Review all status/diff/line endings; stage only task deliverables explicitly. SSH commit/push main. Wait exact-source Build, deployment acceptance and Mobile native preflight CI. Do not tag on failed/pending gates.
4. After gates, create immutable new annotated v0.10.8-alpha tag (currently absent), SSH push; wait actual release and container CI. Verify assets include actual versioned APKs/unsigned IPA/simulator plus manifests/digests and amd64/arm64 image. Only then claim released.
5. Live existing workspace timezone must be chosen by administrator; no value was authorized for that workspace. Live Codeup workflow prompt still contains old batch $json/hardcoded Shanghai; no authenticated browser/Ferry connector available to patch it, and no report/notification/schedule acceptance occurred.
## Delivery / Environment Constraints
- Workdir D:/Projects/github/ferry main, baseline b051505. No new branches/worktrees. SSH pushes only. Windows exec tty:true/login:false; subprocess windowsHide/CREATE_NO_WINDOW. LF/core.autocrlf=false. Local4291 and production untouched during this task.
- No GH CLI login; D:/Tools/gh/bin/gh.exe available, public GitHub API works. Credential-manager probe hung and was cancelled without exposing a token. Browser/CUA host unavailable; Playwright isolated fallback documented.
- v0.10.7-alpha existing21 Release assets lack APK/IPA (verified), old Downloads package now absent. Historical successful mobile run44907032751 source1b8ed68ea1352a8e67cf54958dd7a7591efd7f8b; Android artifact14708281328/iOS14709271088 nonexpired. No old attachments repaired/uploaded.
- .7 already published at1aae286d13d5723efd34ae538e72827cc1ccaef0. Prior authorized Debian192.168.0.4 /root/ferry upgrade .6->.7 healthy, backup20261122T083007Z; no current remote operation. Do not print credentials/master keys. Current task SSH diagnostics were read-only and closed.
- Preserve .7 chunk-authenticated storage compatibility and backups; old servers cannot read newly written format. Upgrade all server replicas before confirming .8 zones/new execution contexts. Native Android debug signing and unsigned IPA are user-accepted, no physical iPhone/store claim.
## Evidence / Checkpoint
External evidence/scripts: C:/Users/FullStack/AppData/Local/Temp/ferry-release-tools/; final logs prefix workspace-timezone-. New release scripts must not rerun hardcoded release-0107 finalizer or move v0.10.7 tag.
Checkpoint:2026-11-22 final integration, before any .8 commit/tag/push.

## Final delivery checkpoint
2026-11-22: Source and Git reconciled after recovery. Verified full workspace log and check completed successfully. Public GitHub Release still v0.10.7-alpha, 21 assets with no APK/IPA. Mobile collection fixed in .8 source, not yet delivered. Next: commit reviewed explicit deliverables, SSH push main, wait Build/deployment/mobile gates, tag .8 only after green, verify actual assets and images. No production/4291/live report mutations.

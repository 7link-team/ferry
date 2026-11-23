# Task State
## Task / Objective
Publish the workspace-timezone/workflow-diagnostic update with actual Android APK and unsigned iOS IPA assets. Latest user says continue; do not stop at source push or Actions artifacts.
## Current Phase
RELEASE DELIVERY / LOCAL ACCEPTANCE COMPLETE
## Current Step
v0.10.11-alpha source and manifests ready, NOT yet committed/tagged. Final local verification PASS. Next self-review/staged review, commit product, push immutable11tag via SSH; then docs-only [skip ci] state checkpoint/push main to avoid duplicate branch workflows. Full tag CI and container workflow remain unchanged. Track publication and verify actual Release downloads before claiming completion.
## Confirmed Constraints
- SSH push only; never move existing tags (.7/.8/.9/.10). No extra branches/worktrees, explicit staging only. Windows tty:true/login:false; hidden child processes. LF/core.autocrlf=false.
- Workspace business timezone is editable IANA; new spaces suggest browser/system as required editable value. Existing spaces stay NULL/unconfirmed until administrator saves; no inferred backfill or historical instant rewrite.
- Backend/API/SQL instants UTC. Confirmed workspace governs business calendar/display/schedules/new process clocks. Workflow attempt zone and clock frozen. Child TZ only, never mutate OS/global Runner/existing terminal; incompatible old Runners refuse required zones.
- Preserve per-conversation API/CLI agent context, unlimited resource defaults with configurable policy and usage recording, no monetary admission caps. Preserve authorization/ownership/protocol/liveness. General conversation policy maintained in platform settings.
- Android debug-signed acceptance APKs; user accepted unsigned iOS device IPA/simulator build without Apple team. Do not claim App Store/Play Store or direct unsigned true-device installation.
- No production/original4291/live Codeup workflow mutations in this task. Live workflow116765975084269568 version2 still old batch$json/hardcodedShanghai; no PDF/WeChat/schedule acceptance. Existing live workspace administrator must confirm business zone.
## Implemented / Source State
- Committed earlier timezone/diagnostic/mobile pipeline work: abbc117, aa29db2,286a9f3,4444baa,b5e3f35. .9 source b5e3f356d92888cbe6908bfacb4dded0bf37236e.
- .10 source62160b123df3e72669ee0ef76cb7d7af8033941f, SSH main/tag pushed. Timezone notice sibling of h1 preserves exact accessible heading; holes list uses existing120px minimum. Both original .9 blockers passed cloud.
- .11 restores UTC placeholder on legacy code-insight custom Input (existing field already declared UTC); confirmed-workspace display unchanged. Original contribution browser assertion retained.
- .11 extends existing minimum-height option: AutoTable actions preserves prior header+controls behavior; row measures/observes full first row. ResourceTable defaults actions; hosts explicitly row; numeric member160/holes120 minima unchanged. ListConfig supports number|row. No second table, dependencies, migrations, permission or API changes.
- Regression unit retains104/148px controls after equal-count replacement, proves172px full padded row, then104px actions mode. RED before fix, GREEN after. No original browser/assertions weakened.
## Verification ? Final .11 Source
- npm production build PASS (includes tsc); actual embedded server+Runner build PASS; cargo check --workspace PASS.
- Full frontend128files/974PASS/1existingSkip. Stable final-frontend-tests.log.
- Fresh production browser block10PASS and remaining browser suites25PASS (35total), exact final scoped source. Includes contribution/theme/insight/refresh/language/typed-values/YAML/reference/execution locale, cron/alerts/marketplace/wizard/repository auth/Actions/manual approvals/monitors/host workspace/table/footer/CRUD. Stable scoped-final/{frontend-block-audit,remaining-browser-audit}.json; all exits0.
- Actual11embedded-server test-templates-ui.mjs PASS, including1440/900/320,ms-MY320,200%font, normal pointer delete/Escape, all filter/row/pagination bounds. scoped-final/templates.log and screenshot templates-ms-MY-320-font-2.png inspected.
- Mobile fixture7PASS; diff check and LF PASS.
- Latest full Rust workspace .10 source1429PASS/26ignored (server1264PASS), no Rust source changes in11. First run five missingBash/sh failures; rerun entire suite with Git bin/usr/bin in test PATH PASS. Fresh11tag Rust gate required.
- Local scripts/test-updater.py cannot resolve npm on Windows (WinError2); no updater source changed. Fresh Linux tag gate required.
## Reproduced Failures / Resolution
- .9 Release skipped: heading contained timezone notice; host preview viewport <120px. Minimal .10 fixes and original cloud checks PASS.
- .10 e2e-build-ui failed at original contribution placeholderUTC assertion (actualnull). Local exact RED reproduced; .11 restores attribute and full original suite PASS.
- First full-row implementation applied globally: original900px table footer and390px CRUD pagination RED. Scoped option preserves original shared default and enables full row only for hosts. All35fresh scoped suites PASS.
- One local embedded .10 build failed removing executable held by concurrent browser fixtures. Final11build ran with mock-API browser scripts only and completed; later actual fixtures ran after build. No source/test workaround.
- Old Nov5 local Actions worker returned invalid execution slot. Rebuilt from current source with scripts/build-actions-workers.sh Windows target; real Runner preview automation PASS. No product workaround.
## Cloud .10 / Mobile / Image Evidence
- Tag Build45092468609 failed only e2e-build-ui at last snapshot; Release cannot publish. .10 Android+iOS(native included), checks/backends, both storage, preview domains and other completed gates PASS. Last pending was macOS native service; no second failure observed. Watcher session5160 polls obsolete10tag; stop when no longer useful.
- Actual10mobile10files downloaded/validated using repository mobile-release.py: ABI/plist/content/version/identity/checksum PASS. These are Actions artifacts, not Release publication.
- .10 public image verified both linuxamd64/arm64 revision62160b1; index/latest sha256:347cccccf83bd9c9abf71f7dbf90d57638d575e725b5a5df9edbef4a84fc98f6. Stable image-verification.json.
- Latest public Release last observed .7,21assets,no mobile. .8/.9/.10 attempts remain immutable and blocked; do not claim installation assets published.
## Evidence / Environment
- Stable directory C:/Users/FullStack/AppData/Local/ferry-verification/v0.10.11-alpha/. scoped-final contains final35browser logs/JSON; final-server-build.log,final-workspace-check.log,final-frontend-tests.log.10evidence and final verifiers under siblingv0.10.10-alpha.
- Partial temporary scripts disappeared during long run (cause undetermined). Stable verification files preserved outsideTemp; do not rely on old temporary scripts.
- Public GitHub API60/h shared limit was reached. Poll new tag jobs at most every300s, no repeated completed image queries; use public GitHub HTML when limited. gh CLI unavailable/not authenticated. Browser plugin unavailable, repository Playwright/Chrome used.
- Stop browser fixtures before rebuilding Windows executable. Original4291 untouched. New target/debug binaries are11; current Actions worker was rebuilt from source.
## Remaining Work / Next Action
1. Commit reviewed11source/manifests/notes/state; create annotatedv0.10.11-alpha; SSH push tag first (product commit has no skip marker).
2. Record exact SHA/run IDs in docs-only [skip ci] commit, then SSH push main; this skips duplicate branch runs while immutable tag runs full unchanged release gates.
3. Track exact11tag/mobile/native/UI/backend/deployment/desktop gates and container. Investigate real failures without weakening tests. Public API limit: use300s interval/public HTML fallback.
4. After Release exists, adapt stable10verify-assets.mjs/image.mjs to11 and exact new source SHA; check31+actual assets, downloads, SHA256SUMS, runner catalog signature and desktop updater refs; full mobile-release.py verify on final Release downloads. Verify both image configs and latest revision.
5. Persist final actual results in MEMORY and docs-only [skip ci] SSH commit, clean tree. Final report actual download links/signing limits; distinguish no production deployment/live report acceptance.
## Review / Acceptance
Local source self-review: scope/reuse/architecture/regression/complexity/verification PASS. Release delivery NOT READY until actual cloud publication/download verification. Execution mode Direct; no new subagents.
## Last Checkpoint
2026-11-23: final scoped11local acceptance complete; ready for reviewed product commit/tag publication.

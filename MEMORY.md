# Task State
## Task / Objective
Deliver workspace-wide business timezone + workflow diagnostics and publish a version with actual APK/unsigned IPA assets. Latest user asks why Release lacks mobile packages. Latest public Release was v0.10.7-alpha (21 assets, no APK/IPA); cause was mobile Actions artifacts not connected to Release collection.
## Current Phase
RELEASE DELIVERY / CLOUD VERIFICATION
## Current Step
v0.10.10-alpha local acceptance PASS; ready to commit and SSH push immutable new tag. Public latest still .7. Full frontend128files/974PASS/1existingSkip; production UI build; fresh server+Runner build; cargo check workspace; mobile fixture7PASS; original workspace/admin and alpha2 browser suites PASS. Real Runner preview automation with freshly built current Actions worker PASS (data/maintenance/preview-automation-20261123-010309/result.json, all original >=120px/overflow/link/review/cleanup assertions preserved, no browser errors; 390px screenshot inspected). Old Nov5 local worker caused invalid execution slot in first attempt; rebuilt from current source, no product workaround. Both regression tests RED before fixes and GREEN afterward. Next commit/push .10; track CI then verify actual Release APK/unsigned IPA/simulator/signatures/checksums and image revision. Never move .9 tag.
## Confirmed Requirements / Constraints
- Workspace editable IANA business zone; new UI detects browser/system as editable required suggestion. Existing spaces stay NULL/unconfirmed until administrator explicitly saves; no inferred backfill or historical instant rewrite.
- Backend/API/SQL instants UTC; confirmed workspace governs business calendars/display/schedules/new process clocks. Running workflow timezone/attempt clock frozen. Child TZ only; no OS/global Runner/existing-terminal mutation. Old incompatible Runners refuse required zones.
- Preserve per-conversation Agent API/CLI context, unlimited resource defaults/usage recording/no monetary admission caps, auth/ownership/fresh authorization/protocol/liveness. General conversation policy maintainable in platform settings.
- SSH push only; immutable tags (never move .7 or .8); no extra branches/worktrees. Explicit staging only. Windows tty:true/login:false; hidden child processes. LF/core.autocrlf=false.
- No production, original4291 or live Codeup workflow mutations in this task. Live workflow version2 still old batch $json/hardcoded Shanghai; no PDF/group notification/schedule acceptance. Existing live workspace timezone administrator must choose once.
## Implemented / Committed
- abbc117: additive nullable workspace timezone migration/scoped settings singleton, authorization/version/lock/audit; schedules/calendar SQL/filters; frozen workflow/process clocks; pre-activation public diagnostics; complete MCP preview schema/JSON Pointer/batch usage; frontend dates/DST/sparse heatmap; Runner/SSH per-channel TZ.
- Mobile reusable pipeline: tag Release waits mobile, package/version/ABI/content/hash checks, merges dist-mobile-* before signed manifests/checksums. Android debug-signed, IPA unsigned, simulator archive; user accepted signing limitations.
- aa29db2: legacy upgrade test explicitly asserts additive timezone:null, preserves all other equality checks. Real alpha.1?.8 SQLite regression PASS; .8 tag all3DB backend upgrade gate PASS.
- 286a9f3: navigation acceptance handles workspace-settings/1 singleton + timezone:null. Actual56page browser/import/export/tags acceptance PASS.
- 4444baa: final tag gate record. .8 tag pushed, Build45045291088 blocked by UI+simulator failures; no Release asset publication.
## Active Uncommitted .9 Correction
- App: compact localized unconfirmed timezone link beside title (full accessible explanation), list min-content/vertical scroll fallback and overflow-x-hidden. Confirmed settings still in navigation.
- ResourceTable: reuse minimum-data mechanism, minDataHeight='row' default; fixed numeric member160 preserved. Pagination stays in flow, previous boundary shadow retained.
- AutoTable: measure header+current first-row actions (floor100), ResizeObserver; depend on rows array so equal-count replacements reattach. Footer/nav/select min-width/max-width constraints + Phone full-width wrapping, page count nowrap.
- Tests: original template pointer/delete checks retained; new Desktop1440/Pad900/Phone320/MalayPhone/200%font geometry and normal delete-dialog/Escape, no page overflow, pagination bounds/screenshots. Last remaining failure is font2 pagination bounds. Temporary FOOTER_LAYOUT log must be removed after diagnosis.
- CRUD fixture categories endpoint previously matched as dictionary detail and returned undefined; now exact real category_counts envelope (no assertions weakened).
- Version manifests/locks bumped .8?.9, new docs/releases/v0.10.9-alpha.md, .8 attempt documented blocked, docs index/date updated. No .9 publication yet.
## Verification
- .8 full Rust workspace PASS, server1264PASS/0FAIL/14ignored; check workspace PASS. Latest full Runner112+5PASS; protocol/SSH acceptance/refusal, actual default-stack command workflow PASS. Explicit ignored real sandbox example and cancellation tests PASS.
- Workspace HTTP32 checks PASS; node inspector7 groups PASS. New workspace creation/settings/sparse heatmap browser320/900/1440 + msMY Phone PASS. Evidence ferry-timezone-qa external.
- Frontend full128files/971PASS/1existingSkip (cmd redirection avoids PowerShell CLIXML capture); after latest row measurement relevant58PASS incl observer regression. Temporary rows.length reversion RED confirmed (104px instead148px), restored.
- .9 production UI build PASS, actual embedded server build PASS, cargo check workspace/fmt/generated catalog320operations PASS; mobile fixture7 and manifests4 PASS.
- Final .9 table browser PASS: empty/single/many rows,1280x800/900x600/390x844, all6theme boundary shadows, frozen columns, member invites/platform lists/long forms. Log0109-table.log.
- Final .9 CRUD browser PASS1280/900/390/320 + long deployment1440/768/390, readonly/forms/actions, no browser errors. Log0109-crud.log. API fixtures not backend persistence proof.
- Template suite Chinese Desktop/Pad/Phone CRUD/chooser PASS; msMY/font2 normal pointer & geometry passed; pagination horizontal containment assertion still FAIL. Do not claim final acceptance.
## Review / Known Issues
- Independent release_list_regression_review PASS after fixing detached-control observer issue. No other present-day source blockers found; root retains actual browser acceptance.
- .8 mobile preflight45039388917 PASS (identical native source), deployment45039388821 PASS, dual-arch sandbox45039388942 PASS.
- .8 tag mobile/iOS failed before any app cases: simctl launch/terminate/uninstall all ETIMEDOUT; results cases=[] and cleanupFAIL. No native timeout/assertion weakened. Fresh .9 CI must pass; this is not proof of app success.
- .8 Server container45045291059 PASS, public OCI verified amd64+arm64 revision4444baa, latest same index sha256:60f0fe2637fca037a4be371a356f59ac8d0e5d51eeb1805e015bfe461ca45857. Already published image, no .8 Release assets.
- Legacy scripts/test-workflow-drafts.py list/detail equality fails identical on .7 and .8; no API/assertion weakened. Relevant new workflow HTTP/node-inspector acceptance passed.
## Remaining Work / Next Action
1. Track tag Build45055302403 and Server container45055302405. Final source b5e3f356d92888cbe6908bfacb4dded0bf37236e. Check preflight native45055260935 too. No move .7/.8/.9 tags.
2. Inspect real CI failures and complete safe in-scope fixes/verification. Fresh iOS job must PASS; do not mask native failure or weaken assertions. Final Release needs both APK ABIs, unsigned IPA/simulator, signing disclosure/checksums.
3. Verify31+ actual Release assets, download mobile files and inspect contents/hashes, signed Runner/updater catalogs; public amd64+arm64 image revisions and latest. Scripts release-0109-verify-{assets,image}.mjs are ready and write release-0109 evidence. Run mobile-release.py verify on downloaded folder.
4. Final durable state-only commit/push (docs-only skip-ci may avoid redundant full builds if appropriate), clean tree; report actual direct links/signing limitations. Existing admin timezone confirmation and live Codeup PDF/WeChat/schedule remain unmodified/unaccepted, not part of release acceptance.
## Evidence / Environment
All external logs/scripts under C:/Users/FullStack/AppData/Local/Temp/ferry-release-tools/. Current prefix release-0108, filenames0109-* for fresh .9. Public nightly.link downloaded ios-failure.zip with native-acceptance results/command logs; no credentials exposed.
- .9 fresh server target/debug/ferry-server.exe. Old .8 binary release-0108/upgrade/new-server.exe useful API backend for Vite preview tests; do not confuse embedded UI versions. .7 baseline target-0.10.7-verification/debug/ferry-server.exe.
- Public API unauthenticated60/h; poll sparingly. GH CLI installed but NOT logged in. Public HTML works. Browser plugin not available; repository isolated Playwright used. No original4291 changes.
- Read-only publication verifier scripts release-0108-verify-assets.mjs (requires31assets, downloads mobile) and release-0108-verify-image.mjs accept tag/SHA but write .8 evidence directory; copy to .9 directory before use, never run .7 finalizers.
## Last Checkpoint
2026-11-23 02:29 UTC+8: .9 main/tag SSH pushed, exact tag cloud verification active.

## Final .9 local acceptance (2026-11-23 02:22+08)
- release-0109/accepted-templates.log PASS: template/app/pipeline CRUD, chooser4widths, real saves/deletes, normal pointer/delete/Escape and all filter/row/pagination bounds at1440/900/320 +msMY320 +font200%; no browser errors. Screenshots accepted-*.png inspected.
- release-0109/accepted-table.log PASS: empty/single/many rows,1280x800/900x600/390x844, all6themes/light-dark boundary shadows/frozen edges, member invites/platform/long forms.
- release-0109/accepted-crud.log PASS: original4widths +long deployments1440/768/390, readonly and controls reachable. Correct categories fixture now follows actual category_counts endpoint, no assertion weakened.
- release-0109/accepted-frontend.log PASS128files/972tests/1existingSkip, standalone after UI. Prior under simultaneous browsers one unrelated activity dialog focus assertion failed; isolated6PASS, final whole suite PASS, no focus source/assertions changed.
- v.9 Cargo check workspace/fmt/catalog320operations PASS, npm production/fresh embedded server PASS; mobile fixtures7 and manifests4 PASS. Publishing command tested against actual metadata-bearing release notes; YAML parsed.
- Final source review PASS after observer dependency fix. Implicit grid column503px caused horizontal propagation; explicit minmax(0,1fr) restores256px content atfont2. Filter form was384px; constrained/wrapping actual source now206px, all controls in viewport. No force clicks or weakened checks. Temporary source diagnostics removed; external probe/artifacts retained only outside deliverables.

## Final pushed .9 evidence
- b5e3f35 committed20reviewed files, SSH main/tag pushed; product worktree clean before this state update. Exact release-note publishing command PASS after stripping doc frontmatter, YAML parsed. No generated/binary/log artifacts staged.
- Final source acceptance release-0109/accepted-{templates,table,crud}.log PASS; source CSS also wraps bounded form/toolbar and row controls. Font2 filter was384px, now206px; all filter/row/pagination bounds +normal pointer assertions pass. Screenshots accepted-ms-MY-320-font-2.png inspected along with desktop.
- Final standalone frontend release-0109/accepted-frontend.log:128files/972PASS/1existingSkip. No assertions weakened. One prior simultaneous-browser run had unrelated activity focus timing failure; isolated6PASS and final full run PASS.

## Publication progress 2026-11-23 02:43+08
- Android tag job PASS. Downloaded actual dist-mobile-android.zip through public nightly.link; both .9 APKs ZIP/content/ABI/SHA256 PASS (arm64 38930473bytes, x86_64 39220830bytes). Evidence release-0109/android-preview/. Interim user link provided https://nightly.link/7link-team/ferry/actions/runs/45055302403/dist-mobile-android.zip; not represented as Release publication.
- Tag iOS and standalone main iOS both entered native interaction acceptance; neither final PASS yet. Tag checks/backends still queued from GitHub capacity. Watcher session87331 external release-0109/watch.py polls bounded rates and writes run-ID jobs JSON; it only logs, never tags/publishes.
- Read-only ios_native_failure_review: .8 timeout cause UNDETERMINED. Install/container lookup succeeded; home screen/Ferry icon, PS has SpringBoard/runningboardd but no Ferry process. Logs omit simulated SpringBoard/Ferry and lack crash/sample; no identified startup-blocking native code, unchanged launch/navigation source. Do not claim infra-only or fixed; .9 full native CI acceptance required. No timeouts/assertions/retries changed.

## Release continuation 2026-11-23
- Exact .9 tag CI completed: Android/iOS/checks/backends/storage/desktop/runner/deployment gates PASS. release SKIPPED due to preview-domains and e2e-build-ui failures. Public Release still .7 (21 assets, no mobile).
- Root causes from public job annotations + downloaded preview-domain-evidence.zip: timezone notice inside h1 changes exact accessible heading (repository events, tags); host preview table viewport below existing >=120px acceptance. No assertion weakened.
- Minimal fixes in progress: timezone notice sibling of h1 in same wrapping header, holes list uses existing minDataHeight=120. Added regressions; both RED before fixes, all55 relevant tests GREEN after. npm production build running.
- Next: build embedded server; run real workspace/admin + auth/audit + preview automation suites with unchanged assertions, then immutable new patch release (never move .9). Mobile .9 now PASS but not published; fresh release must include both APK ABIs + unsigned IPA + simulator with checksums.

## Patch release final local review
- Scope/reuse/architecture/regression/complexity/durable-state self-review PASS. Only two UI blockers, tests, manifests/version and release/state docs changed. No new dependency, migrations, permission or API change. All original browser assertions retained.
- Local Python scripts/test-updater.py cannot launch npm on Windows (WinError2); no updater source changed. Fresh Linux CI updater gate remains required; do not claim it passed locally.
- .9 actual mobile files downloaded and .github/scripts/mobile-release.py verify PASS. .9 image dual-arch source b5e3f35 and latest digest verified. .9 release still SKIPPED; do not represent Actions artifacts as Release assets.

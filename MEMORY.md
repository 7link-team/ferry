# Task State
## Task / Objective
Deliver workspace-wide business timezone + workflow diagnostics and publish a version with actual APK/unsigned IPA assets. Latest user asks why Release lacks mobile packages. Latest public Release was v0.10.7-alpha (21 assets, no APK/IPA); cause was mobile Actions artifacts not connected to Release collection.
## Current Phase
REVIEW / RELEASE DELIVERY
## Current Step
Local v0.10.9-alpha correction is VERIFIED and READY FOR SSH COMMIT/PUSH/TAG (none done yet at this checkpoint). Final source: compact title hint, bounded single grid column, measured table viewport/current-row observers, in-flow pagination/shadows, wrapping/constrained filter/toolbar/row/pagination controls. Fresh .9 production UI and embedded server builds PASS; cargo check workspace/fmt/generated catalog PASS. Final standalone full frontend128files/972PASS/1existingSkip, all three real browser suites PASS including every filter/row/pagination control at msMY320px200% font. Independent reviewer PASS; screenshots inspected. .8 tag/image remain immutable; no .8 Release assets. Next explicitly stage reviewed .9 source/manifests/docs/test deliverables, SSH commit/push main, new annotated .9 tag, then wait actual native/release/container CI and verify public files/image. Do not claim published on push/tag alone.
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
1. Commit reviewed explicit .9 files and SSH push main; create immutable new v0.10.9-alpha tag only after local verification (now PASS), SSH push. No move/delete old tags.
2. Track final Build/release and Server container, inspect real failures. Fresh native iOS job must pass; do not hide/mask native failure. Final release must include both APK ABIs, unsigned IPA and simulator plus checksums/signing disclosure.
3. Verify31+ actual Release assets, download mobile files and inspect package contents/hashes; signed Runner/updater catalogs; public amd64+arm64 image revisions and latest. Then final state-only commit/push, clean tree and actual direct links with signing limits.
4. Existing workspace admin still chooses timezone after upgrade; live Codeup report/PDF/WeChat/schedule remains unmodified/unaccepted in this task.
## Evidence / Environment
All external logs/scripts under C:/Users/FullStack/AppData/Local/Temp/ferry-release-tools/. Current prefix release-0108, filenames0109-* for fresh .9. Public nightly.link downloaded ios-failure.zip with native-acceptance results/command logs; no credentials exposed.
- .9 fresh server target/debug/ferry-server.exe. Old .8 binary release-0108/upgrade/new-server.exe useful API backend for Vite preview tests; do not confuse embedded UI versions. .7 baseline target-0.10.7-verification/debug/ferry-server.exe.
- Public API unauthenticated60/h; poll sparingly. GH CLI installed but NOT logged in. Public HTML works. Browser plugin not available; repository isolated Playwright used. No original4291 changes.
- Read-only publication verifier scripts release-0108-verify-assets.mjs (requires31assets, downloads mobile) and release-0108-verify-image.mjs accept tag/SHA but write .8 evidence directory; copy to .9 directory before use, never run .7 finalizers.
## Last Checkpoint
2026-11-23 02:22 UTC+8: local .9 acceptance PASS; release publication still pending.

## Final .9 local acceptance (2026-11-23 02:22+08)
- release-0109/accepted-templates.log PASS: template/app/pipeline CRUD, chooser4widths, real saves/deletes, normal pointer/delete/Escape and all filter/row/pagination bounds at1440/900/320 +msMY320 +font200%; no browser errors. Screenshots accepted-*.png inspected.
- release-0109/accepted-table.log PASS: empty/single/many rows,1280x800/900x600/390x844, all6themes/light-dark boundary shadows/frozen edges, member invites/platform/long forms.
- release-0109/accepted-crud.log PASS: original4widths +long deployments1440/768/390, readonly and controls reachable. Correct categories fixture now follows actual category_counts endpoint, no assertion weakened.
- release-0109/accepted-frontend.log PASS128files/972tests/1existingSkip, standalone after UI. Prior under simultaneous browsers one unrelated activity dialog focus assertion failed; isolated6PASS, final whole suite PASS, no focus source/assertions changed.
- v.9 Cargo check workspace/fmt/catalog320operations PASS, npm production/fresh embedded server PASS; mobile fixtures7 and manifests4 PASS. Publishing command tested against actual metadata-bearing release notes; YAML parsed.
- Final source review PASS after observer dependency fix. Implicit grid column503px caused horizontal propagation; explicit minmax(0,1fr) restores256px content atfont2. Filter form was384px; constrained/wrapping actual source now206px, all controls in viewport. No force clicks or weakened checks. Temporary source diagnostics removed; external probe/artifacts retained only outside deliverables.

# Task State

## Task / Objective
Finish Release delivery with APK/unsigned IPA assets, and implement the user-approved mobile online update flow. User requested adding the proposed Android verified download/system installer and iOS version-check/distribution-link behavior. They confirmed no keystore or iOS distribution URL: complete flows/configuration and disclose missing setup.

## Current Phase
TESTING / v0.10.12-alpha CLOUD RELEASE GATES

## Current Step
All .11 cloud rerun jobs and Release publication succeeded. Actual .11 release has31assets including APKs/IPA. All10mobile files were downloaded from exact-tag Actions artifacts and matched byte-for-byte to the published Release API SHA256/size. Small signed manifests came from Release. Package content/version/ABI validation against immutable .11tag configuration PASS. .12new update code and final local tests/checks completed; product committed and immutable12tag pushed. Product74053d8e47e395a5f35da2280687821c03bdd2a5 committed; annotated immutable v0.10.12-alpha SSH pushed and remote peeled SHA verified. Exact Build45123101293 and Server image45123101291 target74053d8. Main documentation checkpoint push follows; no further product edits authorized by a failure-free gate.

## Git / Source
- Repository D:/Projects/github/ferry, origin/main contains product74053d8 and docs checkpoints; tree clean after latest checkpoint push.
- Immutable v0.10.11-alpha product60596a319a67dc868eb2204e68d7b111c0bcdef1. Never move tags.
- Committed product contains mobile updater implementation, proven provider fixture coordination fix, version12in7manifests/locks and docs. Explicit staging only. No new branches/worktrees/agents.
- v0.10.12-alpha notes docs/releases/v0.10.12-alpha.md; design/config/review docs/plans/2026-11-23-mobile-client-updates.md; decision ADR.md.

## Confirmed Constraints
- SSH pushes only. Windows tty:true/login:false, hidden child windows, LF/core.autocrlf=false. No blind git add.
- Android acceptance APKs remain debug builds; no production key available. Optional four Actions signing secrets supported as a complete set. iOS remains unsigned, user accepted no Apple team. No store/public signed-install claim.
- No production/original4291/live Codeup workflow mutation. Live workflow116765975084269568version2 remains old batch expression/hardcodedShanghai; PDF/WeChat/schedule not accepted in this task.
- Workspace business time remains administrator-confirmed editable IANA; new creation suggests browser zone; existingNULL unconfirmed retained. Backend/API/SQL UTC, child-only TZ. Preserve authorization, Runner protocol/liveness, per-conversation API/CLI context and unlimited-default configurable resource policy.
- No browser automation on blocked GitHub surface: CUA automatic approval rejected getApp because allowed browser URL could not be verified; no UI inputs made. gh/GCM/env auth unavailable; do not search other credential stores. User independently clicked rerun. Public API reads and authorized SSH Git pushes remain usable.

## Implemented / Reuse
- Shared release config/types extracted from existing desktop updater; desktop selection/install/signature logic retained.
- Existing launcher update section now supports mobile installed version, manual online check, SemVer/channel/ABI selection, release notes as text, current/available/unconfigured/error/retry/progress states, zh-CN/en-US/ms-MY.
- Android streams bytes to fixed private update-cache file, checks published digest/size and removes bad partials. Native Kotlin verifies package identity, strictly increasing versionCode and exact installed signer set; then unknown-source settings or system installer. No silent install, forced uninstall or user-data deletion.
- iOS uses configurable HTTPS distribution landing URL and UIKit handoff. Missing URL disables install with explanation. No invented TestFlight/AppStore link.
- Native commands use existing exact bundled-document guard before work and after awaits. Remote workspace receives no update IPC permission.
- Generator adds scoped non-exported FileProvider, install permission and native plugin source; no AUTO-GENERATED file hand edits. Optional CI keystore via MOBILE_ANDROID_KEYSTORE_BASE64/STORE_PASSWORD/KEY_ALIAS/KEY_PASSWORD. Native config plugins.release.mobile.packageName/iosDistributionUrl. User has none; defaults remain unset.
- Provider test fixture previously paused queued future while awaiting cancellation DB write. Deterministic100ms held-transaction diagnostic reproduced exact ConnectionAcquire(Timeout) RED36.33s; same probe with tokio::join GREEN5.93s. Diagnostic removed. Deliver only concurrent polling fix; original50ms admission,3s cancellation and paid-request assertions unchanged.
- Android harness updated for WebView CDP (noDefaults, page-scoped Network.getCookies). Original assertions retained/HttpOnly strengthened; new guarded update/remote isolation assertions added.

## Verification ? Current Source
- Production npm build PASS; full frontend128files/974PASS/1existingSkip.
- Native Rust15PASS/2existingignored. Real HTTP success/wrong-hash/oversized/truncated download tests PASS; partial cleanup verified.
- Launcher4files/28PASS. Four Playwright viewport/locale cases PASS incl Malay320px/enlarged fonts, no horizontal overflow or console errors; screenshots inspected. Browser plugin absent, regular Playwright used only for isolated local fixture.
- Full Android ARM64/x86_64 APK builds PASS. Fresh .12split APKs staged with version/identity/ABI/content/checksums PASS under stable12/local-mobile.
- Disposable emulator native acceptance14PASS on final harness, incl connection/restart/Back/offline/file picker and denied remote update commands. Existing user emulator5554 untouched. Owned emulator5556 and AVD ClientUpdateAcceptance20261123 shut down/deleted after verification.
- Optional signing configuration PASS via Gradle signingReport with disposable test keystore; all ABI variants use requested test key. Generator idempotence PASS; YAML parse/syntax/diff checks PASS. Test key is outside repo and not a production key.
- iOS source-check PASS only; actual new-source iOS build/native acceptance still REQUIRED on macOS CI. .11rerun iOS PASS proves oldsource, not newupdatecode.
- Root full .12cargo test --workspace session82272 exit0:1429PASS/26existingignored; cargo check --workspace42142exit0; npm typecheck/build PASS. Latest client config-availability status now explicit even without a newer version: build.rs embeds only whether fixed Android signing was configured; UI29PASS; nativeRust15PASS/2ignored; new final split Android build1196awaiting completion. No secret path/key embedded.
- New-source native Rust .12session93820 completed15PASS/2ignored; productionUI99951/fullfrontend30579/Android87960completedPASS. No further code changes pending known failures.

## .11 Release / Cloud Evidence
- Build45101201903 attempt2: iOS137504440110/e2e-runner137504440682/release137513979529 allSUCCESS. User clicked Re-run failed jobs; remaining prior gates retained success. Final API attempt-2-final-jobs.json.
- Actual Release https://github.com/7link-team/ferry/releases/tag/v0.10.11-alpha with31assets. assets-verification.json PASS,10mobile payloads match published hashes; mobile-artifact-provenance.json records exact source. Validate .11with tagged manifests, because current manifests are12.
- .11image actual linuxamd64/arm64 revision60596a3; index/latest sha256019da223efb5104634b8fe1868c24e055f350ab49d3ea91f493136caebf80fdd. image-verification.json PASS. No later image pushed yet.
- .11priorlocal35browser suites PASS and actual embedded template320px/Malay/200%font PASS. No src UI business changes in12; retain original assertions.
- Release CDN APK download on this machine slow20KB/s; matching exact-tag Actions artifacts downloaded quickly. Do not claim fresh full CDN APK downloads. Small Release manifests/signatures verified, actual full mobile bytes validated against published digests.

## Evidence Paths
C:/Users/FullStack/AppData/Local/ferry-verification/v0.10.11-alpha/: final .11browser/runtime evidence; .12logs prefixedv12 and mobile-update*, mobile-updates-qa.json, mobile-update-native-results.json, provider controlledRED/GREEN logs. .12local APK staging under siblingv0.10.12-alpha/local-mobile. No generated artifacts/logs/signing keys staged.

## Remaining / Next Action
1. Track Build45123101293 and image45123101291 for source74053d8. Active watcher71399, script stable12/watch.py polls?300s, saves jobs JSON. Latest: iOS137521172529building simulator; Android137521172652queued; checks advanced to cargo check, no failures observed. Do not start another release or repeat completed local gates without a new failure/change.
2. Require actual new-source iOS compile/native acceptance. If a gate fails inspect its exact evidence and reproduce before fixing. Never skip/weaken assertions or move immutable12tag.
3. After12actualpublication, use prepared stable12/verify-assets.mjs and verify-image.mjs. Download mobile Actions artifacts if CDN is slow and compare full actual payload bytes with published Release digests; record provenance, then validate package/version/ABI. Verify runner manifest signature/updater refs and both image source revisions. Do not claim full CDN downloads when using exact-tag build payloads.
4. Final MEMORY/ADR/notes checkpoint, explicit docs-onlycommit[skip ci]/SSHmainpush, clean tree. Report actual links, fixed-signing/iOS distribution setup requirements. No production deployment/live report success claim.

## Review / Acceptance
Execution Direct. Local feature scope/reuse/architecture/security/regression/complexity reviewed; release acceptance NOT READY pending fullworkspace results and new-source cloud native/publication. No new permission question is needed for already-authorized release.

## Last Checkpoint
2026-11-23T06:09Z: .11Release/mobile verification PASS; .12implementation/localnative verification complete, finalworkspace gates running before commit/tag.

## Pre-commit checkpoint
Full .12workspace1429PASS/26ignored and check exit0; frontend974PASS/1skip, launcher29PASS, nativeRust15PASS/2ignored, productionUIbuild/typecheck PASS. New UI always explains missing signing/distribution after checking. Final new APK split build1196pending; once PASS, stage only explicit product source/docs/version paths, review staged diff, commit/tag/SSHpush. .11Release fully verified and complete. Owned test emulator/AVD cleaned up; existing5554untouched. No12tag exists yet.

## .12 immutable publication checkpoint
2026-11-23T06:17Z: product74053d8e47e395a5f35da2280687821c03bdd2a5, tag object6f71f84b98f874561a87ec0bfd8ef5a83101d9da; remote SSH peeledSHA verified. Build/release45123101293 queued, Server image45123101291in_progress. Final split12APKs build PASS and restaged metadata/hash checks PASS; nativeRust15PASS/2existingignored, launcher29PASS, four fresh viewportlocale fixtures PASS. Source review scope/reuse/architecture/security/regression/complexity/staged hygiene PASS; no binaries/logs/keys committed. Product31files. Separate npmtypecheck exit0; workspace1429/26 result parser corrected for PowerShell UTF16 log encoding (test execution itself exit0). Main docs checkpoint [skip ci] push next. Do not recreate/move12tag; track exact runs and verify publication before claiming release complete.

## Current cloud tracking / user clarification
Main20be78c and immutable12tag74053d8 SSH pushed, local tree clean. User asked whether APK/IPA are separate from run45123101293: confirmed they are reusable-workflow jobs INSIDE this run, linked directly to Android137521172652 and iOS137521172529. Artifacts appear only after upload; one release job publishes them after all gates pass. No separate manual mobile run required. Active watcher71399; no new user decision/permission pending. .11publication already verified; remaining is12cloud validation/publication.

## .12 actual mobile cloud progress
2026-11-23T06:45Z: Android137521172652SUCCESS; actual dist-mobile-android.zip downloaded/ZIP integrity PASS. Both cloud APKs pass actual12version/identity/ABI/package and SHA256 checks, stable12/android-verification.json; payloads under12/mobile. They are Actions artifacts, not yet Release assets. iOS137521172529simulator build and Exercise native iPhone client flows bothSUCCESS; currently building unsigned device IPA. Actual new-source native iOS gate evidence12/ios-native-gate.json. Source74053d8 unchanged, no code fixes required. Latest root run17complete/13active/no failures; watcher71399keeps polling300s. Remaining: IPA staging/fullRelease+image publication/download/hash verification; never restart/move12tag. Main101f578before this documentation checkpoint, tree otherwiseclean.

# Task State — native client correction and v0.10.14-alpha delivery

## Objective / Confirmed scope
Complete native logo correction, Android HTTPS hang fix and polished connection page using actual shared language/theme components, then commit/SSH-push/publish and verify new packages. User said continue after outstanding publication report. Direct; no new agents/worktrees. Do not mutate production or live Codeup workflow. Existing published tags immutable.

## Current Phase / Current Step
TESTING / CLOUD PRE-TAG. Product main 505d6eafed3a4e827ba6dbbed347be0e50c66a75 SSH-pushed. Version manifests/lockfiles .14. NO .14 tag yet. Cloud standalone mobile45180277601: both Linux and macOS native icon byte checks PASS; Android job137711992345 FAILED Gradle buildSrc plugin resolution before app compilation: org.gradle.kotlin.kotlin-dsl6.6.4 not found. Native Rust ARM64 compilation itself succeeded. Diagnostic artifact14821611221 (4,122,631bytes) downloaded in verified ranges, android505-build.log retained. Official plugin POM/module fetched200now and declaresJVM17, matchingCIJDK17; local sameGradle9.6.1 build passed. Adding Gradle info logging and triggering one new main verification to expose repository failure or confirm transient resolution. iOS137711991982 still building simulator; read fresh status. Main root45180277539 pending. Next: inspect new Android retry and actual iOS native acceptance before tagging .14, then complete release/image validation. No icon-check relaxation or speculative Gradle version change.

## Completed implementation
- Android rustls-platform-verifier initialized synchronously through stable JNI helper in native plugin; JVM companion version derives from Cargo.lock. Certificate checking retained. Official .12 APK logcat RED reproduced uninitialized GLOBAL panic. New local packaged APK HTTPS update check, full online login, failure/editability/retry and shared language/local URL guard PASS. No credentials entered.
- Offline native launcher Vite bundle uses real LanguageSwitcher(preservePath)/ThemeSwitcher. Optional locale no-navigation mode preserves exact bundled URL; Web default routing unchanged. Form/card/error/update surfaces use theme tokens; existing native handlers/IDs/security guards retained. zh/en/ms desktop/mobile copy; remaining shared locale choices use English native-copy fallback.
- Native artwork derives from unchanged Web master. Pinned CLI gives filenames/dimensions/resource layout; integer area resampling with premultiplied colors and stable PNG/ICO/ICNS encoding removes CPU-dependent float rounding. Android clipping/margins preserved; iOS fully opaque white. Unit tests integrated into brand:icons:check; existing platform byte comparison unchanged.
- Dockerfile web COPY and .dockerignore now include desktop/ui needed by TypeScript native entry. Exact isolated Docker COPY input build passed theme generator, typecheck and production Vite; no Docker daemon installed locally.

## Root causes / failed immutable .13
- .13 product8564ff61ad3a64cca77c6b0fc248935ca02ad5aa; release45170715241/image45170715059. Docker TS2307 missing native launcher input fixed on .14main. Do NOT call .13 publication successful.
- Diagnostic main c39ba3a run45173565328 Mac artifact14818554355 (3,285,052bytes), ios-client-simulator-and-unsigned-device. Download via nightly.link artifact-ID route with8verifiedranges; run-artifact route404.
- Exact Mac vs Windows c39 blobs: Square142/284/310 and Android432foreground each differ by1 in one blue pixel; iOS1024two RGBpixels, plus ICNS1024payload. Pinned CLI uses floating-point resampling. Original iOSwhite outputs alpha254..255. New helper normalizes source raster and containers without tolerances/ignored assets.

## Local verification (actually executed)
PASS: full frontend128files/975tests/1existingSkip (maxWorkers2,239s); native JS31; shared i18n/theme12; native Rust15/2existingIgnored with unit-test TAURI_CONFIG disabling missing sidecars; typecheck,Web productionbuild,launcherbuild,desktopfmt,diffcheck; mobile packaging7tests/version binding; generator idempotence.
PASS: real Windows AND Mac52-file original outputs canonicalized to identical bytes;3Node image tests (all5PNGfilters, hand-computed transparent-edge resize, encoded pixels). Independent Pillow verifies PNGCRCs/allICO-ICNSframes/32pxcross-containerpixelidentity/all18iOS alpha255.128pxlogo visually inspected. Cloud Linux/macOS latest check now PASS as above.
PASS: earlier local ARM64+x86debugAPK builds and owned emulator5556 native HTTPS/login/update/error/retry/language acceptance; original5554 untouched.5556stopped after checking AVDidentity. Those local packages are modified .12 (before final icon normalization), NOT .14published artifacts or physical ARM64/iOS proof. Need fresh cloud .14packages.
PASS: actual rendered desktop and mobile handlers on1440/820/390/320,zh/en/ms,fourpalettes light/dark,custom,200%font,error/retry,draft/pathpreservation,no app errors/whole-page overflow. Screenshots inspected after theme transitions settled.

## Evidence / Reusable verification scripts
External C:/Users/FullStack/AppData/Local/ferry-verification/: native HTTPS JSON/logs, source local builds/tests, connection-ui/ and connection-ui-desktop/ screenshots+verification.json. v0.10.14-alpha/: runs.json, jobsJSON, mac-icon-diagnostics.zip, originalmac-icons/, mac-icon-comparison.json, Windows+Macnormalizedfolders, check-icon-canonicalization.mjs, compare-mac-icons.py, download-icon-diagnostics.py. Mac comparison reads c39Gitblobs; RGBA comparison uses all channels (not alpha-onlygetbbox).
Previous v0.10.12-alpha/ contains reusable watch.py, download-mobile-artifact.py/download-ios-artifact.py, verify-publication.py, verify-assets.mjs and verify-image.mjs. Adapt tag/run/source IDs, retain actual package provenance. Public API read works (~40requests/hour budget left at last check); use public HTML cards between API reads to avoid quota. Nightly.link artifacts support range downloads for slow CDN.

## Constraints / Tool limitations
No production Android key or Apple distribution/team; user explicitly deferred them. APK debug signing may prevent cross-CI coverage updates; device IPA unsigned, no physical iPhone direct-install claim. No store publication or signing credential invention.
Git remote SSH. Do not move .12/.13tags. Prior good .12product74053d8e47e395a5f35da2280687821c03bdd2a5, release45123101293/image45123101291 verified31assets.
Browser CUAhost unavailable; sky Chrome get_window_state automatic approval rejected because current URL allowed access could not be verified. Stopped UI input. Public GitHub API logs require admin; no credential stores/auth bypass. Explicit CI annotations/diagnostic artifacts used safely instead.
Optional user question pending: cancel obsolete .13runs45170714737/45170715241 to free capacity. No reply assumed. New jobs now running, so do not wait on that optional action. No background watcher remains.

## Review / Remaining Work / Next Action
Self-review local scope/reuse/architecture/regression/complexity/source hygiene PASS; ADR includes design/review matrix and deterministic pipeline rationale. Current working tree changes should be MEMORY only.
1. Read public mobile45180277601 job status; investigate any failing step/annotations and actual logs/artifacts; iOS includes simulator/native UI/unsigned device. Do not tag until mobile build accepted.
2. Tag .14 at accepted product HEAD and SSH-push tag. Monitor exact-tag rootrelease and both image architectures; do not assume main success equals tag success.
3. Download actual exact-tag mobile artifacts, verify versions/package identities/ABIs/content/icons, SHA256/size against published Release digest and SHA256SUMS, signed Runner manifest/updater references. Verify image source revision for amd64/arm64 andlatest. Keep immutable source identity in evidence.
4. Update release notes/state after real publication, commit docs [skip ci], SSH-pushmain, clean tree; concise final release link + signing limits.
## Last Checkpoint
2026-11-23:505d6ea pushed; real Linux/macOS icon checks passed; native packages in progress; no .14tag.

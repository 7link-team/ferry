# Native icon correction ready for cloud verification (.14 still untagged)

## Phase / Current source
IMPLEMENTATION -> TESTING. Main currentlyc39ba3a; local native-icon-images.mjs +3Node tests + regenerated52assets ready to commit. New helper replaces the preliminary local opacity-only normalizer (not retained separately). Existing default Web source artwork unchanged.
## Root cause / proof
Mac job in diagnostic run45173565328 failed and uploaded artifact14818554355, ios-client-simulator-and-unsigned-device (3,285,052bytes). Downloaded via nightly.link artifact-ID route using8verifiedranges; direct run route404. All original files in externalmac-icons/, ZIP retained. compare-mac-icons.py reads expected c39Gitblobs and reports six differing files: Square142/284/310 and Android432foreground each1bluechannel+/-1pixel; iOS1024two RGBpixels; ICNS embedded1024payload. Pinned CLI source confirms floating point resampling/premultiplication.
## Implemented / local verification
- Deterministic integer area resampling with premultiplied alpha, stable PNG/ICO/ICNS encoding, same pinned dimensions/names/Android margin-mask layout; iOS opaque white.
- True Windows+Mac diagnostic outputs (all52files) canonicalized byte-identically by local normalization test. No weakening of source-byte consistency check.
- PNG fixtures all5filters; transparent edge resize hand expectations; encoding identity:3Node tests PASS. brand:icons then brand:icons:check PASS. Independent Pillow: PNGCRCverify, all ICO/ICNS frames decode,32px pixelidentity acrosscontainers,18iOSicons alpha255 PASS. Viewed128pxlogo; appearance faithful.
- Normalizer has its own scripts/native-icon-images.mjs responsibility; exporter reuses it, no added dependencies. CI invokes Node tests through brand:icons:check.
## Next action
Commit only helper/tests/exporter/package script/generatedicons/ADR/MEMORY/.14notes; SSH-pushmain. Discover latest standalone mobile run, inspect actual Linux/macOS iconcheck and native builds. Only tag .14 after those pass; then monitor complete release+image gates and verify assets. No .14 tag yet. Prior .13 immutable and not accepted.

---
# Release correction ? v0.10.14-alpha preparation

## Phase / Evidence
RELEASE/DIAGNOSIS. Immutable .13 product8564ff61ad3a64cca77c6b0fc248935ca02ad5aa pushed; release45170715241 and image45170715059. Image build failed: Docker web stage omitted desktop/ui, causing TS2307 on native launcher import. Fixed Dockerfile COPY plus dockerignore exceptions. Exact Docker COPY isolation reproduced theme check, TypeScript and production Vite build PASS; actual Docker daemon unavailable locally.
Cloud .13 iOS stopped at native icon consistency check; API annotations did not include stale filename. Public job log retrieval needs authentication and browser access host unavailable. Added explicit checker failure annotation so next CI names the file. No credential-store access or approval bypass. Do not call this a successful .13 release.
## Current diagnostic continuation
Main f2e8cb2 pushed .14 manifests and Docker-context correction, then c39ba3a added retained native-icon diagnostic output to existing mobile CI artifacts. Still NO .14 tag. Main-mobile f2e8 run45172460785 failed Linux check explicitly: Square142x142Logo.png stale, so issue is cross-platform generation, not just iOS. Latest diagnostic mobile run45173565328, Android job137688944168 and iOS137688945356 queued at last check. Latest main root45173565853 pending. Download public diagnostic artifact using nightly.link after failure; compare generated vs committed PNG pixels/chunks before changing algorithm or assertions. No weakening consistency check.
CUA browser host unavailable; sky Windows Chrome get_window_state was rejected by automatic review because current browser URL could not be verified. Stopped UI automation. Public API logs require admin; public job page allowed but log endpoint404. Use explicit CI failure annotations and generated diagnostics instead. WSL absent; canceled installation prompt without installing anything.
## Additional verification / queue
Local iOS PNG alpha check found254..255 (including1024 icon), despite white background request. Implemented an RGBA8 PNG scanline normalization in export-native-icons.mjs using only Node zlib: preserve RGB, forcealpha255, encode PNG with valid CRCs. Regenerated all18iOSicons. Independent Pillow check PASS for fullopacity AND exact RGB/dimension preservation against c39 commit; repeated brand:icons:check PASS. These opacity changes remain local/uncommitted until cross-platform diagnosis integrated. External Linux comparison now reads expected c39 blobs, so local regenerated iOS files cannot falsify cross-platform comparison. Latest c39 Android/iOS diagnostic jobs still queued through20:40 local. User was asynchronously offered cancelling old .13 runs45170714737 and45170715241 to free Runner capacity; no reply/approval inferred. No .14 tag. No active long poller remains; public status reads and optional 45s clock sleeps only.
External compare-linux-icons.py downloads nightly.link run45173565328/android-client-build-diagnostics.zip to external evidence folder and compares bytes/RGBA extrema/counts; still404 because jobs not run. PNG samePixels uses all channel differences, not RGBA getbbox (alpha-only trap). Do not substitute a guessed algorithm or weaken checks before actual comparison.
## Next Action
Wait for c39ba3a diagnostic mobile outputs; download android-client-build-diagnostics artifact and compare Square142x142Logo.png (and remaining files) byte/pixel differences. Fix deterministic generation, validate on Windows/Linux/macOS before tagging .14. Preserve all old tags. Current .14 manifests and release notes prepared; local underlying native acceptance below. Cloud iOS and actual container build remain unaccepted.

---
# Release continuation — v0.10.13-alpha

## Phase / Scope
RELEASE. User said continue after local delivery and outstanding commit/push/publication report. Complete SSH main/tag push and cloud release verification for native HTTPS, shared themed launcher and brand icons. Existing .12 tag immutable. Direct; no new agents/worktrees.
## Verified / Decisions
Version manifests and both lockfiles now .13; release notes docs/releases/v0.10.13-alpha.md added. Fresh typecheck/native JS31/shared i18n-theme12/mobile packaging7/icon consistency/version binding/diffcheck/desktop fmt pass. Previous full975frontend and real packaged HTTPS/browser acceptance below remains valid for product changes. Generator preparation additionally made idempotent and SHA-checked twice. No signing material configured or invented.
## Next Action
Main/tag push complete; monitor all required jobs and image architectures, inspect failures, verify actual published assets/checksums/mobile identities and immutable source before claiming publication. Product commit8564ff61ad3a (full git identity authoritative), SSH atomic main/tag push SUCCESS. New immutable tagv0.10.13-alpha. Release Actions45170715241; server image45170715059; main CI45170714737; standalone mobile45170714659. Public API read works; existing gh API auth was unavailable in prior release, do not seek credential stores. Published assets not yet present; cloud verification pending. Do not expose credentials or bypass tool approval gates.
## Last Checkpoint
2026-11-23: precommit release review; no .13 tag existed on remote and origin/main matches local parent e3fcd6b.

---
# Native client follow-up — local acceptance COMPLETE

## Phase / Scope
ACCEPTANCE. Direct; no new agents/worktrees. Native logo correction, Android HTTPS hang and shared themed connection page complete locally. No new version/tag/commit/push/publication or production mutation.
## Implemented / Verified
- Official .12 APK RED: rustls-platform-verifier Android initialization panic. Fixed synchronous JNI initialization and Cargo-selected JVM companion. New packaged APK HTTPS update check returned normally, real https://ferry.7link.top rendered login, failure restored editable controls, shared language preserved exact local URL, retry reached login, no panic. No credentials entered.
- Actual shared LanguageSwitcher(preservePath)/ThemeSwitcher in offline Vite launcher; native guards/form IDs retained; Web locale default unchanged. Form/error/update surfaces use theme tokens. Desktop native copy zh/en/ms complete; other shared locale choices fall back to English native copy.
- Browser mobile AND desktop handlers each passed1440/820/390/320,zh/en/ms,four palettes/light-dark,custom panel,200%font,path/draft preservation,error recovery,zero app errors/whole-page overflow. Screenshots reviewed after transitions settled. Actual iOS/native desktop runtime not run on Windows; not claimed.
- Native icons derive from Web master; isolated export and canonical ICNS chunk ordering make generation repeatable. Android/iOS CI sync hooks present. Both APK icons pixel-match source and contain JNI/JVM verifier.
## Tests / Builds
PASS: full frontend128files/975tests/1existingSkip using maxWorkers2 (239.28s); native JS31; shared i18n/theme12; native Rust15/2existingIgnored using unit-test TAURI_CONFIG with no sidecars. typecheck,Web productionbuild,standalone launcher build,desktop fmt,diffcheck,repeated iconcheck PASS. First unconstrained frontend run Windows subprocess timeout was stopped; bounded rerun passed original assertions unchanged.
PASS: ARM64+x86_64 debug APK builds. Real native acceptance on owned emulator5556; original user5554 untouched. Both are local modified .12 debug packages, NOT published Release replacements and NOT physical ARM64 verification.
## Artifacts / Evidence
External C:/Users/FullStack/AppData/Local/ferry-verification/: apk-https-baseline.{json,log},apk-https-acceptance.json,apk-https-final.log,native-apk-content.json,native-final-android-build.log,native-ui-frontend-bounded.log,native-ui-rust-final.log,native-ui-web-build.log; connection-ui/ and connection-ui-desktop/ screenshots+verification.json. Ad hoc scripts there; no reports/logs/binaries staged.
ARM64: desktop/gen/android/app/build/outputs/apk/arm64/debug/app-arm64-debug.apk,168090817bytes,SHA256fd6d587b5b870c15d9f23b20f036cd955b0a172dffeef4f9bf2bfa58b3d8f732. x86_64: sibling app-x86_64-debug.apk,169386814bytes,SHA2568de22d823120ae09decd6c4346a231b78313cb44c8c8900d4622bd4bd32f4698. Ignored build outputs, not Git deliverables.
## Review / Next Action
Self-review and requested local-behavior acceptance PASS: scope/reuse/architecture/regression/complexity/state/hygiene. ADR records design matrix. Source and generated brand assets remain uncommitted. No database/auth/business/timezone semantics changed. User can accept local ARM64 debug APK on phone; previously deferred signing limits remain. Future publication needs normal version bump/tag/cloud desktop+iOS verification; none started here. Never imply existing .12 Release changed.
## Last Checkpoint
2026-11-23: all local verification completed. Confirmed5556 AVD identity ClientHttpsAcceptance20261123 and stopped only this owned test emulator; original5554 untouched. External diagnostic files retained.

---
# Previous release (completed; retain immutable identity)
# Task State

## Task / Objective
Complete release delivery with APK/unsigned IPA assets, then add the user-approved mobile version-check/update flows and configuration support. User confirmed no Android keystore or iOS distribution URL: implement flows and explicit missing-configuration states without inventing credentials or store links.

## Current Phase
ACCEPTANCE
## Current Step
COMPLETE. v0.10.11-alpha and v0.10.12-alpha publication verified. New mobile update feature shipped in12. All31cloud12jobsSUCCESS; actual Release31assets verified, including full APK/IPA payload byte hashes/package versions/identity/ABI/content, Runner manifest signature and desktop updater references. Both server image architectures/latest match product source. Final documentation checkpoint and clean-tree SSH main push complete after this record.

## Git / Source
- Immutable v0.10.12-alpha product74053d8e47e395a5f35da2280687821c03bdd2a5; annotatedtag object6f71f84b98f874561a87ec0bfd8ef5a83101d9da. Tag/main SSH pushed; subsequent commits are documentation only. Never move tags.
- Immutable11product60596a319a67dc868eb2204e68d7b111c0bcdef1. Both11and12public Release publication verified. Earlier8/9/10attempts remain immutable.
- No new branches/worktrees/agents. No production/original4291/live Codeup workflow mutation.

## Completed Work
- Shared native release config/types reused from desktop; existing desktop signature/update behavior retained.
- Mobile connection settings show installed version, check action, localized version/available/current/error/progress/retry states and release notes as text (zh-CN/en-US/ms-MY).
- Android matches configured repository/tag/channel/ABI, streams to private cache, checks published SHA256/size, then package identity/increasing versionCode/installed signing certificate before user-confirmed system install. Permission return/cancel/retry supported; no silent install/uninstall/data deletion.
- iOS checks versions and opens configured HTTPS distribution landing page. Missing URL disables installation with explanation. No unsigned direct-install claim.
- Exact bundled-document native guards reused; remote workspace receives no update IPC grant. Generator supplies scoped FileProvider/native plugin; no generated-file hand edits.
- Optional complete-set CI signing secrets: MOBILE_ANDROID_KEYSTORE_BASE64/STORE_PASSWORD/KEY_ALIAS/KEY_PASSWORD. Native config plugins.release.mobile.packageName/iosDistributionUrl. Build embeds only whether persistent signing was selected, never its path/key/password. Missing signing/distribution remains explicit even when no newer version exists.
- Provider test fixture polls queued work while writing cancellation via tokio::join. Deterministic held-transaction RED36.33s reproduced CI connection timeout; same probe GREEN5.93s. Probe removed; original50ms admission/3s cancellation/paid-request assertions retained. Production admission policy unchanged.
- Android native harness uses WebView-supported CDP operations, preserves original recovery checks, verifies HttpOnly sessions and denied remote update commands.

## Verification
- cargo test --workspace1429PASS/26existingignored; cargo check --workspacePASS; npm typecheck/productionbuildPASS; frontend128files/974PASS/1existingSkip.
- Native Rust15PASS/2existingignored; real HTTP hash/size/partial cleanup tests. Launcher4files/29PASS; four rendered viewport/locale cases PASS incl Malay320px/enlarged fonts, no horizontal overflow; screenshots inspected.
- Local AndroidARM64/x86_64split APK builds/content/version/hash PASS; owned disposable emulator14nativecasesPASS. Existing user emulator5554untouched; owned5556/AVD removed.
- Generator idempotence PASS; signing configuration tested with disposable key (removed); package fixtures7PASS; workflow YAML/syntax/diff checks PASS. No logs/binaries/keys committed.
- Actual12cloud Android and iOS(simulator/native interaction/unsigned device) SUCCESS. All31rootreleasejobsSUCCESS and all4imagejobsSUCCESS.
- Full actual Actions mobile payloads match every published Release digest/size; all31asset digests match SHA256SUMS. Runner signature/updater references PASS. Small manifests/signatures downloaded from Release; do not claim fresh full CDN APK downloads (CDN was slow; exact-tag Actions bytes matched published hashes).

## Actual Publication / Evidence
- Release https://github.com/7link-team/ferry/releases/tag/v0.10.12-alpha, published2026-11-23T07:57:35Z,31assets.
- Build45123101293source74053d8: all31SUCCESS. Image45123101291: all4SUCCESS.
- Official run artifacts Android14794234765/iOS14795168662; both inside the same rootrun via reusable mobile workflow (user question answered with direct task/artifact links).
- ghcr.io/7link-team/ferry-server:v0.10.12-alpha, actual amd64/arm64 revision74053d8; index/latest sha256ff6d3cd5a073002699899d71f0030a02646a174b1e40314caa97951fbf46100d.
- Stable12evidence C:/Users/FullStack/AppData/Local/ferry-verification/v0.10.12-alpha/: final-jobs.json, published-release.json, assets-verification.json, mobile-artifact-provenance.json, mobile-verification.json, image-verification.json, publication-verification.log. Full mobile payloads in mobile/ and root. Final validatorPASS. Watcher71399stopped after independently confirming all jobs completed; no polling remains necessary.
- Stable11evidence siblingv0.10.11-alpha/: .11fullrelease/mobile/imagePASS plus new feature local logs v12-*/mobile-update*, UI/native screenshots, controlled providerRED/GREEN. Earlier11local35browser suites and actual embedded template/Malay320/200%fontPASS; no business src UI changed in12.

## Confirmed Limits / Decisions
- User intentionally has no production Android key or Apple distribution link/team. APKs remain debug acceptance builds; device IPA remains unsigned, cannot directly install on a physical iPhone. Actual signed/store delivery was not performed. Cross-CI Android coverage updates require a persistent matching key; no retroactive debug-signature compatibility claim.
- Feature first requires manual installation of a compatible new client; old clients do not gain native update code through a server-only upgrade.
- Preserve workspace IANA/UTC/child-only-TZ policy, per-conversation API/CLI context, unlimited-default configurable resource policy and authorization/Runner liveness. No new database migration or business-flow change.
- Live workflow116765975084269568version2/report/PDF/WeChat/schedule not accepted here; old live timezone requires administrator confirmation. No production deployment claim.
- GitHub browser getApp earlier auto-review rejected unknown allowed URL; no UI inputs made. gh/GCM/env API auth unavailable; user reran11manually. Public read API and authorized SSH push were used. Do not search credential stores or bypass browser gate.

## Review / Acceptance
PASS for requested no-key flow/configuration support and Release delivery. Scope/reuse/architecture/regression/security/complexity/durable-state/staged hygiene reviewed; acceptance kept by Main Agent. Execution Direct. Source and final package/image identity match immutable tag. No new source work remains.

## Next Action
Nothing outstanding for this task; final docs checkpoint SSH-pushed and working tree clean. Future fixed Android keystore or iOS distribution configuration waits on the maintainer only when available; user explicitly deferred it, so no pending permission/question. Do not restart builds, retag or deploy production unasked.

## Last Checkpoint
2026-11-23T07:59Z: complete publication, package/image validation and final acceptance.

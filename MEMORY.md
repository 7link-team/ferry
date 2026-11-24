# Task State

## Objectives / Confirmed scope
1. Production Codeup weekly workflow116765975084269568/workspace111019986952126464: previous-week commits -> project/frontend/backend/App changelog and personal contribution -> attractive PDF -> ????-ferry, then Monday09:00 workspace-time schedule. Actual production delivery still required.
2. User follow-ups: execution history height/logging/prompt help and independent workspace name/avatar/timezone maintenance; already pushed4897b63, upgrade/browser adaptations2be1d0b.
3. Native v0.10.14-alpha publication COMPLETE. Later source changes require a later immutable version; never move tags.
4. Navigation/conversation continuity8c657ca and dashboard timestamp interpolation9719db5 already locally accepted and pushed. Latest request: another actual H5 audit and optimization for app-like interaction.

## Current Phase / Current Step
REVIEW / H5 local acceptance PASS; source ready for explicit commit/push. No production deployment/workflow writes occurred. Preserve all earlier decisions in ADR.md. Direct execution; no agents/worktrees added.

## H5 verified findings and implementation
- Actual baseline375x667 composer bottom726/navigation803,320x568 composer774/navigation851: min-content chat grid/feed minima pushed controls outside screen. Baseline screenshots conversation-pending-0fUzSf,375portrait inspected.
- Shared PlatformFrame measures visualViewport height/offset via rAF; focused editor ResizeObserver handles font/content changes. Invalid zero height ignored; absent API uses innerHeight; pinch zoom not resized away, readonly controls do not trigger compact input mode; listeners/attrs/CSS vars cleaned on unmount.
- Mobile frame fixed within visual viewport/safe insets; chat minmax0 grid and min0 feed keep message scrolling independent. Forms scroll internally. Focused editor in short/clipped viewport temporarily hides global chrome, restores after blur/viewport recovery; does not remount or rewrite drafts.
- Chat action/filter strips retain all controls in bounded horizontal scroll regions. Mobile composer label shortens send/queue (all6locales); accessible shortcut label and desktop behavior retained. Icon controls44px, tools strip>=44px even200%font; send text can shrink/wrap. Textarea visually constrained without overwriting saved manual height; mobile text fields>=16px.
- Conversation drawer and shared modal center/maxheight use visual viewport, including keyboard offset. Safe areas and enlarged text respected. No new backend/schema/audit/permission/business/API/dependency/config/retention/limit changes.

## Actual verification / Evidence
PASS full frontend130files991tests with1pre-existing skip. Final targeted Composer/frame/App57tests PASS. Unit viewport tests cover fallback, focus, readonly, zoom, offset, cleanup and retained editor. PowerShell CLI XML broke an initial full-test log capture; verified/cancelled only that wrapper, reran identical tests directly through Node with raw capture, exit0 in41s. LOCALAPPDATA/ferry-verification/h5-audit/unit-result.json and unit-tests-direct.log.
PASS final Web production build (includes typecheck) and embedded server build; logs in h5-audit. No Rust source changed.
PASS final enhanced H5 Chromium suite: desktop1440,Pad900,phone390/320; zh/en/ms375x667,320x568,667x375,375x360; visual-only keyboard360px with20pxoffset, drawer fit and keyboard dismissal;200%font/dark/reduced motion; send/tools/navigation bounds, independent message scrolling, no page horizontal overflow, drafts and288pxresize preference retained. Prior approvals/allow/deny/retry/badges/host probes regressions retained. Final screenshots LOCALAPPDATA/Temp/conversation-pending-EWFEZT; large-font/dark screenshot visually inspected after toolbar refinement. Tests use stateful API fixtures, no actual Agent execution.
PASS actual backend final CRUD(detail-rfB0Af): three configurations at1440/900/390/320; no document reload on edit/cancel/save;16pxmobile fields; actual PATCH/version preservation;409retains draft;EN/MS150%font no overflow/errors.
PASS actual backend final dialogs(dialog-layout-jSvgFI): long workflow/shared action bodies scroll independently while title/footer remain visible; visual-only keyboard360+offset20 preserves last input and dialog controls; group/alert create routes persist through ordinary API.
PASS actual backend table layout: empty/single/long/member rows, internal two-axis scroll and pinned pagination, opaque/frozen edges in6themes light/dark;1280x800/900x600/390x844. PASS CRUD layout fixtures1280/900/390/320sticky actions/detail-edit-create/read-only, wide deployment action reachability.
Self-review PASS: scope/reuse/architecture/complexity/regression/commit hygiene/LF/durable state. Local H5 final acceptance PASS. Figma unavailable, not claimed. Real OS keyboard/native installation and production deployment NOT performed.

## Production workflow / Outstanding blocker
- Last inspected real run118539555044327424(#3) passed period+commits, failed /nodes/3/params/prompt before analyze. Batch ai.agent.run has no current$json; saved prompt interpolates JSON.stringify($json). Real sandbox proves error. Recommend plain instructions (remove leading= and interpolation): Agent automatically appends all input JSON once. Explicit $input.all().map(item=>item.json) also tested but duplicates input.
- Initial#2 failure118383005512237056 was actual upstream401INVALID_API_KEY; native error preservation d27e621 pushed/tested, user repaired token. Do not assume timezone/Runner auth.
- Last inspected draft v2 still has Shanghai prompt/settings and old company-mobile target; must use configured workspace clock and ????-ferry. No actual analyze/PDF/delivery success or schedule enabled claim.
- No normal Ferry API/MCP login available; browser tools removed after URL-validation refusals. User personally editing/rerunning. Never bypass via production SQL writes, forged sessions or borrowed cookies. SSH read-only diagnostics authorized, sessions explicitly closed. Never repeat passwords or repeatedly ask to reopen Chrome.

## Publication / Cloud gates
- v0.10.14-alpha tag immutable at d27e6219f9c69e626dd1dc4812be594e8e396cb8. User reran original iOS failed job;45313356617attempt2 SUCCESS; exact-tag image45313356564SUCCESS. Public31assets verified: downloaded mobile payload digests/SHA256SUMS/version/package contents, signed Runner manifest and5target references,5desktop updater references/signatures. Evidence LOCALAPPDATA/ferry-verification/v0.10.14-alpha/assets-verification.json.
- AndroiddebugAPK/unsignedIPA reflect user-deferred signing: no production keystore or Apple team/distribution. IPA cannot directly install on iPhone. .14 excludes later4897b63/8c657ca/9719db5/H5 changes.
- Older main45318798793: upgrade/browser test failures adapted in2be1d0b; Windows job138192004806 also failed HTTP403rate-limit at previous-release lookup. Need inspect fresh cloud result before any speculative CI fix. Earlier new source2be1d0b run45322550654was in_progress at last check. Source8c657ca run45335634141pending/deployment45335634140in_progress at its last check. Refresh current exact-source CI after push; none of these stale statuses prove latest acceptance.

## Confirmed constraints
Workspace business timezone; UTC APIs/storage; new space browser suggestion editable, existing requires confirmation. No implicit Shanghai. Shared resource forms/tables authoritative. Preserve approval modes, budget/limit decisions, immutable release tags, previous native logo/HTTPS/update behavior and encryption-format compatibility decisions in ADR.md. Never deploy or mutate production database as a workaround for missing normal authorization.
Windows exec tty:true/login:false; hide child processes. LF/core.autocrlf=false. Explicit staging only, SSH push. No build/binary/log/temp files in Git. Audit/test artifacts external or ignored. Prior temporary old-source cleanup was policy-rejected; no bypass attempted.

## Next concrete action
1. Explicitly stage audited H5 source/test/i18n plus MEMORY/ADR, inspect staged diff, commit and SSH-push main. Working tree must contain no unrelated changes.
2. Inspect exact new main CI/deployment results; fix verified failures before preparing next immutable release. Do not mark old .14 as including these fixes.
3. Original production weekly report still requires normal authorized configuration and real analyze/PDF/enterprise delivery before publish/schedule. Local H5 acceptance does not resolve that independent goal.

## Last checkpoint
2026-11-24: final H5 screenshots/browser/backend acceptance and reviews PASS; awaiting explicit commit/push, cloud gates/release and production workflow acceptance remain separate outstanding work.

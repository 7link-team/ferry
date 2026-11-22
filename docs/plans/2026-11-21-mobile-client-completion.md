---
documentation_revision: "1.0"
organized_on: "2026-11-22"
source_updated_on: "2026-11-21"
---

# Full-platform client completion implementation plan

> Execute inline with superpowers:executing-plans, with independently verified stages. User explicitly authorized inspection and implementation in the active goal; no additional execution-mode permission is needed.

**Goal:** complete the outstanding stability/conversation work, then deliver tested Android and iOS client builds with user-configured servers and usable phone/tablet layouts.

**Architecture:** reuse Tauri 2, the existing connection preferences and launcher, and the server-rendered React product. Desktop retains service/client modes; mobile has only a remote client and no server/sandbox/Actions sidecars. Model history belongs to application conversation + Agent, with provider-native resume or persisted API history.

**Tech stack:** Rust, Tauri 2, React/TypeScript, existing axum service, Android SDK/NDK, Xcode CI.

## Requirements / scope

- No deployment-specific default server. Mobile empty address is invalid; Desktop empty address continues to select its bundled service.
- Validate and probe configured HTTP(S) endpoints, persist successful selections, restore on launch, recover access to connection settings when offline or changing servers. Usernames/passwords are entered through the selected server's normal login, not saved in connection settings.
- Same app conversation + Agent resumes one logical context. Different conversations with the same Agent never share model history. Preserve group member separation; do not equate new OS processes/HTTP requests with new model conversations.
- Real model-generated introduction on new single-chat/new Agent group membership once (explicit user choice); refresh and ordinary messages do not reintroduce/reset context. Reuse platform-maintained conversation policy and Agent descriptions; do not invent capabilities or hardcode global policy text.
- Complete previously approved default-unlimited/configurable resource work before final combined release. Preserve auth, approval modes, signatures, workspace isolation, business invariants and connection liveness detection.
- Phone320–767, Pad768–1199, Desktop>=1200: all core controls reachable, no whole-page horizontal scrolling; large tables/editors scroll only in named containers. Account for keyboard, safe areas, long translations and Android back behavior.
- No App Store/Play Store publication. Android deliverable must be installable; identify signing mode. iOS simulator/device builds and unsigned vs signed distribution must be clearly distinguished. The user confirmed no Apple team/certificate is configured: complete adaptation and unsigned simulator/device builds; do not claim physical-device installation.

## Existing implementation / reuse

- `desktop/src/server.rs`: `Preferences { server_url }`, connection.json persistence, isolated launcher commands, Desktop service lifecycle, remote workspace navigation.
- `desktop/src/lib.rs`, `desktop/Cargo.toml`, `desktop/tauri.conf.json`: desktop-only tray/plugins/sidecars currently unconditional; isolate by target before mobile compilation.
- `desktop/ui/{index.html,launcher.js}`, `desktop/launcher.test.js`: current connection and updater UI/tests; extend this flow, not a parallel app.
- `src/app/MobileNavigation.tsx`, AppHeader and resource/form/dialog components, `src/style.css`: reuse navigation and responsive contracts; audit rendered failures before edits.
- `ai_conversations`, `ai_chat_agents`, `ai_turns` and messages: existing native context and scoped message persistence. Normal CLI resume/group separation tests pass; at the inspected baseline, app conversation creation rejected API Agents. Do not claim workflow API support is chat support.
- Root checkout `fix/default-unlimited-resource-policy` contains substantial uncommitted approved work. Preserve it, verify and integrate surgically; do not overwrite it with main files or package it as accepted before checks.

## Execution / decomposition

Hybrid, staged execution under subagent-driven-development. Mobile Desktop-source unit and root resource-policy unit have separate file/worktree ownership. Read-only API investigation supplies evidence; root retains architecture/integration/acceptance. This revises the initial direct choice after execution-skill review and verified independent boundaries. Existing storage worktree is already isolated; feature branch `feat/mobile-client-completion` continues there. Frozen release `v0.10.6-alpha` is independent and must never be retagged.

### Stage 0 — finish storage hotfix release and deployment

- [x] Source acceptance:1125 server +143 other workspace +1 real NATS regression PASS.
- [x] Publish immutable version/tag b2912d2, launch release44864681192 and image44864681131.
- [x] On actual deployment host, disposable PG+NATS+old image reproduces exact storageClusterRequiresS3 rejection, execution503, total0; cleanup verified.
- [x] Wait and assess all required CI gates; inspect any actual failure, never weaken assertions.
- [x] Verify signed assets/checksums/version bindings and amd64+arm64 OCI revision.
- [x] Pull image, run same isolated authenticated workflow with local storage and NATS, verify answer2 and persistence after server restart.
- [x] Guard idle work, back up config/SQL/full volume, server-only Compose upgrade with unchanged env/keys/mounts/dependencies and old-image rollback. Verify actual app-user storage I/O and matching public/internal instance/version. No business report or notification execution.

### Stage 1 — complete resource policy and stability work

- [ ] Reconcile root dirty branch, its actual diffs and prior executed tests with MEMORY; integrate main storage fix while preserving optional limits.
- [ ] Run pending cancellation/lease, file collection, Runner, preview/SSH/cluster and configured-limit regressions; resolve actual failures before broad testing.
- [ ] Finish remaining source limit audit against the approved48-field ResourcePolicy; distinguish semantic bounds from resource caps. Complete actual large streaming/pressure acceptance and3-viewport platform configuration QA.
- [ ] Workspace check/test, frontend test/typecheck/build, generator checks; review staged deliverables, commit to feature branch and integrate with current main without losing either change set.

### Stage 2 — conversation continuity and introduction

- [x] Trace current API provider loop, conversation claim/lease/events/approval lifecycle and room history; document the exact reused execution contract before changing API behavior. Do not route API models through Runner or copy the workflow's GET-only policy into an interactive conversation.
- [x] Implement server-side API conversation execution using existing provider credentials, usage journal and platform tool authorization. Serialize rounds per conversation/Agent, preserve native role/tool/result history, use provider conversation handles only with verified scoping. Preserve known/unknown-call replay protection and existing approval modes.
- [x] Verify same pair multi-turn continuity, same Agent/different chats isolation, two Agents/group isolation, restart/cancel/disconnect/error recovery and authorization revocation; never silently discard history on failure.
- [x] Reuse platform conversation policy for configurable once-per-membership introduction and existing Agent description/name; introduction must not invent capabilities or reset native context. Reuse existing message sequencing/deduplication, with race/retry/refresh coverage and all6 existing locales. Backend80 conversation tests including9 introduction tests, shared UI regressions and independent source review passed; combined resource integration remains a later gate.

### Stage 3 — client-only native shell

- [x] Add failing launcher tests for mobile empty-address rejection, no local/updater controls, first-run configuration, restored URL, and retry/change server. Keep existing Desktop tests.
- [x] Target-gate tray/autostart/single-instance/updater and server lifecycle; expose a mobile entry point and remote-only connection behavior. Keep untrusted remote pages without privileged native command access.
- [x] Reuse durable connection preferences and launcher UI, add status/error/offline/settings recovery and3-locale strings. Use a single mobile webview and verified navigation callbacks; no iframe authentication substitute.
- [x] Add mobile Tauri config overrides excluding sidecars/desktop resources/updater artifacts; initialize native Android/iOS projects with official CLI. Change generated output only through generator/config inputs.
- [x] Install Android build in emulator and exercise fresh launch/address/login/restart/server change/offline recovery/back/navigation/file selection.

### Stage 4 — responsive acceptance and packaging

- [x] Audit actual login, shell/navigation, conversations/member panel/approval/questions/execution details, common CRUD forms/list views, host terminal/file tools, workflow editor and platform settings in3 locales at320/375/390/768/900/1440 widths. Current source rendered:126 general cases,27 nonempty conversation cases,144 intrinsic regression cases and36 platform resource cases. These are rendered UI fixtures, not backend authorization evidence; remaining findings stay open below.
- [x] Fix reproduced shared layout failures at component configuration/CSS level; no new unrelated redesign/table fork. Verify keyboard focus/input visibility, safe-area actions, terminal final row, touch44px and text expansion. The user explicitly approved viewport-scaled diagram controls:3locale320px actual zoom/pan/action cases pass; zoom controls remain44px and embedded targets enlarge from15.27px to45.59px. Final6representative chat cases also pass on the combined renderer.
- [x] Android emulator portrait/landscape + browser matrix; iOS simulator on macOS CI, arm64 device compilation and signing path. Keep screenshots/test logs as artifacts, not source files. Exact final Android APK/current renderer passed8 native theme/orientation cases and2 real IME Send flows; shell acceptance covers first configuration/restart/offline/back/file picker. iOS CI44907032751 passed12 native flows and cleanup; physical-iPhone installation is excluded by the confirmed unsigned-build scope.
- [x] Build final Android APK/AAB and iOS simulator/device artifacts against the verified combined version; verify contents contain no server sidecars, correct identity/version/checksums and server setup remains blank initially. Android arm64/x86_64 APKs (debug signed), unsigned arm64 device IPA and arm64 simulator app ZIP are verified in Downloads/Ferry-0.10.7-alpha, with source provenance and SHA256SUMS. APK is the Android deliverable; no store/AAB publication was requested. iOS numeric bundle version0.10.7 accompanies package version0.10.7-alpha.
- [ ] Run original-request acceptance, self-review, clean/stage scoped source, SSH push, final release gates and deliver actual paths/links with truthful signing/platform limitations.

Current acceptance checkpoint: the user explicitly chose preserving zoom-scaled diagram controls and zooming in to operate; this is a canvas-only touch-size exception, not global44px compliance. Final model/introduction/resource/common-form144 cases and legacy-setting27 real API/UI cases pass. The dedicated platform-admin fixture preserves ordinary-actor403 and full Linux pipeline CI passes. The3locale zoom-and-operate smoke and6representative conversation checks also pass. Remaining checks: final source CI/integration and delivery audit. No main release or production upgrade has been claimed.

## Files to change / add / preserve

- Modify existing desktop Cargo/config/lib/server/launcher/test files; shared responsive components only where rendered evidence requires changes; conversation execution/policy/messages/groups and tests; existing resource-policy paths from the approved branch.
- Add only real boundaries: mobile-specific native connection/lifecycle module/config where Desktop reuse requires separation; conversation API executor if existing provider code cannot host both flows; mobile CI/build acceptance scripts; source i18n entries. Determine exact API interfaces from Stage2 source before edits, not invented signatures.
- Preserve business resource models, workspace authorization, existing Desktop service/client behavior, production secrets/data, signatures, old release tags and unrelated historical tasks. No unrelated dependency upgrades, filesystem migration, store publication or push messaging.

## Design review gate (applies to each stage/flow)

| Dimension | Source-backed contract |
|---|---|
| Facts | Sources above; API gaps/mobile target behavior require source/real-build verification, not external samples |
| Scope / exclusions | Named stages only; no store publication/offline server/push notifications/unrelated redesign |
| Rollout | Existing release/version delivery, no new flag; frozenv6 unchanged |
| Flow closure | Mobile unconfigured→checking→connected; invalid/offline→editable error; restore can return to settings; no blank-address service branch on mobile |
| Exceptions/extremes | Empty/invalid URL, unreachable/invalid server, long hostname/translation, tiny screen, keyboard, large list, interrupted session |
| Audit | Existing server resource/session audit; connection preference is local-only; no secret values in diagnostics |
| Deletion/retention | Existing server retention unchanged; changing client server replaces selected local URL, no remote data deletion |
| Permissions/isolation | Existing workspace/user/Agent role intersection and approval modes; remote content receives no native privileges; context scoped per conversation/Agent |
| Calculation | No new monetary calculations; preserve usage records and approved removal of monetary admission |
| Mode differences | Desktop service/client unchanged; Android/iOS remote-only; new/restore/change/error modes defined above |
| Field linkage | Server URL user-supplied, no default, required on mobile; save after verified connection; descriptions/policy from existing settings |
| State operations | Prevent duplicate connect/intro/turn/replay; changing address keeps ability to retry; existing cancellations and permissions stay authoritative |
| Validation | Reuse URL/parser/API compatibility and existing policy validation; no invented resource ceilings; exact missing native signing inputs remain explicit |
| Data model | Reuse connection.json; private API context and turn introduction provenance use additive guarded migrations documented exactly in ADR.md, entities.rs and migration/{chat_api,ai_model_output,introductions}.rs; no historical rewrite |
| Time audit | No new audited SQL model planned for mobile; existing resource-policy migration and any API/message change must retain established audit conventions |
| Indexes | Scoped API context identity uniqueness and introduction-chain lookup indexes are documented in ADR.md and generated by guarded migrations; existing client-id uniqueness reused |
| SQL examples | No mobile DDL; exact API context, model output and introduction DDL derives from the ADR constraints and migration source, with guarded non-destructive down paths |
| Copy | All new launcher/server/UI text covers zh-CN/en-US/ms-MY, including long Malay copy; configured template content stays user-maintained |
| Config sources | User server URL; existing platform conversation/resource policy; build signing configuration; no deployment endpoint hardcoding |
| All states | Default/loading/empty/error/offline/cancelled/succeeded; unsupported API currently reported as gap until implemented |
| Core impact | Connection/login/conversation/terminal and build output; test Desktop and server regressions |
| Historical data | No mobile data rewrite; preserve old settings and schema guards; resource-policy migration already approved separately |
| Third parties | Existing Tauri/OS toolchains/provider APIs; signing depends on Apple credentials; no new business integration |
| Compatibility | Old Desktop prefs migrate unchanged; server protocol probe; no Runner allocation for pure API; old tags immutable |
| Conflicts | Mobile cannot use tray/multiple windows/updater/sidecars; isolate those code paths rather than stub server behavior |
| Testability | Executable stages above, native emulator/install/restart and API isolation tests; compiler success alone is insufficient |
| Traceability | Stage tests and artifact/screenshots map to each requirement; Figma not provided, no false coverage claim |
| Responsive delivery | Desktop1440 regular toolbar/sidebar; Pad900 collapse/wrap; Phone320/390 single column, bottom navigation, confined data scrolling, safe areas/keyboard |
| Ambiguity | No Apple signing configured: unsigned builds confirmed; model output parameter placement, model-generated greeting, and storage-format downgrade limitation explicitly confirmed. Native and large-file acceptance remain pending actual results |

## Risks / acceptance

Apple device installation requires valid signing; unsigned IPA is not reported as installable. Native browser automation pipe is unavailable and Browser plugin absent, so use documented Playwright fallback plus actual Android emulator and macOS CI simulator. Pending resource work and new API chat are substantial; maintain separate tested checkpoints and never imply frozen storage release contains them. Acceptance requires actual produced binaries and evidence for every stage, not just this plan.

## Current delivery checkpoint (2026-11-21)

Source commits 5dc524e/abe6112/da611df/d5cce8a are SSH-pushed on the feature branch, not merged or deployed. Latest shared UI945 tests pass/1 existing skip and production build passes. Android core/native bounds/IME tests pass but intrinsic small-portrait containment and native double-inset handoff are being repaired. iOS compiled; first native run exposed bundled-root rejection, now repaired and awaiting actual CI rerun. Resource normal default-stack1329 tests pass/26 existing ignores, but three independent storage findings (clock, multipart, journal writer transactions) and final >1GiB runtime remain blockers. See compact active MEMORY.md for exact owners, logs and next commands. No blanket acceptance claim.

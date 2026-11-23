# Task State

## Task / Objective
Priority: make production Codeup weekly workflow 116765975084269568 actually succeed through PDF and enterprise notification, Monday09:00 workspace time. Keep native v0.10.14-alpha release pending; no immutable tag moved. Workspace111019986952126464. Latest channel requested by user: enterprise chat ferry (not old company-mobile).

## Current Phase / Current Step
TESTING / CLOUD: local native authentication diagnosis fix verified and SSH-pushed in d27e621; launcher CI order fix in94e6033. Working tree checked clean after source push. Production workflow has NOT been fixed, rerun, published or scheduled in this turn.

## Confirmed facts / Decisions
- Execution118383005512237056 period failed after31s. ASPEED native thread01a2092d-aa97-7db1-b423-ace024e4516a has six upstream401 INVALID_API_KEY responses. Root issue is configured model-service credential rejection; no evidence of timezone/Runner auth causing this execution failure.
- Agent115434614507438080 uses model115398114952609792 (sub2api codex CLI), provider115397742670381056 and credential115397742666186752 (sub2api codex token). Credential last updated Nov15; no custom model header overrides. ASPEED and Mac Runners online0.10.12; server0.10.12.
- Production draft v2 still explicitly embeds Shanghai date prompt/settings and old company-mobile notification channel. Final graph must use workspace frozen clock, requested enterprise channel, verified PDF/delivery, then Monday09:00 schedule. UTC persistence/workspace business zone remains user policy.
- Normal platform API authorization required for config writes and runs; do not bypass with SQL, forged identity/session or borrowed browser cookies. Read-only SSH diagnostics authorized. No usable Ferry MCP/API login currently. User asked local testing first; do not block independent tests on login.
- Fix reuses existing Session::Failed. Terminal Codex authentication failure becomes static allowlisted localized identity; raw diagnostics never forwarded. Failed protocol/process execution must not send Finished first (server reader stops on Finished). Six backend locales supplied, workflow end reason remains agent_failed. No wire schema/dependency/database/config changes.

## Completed Work / Actual Verification
- RED regression demonstrated lost terminal auth cause; GREEN Runner agent34 tests.
- Full Runner cargo test:113 passed/10 existing ignored, CLI5 passed. Two opt-in real-process checks also explicitly run: valid custom upstream model/path/auth header matches; upstream401 yields actionable safe error with no premature Finished. No test-only credentials reused in production.
- Real local Codex date prompt: since2026-11-16T00:00:00+08:00 until2026-11-22T23:59:59.999+08:00; independent calendar assertions pass. First scratch script mangled Unicode through PowerShell stdin; corrected using escaped Unicode, not a product defect.
- Real sandbox typed-expression test and published workflow example save test PASS with absolute FERRY_SANDBOX_PATH. Transport error localization and preview guidance PASS. PDF8 tests PASS; exported two-page existing renderer sample visually inspected with PyMuPDF (Poppler unavailable), no broken CJK/layout. Sample is synthetic renderer evidence, NOT real weekly report.
- cargo check --workspace PASS (existing warnings); cargo fmt --check and git diff --check PASS. Workflow suite:183 passed,3 failed due missing sh/cygpath,3 ignored. After process-local Git PATH correction, all13 host tests including the3 failures PASS; all3 ignored real-sandbox tests explicitly run PASS. Thus every workflow test exercised successfully across these runs. No assertion/source change for shell setup.

## Remaining Work / Next Action
1. Local self-review: scope/reuse/architecture/regression/complexity/durable state PASS. Original production end-to-end acceptance NOT READY: credential rejected, actual PDF/delivery/schedule unverified. No test processes remain.
2. Read new main d27e621 Build/release45226851695 (queued at last check) and deployment acceptance45226851603 (in progress). Source commits94e6033+d27e621 SSH-pushed. No .14 tag until native/root CI accepted. Only explicitly stage deliverables.
3. Production needs an accepted API key in existing credential sub2api codex. User can update through platform UI without sharing secret. Once normal API authorization exists, repair graph timezone/channel and run actual chain; verify real PDF and enterprise delivery before enabling schedule. Do not claim local tests equal this acceptance.
4. Complete pending native release below, preserve separate task.

## Pending native v0.10.14-alpha publication
- Main origin d27e621 (auth diagnostic fix, preceded by94e6033 launcher CI order); previous origin9ca4091d97. Versions .14; no .14 tag. Product505d6ea deterministic icons; f2e8cb2 Docker context; c39ba3a diagnostic artifacts. .13 tag8564ff61 immutable, publication failed; previous .12 release verified31assets.
- Latest mobile45181740341 source9ca4091 SUCCESS AndroidAPK+iOS simulator/native acceptance+unsignedIPA. Prior transient Gradle plugin failure resolved on retry without version downgrade.
- Main45181740148 failed Linux desktop137759371274 and MacARM137759371668 at direct Cargo tests: desktop/ui-dist absent before bundler. Uncommitted build.yml adds npm run build:launcher after Web build and before desktop tests. Local clean-dist RED/GREEN native15pass/2existingIgnored evidence recorded.
- User cancelled obsolete .13 builds45170714737/45170715241; no pending cancellation action. Do not re-ask.
- Product already implemented: rustls Android JNI init (oldAPK panic RED/new localAPK HTTPS/login/update/error/retry GREEN); native launcher uses actual shared language/theme components, preserves bundled URL;52 icons deterministic Windows/Mac bytes including opaqueiOS; Docker COPY includes desktop/ui. Local full frontend975pass/1skip, nativeJS31, shared12, nativeRust15/2ignored, browser1440/820/390/320 three locales/themes/200percent font PASS. Those are earlier actual results, not newly rerun this turn.
- No production Android keystore or Apple team/distribution: user deferred. APK debug, IPA unsigned; never claim direct iPhone install or cross-CI signing compatibility.
- After accepted new main/tag CI: publish .14 immutable tag, verify exact-tag mobile/desktop Release assets/hash/source and both image architectures/latest, then clean state/docs. No .14 publication claimed yet.

## Evidence / Tools / Constraints
External evidence C:/Users/FullStack/AppData/Local/ferry-verification/: workflow-116765975084269568 has realCodex/date/mock401/PDF rendered evidence; v0.10.14-alpha has cloud diagnostics and native build RED/GREEN. .12 folder has reusable public artifact/release verification scripts. Public GitHub API/nightly.link range downloads usable; logs require auth. CUA host unavailable and earlier browser review denied address validation; no credential bypass.
SSH root192.168.0.4 session18954 was explicitly closed after diagnostics. Never repeat password. Postgres container ferry-postgres-1 DBferry userpostgres; server ferry-ferry-1. Limit queries to target workspace/resources; no secret printing. Native Runner state LOCALAPPDATA/ferry-runner-agent, target provider home contains exact failed Codex logs.
Direct execution; no new agents/worktrees. LF, explicit staging, SSH push; preserve preexisting build.yml fix. ADR auth diagnosis design persisted.

## Last Checkpoint
2026-11-24: confirmed401 root cause, local date/auth/PDF/sandbox checks passed; all workflow cases exercised successfully (shell PATH corrected for3); production remains unverified; release outstanding.

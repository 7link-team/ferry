# Desktop unattended Windows updates — implementation plan

> Execute directly using executing-plans and test-driven-development. Independent review of the privileged boundary and final implementation. User approved this approach on 2026-10-28; do not re-request approval for implementation.

## Goal / Requirements
Desktop upgrade completes automatically after graceful work drain. No maintenance/scope wizard. CurrentUser uses ordinary privileges; AllUsers uses a protected native update service installed during the initial administrator-authorized install. Existing installs may need one authorized migration. Preserve user scope/directory/data, restart in the original user's session, verify version health and roll back failed activation.

## Existing implementation / Reuse
`desktop/src/updater.rs`: Tauri signed download with signed-version binding, runtime drain, resume marker; NSIS supports both scopes and passive update/restart. Runner already supplies tested Windows SCM and protected-directory ACL patterns. Reuse Windows SCM, Tauri/Minisign key, existing NSIS packaging, and native Windows acceptance infrastructure. Do not grant the desktop administrator rights or reuse the host runner's network command channel for privileged desktop upgrades.

## Execution / Files
Direct implementation: IPC/installer/lifecycle boundaries require one owner. Independent security review before final acceptance.
- Add `crates/desktop-updater` for a small separately executable Windows broker/unelevated helper; compile on other platforms for signature/policy tests, no non-Windows install changes.
- Change workspace/lock, Windows packaging, `desktop/src/updater.rs` and desktop helper handoff module, NSIS installer, native acceptance scripts/workflow.
- Preserve T15/list uncommitted work, K8s/Helm exclusion, AgentRunner functionality and macOS/Linux desktop update flow.
- No local heavy compilation. Test/build in Actions. Existing public alpha11 stays unchanged until all next-alpha gates pass.

## Design / privileged boundary
- SCM service runs under SYSTEM; immutable registered install root and per-product protected state. Configure identity/public key at build time from existing desktop config. Normal callers cannot provide executable path, destination, shell command or installer arguments.
- Local named-pipe transport rejects remote clients, bounds headers/payload/time, serializes updates, and authenticates caller session for any process handling. Receive bytes into an administrator/SYSTEM-only staging directory, independently verify Minisign bytes and signed version/Windows installer filename, reject downgrade/relabel/tamper before running anything.
- Installer command is fixed silent/update/AllUsers/fixed-directory with restart suppressed. The original user's unelevated helper performs desktop restart; SYSTEM must not launch the UI. Other users' busy app/service processes must not be forcibly killed to make installation succeed.
- Desktop downloads/verifies first and drains active local work. Helper obtains parent process handle before exit and waits for actual old-process exit. CurrentUser helper runs silent installer without elevation; AllUsers helper submits to broker. Broker/assistant cannot treat an installer exit alone as successful health.
- Protected journal tracks pending/current/previous and recoverable payloads. Keep a trusted rollback baseline established by initial elevated installation; later updates retain verified prior installer. Failed/interrupted activation recovers deterministically; preserve user data. Reject arbitrary file redirection/reparse points and mutation races during verification/execution.
- Service binary lifecycle uses versioned protected helper executables: installer can update SCM image path without overwriting the running broker; switch only after transaction completion. Uninstall removes registration safely. Do not deadlock service stop against its running installer.

## Tasks / Tests / Acceptance
- [ ] Signature/admission policy: genuine fixture signatures; wrong key/product filename/version, downgrade, malformed/oversize request and runtime-key override rejected. Prove failures before implementation.
- [ ] Native broker: install/start/uninstall, protected storage and named-pipe permissions; standard user can submit only valid fixed-scope update, no arbitrary executable/path/arguments; interrupted stream leaves no executable action. Test as a genuinely non-elevated user, not an admin CI process.
- [ ] Helper+desktop: parent exit after existing graceful service drain, immutable package handoff, original-user restart and health/rollback protocol. Busy other sessions abort instead of force-kill. Regress live task drain.
- [ ] NSIS hooks: first AllUsers install registers updater with existing initial UAC authorization; CurrentUser registers no privileged service. Reinstall/update/uninstall are idempotent. Suppress installer restart in broker mode; keep legacy client update compatibility.
- [ ] Windows actual two-version acceptance: both installation scopes, standard-user update with no elevation prompt/maintenance wizard, scope/data preserved, automatic user-session launch, failed installer/health rollback, interrupted update recovery, hostile pipe inputs and user-writable/reparse paths refused.
- [ ] Independent review, full required CI and next-alpha integration after T15/T16/UI gates.

## Review focus / Risks
Privileged execution of an attacker-controlled path; signature/TOCTOU and downgrade; user-session restart versus SYSTEM; service self-update deadlock and locked executable; concurrent user activity; install success but failed health. Tests must prove these boundaries before claiming unattended AllUsers updates work.

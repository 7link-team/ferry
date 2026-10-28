# T16 — 本次 alpha 的 Agent Runner 生命周期

## Goal
用户明确要求 Agent Runner 服务安装、自动恢复和平滑升级本版本交付，不能后延。

## Confirmed Requirements
- Linux、macOS、Windows 一键安装为系统服务，开机自启。
- Agent 进程异常退出后由系统服务管理器重启；网络断开后沿用已有重连。
- 自动升级不主动打断在途任务：停止接收新任务、等待原任务完成、验证更新、切换版本、健康检查、失败回退。
- Agent 切换时可短暂离线；不宣称零断连或任意失败任务自动续跑。
- 与代码连接修正 T15、已完成本地验收的共享表格修复一起纳入下一 alpha；不能把仅能手工启动的 Runner 当作此项交付。

## Existing Implementation / Reuse
已有：ferry-runner 注册/长期凭据、心跳和1–30秒重连、独立会话、HostChannel、平台与架构识别。CI发布 Linux x64/arm64、macOS x64/arm64、Windows x64 五个 Runner 二进制。桌面更新已提供签名发布经验，但不等同于 Agent 自更新。
缺失：安装服务命令及安装脚本、系统服务托管、Runner 更新协议和产物签名验证、任务排空和版本切换/回退。

## Design / Execution
使用现有 Rust workspace 和协议，在系统原生服务管理器上实现 Linux systemd、macOS launchd、Windows SCM；不另外引入托管平台。默认延续已有注册身份，重复安装不能消耗新令牌或覆盖身份。安装权限按系统服务所需处理，测试不修改开发者真实系统服务。
升级必须按整项任务排空，不仅按当前 WebSocket 会话计数（流水线步骤之间也属于在途任务）。保留活跃任务的后续通道访问；禁止新任务进入待升级主机。升级下载/验证失败保持旧版本，切换失败恢复旧版本，不无限重试坏版本。版本、平台和架构必须绑定校验。新旧协议兼容和原有命令行启动均需回归。
Direct implementation initially: lifecycle/protocol boundary needs one owner; final independent review after actual verification. No new worktree/disk-heavy duplicated caches.

## Tasks
- [ ] 先写服务配置/安装重复执行/身份保留的失败测试，再实现原生服务安装与生命周期命令。
- [ ] 实现平台匹配的安装脚本、Runner 下载及完整性/签名校验；主机页提供可复制安装命令。
- [ ] 实现服务端任务排空、Agent会话跟踪、升级状态和版本状态；兼容旧Runner。
- [ ] 实现自动更新、健康检查、失败回退和故障恢复；持久化状态以处理断电/进程中断。
- [ ] 本地隔离集成测试，并在原生Linux/macOS/Windows CI中验证安装/启动/重启/升级/回退；无法运行的项目明确标为未验收。
- [ ] 与T15新代码连接链路执行SSH/Agent两路回归；自查、独立审查、需求验收、发布新alpha并验证下载产物。

## Release Acceptance Gates
- 服务安装成功、重复安装安全、开机自启注册有效。
- 杀死Agent后自动恢复且保持同一主机身份；断网和服务端重启自动重连。
- 长任务执行中发起升级，原任务完成并保留日志/产物，再切换；新任务不会误派给正在升级的Agent。
- 更新包篡改、错误平台/架构/版本、不兼容协议均拒绝；旧服务仍可用。
- 新版本启动/握手失败回退，状态可见；升级过程中重启能恢复到确定版本。
- 旧命令行Runner、SSH、权限/租户边界、Actions任务取消与清理不回归。
- Windows系统服务验证不等于Windows Actions已支持；不扩大现有Actions操作系统兼容范围。

## Current Evidence
2026-10-28：完成代码与CI调查，尚无T16产品代码修改，以上服务和升级验收未运行。已有执行通道验收不作为T16完成证据。

## Service-stage implementation checkpoint
- Native service CLI implemented (install/start/stop/status/uninstall), preserving existing foreground arguments and host identity. Reinstall does not replace a running executable; update lifecycle remains a separate pending stage.
- Configuration binds service name and user/system scope. Reject same-name/different-directory native registration, unsafe filesystem ownership/write permissions and shared identity directories. Native Windows SCM recovery uses required START permission; Windows configuration and ACL handling still require native CI evidence.
- Runner SIGTERM/SCM stop stops receiving sessions and waits for active sessions. This does not yet provide whole-pipeline upgrade draining.
- Local real macOS LaunchAgent acceptance: registration, startup, repeated install without restart, kill/recovery, explicit stop/start, uninstall retaining identity. Native CI for systemd/launchd/SCM added as a release dependency. Full reboot and update rollback not yet tested.
- Workspace tests:141 passed,1existing live-SSH test ignored; macOS service lifecycle passed after independent-review fixes. Linux/Windows native results pending; not release-ready.

## Native service and reservation acceptance — 2026-10-28
- GitHub Actions run `40630691384` at `2ddbeb0`: Ubuntu 24.04, macOS 15 and Windows 2022 jobs all passed. Native acceptance covers install/enrolment, reinstall without restart or identity replacement, crash recovery, explicit stop/start, uninstall preserving identity, and rejection of conflicting service paths. Windows also rejects user-writable state directories.
- Each platform runs runner unit/CLI tests, four reservation tests, and two real database/stop cancellation tests. Whole Actions tasks reserve their hosts; download streams and terminal pumps retain shared leases. Waiting tasks respond to cancellation without aborting active-task cleanup.
- Windows installation isolates Windows PowerShell module lookup from PowerShell 7 and recognizes OWNER RIGHTS only after verifying the actual owner. Independent review found no remaining blocking issues in this stage.
- Build/test execution moved to GitHub Actions after local disk cleanup. This stage does not implement signed agent updates, update health checks/rollback, installation download UI, or a complete machine reboot test. Those release gates remain open.

## Signed updater implementation plan
Goal: extend the approved graceful update flow with verifiable release artifacts and recoverable version switching.
Reuse: existing desktop Minisign release key, GitHub release assets, native service registration, protected state directory, task reservations and existing enrolment identity.
Execution: direct implementation of tightly coupled protocol/lifecycle code; independent review of each security/lifecycle stage. No local heavy builds or duplicated worktrees.

- [x] Release catalog: produce `runner-update.json` with schema/protocol/version and exact five target artifacts, byte sizes and SHA-256 hashes. Sign the catalog with the existing release key; publish its signature. Verify actual catalog bytes before parsing/trusting metadata, then verify target/version/protocol/size/hash before any executable is staged. Tests include tampering, relabelled version, wrong platform/protocol, truncated payload and downgrade rejection.
- [ ] Version switching: native service retains a stable supervisor executable, starts a runner from a protected version directory, and journals current/pending/previous versions atomically. New process must authenticate back to the server before confirmation. Failed startup/handshake rolls back; restart during a pending update resumes rollback rather than trusting an unconfirmed executable. Windows never overwrites an executable while it is running.
- [ ] Server control: authorize updates through existing workspace host permissions; acquire exclusive host reservation after complete tasks drain, request staged switch, retain exclusion until confirmation or rollback. Advertise updater capability so older/foreground runners receive an explicit unsupported result.
- [ ] Install/update UI: reuse the host page and registration tokens, provide platform-specific install commands and show version/update state; no new standalone management page.
- [ ] Native Actions acceptance: validate signed staged artifacts, successful upgrade, invalid artifact rejection, failed-new-version rollback, interrupted switch recovery and in-flight task completion on Linux/macOS/Windows. Release remains blocked on these gates and T15.
Files: `.github/scripts/runner-manifest.mjs`, release/native CI, `crates/runner/src/update.rs`, existing service/main/protocol/server runner files and host form as needed. No K8s/Helm or unrelated UI work.

### Signed-catalog acceptance — 2026-10-28
- CI `40635356494` at `7d03ccf`: Linux, macOS and Windows all passed expanded signed-update verification, actual CLI checks and existing native service/reservation/cancellation regression. Throwaway keys used only in CI; no release key or fixture-key executable is published by these tests.
- Release pipeline checks package/Cargo/tag agreement, signs the catalog with the existing release key, then verifies with the fixed public key and regenerates metadata to match every artifact. Independent review found no remaining blockers for this stage.
- This proves update authenticity/integrity/version/platform validation, not automatic service version switching or rollback. Those tasks above remain open.

### Supervisor implementation details
- Native service entry remains stable and supervises one versioned worker process. The existing runner executable gains a hidden managed-worker entry; normal foreground invocation is unchanged.
- Parent-to-worker stdin EOF requests graceful stop. Worker-to-parent stdout carries readiness only after authenticated Welcome, and an activation request only after old sessions drain. Logs remain stderr. Unexpected worker exit restarts through the existing native service recovery policy.
- Signed staged artifacts are immutable under `versions/<sha256>/`; parent re-verifies before launch. Journal atomically records current/previous/pending. Startup discards unconfirmed pending activation and launches last confirmed version. Candidate failure or health timeout rolls back; confirmed version advances only after matching version readiness while child is alive.
- No direct service-stop/update command will bypass whole-task draining: network activation is requested through the existing authenticated runner control connection after server exclusive reservation (server integration follows the supervisor stage).
- Tests must exercise real child processes, readiness, failed child, interrupted pending journal, graceful stop, and existing native service recovery on all three CI systems.

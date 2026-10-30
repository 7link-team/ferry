# Project Memory

## Release gate repairs — 2026-10-30 08:09 +08
- Main HEAD e9dd64d was queued behind 2d05230 run40975364895. No alpha14 tag/publication or installed upgrade yet.
- Reproduced MySQL startup failure in isolated temporary MySQL8.4 cluster: unshipped host_connection_command_v1 added TEXT DEFAULT '' (rejected by MySQL). Changed fixed-size sealed enrollment token to VARCHAR(512) DEFAULT ''; new/old SQLite and PG types remain compatible. Real SQLite/MySQL8.4/PostgreSQL16 acceptance each123 PASS; temporary DBs stopped, existing business DB data untouched. Logs TEMP/ferry-db-acceptance-h_k8ljqe/suite.log.
- Browser Marketplace-stage actual Actions regression reproduced locally: restricted editor returning to list requested deployments/tags filters without read grants (uncaught rejected options); added permission filtering and visible option-load error handling. Monaco0.54 WordHighlighter Delayer.cancel rejects an ignored debounce promise on mode disposal; disable that optional occurrences highlight, retaining YAML syntax/validation/completion. Tried explicit disposal order, did not fix; reverted it. Current minimal fix real scripts/test-actions-ui.mjs PASS end-to-end on Windows including expected unsupported-platform execution failure, restricted edit and no pageerror. Linux actual Actions success remains CI gate. No assertion weakened.
- Fresh frontend127 PASS + new classification regression4 PASS, build PASS; migration schema5 PASS; fmt/diff PASS. Alternate cargo rustc -o target/debug/ferry-server-ui.exe used for fresh embedded UI because running4291 locks target/debug/ferry-server.exe; don't stop user service just for testing. test-actions-ui now supports FERRY_BINARY.
- Next push these repairs; require successful latest full main CI, then annotated alpha14 tag and tag gates/public signature checks, then local data backup/install/Runner upgrade to4291 as previously planned. No new user authorization needed.

## Release continuation checkpoint ? 2026-10-30 07:40 +08
- Latest update 07:53: Windows Runner stop control previously returned immediately after SCM stop request; rebind could replace an in-use executable. Added OS process-exit wait with bounded timeout before replacement; real Windows child-process timeout/exit regression PASS; full Runner 20 unit +3 CLI PASS, one signed fixture ignored locally and required in CI. Native privileged rebind remains CI gate.
- Main run 40975364895 (2d05230) started after old 3651ab0 run completed in29m19s. All old SSH/Runner/pipeline/deployment variants passed; old Windows installer failed, now diagnostic annotations added to installer PS and desktop smoke JS; latest fixes will queue behind this running source. Current source includes annotations and current run-log/workspace fixture locators.
- Local 5173 Ferry Vite is no longer listening; only unrelated Python wildcard service remains there. 4290 and4291 Ferry backends are online; ASPEED alpha13 offline. Attempt to restore missing Vite via hidden Start-Process was automatically rejected with only reason "blocked by policy"; do not retry or disguise that action. Explain this blocker accurately. User already informed. After publication attempt authorized installed backend/Runner upgrade; if service restoration remains blocked, clearly report it rather than claim local upgrade complete.
- Current HEAD 35621147ba9bf0f3e1993a1594fd7c06c586328f on main, pushed through ssh.github.com:443. Previous source 3651ab0 CI 40972493098 still running desktop Intel/Windows and deployments; known failures are stale confirmations/bootstrap/dashboard fixtures. Fixes committed in a530c9a. Latest browser-run log/host workspace and isolated Vite port fixtures committed in 3562114. Full main workflow pending behind older main because concurrency cancel=false.
- Fresh tests: workspace/maintenance/Cron/manifest/marketplace browser PASS; token metadata gate 5 PASS; SQLite backends 123 PASS (Git Bash test-only curl stdin wrapper/Python alias); table geometry/row interactions PASS at 1280/900/390, members/platform/form scrolling PASS. Legacy test-build-ui local form validation PASS but full execution unsupported by Windows legacy POSIX executor, so full real Git/log/terminal path awaits Linux CI. Updated its locators to current steps/logs/new-window terminal and workspace file splits; no execution assertions removed.
- GitHub anonymous API rate remaining 9 at 07:40, reset epoch 1793317993. Query sparingly. a530c9a run 40973978956 pending; latest 3562114 run not yet queried. No tag/release yet. Existing all tag release gates must pass before publication.
- Prepared TEMP/ferry-release-tools/upgrade-local.ps1 (not executed) validates hashes + uses installed alpha13 Runner verifier, backs up protected Runner state, installs NSIS /S /AllUsers, replaces base supervisor only when no current/pending managed journal, preserves runner.json hash, rolls back runner binary on failure. Requires OS UAC once; does not touch backend process itself. Before invocation back up SQLite/key for both data dirs, stop exact Ferry backend processes; after install restart 4290/4291 as ordinary user using released server and existing data, leave Vite proxy 5173 pointing4291. If managed Runner journal exists inspect rather than overwriting it.
- Public signed release verification helper TEMP/ferry-release-tools/verify-release.mjs expects version and exact SHA; downloads verified Windows installer/Runner and signed catalog to TEMP/ferry-release-tools/0.1.0-alpha.14. Not yet run.
- Next: wait/check latest full CI, fix failures without weakening assertions, create annotated alpha14 tag only when verified, verify public release/container, back up and upgrade local installed app + ASPEED Runner, confirm same host ID/IP/actual monitor requests. User explicitly authorized release + local upgrade, no new chat permission needed.

## Active alpha.14 release — 2026-10-30
- User explicitly requests publication now, followed by upgrading the running local Ferry and ASPEED Runner. This supersedes earlier no-release/no-change-to-4290 limits for the post-release upgrade; preserve both local data directories and host identities.
- Phase: REVIEW / TESTING. Release target 0.1.0-alpha.14 includes current host workspace/files, Runner connection/reset/IP/monitor compatibility, SSH jump/heartbeat/monitor, alerts, Cron forms, pipeline canvas/manual approval/run history/logs, and shared list/layout corrections.
- Fresh local frontend: 127 tests PASS. Previous Rust workspace run had Windows updater named-pipe access denied and one Actions shell fixture failure; complete release CI must resolve or explain these, never publish a failed build.
- Development backend 4291 restored as PID 77548; 5173 proxies same instance. ASPEED alpha.13 reconnected and remained online across monitor polling. Installed old Runner needs signed alpha.14 upgrade for detailed monitoring.
- Fresh acceptance: frontend 127 PASS/build PASS; protocol 3 PASS, Runner 22 PASS (signed fixture test separately covered by CI), server 141 PASS/2 ignored and the single missing-sh failure PASS after adding installed Git/bin to this test process PATH. cargo check/fmt and manifest tests PASS. Chrome host workspace/canvas/Runner connection/templates and no-host manual gate acceptance PASS. File creation now leaves preview closed as requested.
- GitHub API credentials are unavailable locally; SSH authentication works through ssh.github.com:443 as robotism. Use existing tag-triggered build.yml release job (all required gates before publication), not authenticated dispatch. Public API can inspect runs. Prior main e2e failed on outdated template browser fixture; updated modal/confirmation fixture now passes locally.
- Next: commit/push version-synchronized source via SSH, require successful main build.yml, push annotated v0.1.0-alpha.14 tag, require successful tag build/publication, verify public signed artifacts and container image, then upgrade local installed Ferry and Runner and verify actual monitor data.

## ASPEED 监控断线 — 2026-10-30
- 根因：本机已安装 `alpha.13` 旧 Agent（Windows 服务 `runner-agent`）不识别新增的 `Monitor` 会话类型；主机监控每 30 秒请求一次，旧 Agent 解析未知控制帧后断开重连。`4291` 日志证实每分钟接入 30 秒后发生 WebSocket 无关闭握手重置。
- 修复：Agent 握手新增兼容的 `supports_monitor` 能力字段（旧 Agent 缺省 false），服务端对旧 Agent 拒绝详细监控会话并提示升级，避免发出未知帧。新 Agent 宣告支持；增加旧 Agent 不收帧的回归测试与协议兼容测试。
- 验证：协议 3 项、Runner 22 项、服务端定向回归、`cargo check --workspace --locked`、`cargo fmt --all -- --check`、`git diff --check` 通过。`4291` 已重建并启动为 PID 83708，ASPEED 连续在线超过两分钟，旧断线循环消失。旧 Agent 的详细图表仍须新版本安装后才可用。
- 另项：本机 `cargo test --workspace --locked` 因桌面升级器 Windows 命名管道权限测试 `exclusive_local_pipe_authenticates_both_peer_processes` 报 OS error 5 而失败；单独重跑仍失败。`cargo test -p ferry-server` 的 144 项中 141 通过、2 忽略、1 项 Actions checkout 脚本因本机缺少外部程序失败。两项未涉及本轮改动，发布前仍须复核 CI。自动执行审查拒绝了停止并重启 `5173` Vite 的诊断命令；未重试或绕过，当前 `5173` 保持运行。

## 主机报警监控 — 2026-10-29
- 主机详情「报警监控」已接入按主机过滤的报警规则和告警事件，支持直接新建并预选当前主机；修复表单初始化清掉预选主机、目标过滤可覆盖主机限定的问题。
- 后端告警事件允许 `target_ref` 过滤，资源筛选将其按雪花 ID 数值绑定；新增大整数绑定测试。
- 验证：完整前端 127 项通过、`npm run typecheck`、`npm run build`、`cargo check --workspace --locked`、服务端资源单测 5 项、`cargo fmt --all -- --check`、`git diff --check` 通过。`PLAYWRIGHT_CHROME` 指向本机 Chrome 的 `scripts/test-alerts-ui.mjs` 真实浏览器验收通过，覆盖主机页签、预选主机、离线事件与规则全流程。
- 本机 `4291` 独立开发后端已启动（PID 68044），`5173/api/status` 正常；不碰 `4290`。本次改动未提交，仍需按当前总任务完成整体复核与发布。

## 当前开发任务 — 2026-10-29 夜（继续）
- 用户要求：完成此前未交付的主机/流水线改动、Cron 默认周期表单与表达式双向同步、宽屏留白排查；测试通过后提交并发布新版本。发布已明确授权；不可动本机 4290 服务。
- 本轮新增：CronEditor 默认周期表单（分钟/小时/日/周/月）与 Cron 输入常驻、即时校验并阻止非法表单保存；共享表单改为宽屏居中。Runner/SSH 统一 `HostChannel::monitor`，详情显示 CPU/内存/逐盘/逐网卡；SSH 每 30 秒探测并更新在线状态/最近在线时间，支持 Linux 和 Windows PowerShell 采集；CPU 告警指标、Runner 心跳 CPU 指标。
- 验证：前端曾 121 项通过、构建通过；Rust check 与 SSH 解析单测通过。本轮完整前端及 Rust 测试正在运行，最新 UI 改动仍需重新构建及浏览器脚本。`cargo fmt --all -- --check` 和 `git diff --check` 已通过。Linux/Windows 真实 SSH 监控以及 Runner 实机联动尚待验收。
- 下一步：等待本轮测试结果，修复失败；重建前端、运行浏览器脚本；构建 4291 后端并仅重启 4291、验收 5173；复核差异与发布流程，更新版本号并提交推送，等待 CI 门禁后通过 publish-verified 发版。

## 当前开发任务 — 2026-10-29 晚
- 阶段：实现中，尚未提交推送或更新本机 4291 后端。用户新增：SSH 跳板主机可复用、Runner 详情常驻连接命令与重置、Runner IP、SSH 定时心跳、两种接入的 CPU/内存/每盘/每物理网卡监控与告警、流水线分组排序、全局 hover、树目录固定项名称一致。
- 已编码：文件删除确认/新建文件与文件夹；列表图标和删除确认；Runner 详情常驻命令与重置（专属令牌存密文、旧令牌/密钥失效、断开当前连接），列表移除接入入口；SSH 单级跳板和引用约束；Runner 安装器 `--rebind` 更新凭据；流水线分组排序和 hover；固定项统一为「全部/未分组+对象名」。Runner IP 代码此前已有，需确保重新绑定后的服务二进制更新。
- 已验证：此前文件与连接前端 118 项、构建、Chrome 脚本通过；Rust server/runner `cargo check` 通过；Runner 令牌单测通过。新增跳板实时 SSH 测试本机无 Docker/sshd，仅完成编译并跳过实际连接，待 CI。最近新增分组排序与 rebind 后尚需重跑测试。
- 下一步：完成双接入监控与 SSH 周期检查、告警，测试 rebind/跳板/分组；构建并重启独立 4291 服务（不动 4290），更新 5173 前端；复核全量 diff 和 LF、提交推送 main、查看 CI 的 SSH/Runner/前端门禁。不要自动发布版本。

## 本机后端与操作列 — 2026-10-29
- 当前阶段：本机开发服务已切到新版后端，功能验收进行中；待提交推送并查看 CI。不要自动发布版本。
- `127.0.0.1:5173` 的 Vite 通过 `FERRY_DEV_API_TARGET=http://127.0.0.1:4291` 代理新服务；4291 使用 `data/dev-backend` 中从桌面版 SQLite 一致性备份得到的独立数据。已安装的 4290 服务未改动。重启 Vite 时须带同一环境变量；`data/` 不提交。
- 共享列表操作使用图标与 tooltip，其他动作在左，“查看、编辑、删除”相邻排在右侧；删除走明确对象的二次确认弹窗。文件管理增加新建文件夹与空文件，服务端通过 Runner/SSH 独占创建防覆盖；Runner 需更新后才支持新增会话帧。列表中的“接入 Agent”改名“重新接入 Agent”。
- 已验证：`npm test -- --run` 116 项通过、`npm run build` 通过、Chrome 工作台脚本通过；`cargo build --locked -p ferry-server -p ferry-runner` 通过。`cargo test` 本机服务端 132 项通过、1 项旧 Actions checkout 测试因 Windows 缺少外部程序失败，协议/Runner 单测通过。4291/5173 的实例 ID 一致，新创建文件 API 在 5173 返回 401（未登录），旧 4290 返回 405。
- 下一步：复核 diff/格式，提交推送，查看 CI 的 checks、e2e-runner 和 SSH 测试；本地 5173 和 4291 会话在本次运行中保持开启。

## 主机工作台与分组排序 — 2026-10-29
- 当前阶段：前端与浏览器验收通过，待提交推送并由 CI 编译验证 Rust；不自动发布版本。
- 主机组树支持同级上下调整并持久化；主机列表和详情可新标签打开全屏工作台。工作台支持终端/文件标签、递归左右或上下分屏、目录快速终端、文件预览和文本编辑保存。
- Runner 安装命令支持 Windows 提权；新 Runner 上报 IP，服务端登记和重连时保存，旧 Runner 协议仍兼容。现有安装须升级 Runner 后才会上报 IP。
- 验证：`npm test -- --run` 115 项通过、`npm run build` 通过、`git diff --check` 通过；`scripts/test-host-workspace-ui.mjs` 已通过 Chrome 桌面与移动视口检查。Rust 本机按项目约束未编译，CI 待执行。
- 下一步：检查并提交暂存内容，推送 main，等待 CI 的服务端/Runner 测试；不自动发版。

## 本地 UI 与终端修复 — 2026-10-29
- 当前阶段：本地验收完成，待提交并在 CI 验证 Rust 和真实 Runner 联动；不自动发布版本。
- 流水线全屏编辑器顶部合并设置、模式切换与模板操作；任务需逐个绑定执行主机，缺失时画布标红。分组树内管理流水线与主机组，旧分组地址转到对应列表。人工卡点尚无服务端暂停/审批机制，未实现。
- 主机添加页 Runner/SSH tab 固定，SSH 可就地创建加密凭据；主机详情标签按只读数据展示。文件页可在当前或指定目录打开侧边终端；Vite 代理启用 WebSocket 转发。
- 验证：`npm test` 114 项通过、`npm run build`、`git diff --check` 通过；`test-actions-canvas.mjs` 浏览器检查通过。无鉴权 WebSocket 握手经 5173 转发后端并返回 401，修复前请求超时。`test-build-ui.mjs` 已加入真实 Runner 目录终端 `pwd` 验收。
- 限制：本机没有 `target/debug` 服务端/Runner，按项目约束不在本地编译 Rust/Go；真实终端目录和服务端校验等待 CI。已有本地 5173/4290 服务不包含新的 Rust 二进制。


## 当前状态 — 2026-10-29
- **交付完成**：`v0.1.0-alpha.13` 已发布，提交 `75796c08600c916064d88acf09dc1c670cf89287`，发布来源分支为 `feat/actions-pipelines`（现已合并并删除）。
- 发布页：https://github.com/7link-team/ferry/releases/tag/v0.1.0-alpha.13 。当前没有待执行的发版操作。
- 已交付：T15 仓库连接与流水线节点配置、串并行执行、标签/环境管理；T16 Runner 一键安装、系统服务和签名升级能力。
- 验证：完整构建 `40794997196`、专项验收 `40794987562`、镜像构建 `40794996760`、发布 `40801930031`、镜像版本标记 `40802064375` 均成功；21 个公开产物摘要和更新清单签名已校验。
- 已知问题：Windows 旧版升级验收曾出现一次应用退出超时，同提交复验通过，根因尚未确认，发布说明已披露。
- 未完成：T17 Windows 桌面免交互、免重复 UAC 的独立升级器。真实第三方私有账户验收未完成；K8s/Helm 按用户要求暂缓。
- 约束：磁盘空间受限，不在本地执行 Rust/Go 编译和测试；重构建与打包使用 GitHub Actions。不新建 worktree。忽略误发的其他项目需求。
- 长期决策见 [ADR.md](ADR.md)。状态与决策现统一维护于根目录，取代原 `.claude` 路径及“仅保留本地、不提交状态文件”的旧约定；本文不得保存凭据或临时敏感信息。
- 本次整理：两份记录已迁至根目录，引用已修正；文档提交 `30260e4` 连同功能分支已快进合并并推送到 `main`。本地和远程 `feat/actions-pipelines`、`feat/platform-initialization` 已删除。后续已检查并归档旧工作区的独有验收记录，删除 `ferry-platform`、`ferry-desktop-fix` 两个工作区及对应本地分支，并删除远程 `fix/desktop-upgrade-lifecycle`；远程原本没有 `feat/workspace-management`。仅保留主工作区与 main 分支。当前主工作区位于 `main`。类型检查通过，前端 98 项通过、1 项平台跳过；运行时代码与已通过完整验收的 alpha.13 一致。主分支构建 `40803470243` 和镜像 `40803470338` 已触发，记录时尚在运行，不能视为通过。本次文档整理不改变 alpha.13 发布产物，勿自动重复发版。

## 历史交接记录

以下内容保留原记录，版本、进行中状态和下一步均对应当时时点；恢复任务时优先核对上面的当前摘要、Git 状态和实际 CI 结果。

## Objective / confirmed scope
- Next alpha includes complete Agent Runner lifecycle, corrected code connections and pending shared-list UI. No new release until release gates pass. Public latest remains v0.1.0-alpha.11.
- Connections choose platform/auth and reuse credentials; pipelines select visible repository/branch. GitHub/GitLab/Gitee/Gitea/Codeup/plainGit, no internalGit hosting. Platform admin configures OAuth apps dynamically; workspaces authorize accounts.
- GitHub Actions-compatible YAML/canvas; multiple environments selected at run time, group/app/tag management. K8s/Helm deferred.
- User withdrew merchant8091 and top progressbar requests as wrong project. Ignore them.
- No local cargo build/test/check or heavyGo build. Heavy builds/tests run GitHubActions. No newworktree/goal. .claude state/decisions remain uncommitted.
- Every user-facing message starts `✅ CLAUDE.md loaded 🎉`. Repeated continuation authorizes these approved tasks; no further permission needed.

## Current phase / exact next action
IMPLEMENTATION / TESTING. Branch feat/actions-pipelines, HEAD30b1334 PUSHED. Active T17 checkpoint at end; T15/T16 release scope retained.
Latest three-platform CI40653525532 ALL_SUCCESS (final job API verified): Linux122698996057 fullActions+upgrade, macOS122698996410, Windows122698996338. WholeActions/artifacts/logs/cleanup/queuedcancellation/newtaskexclusion and native upgrade/restart/fault suites passed. Next implement release discovery/install/update host UI; also new desktop UAC concern below.
Prior three-platform native/update/restart/DB-failure suite b9db1ec CI40648148786 ALL_SUCCESS. Host install/update UI, release discovery, T15 and compact-list UI still not implemented. No release.

## Baseline delivery
- alpha11 released5fbc883; release40603456149 and image40603487762/40603456133 passed, published installers/signatures/manifests/checksums/multiarch image verified. Includes prior Actions, old repository selection; excludesT15/T16.
- Earlier fullregression frontend73PASS, Rust131PASS+1existingliveSSHignored, pipeline154, classification29 eachSQLite/MySQL/Postgres, Go racePASS, desktop lifecycle/update acceptance. External private-provider real accounts still unverified.

## T16 implementation and verified stages
### Native service / task reservations
- Native systemd/launchd/WindowsSCM CLI install/start/stop/status/uninstall; foreground retained. Protected state ownership/ACL; duplicate install preserves executable/identity/runningservice; conflict paths rejected; uninstall preserves data.
- Windows PowerShell searchpath isolated; OWNER RIGHTS accepted only after owner trusted. Stop drains sessions, SCMStopPending checkpoints, native crash restart.
- runner_updates.rs fair perhost read/write reservations; wholeActions task holds leases across jobs. Task-local reuse prevents laterjob deadlock with queued upgrade writer; streams/PTYPumps retain leases. WaitingActions cancellation releases partialreservations without aborting activecleanup.
- CI40630691384 all3OSPASS. No whole-machine reboot acceptance.

### Signed updates / supervisor
- Release runner-update.json binds schema/protocol/version/five exact target artifacts/HTTPSURL/size/SHA256; signed with existing desktopMinisign key, independently verifies signature and exact asset catalog, package/Cargo/tag consistency.
- build.rs embeds fixed publickey and target; verification precedes metadata trust; rejects downgrade/wrongtarget/version/protocol/tampering/size/hash. Existing key supports fixture compile override only; runtime cannot replace it.
- Supervisor stable native entry launches immutable signed versions/<sha256>/runner. Worker stdoutJSON readiness follows authenticatedWelcome; parent persists journal then confirms via stdin; worker sends UpdateHealth only afterward.
- Parent holds stdin separately: Child.wait otherwise closes it. EOF graceful stop. WindowsJobObject killonclose prevents orphan worker; stderr protectedservice.log.
- Journal current/previous/pending/failed_version; atomic persisted state and durable staged directories (Unixfsync/WindowsMoveFileExWRITE_THROUGH). Pendingstartup rollback; candidatebadexec/wrongversion/60shealth timeout rollback. Corrupt/missing/nonexecutablecurrent falls backprevious/bootstrap.
- Native worker signedHTTPSdownload bounded300s/redirect5/nativeOSroots/signedsize, stagevalidate, drain, activation. No overwriting runningWindowsbinary.
- Control Update and reports UpdateHealth/UpdateFailed; Hello.managed_updates defaultfalse keeps oldclients/foreground compatible. GET/POST workspacehosts/id/runner-update permissions+tenantfilter; POSTcurrently explicitversion/manifest/signature, no releasediscovery/UI.
- Update job outlives HTTP disconnect; wholehostexclusive from drain through health/rollback.420stimeoutunknown retainslock.

### Server persistence / reviewed fixes
- Migration hosts.agent_update nullableJSONtext; GETdurable state. Initialadmission independenttask; persist beforedispatch. Startup recover before schedulers/listener acquires pendingexclusion; draining never-sent becomesfailed; updating/unknown awaits managedconfirmedhealth, version/failed_version reconciles.
- transition_update retriesDBwrite while caller retainslock; only durableterminal allows newtasks. Auditfailure cleanup uses same retry.
- Reviewer /root/review_runner_service found durability/currentfallback and serverpersistentstate issues (fixed), then P2audit+cleanupwrite failure leavingdraining (fixed18c2e68 after realSQLite RED0ea42c3). No newP1 in review; fullActions+upgrade acceptance required.
- RealSQLite trigger test asserts audit+cleanupfailure keeps recoveryalive then completesfailed without dispatch. Recoverytest asserts absent/unconfirmedhealth and failedterminalwrite keepnewtasks blocked until statusdurable.

## Verification evidence
- Signing40635356494 all3PASS; service/supervisor/control40640276102 all3PASS.
- d2367d8 CI40645375279 all3PASS: realTLSsigned9.9.9 upgrade, PTYdrain, badexec9.9.10/wrongversion9.9.11 rollback, supervisorcrashpending9.9.12, confirmedversion missing/corrupt/nonexecutable fallback, identity/lifecycle.
- 523a77f CI40646440663 all3PASS: completedstatus persists and serverrestart duringpending then supervisorcrash rollback.
- b9db1ec CI40648148786 all3PASS: above plus auditwrite/recoveryexclusionfaults. Nativeworkflow runs runner::tests,runner_updates::tests,actions_run::tests.
- 1fec6dc CI40648839609 macOS/WindowsPASS: serverrestart during blockedHTTPS9.9.13 download cancels preparation, healthyoriginal9.9.9 confirmed, nopending. Linuxcombined test didnotfinish (testbugbelow).
- aa0b3a0 CI40652687971 Linuxlog /tmp/t16-actions-bounded.log PROVES actual2jobActions task, artifactupload/download, logs/cleanup, queuedcancellation/newtaskexclusion, and actualsignedupgrade passed. Subsequentcrashtest killedharness due testbug; notwholePASS.
- Latest559157a corrects testbug, CI40653525532 running. LocalPythonAST/gitdiffcheckPASS; rustfmt applied only relevantserverfiles; no localheavybuild.

## Current acceptance harness and resolved failures
- test-runner-upgrade.py uses throwawaysigningkey/baseline/candidate builds onlyCI, realHTTPSfixture with temporarytrustedCA. Badexec/wrongversion/stalled/slowdownload variants. Fixtureexecutables never released.
- OpenSSL1.1 duplicated criticalbasicConstraints fromsystemconfig+-addext => macOSUnknownIssuer. Confirmedcertificate dump; ownCAconfig fixed, OSsecurityverify retained. WindowsreadonlyFlushFileBuffers error5 fixed by keeping finalpostchmodfsyncUnixonly (Windows writablehandle already flushed).
- RawWindowsWSclient answersConPTYcursorquery; UTF8diagnostics. Versionassertions use runner-update endpoint, nothostresource.
- LinuxActions build worker onlylinux/amd64 viaexistingGo script;2job hostimage workflow with filebarriers, artifacttransfer, logs/tempdircleanup; triggerupgrade duringjob1+PTY, ensure laterjob runs while upgradingwaits, newtask/cancelwait verified.
- TESTBUG: top-level `for name in ['build.workdir','verify.workdir']` overwrote globalnative servicename; pid()returned0; os.kill(0,9) killedtestprocessgroup (includingCIlogcollector), appearinghung.559157a renamedvariable and assertsPID>1 before crash injection. No productdrain failure established.
- RootCI Linux timeout-k15s300s remains as boundedharness; SIGTERM outputs diagnostics; cleanup releases ownjobbarriers. Obsolete40648472603/40648839609/40649818860 cancellationrequested; firstcompletedcancelled withmissingjoblogs. Latestrun survivesbugfixpendingverification.

## T16 remaining / next phase
1. Verify latest whole3OS suite and fix actualfailure if any. Add any required activecancellation/cleanup acceptance not already covered; don't conflateunitlocks with actualwholeActions evidence.
2. Signed releasediscovery, onelineinstaller scripts and hostpageinstall/updatecontrols (reusehostpage, nonewmanagementpage). CurrentPOSTmanualcatalog isn't finished UX.
3. T15 and sharedUI below, fullregression/review/release gates, nextalpha. NoK8s/Helm.

## T15 connection correction pending
- `docs/tasks/T15-code-connections.md` plan uncommitted; no T15 product implementation yet.
- `scripts/test-repository-auth.py` has17-line expectedRED new /code-connections API test; missing endpoint returns SPAHTML. /tmp/t15-connections-red.log.
- Current RepositorySource requires URL/default branch; PipelineRepository duplicates authorization. Existing repository_connections singleton byworkspace/user/provider can overwrite credentials.
- Reuse connection/vault resources for named connections, retain repositories as internal concrete bindings for historical compatibility. Existing PAT credentials referenced, not copied/rewritten; no auth tab.
- Need actual provider visible-repo→branch→checkout/build acceptance over SSH and Agent.

## Pending UI changes (uncommitted)
- ResourceTable/App/Platform viewport-filling table/sticky header/footer/edge columns implemented, Chromium accepted. /tmp/t15-table-layout-green.log covers empty/single/65rows,1280/900/390width, x/y scrolling, invite expansion/platform forms. npm73PASS andbuildPASS prior to cleanup. Independent review no blockers.
- Files: src/components/ResourceTable.tsx, src/App.tsx, src/Platform.tsx, scripts/test-table-layout-ui.mjs. Do not duplicate/revert.
- Compact filters and unified row of operations/column/import/export/create requested, not implemented. Common filters visible; extra conditions collapsed with active count.

## Disk and local state
- Two independent APFS containers: project300GB, system200GB. Project cleanup24→56GiB free; system600MiB→13GiB free. Most recent df unchanged56/13GiB.
- Removed regenerable Rust debug caches (kept runnable top binaries/workers), Go/npm/Bun caches, old release downloads,195 inactive old E2E fixtures and old Docker installer staging. No user databases/Docker volumes/source/credentials or active app data removed. Active swap15GiB not deleted; no reboot/apps closed.
- Persistent .claude decisions contain approved product semantics. Review agents may be reused for scoped independent review per requesting-code-review skill; no implementation delegation needed.

## Last checkpoint
559157a pushed; pollCI40653525532. No pendingexec sessions or uncommitted formatter changes.

## Latest user steering — desktop upgrade / Windows UAC
User asks whether independent updater should stopoldservices then upgrade automatically without installerwizard/UACeverytime. Inspected desktop/tauri.conf.json: NSISinstallModeboth, updaterinstallModepassive; desktop/windows/installer.nsi /UPDATE enables passive+restart. This hides wizard but does not remove per-machineinstaller UAC.
Explained: dedicated upgrade executor is appropriate; graceful drain before stopping/replacing, health+rollback. Per-user upgrades normally no elevation; all-users needs pre-authorized protected updater service installed with initialadminapproval. Cannot bypass OS privilege enforcement; existing oldinstallation migration may require oneauthorization. AgentRunner already uses native service supervisor; desktop privilegedupdater is NOT implemented and must not be conflated. No new desktop code changed yet. Preserve originalT15/T16/release objective.

## Active T17 — desktop unattended Windows updates
- User approved implementation; plan docs/tasks/T17-desktop-unattended-updates.md. HEAD30b1334 pushed. Public alpha11 unchanged; no release/version bump. Existing T15/sharedUI uncommitted work preserved.
- CurrentUser ordinary helper, AllUsers initially authorized protected SYSTEM updater service. Dedicated signed ZIP replaces NSIS for subsequent updates because both-scope NSIS Highest can still prompt UAC for unelevated admins. Initial install/legacy compatibility stays NSIS. Plan corrected contradictory silent-installer sentence.
- New workspace crate crates/desktop-updater: embedded product/identifier/key; compile-time signing fixture override only, never runtime trust override. Existing runner fixture script removes new crate from copied workspace.
- Admission: streaming Minisign, 2GiB bound, signed product/version/windowsZIP filename, strict SemVer precedence. RED40659156768 -> GREEN40660700033.
- Extraction: fresh protected empty directory, bounded size/count, safe Windows paths, no links/reserved/ADS/casealiases/duplicates/overwrite, filefsync. RED40661332369; zip9API + has_duplicate_names fixes; GREEN40663803246.
- Transactions: backups+SHA256, durable Prepared before mutation, atomic replacements, Applied/Confirmed/RolledBack, recover unconfirmed, corruptbackups fail preflight, unrelatedfiles preserved. RED40664201204 -> GREEN40665289783.
- Fault fixes: save failure retains prior in-memory phase and can retry; modified newlyintroduced files fail preflight before rollback mutates anything. RED40665998516 -> GREEN40666375415. Actual Windows deny-delete handle tests partialapply and recovery after release.
- Windows InstallationLock: concurrent read handles for desktops; exclusive updater prevents new sessions, remaining desktop prevents update; no create/truncate/leafreparse. RED40666471875 -> GREEN40666881208. Real subprocess kill releases lock GREEN40667146479. Installer-protected ancestor/ACL provisioning and actual separate-SID/session tests still missing.
- Independent reviewer /root/review_desktop_updater found P1 journal may exceed8MiB recovery bound, P2 rolledback deletion reconciliation, P2 interruptedprepare withoutjournal. Accepted all, tests RED40667907896 -> fixes8e85ef6 GREEN40668349762. Journal saves reserve longest phase size; terminal RolledBack rechecks newlyintroduced files without rewriting old originals; Recovery::Unprepared explicitly leaves stage untouched for caller discard/retry under lock. No hardwarepowercut claim; filesystem-state reconstruction test only. No deferred findings/rulings beyond approvedZIP design.
- Transfer module: bounded16KiB JSON header(version/signature/size only), max2GiB payload,300s totaldeadline, protected temporaryfile, workerthread signatureverification, no path/command accepted; failed/truncated/cancelled transfer removes stage. RED40668791646 -> implementationde7e2a1 GREEN40669322856. Cancellationtest needed matches! instead of unwrap_err(Debug bound); fixed30b1334, latestGREEN40669864765 Windows+Linux.
- Verification: all latest crate tests/signing fixtures pass in Actions40669864765. Local rustfmt--check and gitdiffcheck PASS, no localCargo/Go build/test/check. Only offlineCargo lockresolution. Build stays in GitHubActions.

## T17 exact remaining / next action
1. Implement native Windows named-pipe transport + protected install registration/SCM broker, with actual standard-user tests. Reuse existing runner SCM/ACL patterns. Current crate is a library; NO actual service or helper binary yet.
2. Desktop/helper handoff after graceful drain; hold parentprocess handle and await actualexit; original-user restart + nonce/process/path/version/session-bound health. Health mode must not start sidecar/workspace/DB migrations before confirmation. Broker never launches UI asSYSTEM. Failedactivation rollback is pre-business, not DBrollback.
3. Machinewide maintenance lock must cover desktop/sidecar life, recovery-before-start, and explicit health-mode exception during updaterexclusive ownership. Other sessions cause refusal, no forcedkill.
4. NSIS initial provisioning/uninstall, versioned helper/broker selfupdate with oldSCMpathfallback, Windows signed ZIP releaseassets and compatiblemanifest selection. No caller-provided executable/destination/commands; first-instance local-only pipe with serveridentity verification; bounded streams/time. AllUsers protected staging/registration fixed roots.
5. Native two-version acceptance bothscopes, standarduser and unelevatedadmin, no wizard/UAC, gracefulbusywork, auto originaluser restart, interruptedhealth/recovery, hostileIPC/reparse and multipleuser races. Independent final review/fullrelease gates, then remainingT15/T16/UI and nextalpha.
- No pending tool sessions. All changes committed except pre-existing UI/auth/docs and local .claude state. No user input needed to continue approved work.

## Active T17 IPC continuation
- Native exclusive named-pipe + kernel peer Process handles: RED40677035407 -> ff7b360 GREEN40677455282 Windows/Linux. Localonly, firstinstance, max1, AU dataread/write only (not genericwrite/createpipe), client SECURITY_IDENTIFICATION, serverPID matches live expectedprocess; OS clientPID, canonical caseinsensitive processimage check.
- f07cc30 adds tokenidentity stub and real SYSTEM/standarduser acceptance fixture (scheduledtasks+ephemeral nonadminuser, protected testroot; cleanupfinally). CurrentCI40678077499 expectedRED; inspectthenimplement token SID/session/elevation. No localRustbuild.
- Next: verify true standarduser can query SYSTEMservice identity/connect/readwrite and spoofserver refused; fix actualrights failures. Then protected registration/SCM broker and helper integration; desktopupgrade remains incomplete.

- Token identity implementation c2195a8: SID/session/elevation from native process token, aligned GetTokenInformation buffer. Unit19/19Windows passed; actual SYSTEM/standarduser harness NOT yet passed. 40678520788 failed PowerShell5 moduleautoload due inherited PS7PSModulePath; dc93953 removesit. 40679049183 nativefixture failed but CREATE_NO_WINDOW without captured pipes lost diagnostics. 2b7e560 capturesstdout/stderr and taskstatus; CI40679734066 pending. Inspect log before guessing permissions fixes.
- Native fixture creates random standardlocaluser, SYSTEM+Limited scheduledtasks, protected ProgramData testroot with only client-result writable, removes tasks/user/root in finally. Process open currently requests QUERY_LIMITED_INFORMATION|SYNCHRONIZE; ordinaryuser query ofSYSTEMtoken may be denied—needs actual evidence. Future helper should authenticate SCMregisteredprocess, no extra elevation to inspecttoken.
- Integration constraints identified (not implemented): new Windows updater target should be separate from legacy latest.json windows-x86_64 NSIS entry, so old alpha clients retain compatible installers. Helper parent-exit handoff must use actual process handle and hidden child with explicit pipes; no extra stdout writes after parent exits. AllUsers service default DACL may not grant ordinary users SERVICE_START; initial provisioning must explicitly allow start/query only if helper is responsible for broker restart, never stop/change-config. Health startup must gate frontend connection/open-workspace commands until service confirmation, because merely pausing Rust setup does not stop webview JS from invoking sidecar startup. These are technical requirements of approved T17, not completed features.

## Release priority checkpoint — alpha.12 (2026-10-28)
- User explicitly requests urgent test release. T16 one-command native service bootstrap prioritized; T15 connections and T17 unattended desktop upgrade remain incomplete and must be disclosed.
- Current public alpha.11; working versions bumped alpha.12 via npm version and cargo update --offline --workspace (only local packages changed). No tag yet.
- T16 bootstrap server + shared signed verifier committed bc3e119. CI40686990135 Linux/macOS PASS; Windows failed PS5 UTF8 ellipsis parsing. Installer now ASCII, actual stdout assertion added. Native rerun pending.
- Review fixes: HTTPS-only remote origin/UI/scripts; strict loopback HTTP test allowance; bootstrap fixture now checks real backend token lifecycle and confirms old token401 / fresh token identity preservation. Backend consumed/unbound token expires regardless of used_at, real API/SQLite regression added.
- Local tests 81PASS/1WindowsSKIP, install17PASS, token fixture5PASS, typecheck/build/browserPASS. Rust formatter revealed previous Runner code formatting; cargo fmt applied only formatting, no local compile.
- UI shared table + RunnerConnection uncommitted, accepted prior tests preserved. Windows command test wired native CI; browser/table tests wired release e2e.
- Final review found initial script redirect downgrade and Windows unprotected TEMP staging. UI agent fixing via HTTPS-only redirect and in-memory script execution in native PS5, no temporary script. Await agent before stage.
- Preserve uncommitted T15 test-repository-auth.py RED and docs/tasks/T15-code-connections.md. Do not stage .claude.
- Next: integrate final command fixes; local relevant tests; commit scoped files/version, push; full build workflow gates+native3OS, fix failures; publish v0.1.0-alpha.12 with honest scope/known limitations; verify assets/catalog/downloads.
- c8200f8 committed/pushed complete alpha12 bootstrap/UI/version; preserved T15 dirty files. Review's final2 blockers fixed and independently re-reviewed clear. No localRustbuild.
- Full release dry-run40690976414 and container40690983668 dispatched at c8200f8; native40690880004 automatically triggered. Plan: after all gates succeed, publish already-verified release-dry-run assets via gh release create --target c8200f8 --prerelease, avoiding duplicate compilation; container release tag via existing version job. Notes prepared /tmp/ferry-alpha12-notes.md. No public release/tag yet.
- First full run40690976414: checks, backends, SSH/Agent/pipelines/UI/manydeployments PASS. Windows native lifecycle assertions ALL passed but fixture cleanup failed WinError32: Python sqlite connection context commits but does not close. Fixed with contextlib.closing + commit in491b461; standalone native40692715095 rerunning. Productcode unchanged.
- First image40690983668 failed missing desktop/tauri.conf.json for include_str; server also embeds installers. Fixed Dockerfile COPY and .dockerignore exception in2d73ebb. Current final source2d73ebb; new full40693331588 queued behind prior full; new image40693338225. Intermediate queuedfull40692719641 and staleimage40692726063 cancelled to avoid redundant work. Original full still finishing desktop builds/caches; do not publish its artifacts (outdated source).
- Final intended publish source now2d73ebb; notes /tmp/ferry-alpha12-notes.md. Waitfull40693331588 plusimage40693338225, then download release-dry-run to project volume and verify checksums/signatures before publishing. Native491b461 gives early bootstrap feedback but finalfull repeats at finalcommit.
- Native491b461 exposed real Windows default ProgramData ACL issue (not test cleanup): std create_dir_all inherited Users Write before secure helper. Fixed f6a641b: Unix creation unchanged; Windows existing ancestor validation + atomic protected Directory.CreateDirectory ACL + validate actual object. Independent finalreview clear. Regression rejects existing unsafe state with unchanged ACL/sentinel before fresh install.
- Native40695392652 atf6a641b: Windows generated command, lifecycle, default bootstrap and signedupgrade/rollback ALLPASS (postcache ongoing when checked); Linux jobPASS; macOS upgrade finishing. No need furtherfix without newevidence.
- Final full40695395329 atf6a641b pending behind originalfull40690976414; original IntelMac installer now built and postcache13:26UTC. Cancelled intermediatefull40693331588 to avoid stale rebuild. Final container40695402646 atf6a641b running; prior2d73ebb container40693338225 PASSbotharchitectures, proving DockerCOPYfix.
- Publish target must now f6a641b18268be4d57c73ff34de8c934e2a8f650 (unless later requiredfix). Release notesfile same, no tag/release yet. Remaining: full final gates+container finish; download/verify release-dry-run, publish prerelease via gh release create --target SHA with thoseassets; verify public URLs/catalog/latest manifest/container tag; report download link and T15/T17limitations.
- Finalfull40695395329 e2e-build-ui failed immediate assertion test-templates-ui.mjs:94 changedparameters invalidategeneration. SourceResourceForm invalidation is useEffect->parent report; locator.isDisabled does not wait React's following render. Added bounded5s wait then kept originalassert; local existingserver browser fullapp/pipeline/configtemplate flows PASS. Test-only stabilization committed AFTER release product f6a641b; no productcode changed. Finalrelease still targetf6a641b. Need rerun failed originalCI job after fullrun finishes; API rerun-job while overallactive refused. This is a diagnosed synchronizationflakytest, not silenttest suppression. If originalrerun fails again investigate rather than loop retries.
- Currentfull othersgreen, remainingMacIntel/Windowsdesktop and deployments. Container40695402646 atf6a641b PASS botharch. Verify script ready /tmp/ferry-alpha12-verify.mjs; runwith downloadedartifactdir, verifies allchecksums/allMinisign signatures/5runner+5desktop targets. No release yet.
- Full40695395329 attempt1 finished: alldesktop packages (Linux/macARM/macIntel/Windows), checks, allnativeRunner, backends/deployment/SSH/Agent/pipeline regressions PASS; only diagnosed templateUI immediateassert failed. Started `gh run rerun 40695395329 --failed` (attempt2) once afterdiagnosis, currentlye2e-build-ui compilingworkers; releasecollection will run on success. No blindretryloop. Test-only fixc9cdbbc is branchHEAD, releaseproduct/tag remains f6a641b; verified sole diff3testwaitlines.
- Publishverification scripts prepared /tmp/ferry-alpha12-verify.mjs and /tmp/ferry-alpha12-metadata-smoke.py. Latter afterpublish extracts signedIntelMacapp sidecar into own project-volume temporaryfixture and tests realpubliccatalog for5targets, exactscripts, tokennotconsumed; doesnotinstallservice. Usermachinearchx86_64. Alltempartifactdownloads must useprojectvolume, cleanupownfilesafterpublicverification.
- Full40695395329 attempt2 SUCCESS including releasecollection/signing/checksums. Browser originalassert rerun PASS after diagnosedrace; no secondretry. Allpublication gatesgreen. Downloading release-dry-run to /Volumes/Macintosh/Projects/aspeed/ferry-alpha12-release (execsession96554). Next run /tmp/ferry-alpha12-verify.mjs onthatdir, then gh release create v0.1.0-alpha.12 --target f6a641b18268be4d57c73ff34de8c934e2a8f650 --prerelease --notes-file /tmp/ferry-alpha12-notes.md with ALLfiles. Notesnowcontain CIlinks+browserretrydisclosure. No tag/releasecreatedyet.
- Local artifactdownload40329 only16MiB/5min of352MiB (~very slow); stopped ownedprocess TERM rather than wait hours. Tempfile /private/var/folders/36/x9500b6s6834kny2k4j8t03c0000gn/T/gh-artifact.608086925.zip owned bydownload, remove afterprocessgone. Assetdest mayempty; preserveanyunrelatedfiles.
- Added publish-verified.yml commits8ad0dad+9316e07 (branchHEAD): verifies successfulsame-repo build.yml run, checks out exactheadSHA, downloadsrelease-dry-run withinActions, checks allchecksums/runnercatalog/allMinisignsignatures/desktopmanifest then ghreleasecreate exactSHA without rebuild/localtransfer. Pushregistration skipspublish; onlyworkflow_dispatch publishes. Firstdispatch404beforeregistration resolved bypushregistrationjob.
- Active actualpublication40704631437 with source run40695395329 -> targetf6a641b, prereleasealpha12, notesinputfrom/tmp/ferry-alpha12-notes.md. Next inspectthisworkflow. Its GITHUB_TOKEN won't triggercontainerreleaseevent, so afterpublish dispatch existingserver-image.yml --ref v0.1.0-alpha.12 -f release_tag=v0.1.0-alpha.12 to tagverified f6a641b image. Verify publicassetnames/digests/catalog/version. Localfullartifactdownload aborted; runtime releasedsidecarsmoke optionalonlyifsmallsidecarcanfetch promptly, do not claimitran.

## Delivery completed — v0.1.0-alpha.12
- PUBLIC prerelease https://github.com/7link-team/ferry/releases/tag/v0.1.0-alpha.12 published2026-10-28T14:18:57Z; immutabletagf6a641b18268be4d57c73ff34de8c934e2a8f650 verifiedviaGitHubAPI. BranchHEAD9316e07 adds test-syncfix + verifiedpublicationworkflow, no productchangespastrelease.
- Publishworkflow40704631437 SUCCESS: same-repo successfulfullrun verified, exactsourcecheckout,21assets checksums+allMinisignsignatures+bothmanifestbindings checked inActions, uploadedwithoutlocaltransfer. Finalfull40695395329 SUCCESS (one diagnosedtemplatebrowserrace rerun); standalone3OSnative40695392652 SUCCESS; sourcecontainer40695402646 SUCCESS.
- Container versionworkflow40704946255 SUCCESS: ghcr.io/7link-team/ferry-server:v0.1.0-alpha.12 points toverified f6a641b amd64+arm64 image.
- PostpublishPUBLIC GET latest.json,runner-update.json,.sig,SHA256SUMS allsuccess. Cryptographiccatalogsignatureverified; all21GitHubassetdigests matchpublishedchecksumlist; all5Runner targets size/hash/url and5desktopupdate targets matchuploadedassets. WindowsURL https://github.com/7link-team/ferry/releases/download/v0.1.0-alpha.12/Ferry_0.1.0-alpha.12_x64-setup.exe . Prerelease=true,draft=false.
- Local slowartifactdownload terminated; owned29,671,424-byte partialzip removed and emptyprojectvolumedest removed. No localRust/Go builds. Optionalreleasedsidecarpublicmetadata runtimefixture was NOT run (artifactnetworktoo slow); do not claimitwas. Nativebootstrap fixture+signedverificationCI and publicmetadata/digest checks are actualevidence.
- Selfreview/acceptance for urgentalpha release PASS: requestedRunnerOSflow+nativeinstallation+testgates+publicrelease delivered; noT15/T17completionclaim. Release notes explicitly retain T17 desktopautomatic/UAC-freeupgrade and T15 connectionredesign incomplete; K8s/Helm excluded.
- Workingtreepreserves prior T15 RED scripts/test-repository-auth.py and untracked docs/tasks/T15-code-connections.md, pluslocal.claude state. No unpushedcommits. Urgentrelease taskcomplete. Nextuseraction: installalpha12, use主机→连接主机 to testactualenvironment. Futurework: originalT15/T17 backlog remains peruserapprovedscope; do not silentlyrestartnewfeaturesafterthisdelivery.

## Active T15 correction — 2026-10-29
User alpha12 feedback requires pipelinevisiblewidth, sources/hosts in nodes, serial+parallel actualexecution, tags/env selectable/input, no defaultbranch inconnection, reuse existingcredentials. Originalconfirmeddesign expanded in docs/tasks/T15-code-connections.md. No newapprovalneeded.
- Backend committed/pushed6892133: named /code-connections CRUD/OAuth/branches withcredentialreferences and migration (SQLite rebuild/Postgresconstraint/MySQLindex); nodecheckout_bindings {job_id,step_id,connection_id,repository_url,ref,path}; scopedruntime snapshots; FuturesUnordered DAGparallelsharedlogseq/cancellationdraining; globalhostoptional fornodebindings/env withlegacypreserved.
- ActiveCI backend Pipeline editing validation40726036374 (serverunit+3DBauth+classification+livepipelineActions), native40726036290 auto. No localRust/Go build/test/check; rustfmt only.
- Agents: code_connections_backend finished backend+focusedworkflow. actions_execution_nodes finishedserver/actions+live scripts/test-actions-execution.py (actualtwoGit repos,nodeorder,parallelbarriers,SSHjoin,cancellation) hook into test-pipelines.py. pipeline_canvas_ui finished ActionsEditor graph/RepositorySource namedform; now updating ONLY scripts/test-actions-ui.mjs and test-repository-auth-ui.mjs realoldregressions to newUX. review_actions_full read-only reviewingbackend/security, pendingfindings.
- Uncommittedparentintegration: resources/nav nowexistingrepositoriesURL points code-connections; no branchfield; ResourceFormwidth full forpipelines; basicActionsglobalrepo/hosthidden (legacyretained); sharedcreateOption multiselectsearch+inlinecreation for tags/environments; callbackbridgeconnection_id; PipelineRepository rewrite nodepicker choosesconnection→visible repo→branch/ref/path, assignsstablecheckoutstep.id. ActionsEditor.renderCheckout usesstablevaliditycallback. Tests nodepicker PASS; wideform PASS; inlinecreate-tag test pending session61880. npmtypecheck/test session22529 previous beforelasttagtest, needreadresult. RemoveunusedRepositoryinterface done.
- Newfrontendfiles agents: actions-workflow/test, ActionsEditor; RepositorySource/test; scripts/test-actions-canvas.mjs and test-named-connection-ui.mjs. Parent new PipelineRepository.test.tsx. Local priorfull93passed1WindowsSkip; currentchanges requirefinal typecheck/build/browserintegration.
- Next: inspectfocusedCI compile/runtimefailures first; finishUIactualbrowser scripts viaagent, add actualintegratednewpipeline no globalfields +nodechoices +classification validation; reviewfixes; stagefrontend and push finaltargetCI. Do NOT publish alpha13 until allfixes verified; no versionbumpyet. alpha12 remainspublic. T17 desktopupdater remainsunfinished andoutofcurrentfeedbackscope.

## T15 review and acceptance continuation — 2026-10-29 00:55
- d88c5b0 pushed frontend/node controls + collision fixes. CI40728743253 compiled server/runner and serverunit PASS; first real repo browser failed solely normalized storedbaseURL trailing slash assertion at185. Corrected only persisted-form expected URL; unsaved draft expectation unchanged.
- Current local frontend98PASS/1WindowsSkip and typecheck PASS. Two new reviews reproduced RED then GREEN: credentials afterfirst200 now paginated; valid YAML node deletion cleans binding metadata, invalid YAML retainsdraft. Imported checkout with.repository/ref/path cleaned on boundpanel edits (agent RED/GREEN).
- Reviewer accepted atomic artifact publication, private per-invocation Docker namespace and per-job legacycheckout fixes by inspection; actualActionsruntime still pending.
- Necessary legacy compatibility: transfers adds checkout_bindings JSON + CSV/XLSX roundtrip. Marketplace optional repository_selection resolves owned connection and materializes old repository atomically; frontend sharedconnectionpicker nowreused, preserves existingIDs. Agentscompleted, integratedlocal tests PASS; realmarketplace runtime pending.
- Focus workflow now independently runs suites after successfulcompile evenif anotherfails, includes marketplace/transferSQLite, browserloop doesnot skip later suites. Need push all pending deliverablefiles excluding.claude, then follownewCI. Fullrelease/versionbumppending; no alpha13 yet. No localRust/Go compilation (constraint retained). Localrustfmt via /Users/apple/.cargo/bin/rustfmt.
- 7468044 pushed schedulerdeadlockfix. Native d88Linuxfailed terminal/HTTP/shutdown timeout whileActionsrunning; rootcause SQLitepool1 statusawait stoppedpollingjob holdingconnection. Fix pollstatus concurrentwithrunningfutures. Added actualSQLite regression; latestunstagedNotifystrengthening removes timingassumption (mustcommitbeforefinal). Reviewer recheckedno newblockingfindings, UIreviewfixesacceptedbyinspection.
- Currentfocused40732238968, native40732239047 at7468044. Cancelled stale d88native40728743543 after WindowsPASS/LinuxFAIL; macOS stalehung. f8focused40730609304 cancelled bynewpush aftercompile/serverunitPASS whilebrowserrunning (deadlocklikely).
- Remainingtest-onlydiff actions_run Notifyregression; .claudelocalstate preserved. Next inspectnewfocusedCI andnative; fix actualruntimefailures, thencommitNotifytest and finalvalidation. No userinputrequired.
- 746focused40732238968: serverunit,3DBconnections,classification,liveActions(allnewparallel/cancel/collision/privatecheckout checks),transfer+marketplaceAPI PASS. Browserprivateconnection→realcheckout→exactcommitartifact PASS. Onlybrowserfailures: CSS fractionalcoordinate roundtrip comparedexact (211.333 vs211.333333), marketrow locator caseinsensitive substring matchedlegacy+new. Fixed <0.001CSStolerance and exactcellselector, preserving outcomeassertions.
- d0e78d6 latest pushed testonly: strengthenedNotifyDBtest + browserselectors/precision + focusedcache-on-failure. Finalfocused40733975250 pending. Native40732239047 atidenticalproduct7468044 ALL3OS SUCCESS; canceledredundantnative40733975946 triggeredbytestonlychange. Productcodeunchanged between746andd0. No alpha13 publishscopeinitiated.

## T15 current feedback ACCEPTANCE — 2026-10-29 01:24
- Final focused40733975250 atd0e78d6 ALL TEST STEPS SUCCESS (cachepostpending whenrecorded): npm98PASS1platformskip/typecheck/build, serverunit includingdeterministicSQLiteNotifyregression, canvas+namedconnection+realprivatecheckout+fullActionsUI+marketplaceUI,3DBconnections,classification,liveActionsparallel/dependency/cancel/artifactcollision/containercollision,mixedlegacy,CSV/XLSX+marketplaceAPI.
- Native40732239047 atidenticalproduct7468044 overallSUCCESS Linux/Windows/macOS. d0e78d6 only test/workflow edits; redundantnativecanceled.
- Selfreview+finalacceptance PASS forlatestuserfeedback; independentreview noremainingblockingfindings. All sourcechangespushed. Onlylocal.claudestate remainsuncommitted. Docsacceptanceupdatependingcommit. No newinstaller/release made; alpha12 remainslatestpublic anddoesnotcontainT15fixes. Thirdpartyprivateaccounts unavailable, isolatedproviderOAuth/HTTPfixtures +realGit/SSH/Runner used. T17 remainsunfinished outsidecurrentfeedback.

## Active alpha.13 release — 2026-10-29
User continued after nextalpha proposal, explicitlyauthorizes packaging/publication. Target7780dfed0081711a05ae96e0510e998606f7a6ea (onlyversionschanged afteracceptedT15). npm/Cargo/desktop versions0.1.0-alpha.13; locksregeneratedoffline onlylocalpackageschanged. Versionconsistency/diffcheckPASS; no localRust/Go builds.
- Fullbuild40789031725; image40789037062; auto native40789011162 and focused40789010962. Next inspectgates, fixactualfailures, publishviaexistingpublish-verified.yml withsuccessfulfullrunID and /tmp/ferry-alpha13-notes.md. Then dispatchserver-image versiontag workflow and verifypublicassets/signatures/catalog/digests.
- No release/tag yet. NotesexplicitlyT17desktopunattendedupgradeunfinished; thirdpartyprivateaccountsnotavailable; K8s/Helm deferred. No featureexpansion. Local.claudestateexcludedfromcommits. Use Actions for allheavybuilds andartifactpublication; avoidslowlocaldownloads.
- Full40789031725 found2rootcauses: strengthenedSQLiteNotifytest inspectedownershipbeforequerywait and could missnotification (test-onlyrace); oldpipeline template blanktarget_kind nowemptyhid hostpicker (productcompatibilitybug). Also originalbuildUIfixturestillusedremoveddefaultbranch connectionUI. RED actions-form test confirmedtargetkind; minimalfix80b33c5 restoreshiddenkindhost/nohostselected, clearscheckoutbindingsonlegacygeneration, testNotifynowtwo-wayhandshake, buildUIcreatesnewconnectionthen seedslegacyrepositoryforoldpipelinecoverage. Local98PASS1skip/typecheck/build/rustfmt/syntaxPASS.
- Newrelease target80b33c5d2fd339717798778d895d441e6a5adf0d; full40789828363, image40789828873. Cancelledstale778full/image andduplicateautofocused/native at80. Finalfullincludesallgates. No tagrelease yet; notes/publicverify scriptready/tmp/ferry-alpha13-*. Publicverificationrequires targetSHAargument.
- Full80gates: checksPASS, all3nativePASS, SSH/Runner/variablesPASS; e2e-build-ui oldlistbutton still'创建'now'创建连接'; mixedlegacyActionsfixture omittednativecheckout thenexpectedfiles inactisolatedworkspace (correctfixture addscheckoutbeforecat); upgradeAPIcomparison lackednewemptycheckout_bindingsdefault. Fixedtestonly9b292a6+eee285a. Sourceproductunchangedsince80. Focused40790824284 at9b includes templates/buildUI addedtobrowserlist; pending. Oldfull80stillfinishinginstallers/deployments tosavecaches; image80pending. Newfull/imageeeequeuedusingexistingworkflow; inspectlistforIDs. No releaseyet.
- Finalintendedreleaseeee285a (getfullSHA via gitrevparse; productidentical80). Finalfull40791064559 queuedbehindold80full40789828363 (remainingLinuxinstaller/IntelMaccompile/Compose3rounds). Finalimage40791064237 SUCCESS botharches. Prior80image40789828873alsoSUCCESS. Focus9b40790824284 browserALLPASS incltemplates/buildUI; currentlyliveActions (mixedcheckoutfix).
- Old80fullWindowsdesktopinstallation/scopes/upgradesPASS,MacARMdesktopPASS, all3nativePASS, checksPASS. Oldknownfailuresonlytests nowcorrectedeee; letoldremainingdesktopfinishforcache thenfinalfullmustSUCCESS beforepublish. Existingpublicationworkflowcanonlypromotesuccessfulbuild.yml; doNOTpublisholdfailedrun.
- Focus40790824284 SUCCESS aftertestfixturecorrections; browserincludeslegacytemplates/buildUI andrealActionsmixedcheckout PASS. Finalimage40791064237 SUCCESS at eee285a.
- Old80full40789828363 completedfailure (onlyknownbackendsupgrade-default/buildUIbutton/mixedcheckoutfixture; allinstallersinclIntelMacandallothergatespassed). Allplatformcachesready. Finalfull40791064559 nowrunning at eee285a, no failuresyet. Need waitsandinspect, publishonlythissuccessfulfullrun. /tmp/ferry-alpha13-notes.md includescorrectfinalfull/imageURLs. No publicalpha13 yet.

- Full eee40791064559 exposed fixture failures: Docker identity commands defaulted to bash absent from alpine testimage; canvas count asserted before async edgesrender. Logs /tmp/ferry-alpha13-job-123186008903.log and ...8931.log. Minimal test-only75796c08600c916064d88acf09dc1c670cf89287 explicitly shellsh and bounded5s exactedgecounts; allassertionsretained. Localcanvas3consecutivePASS,PythonAST/diffcheckPASS. Newfull40794997196 pendingoldfull; image40794996760 +focused40794987562 running. NotesCIlinksupdated; releaseonlysuccessfulnewfull. No tag/releaseyet. Productunchangedsince80b33c5.
- Final75796c0 focused40794987562 SUCCESS includingrealActions+allbrowser+3DB; image40794996760 SUCCESS botharches. Oldfull40791064559 completedfailureONLYknown2testfixtures; allinstallers/native/othergatesPASS. Finalfull40794997196 nowrunning,20jobsstarted. Awaitthisfullsuccess thenpublish.
- Finalfull40794997196: checks/backends/all3nativeRunner/SSH/Agent/UI/variables/Docker/native/stack/groups and pipelines3rounds SUCCESS. Bothpriorfixturefailuresresolvedinfullrun. RemainingCompose3rounds +Linux/Windows/IntelMacdesktop; ARMdesktopSUCCESS. Stillnopublicrelease.
- Finalfull40794997196 attempt1 FAILED onlyWindowsinstaller test at alpha7 CurrentUser UPDATE cleanup: auto restart visiblewindow found, CloseMainWindow loop didnotexit30s. AllotherjobsSUCCESS includingallinstallersnonWindows/pipelines3rounds/UI/native. Same desktop+installer/nativeUI scripts unchanged since80 and passed80full+eeefull (gitdiffconfirmed). Log /tmp/ferry-alpha13-final-failed.log. CauseNOTyetproven; initiated ONE failedjob rerun attempt2 to determine reproducibility, not claimingfix. If repeats investigateWindowswindowenumeration/lifecycle withdiagnostics, doNOTblindrerunagain. No release.
- Full40794997196 attempt2 WindowsALLPASS includingCurrentUser/AllUsers oldalpha4/6/7 upgrade+autolaunch+exit; prior30sexittimeoutdidnotreproduce, rootcauseunconfirmed, release notes explicitlydisclose. No assertionschanged. Windowscachefinished; releasecollectionrunning. Publishpayload /tmp/ferry-alpha13-publish.json ready, exactsource75796c0. Do notdispatchuntilfullconclusionsuccess.

## Delivered alpha.13 — 2026-10-29
- PUBLIC prerelease https://github.com/7link-team/ferry/releases/tag/v0.1.0-alpha.13 published2026-10-29T01:41:56Z; exacttag75796c08600c916064d88acf09dc1c670cf89287 verified. Windows https://github.com/7link-team/ferry/releases/download/v0.1.0-alpha.13/Ferry_0.1.0-alpha.13_x64-setup.exe (28,378,823bytes).
- Full40794997196 SUCCESS attempt2. FirstattemptWindowsoldalpha7upgradetestautostartsuccessbutnormalexit30stimeout; samecommitfailedjobrerunpassedallCurrentUser/AllUsers/scopes/updates+exit. Rootcauseunconfirmed; explicitlydisclosedinrelease notes. Allotherfullgatespassedfirstattempt. Focus40794987562SUCCESS; image40794996760SUCCESSbotharch.
- Publish40801930031SUCCESS verifieschecksums/Minisign/catalog/desktopmanifestanduploads21assetsfromsuccessfulfullrun. Imageversion40802064375SUCCESS verifiesamd64+arm64 and exactverifiedSHAmanifesttag ghcr.io/7link-team/ferry-server:v0.1.0-alpha.13.
- Publicverification /tmp/ferry-alpha13-public-verify.mjs 75796c08600c916064d88acf09dc1c670cf89287 PASS: exacttag/prerelease/draftfalse,21assetdigests,cryptographicRunnercatalogsignature,5runner+5desktopupdatebindings andsignatures. No localheavybuild/download.
- Selfreview + originalrequirementacceptance complete forT15andrelease. Userrequestedfullwidth/nodesources+executors/actualserialparallel/tag-envselection/connectionwithoutbranch/existingcredentials allcoveredbybrowser+unit+realexecutionchecks. T17 desktopunattendedUAC-freeupdater remainsunfinished; thirdpartyprivateaccountsnotavailable; K8s/Helmdeferred. Do notclaimthesecomplete.
- Release task COMPLETE. Workingtree onlylocal.claude files (excludedfromcommits). Usernext: testpublishedalpha13. FutureWindowsflakyexitinvestigation/T17remain distinct; no pending releaseactions.


## T10 旧工作区验收记录归档 — 2026-10-29
来源：`ferry-platform` 工作区未提交的历史交接记录；以下是当时的验收结果，不代表本次重新执行了测试。
- 提交 `bf8843d` 实现工作空间成员、角色、邀请及权限过滤的真实仪表盘；包含 owner/内置角色保护、越权和使用中角色限制、版本冲突保护，邀请轮换与加入串行化。
- 当时最终验收：SQLite/MySQL 8.4/PostgreSQL 17 各 17 组 API 检查（含真实失败流水线指标）、各 99 项原有回归；三库 alpha.6 升级检查保留角色/成员版本及原业务数据；Chromium 工作空间和平台回归通过；前端 39 项、Rust workspace 92 项通过/1 项跳过。
- 当时主工作区集成验收：类型检查、构建、44 项前端测试、99 项 Rust 测试/1 项跳过、17 组 API、工作空间与变量页面浏览器测试通过。
- 临时数据库容器及卷已在当时删除。旧记录中“等待 T8 保留语义”的待办已被后续用户决策及发布状态取代，不再作为当前待办。
- 两个旧分支的全部提交现均已包含在 main；没有需额外合并的源代码。旧工作区新增的 T9 超管决策已由 ADR.md 的 T9 条目完整覆盖。

- 清理完成：删除前两个分支均通过 main 祖先检查；有用记录已提交归档（37a1926）。删除的是旧工作区及约 5 GB 构建缓存，node_modules 仅解除软链接，主项目依赖保留。没有删除未合并代码，未改动任何发布标签。

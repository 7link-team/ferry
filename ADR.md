# Architecture Decision Records

长期产品与架构决策。历史条目的状态对应记录时点；后续明确决策优先，当前交付状态见 [MEMORY.md](MEMORY.md)。
## T2 流水线定义
- Decision：definition 在资源 API 和数据库中使用 JSON 字符串，纯函数校验后返回结构化 Definition。
- Reason：任务明确以 &str JSON 为输入，沿用已有资源字段风格；不引入执行机制。
- Alternatives Considered：直接接收 JSON 对象；本任务不额外增加双格式接口。
- Tradeoffs：客户端需序列化定义；workdir 必填，env 和步骤 timeout_seconds 可省略，未知字段拒绝，避免误拼静默失效。
- Scope：仅 pipelines 服务端。
- Status：已实施。
## T2 shell 与模板边界
- Decision：允许 run 中换行、分号、反引号和 $()；拒绝 NUL；模板只接受四个精确名称，不折叠空格或大小写。
- Reason：run 是运维编写的 shell 程序；定义校验不充当执行沙箱。
- Alternatives Considered：禁止 shell 元字符会阻止正常构建脚本。
- Tradeoffs：执行权限及模板替换安全属于后续执行引擎设计。
- Scope：T2 纯函数校验和对抗测试。
- Status：已实施并单测通过。

## T3a 数据层基础机制
- Decision：新 locks / worker_ids 用 SeaORM 实体与 schema builder，追加迁移；保留旧主键、旧迁移及 db::id 调用。
- Reason：用户明确执行 T3a，其禁止事项限定本次范围；§18.5 全量改造属于后续工作。
- Alternatives Considered：同步重写全部数据层；超出任务范围，未做。
- Tradeoffs：仓库暂时并存既有 raw SQL 与新 ORM 查询；新增机制没有 raw SQL。
- Scope：T3a；Status：实施。

## T3a 租约与分配原子性
- Decision：10 秒心跳 / 60 秒 TTL；本地使用 Instant，从数据库写入开始时刻计时；等待发号时也检查租约。分配在事务内取得锁，并通过旧 owner/heartbeat 条件更新槽位，避免回收期间覆盖刚恢复的心跳。
- Reason：墙钟回拨不能延长本地安全期限，慢响应不能额外续命；分配失败或取消必须回滚释放锁。
- Alternatives Considered：事务外持有 TTL 锁；操作超过 TTL 时无法确保串行分配。
- Tradeoffs：锁竞争/候选槽位并发续期时明确拒绝启动，不增加隐式重试。跨实例租约仍按设计依赖主机时钟同步；复制数据目录不能同时启动同一个 instance_id。
- Scope：worker 生命周期；Status：实施。

## T3a 优雅关闭与重启复用
- Decision：优雅关闭先停心跳/发号，再将本 owner 的 heartbeat_at 置 0，保留槽位映射。
- Reason：同时满足立即释放槽位和同 instance_id 重启优先复用；若槽位已被其他实例回收，则走最小空闲槽位分配。
- Alternatives Considered：删除行会丢失重启亲和性；不释放则占用到 TTL。
- Tradeoffs：保留最多 1024 行；Status：实施。

## T3b ID 存储与边界
- Decision：重写旧迁移的 41 个业务 ID 列为 BIGINT；保留原生 SQL。State::next_id 返回 Result<Id>，失败转 503；Id 绑定为 i64，JSON 序列化为字符串。移除 db::id。
- Reason：任务书与 §18.2/18.5；逐表核对实际为 41 列（14 表），不是预估约 39 列。
- Alternatives Considered：兼容迁移、SeaORM 实体改造；均不属于 T3b。
- Tradeoffs：本地旧测试库重建；可选引用空字符串输入解析为 0，输出为 "0"，校验允许 0 回传；必需引用仍由现有校验拒绝不存在的记录。请求体沿用字符串接收并显式解析，保持既有 400 JSON 错误响应。
- Scope：T3b；Status：实施，三库首轮各 98/98。

## T3c 强类型数据访问
- Decision：16 张表的实体集中在 entities.rs；业务读写使用 Entity/ActiveModel/Select，resource::list 泛型查询保留 Spec 白名单后解析实体 Column，Condition 绑定所有值。
- Reason：任务书和设计 §18.5；不增加继承式公共模型或运行时 SQL 转译层。
- Alternatives Considered：保留字符串 SQL 辅助层、取消列名白名单；均违背任务要求。
- Tradeoffs：host_tags 尚无业务调用者，但任务要求每张表均有镜像，针对该模块以有原因的 expect(dead_code) 标注，不伪造调用。实体不派生 Serialize，HTTP 显式映射 Id。凭据列表/详情通过 Summary 投影只读取公开字段及 has_secret 计算结果，保留原查询层不读取密文的防护；共享list支持类型化投影。
- Scope：只改数据访问；迁移 DDL/执行逻辑不动，migration.rs 仅 schema 测试移除已删除的 db helper 依赖；SQLite PRAGMA 元数据检查保留注释说明。
- Status：完成；三库基线及最终各 99/99，全部门槛退出0，两次变异正确失败并恢复验证通过。

## T3d Schema builder
- Decision：已有迁移全部改为 Table/Index/Alter builder，保留名称与顺序；索引和加列通过 SchemaManager 存在性检查重跑。MySQL 设置 utf8mb4_bin。
- Reason：任务书要求删除 DDL 字符串转换，同时保留 MySQL 可启动、大小写与幂等保护。
- Alternatives Considered：直接 .text() 在 MySQL 上会生成 TEXT 而非 LONGTEXT（SeaQuery 1.0.2 源码已确认）；只使用它会缩小原容量且违反无裸 TEXT 保护。
- Tradeoffs：长列以 .text() 定义，MySQL 专用 builder .custom("longtext") 保留容量；没有 SQL 字符串改写。SQLite 的 BigInteger 正常生成 INTEGER，元数据检查适配，并以实际 MySQL 生成 SQL 验证同一41列仍为 BIGINT，不能退化为32位整数。
- Scope：T3d 迁移与其保护测试；业务、JSON、错误文案和矩阵断言不变。
- Status：完成；全部门槛退出0，新建三库各99/99，两库last/all重放无业务数据或结构变化；变异退出101且恢复后通过。

## T4 执行引擎（进行中）
- Decision：三张独立表；工作空间行的计数器在事务内 UPDATE + 读取分配 number，唯一索引兜底。触发保存定义、仓库配置与目标主机快照；队列 CAS 领取，线性执行。
- Reason：工作空间内并发发号、运行历史可复现、单实例调度约束。
- Alternatives Considered：MAX(number)+1 先查后写有竞态；凭据放 exec/env argv 会泄露，因此拒绝。
- Tradeoffs：HostChannel 捕获输出后返回（沿用既有单步输出上限），本期按步分片。取消只停止等待，远端可能继续；清理临时目录后再结束执行器。取消状态可早于清理完成。
- Scope：T4 后端、真实验收脚本与 §17 文档；不改 UI、不加 exec HTTP、不 commit/push。
- Status：主机组按已告知的最小单目标假设，触发时选直接成员中 ID 最小的在线主机并固定 host_id；无在线成员拒绝，不做多机编排。触发可提供 branch/commit 模板上下文，缺省为仓库默认分支/空字符串。完成：三库原矩阵各99/99，真实执行各44/44，两次变异正确失败并恢复，全部门槛退出0。

## T5 构建界面
- Decision：仓库、流水线、历史仅配置现有 ResourceTable/ResourceForm；DSL 新增 code 与 resetOnChange，旧字段行为不变。唯一新增页面为 PipelineRunDetail，日志分片为其内部 memo 渲染单元。
- Reason：任务明确禁止另写资源表格/表单；JSON 需等宽多行，目标切换需清空旧引用。
- Alternatives Considered：手写资源组件、修改服务端以返回关联名称，均不在范围内。
- Tradeoffs：当前 API 只返回关联 ID，界面明确标注流水线/主机/触发人 ID。日志先读取状态、再按 next_seq 排空每页200片，终态排空后停止；隐藏abort并暂停，恢复可见立即读取未完成任务；稳定对象+seq+memo保留旧日志DOM，只有在底部才跟随。
- Scope：T5 前端及验收脚本；没有服务端、依赖、ResourceTable行为改动，无commit/push。
- Status：完成。23项前端测试通过；真实Chromium→服务端→runner→HTTP Git链路通过；终态继续轮询变异被三项测试捕获，恢复后通过。

## T6 CI / release preparation
- Decision：使用四个 e2e matrix job，每个连续三轮且任一轮失败立即退出；日志与 Summary 标注轮次/run id/SHA。release needs 包含整个 e2e matrix。
- Reason：失败按 harness 定位，连续运行覆盖清理竞态；不是失败重试，不能用后续成功掩盖失败。
- Alternatives Considered：合并单 job 会延长反馈并让前项失败遮住后项；触发三次完整打包成本更高。
- Tradeoffs：三轮共享同一 runner；只能提供已实际运行的证据，不能数学证明永无 flaky。
- Scope：T6；runner exec 复用 pipeline API，UI 用临时裸仓库 fixture branch 兼容 detached HEAD；无产品 API 变更。
- Status：本地四个harness均连续三轮通过；CI 三轮和 dispatch 发布产物收集必须用户提交推送后验证。不 commit/push/tag/release。手动触发即使选择 tag 也不发布，只有 push v tag 调用发布 action。
- T6 stability finding：本地UI第2轮实际复现状态/日志轮询竞态。用fixture文件屏障同步观察与后续执行，替换固定sleep；保留原断言并重新跑连续三轮。不是重试，不改产品轮询行为。

## T6 远端验收补记（2026-10-25）
- Decision：T6 验收通过；原“等待推送/CI待验证”状态已被远端证据取代。
- Reason：f1d7b0f 对应 runs 40110398175、40112246163、40113131064 全部 success，逐一核对四个 harness 每轮日志（每个 job 3/3）。
- Tradeoffs：三连绿满足门槛，不代表永无 flaky；发布演练验证收集/校验和生成/上传，不代表已正式发布。
- Scope：本次仅验收与状态同步。f1d7b0f 的临时 Git 仓库修复消除 UI fixture 对项目浅克隆历史的依赖。
- Status：两次 dispatch 均上传 10 个资产和 SHA256SUMS.txt；发布 action skipped。正式发版仍由用户决定。

## 首个预发布版本
- Decision：用户确认 v0.1.0-alpha.1；所有产品版本配置一致，GitHub Release 必须 prerelease=true。
- Reason：用户澄清当前不是正式版；保留0.1.0用于未来正式版。
- Scope：仅版本元数据和发布；误推v0.1.0已取消流程并删除标签，没有发布资产或版本镜像。
- Status：已发布。d607b63；main CI 40147891295、镜像40147891375及tag发布40148433585、镜像版本40148433485均success。Release prerelease=true；11资产齐全，校验和匹配；双架构镜像已发布。

## 发布门槛捕获 Git ASKPASS 用户名参数泄露
- Decision：使用 Git credential helper 标准管道协议，禁用 ASKPASS 回退，保留临时文件权限与清理机制。
- Reason：alpha.2 标签 CI 真实捕获 SSH 进程参数含凭据用户名；增加凭据进程采样窗口后，旧实现两种通道均稳定失败，新实现连续三轮各46项通过。
- Alternatives Considered：仅重跑测试、放宽用户名断言均不能解决问题，未采用。
- Tradeoffs：不再依赖 ASKPASS 提示文本；密码/用户名通过 helper 标准输入输出传递，不进入命令参数。
- Scope：发版必要修复及回归测试；Status：6e58782 已推送。

## 后续预发布使用 alpha.3
- Decision：保留已公开 alpha.2 Git/容器标签，后续发布 alpha.3。
- Reason：alpha.2 桌面 Release 因真实测试失败未发布，而版本容器已公开；不覆盖公开版本。用户已授权下一次预发布，“继续”“都要”覆盖后续发版。先前额外版本确认被明确撤回为过度谨慎，不将沉默当作授权。
- Alternatives Considered：覆盖 alpha.2 标签/镜像，需要改变已公开引用，未采用。
- Tradeoffs：版本序号跳过没有桌面发布的 alpha.2；alpha.1 用户手动安装 alpha.3 后启用后续应用内更新。
- Scope：版本与发版；Status：已发布 v0.1.0-alpha.3，abce34f；主分支及标签 CI/镜像全部成功，19项资产及签名/校验和通过最终验证。

## Desktop blank-address launch defect
- Decision: prioritize user's installed desktop blocker ahead of approved variables work. Empty address starts the existing bundled server, stores data under Tauri per-user app_local_data_dir, and waits for existing /api/status plus matching instance-id. Reject occupied port; never attach blindly. Hide launcher only after workspace creation; restore on workspace close. Stop owned child on app exit; suppress Windows console.
- Reason: released alpha.3 server.rs contained only URL navigation, no actual sidecar lifecycle or launcher hiding; installer tests did not exercise first launch.
- Alternatives Considered: asking user to manually run server or enter localhost would not implement documented blank-address behavior.
- Tradeoffs: native installed Windows UI still requires verification; backend lifecycle has real local process coverage and a new CI matrix gate. No server API/data migration or variable design changes.
- Scope / Status: local desktop hotfix implemented and tested; not yet published. Variables remain authorized and queued.

## alpha.4 桌面热修复交付
- Decision：发布 v0.1.0-alpha.4 测试版；用户“继续”覆盖 Windows 验收和发版，密钥变量模块保留为后续已授权工作。
- Reason：alpha.3 留空地址没有启动本地服务，连接窗口也未隐藏；本次补齐既定桌面服务生命周期。
- Alternatives Considered：手动启动后端不能满足安装后留空使用；不绕过 Windows 真实测试。
- Tradeoffs：Windows 单元测试首次暴露控件声明缺失（Tauri 只嵌入普通应用），CI 提取打包应用同一 manifest 给测试程序，全部原断言保留；非产品业务改动。
- Scope / Status：1495453 热修复、a0365af 测试声明修正已推送；main40197659996/tag40198613433及镜像全部成功。Windows 两种安装范围均验证真实页面、隐藏/恢复、重开和退出清理；19公开资产、18校验和、5更新签名及版本绑定全部通过。alpha.4 已发布，不是正式版。原密钥变量工作尚未实施，不视作取消。

## T8 独立资源实现 / 保留策略待答复
- Decision：继续用户已批准的密钥变量模块；独立实现加密资源、共享表单、作用域解析和测试，尚不决定旧密钥保留。
- Reason：回滚恢复当时值会加密保留旧密钥；回滚使用当前值会改变完整配置恢复语义。2026-10-25已发异步二选一问题，推荐历史加密快照，尚未收到答复。CLAUDE §2适用，不将继续模块等同于选择保留政策。
- Alternatives Considered：历史快照/当前值两方案已写docs/tasks/T8-variables.md。
- Tradeoffs：实际Compose交付尚未接入。pure render明确标记未绑定引用，执行入口拒绝标记，绝不静默漏掉环境项。
- Scope / Status：本地变量资源/共享UI/测试已实施；回滚与快照待选择。没有commit/push/release；不影响已发布alpha.4。

## Desktop automatic upgrade / tray (alpha.5 testing)
- Decision: signed download precedes private stdin EOF drain; stop dequeuing and reject new browser work while preserving runner transport for active steps/cleanup; await exit0 before installation. Timeout aborts install without killing work. Save one-use workspace resume marker; passive installer restarts automatically.
- Reason: Windows updater directly exits process and bypasses ordinary RunEvent cleanup; prior basicUi repeats installer choices, tray dependency alone never created a tray.
- Alternatives considered: force-kill active sidecar, public shutdown endpoint, asking users to manually end work do not satisfy requested graceful automatic upgrade.
- Tradeoffs: old binaries cannot acquire this lifecycle before first upgrade; old-client installer arguments are supported by minimal vendored CLI2.12.1 template modification, with first-install scope choice preserved. A one-time old-client transition limitation must be disclosed.
- Scope/status: user-authorized desktop fix, commits through6674792; release alpha.5 NOT YET published. Windows CurrentUser real checks passed; AllUsers overflow geometry verification pending final CI40208925861. No platform-superadmin role or first-run authentication policy changed. T8 snapshot retention choice still pending.

## alpha.5 发布验收
- Decision：发布 v0.1.0-alpha.5 测试版，commit6674792；桌面修复验收PASS，密钥变量仍保留本地未发布。
- Evidence：最终dispatch40208925861/tag40210357191全部成功；Windows两种范围实测通过托盘（含系统折叠区）、隐藏/恢复、跨版本无交互安装和自动重启；19公开资产、18校验和、5签名/版本绑定验证通过。两个镜像工作流成功，匿名registry验证实际ferry-server:v0.1.0-alpha.5包含amd64/arm64。
- Tradeoff：已发布说明明确旧客户端第一次升级仍需等待任务结束、退出再直接更新；新生命周期只能从新客户端发起后生效。主分支重复CI40210356975的checks仍在安装OS依赖，其余job已通过；不谎称该重复job已完成，同commit发布检查已全绿。
- Status：公开下载和应用内更新已可用。账号仍为注册/空间拥有者，无默认超管；首次初始化只提出建议，未擅自改变权限模型。T8旧密钥保留选择仍待答复。

## T9 — confirmed platform boundary and alpha.6 delivery
- Decision: user explicitly selected a real platform superadmin and platform management page; prior workspace-owner-only recommendation is superseded. Status: APPROVED and IMPLEMENTED in0bb0f51.
- Reason: multi-account/multi-workspace platform needs global administration distinct from tenant ownership. Global routes /_platform and /api/platform avoid collision with existing valid workspace slug platform.
- Security/compatibility: no default password, no automatic promotion of legacy users. One-time private operator proof plus human credentials; existing account claim requires its old password. Desktop owns proof handoff; server operator supplies it. Existing authenticated installs are prompted but may defer to continue workspace use. Parallel setup loser sees a login form.
- Transaction boundary: singleton platform lock serializes setup, account privilege/status mutations, registration gate/settings. Last active admin protected and actor rechecked; affected sessions revoked. Workspace creation and audit reused within transaction; shared form/list DSL extended with explicit global endpoint.
- Registration policy: enabled after initialization by default, configurable by superadmin. No password-reset or workspace-delete feature added.
- Integration: original T8 changes restored on main, migration order PlatformInitialization then Variables; T8 remains uncommitted and excluded from alpha.6. No decision on T8 secret-retention/rollback is implied.

## T8 历史变量回滚（已确认）
Decision：用户明确选择恢复历史变量值；revision变量快照以现有vault加密，AAD绑定workspace/revision，随历史删除。Reason：维持精确回滚。Alternatives：使用当前变量不保留旧密钥（未选择）。Tradeoffs：旧密钥可能已失效，UI提示。Scope：Compose变量引用，不扩展流水线。Status：2026-10-26用户已确认，实施中。

## T11 — creation-time app/pipeline templates
Decision: template CRUD plus shared form selection/parameter generation, no live binding. Reason: confirmed next scope reuses complete existing deployment/pipeline creation and execution. Typed .Values parameters only; app secret inputs are existing variable names and generate valueFrom references, never plaintext; pipeline secret explicitly rejected until engine supports variables. Tradeoffs: template updates require explicit reload; deletion leaves created resources intact; no service config file templates or marketplace. Status: implemented and locally accepted 2026-10-26; no release/version change.

## T12 full product completion — APPROVED 2026-10-26
User explicitly confirmed complete product scope including Docker/native/Swarm/K8s/Helm/artifacts/configfiles in addition to remaining navigation and import/export. External hosted Git platforms only (GitHub/GitLab/Gitee/Codeup), no internal Git hosting. Cron explicit timezone; missed schedule not caught up, overlapping skipped with record, retries default off but configurable. History retained until admin sets retention days. Alerts in-app and configuredWebhook. Implement in stages, real acceptance, no release requested.

## T12 alerts — APPROVED
Decision: user confirmed configurable thresholds, one unresolved event during continuous abnormality, record recovery, one Webhook attempt for trigger and recovery, failure recorded without retry. Reason: avoid notification storms. Scope: alerts stage3, no unsolicited external notification; tests use owned local receiver. Status: confirmed, implementation next.

## T12 external repository authorization — APPROVED
Decision: user explicitly says implement now; prefer browser/web authorization where supported, use API authorization where web flow is unsuitable/unavailable. Reason: user should not have to manually provision every integration token. Scope: GitHub/GitLab/Gitee/Codeup connections, authorization status/expiry/reauthorization; encrypted credentials reused. Tradeoff: deployment operator must configure registered OAuth app credentials/redirects; no actual private provider accounts were supplied, do not claim live private authorization acceptance. Status: user confirmed, implementation pending; not blocked waiting for testrepo question.

## 2026-10-27 release scope amendment
Decision: user defers K8s/Helm, requests every other remaining feature completed/tested and next alpha published for testing. Release now explicitly authorized, supersedes prior no-release scope. Keep Docker/native/Swarm/groups/artifacts/config/import-export plus desktop fixes. Do not silently treat next alpha as only existing changes.

## T12 final correctness fixes
- Shared creation transaction uses stable existing workspace row lock; actual MySQL concurrent import confirmations proved gap/insert deadlock in prior ephemeral lock table. Reuse existing lock_workspace, no retry/fallback policy added.
- Scope audit found host_tags unused; implemented original many-to-many intent via shared host multiselect/filter/CRUD/import. Omitted tag_ids preserves existing associations; explicit [] clears. Referenced tags and groups follow existing refusal-to-orphan policy. Parent groups validated for same workspace/cycles.
- Local acceptance complete for non-K8s/Helm scope, external live provider accounts remain unavailable as already disclosed. Bump next authorized alpha to0.1.0-alpha.7; remote CI/Windows/signed assets are delivery gates, no further user confirmation required.

## alpha.7 release acceptance — 2026-10-27
Decision: publish authorized non-K8s/Helm completion as prerelease v0.1.0-alpha.7 at8b4125c. Main40386314056, tag40394778911, images40386314066/40394779052 allSUCCESS.19public assets/18checksums/5signed versions verified. Windows both scopes andalpha4/6 upgrades, connection reopening/tray/lifecycle passed. CI systemd/tmp fixture race and positional pipeline-form selector fixed without relaxing assertions. Live third-party private accounts remain unverified; no default admin password; K8s/Helm deferred. Status: DELIVERED, release notes disclose limitations.

## Deployment editing UI — confirmed 2026-10-27
Decision: deployment configuration is a form; no drag/reorder or graph controls. User explicitly distinguishes build workflow visualization (React Flow/GitHub Actions style) from deployment settings. Scope: existing application manifest editor, preserve source editing and existing server contract; no new build graph implementation in this correction. YAML parser dependency used to preserve existing YAML/JSON editing safely. Status: deployment editor implemented, final browser acceptance pending.

## alpha.9 release acceptance — 2026-10-27
Decision: deliver authorized form/menu/repository/pipeline corrections as prerelease0.1.0-alpha.9 atfe4db1a; preserve publicalpha8tag/image, alpha8desktopneverpublished. Reason: alpha8 gates exposed draft reset and native rollback race; both fixed and regression verified. Main40443729070/tag40445832180 SUCCESS; main/tag images40443729071/40445832022SUCCESS. TagSwarmfixture cleanup race required unchanged full3-runjob rerun, final3x53PASS; no product assertions relaxed.19publicassets/18checksums/5signatures/versionbindings/anonymousOCIamd64+arm64PASS. Scope: deployment form, not build graph; K8s/Helm deferred, real private provider OAuth accounts still unverified. Status: DELIVERED; no further release action pending.

## Dynamic platform OAuth and unified repository creation — 2026-10-27
Decision: user requests dynamic platform app configuration and explicitly rejects repository/auth tabs. One repository create/edit form chooses GitHub/GitLab/Gitee/Gitea/Codeup/plainGit; provider web OAuth or validatedPAT then actual repositoryselection; plainGit inline password/SSH/token atomically saved withrepository. Reason: priorPATshortcut onlycreatedcredential and didnotconnectprovider. Alternatives: separateauthorizationtab/manualcredentialbinding rejectedbyuser. Scope: existingplatformsettings+sharedResourceForm+connection/vault reused, noGit hosting, noextraAPI-tokenmodule. Appsettingssuperadminonly/versioned/audited/encrypted; envfallbackuntilwebsave, storedsnapshotauthoritative thereafter; secretomitted/blankretains, disablekeepsconfig. Workspaces authorize separately; existingactor/providerconnection IDs preserved onreauthorization. PATtargetsboundtoconnectionindependentfromOAuthenable. CodeupAPIhasnoPATuseridentity, requiresGitusernameandorganization forcentralAPI; listverificationneverclaimedidentityverification. GiteaOAuth+PATincluded; marketGiteafileadapternotexpanded. BrowserCOOPisolationusesstate-boundrandomBroadcastChannel, nosecretstoredinbrowserstorage. Status: implemented/localacceptancePASS; realexternalprivateaccounts andinstalledWebViewuntested, no newreleaseyet.

## T14 multi-environment pipelines — APPROVED
Decision: user explicitly selected "同一流水线支持多个环境，运行时选择（推荐）" on2026-10-27. Implement multiple environment bindings per pipeline; choose one at trigger; freeze chosen environment's executor/variables into history. Existing apps/deployments and tags reused, no duplicate app registry. Status: approved, implementation starting. Earlier pending-environment statements superseded.

## T15 explicit user correction — 2026-10-28
Decision: code connection consists of platform + auth, existing credentials reusable; repository/branch chosen at pipeline build configuration. Supersedes previous decision requiring repository choice at connection creation. Reason: user explicitly rejects premature branch/repository binding and repeated credential input. Scope: connection management and pipeline source selection; preserve old source bindings/history. Status: APPROVED by explicit request; implementation pending.

## T16 Runner lifecycle required in next alpha — APPROVED
Decision: user explicitly says “agent runner 这个版本就要” after discussion of service installation, auto recovery and graceful automatic upgrade. Include Linux/macOS/Windows native services/autostart/crash restart and drain→verified upgrade→healthcheck→rollback in next alpha, not a later release. Existing network reconnect retained. Boundary: brief agent offline during replacement is disclosed; no zero-disconnect/automatic failed-job resume claim. Scope includes necessary signed artifacts and native platform acceptance. Status APPROVED, not implemented.

## T17 desktop unattended Windows updates — APPROVED 2026-10-28
Decision: user approved independent updater, ordinary helper for CurrentUser and protected initially-authorized native updater service for AllUsers. Preserve graceful task drain and automatic original-user restart; eliminate repeated installer wizard/UAC on already provisioned installs. Old installation migration may need one authorization; no OS privilege bypass. Scope: desktop Windows update lifecycle plus necessary signed broker/installer/native tests. AgentRunner supervisor remains separate. Implementation plan docs/tasks/T17-desktop-unattended-updates.md; not implemented/accepted yet.

## 根目录共享项目记录 — 2026-10-29
- Decision：将原 `.claude/decisions.md` 移至根目录 `ADR.md`，原 `.claude/task-state.md` 移至 `MEMORY.md`。ADR 保存长期决策，MEMORY 保存当前状态、限制、验证证据和交接记录。
- Reason：用户要求保留有价值的记录并移至根目录，供不同开发工具共同使用；采用标准拼写 MEMORY。
- Scope：项目文档及其引用；不改变运行时行为。原路径不再维护。
- Status：已实施。历史记录保留，当前状态摘要优先于历史检查点。

## 工作空间 ID 与统一资源路径 — 2026-10-30
- Decision：用户明确选择统一资源路径、去掉菜单分组。页面 `/workspaces/{workspaceId}/{resource}`，API `/api/workspaces/{workspaceId}/{resource}`；节点为 `nodes`、主机分组为 `node-groups`。平台页面 `/platform/{resource}`。菜单和面包屑保留业务层次。
- Identity：使用数据库已有雪花 ID，JSON 和浏览器全程保留十进制字符串。名称/slug 不再作为接口作用域索引；保留 metadata，不修改既有 ID、数据和权限资源名。
- Operations：HTTP 方法配合名词子资源（`runs`、`connection-tests`、`cancellations`、`renderings` 等）；排序用 PATCH `ordering`，批量删除用 DELETE 集合。共享资源用 `scope` 查询参数。
- Compatibility：旧页面书签在登录后通过身份中的可访问工作空间映射为 ID，保留详情和查询；API 拒绝 slug，不增加旧 API 别名。Runner 机器连接协议与安装分发端点维持兼容。
- Supersedes：旧的 slug/菜单分组页面路径和 `/actions/{verb}` 接口约定；不涉及数据库迁移。
- Status：用户已批准，当前实现以 MEMORY.md 验证记录为准。

## Windows terminal start and resource detail layout — 2026-10-30
- User requests a user directory as the default Windows terminal CWD. Prefer own user profile, then active console/RDP user profile when running as LocalSystem; no logged-in user falls back to PUBLIC/system drive. Do not change service identity or permissions.
- All schema-driven detail pages reuse ResourceForm with dedicated read-only section/field styling and available width. Existing editors, secrets, permissions, actions and routes retain their behavior.

## 节点报警上下文与应用菜单 — 2026-10-30
- 用户要求节点详情新增报警规则时保留层级与上下文。复用 ResourceForm 在详情弹窗内创建，固定关联当前节点，忽略外部查询中的复制和目标预填；保存后刷新当前规则列表，保存及取消均不跳转到其他模块。不增加重复 API 或页面资源路由。
- 一级菜单“部署”改为“应用”，共享导航定义同时更新菜单、无障碍名称和面包屑。现有资源 URL、部署操作名称及 API 权限语义不变。
- 已随 `v0.1.0-alpha.21` 交付，源码 `2d9d3b5`；完整 CI 和实际浏览器验收通过，4291 开发服务已更新。后续状态以 MEMORY.md 顶部为准。
# Codeup PAT contract hardening — 2026-10-30

Decision: extend the existing repository adapter and shared checkout UI, retaining named connection IDs, vault references, ownership/workspace permissions and native Git credentials. Verified against current Aliyun PAT API docs: central uses organizations/{organizationId}, Region omits it; repositoryId accepts the encoded FULL path including organization; lists are arrays with x-next-page/x-total-pages. Use header pagination with existing count fallback; decode URL path once before encoding an API segment, using the already locked percent-encoding crate as a direct dependency. Add paged branch suggestions and current-page retry without changing an entered ref. Distinguish upstream authorization/permission/rate/service errors without exposing response bodies or forwarding credentials through redirects. No AccessKey integration, Git hosting or execution-platform expansion. Live Aliyun account acceptance requires a locally configured connection; fixtures must never be represented as live cloud acceptance.
## Credential management module boundaries — 2026-10-31
User requires separate host, build and application authorization pages. Reuse one encrypted credential store with immutable module scope and scoped lists/pickers; retain existing credential permissions. Use existing REST resource `/workspaces/{id}/credentials?scope=host|build|deploy`, keeping menu/breadcrumb context. Upgrade preserves IDs, ciphertext and existing references; host-referenced legacy credentials belong to host, marketplace-referenced credentials to deploy, other legacy credentials retain build ownership. Existing historical references continue to resolve; this change does not redesign repository-connection ownership or runtime RBAC.

## Optional NATS JetStream server clustering - APPROVED 2026-10-31
Decision: user approves NATS JetStream for node connection ownership and cross-instance commands/terminal/files/monitor access. Preserve standalone operation without broker. Use expiring ownership with fencing, bounded streams and no mutating-command replay. Owner loss ends existing terminal; Runner reconnect allows a new session. Monitoring recent history remains owner memory (one hour); no fabricated backfill or long-term TSDB this round. Scope includes cross-instance resets and upgrade/task exclusion. Shared business database and vault master key required; separate data directories per server. Status: approved, implementation/testing in progress, not delivered.

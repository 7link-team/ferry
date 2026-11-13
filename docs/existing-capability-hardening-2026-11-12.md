# 现有能力完善与验收范围

日期：2026-11-12。用户确认：OKR 暂不做，先完善现有能力。本文是现状与验收顺序，不新增组织树、OKR、计划管理等业务模块，不将设计文档或菜单存在视为已验收。

## 现有产品围绕什么闭环完善

用户提出的目标链：需求 → 生成/确认执行方案与脚本 → 准备工具和依赖 → 执行 → 内部验收 → 必要时由 human 审批裁决 → 交付。执行失败或验收不通过不得被包装成完成；回到有证据的修正步骤，再形成新版本、新执行与新验收结果。

当前源码已经有会话、Agent 平台工具、工作流/流水线、Runner、产物、预览和若干审批机制，但不能据此宣称这条链已经由平台自动贯通。本次视频源码是对话中的编码助手生成的，测试夹具负责建立 Git 源与绑定；并非 Ferry 会话内的 Agent 已自主完成需求解析、创建计划、生成脚本、绑定执行器并发起任务。

## 按现有实现核对

| 能力 | 项目内依据 | 需要补实的成熟度证据 |
|---|---|---|
| 需求会话、原生 Agent 执行与工具调用 | `crates/server/src/conversations/`、`crates/runner/src/agent/`、`docs/agent-platform-tools-design.md` | 同一需求在会话、脚本版本、执行、验收与交付之间有可跳转关联；不能用编码助手在平台外代办冒充平台闭环 |
| 流水线与版本化 checkout | `crates/server/src/pipelines/actions_run/checkout.rs`、`actions_run/job.rs` | 空工作目录检出固定提交，依赖有声明，脚本与产物不依赖测试机绝对路径；成功与失败日志都能复查 |
| React Flow 节点工作流 | `src/features/workflows/WorkflowEditor.tsx`、`crates/server/src/workflows/` | 旧 `host.command` 实测被拒，但后续在当前隔离服务重测普通命令已成功，核对了 stdout 和实际 Runner ID；不可再归为能力不存在。节点视频端到端仍未通过，不能用另一种流水线成功替它验收 |
| 多平台 Runner 与服务化 | `crates/runner/src/service.rs`、`cli.rs`、`crates/protocol/src/agent.rs` | CLI 已有服务安装/启停/状态代码；仍需逐系统实跑，不能把 Rust 跨平台源码等同于 Mac/Linux/Windows 全部验证 |
| 远程预览、文件和终端 | `crates/server/src/holes/`、`src/features/hosts/HostFiles.tsx`、`HostTerminal.tsx` | 校验隧道到期/撤销/断线后释放、访问权限、Web/API 可达与终端重连；当前有域名/IP端口/Cloudflare 模式，不能称已实现 frp |
| 定时、监控与通知 | `crates/server/src/cron_jobs.rs`、`monitoring/`、`notifications/` | 定时触发、重启后恢复、告警去重及通知投递失败要在真实环境串起来；本轮没有重做这些模块的端到端测试 |
| 人工审批 | `crates/server/src/main.rs` 中任务 approval 路由、Runner 原生工具审批 | 已有危险操作/任务门禁不等同于统一交付验收；验收对象、验收版本、责任人和下游接收仍须逐项核对，不能只增加一个“确认”按钮 |
| 记录与知识 | `crates/server/src/conversations.rs`、`content_assets.rs`、既有知识学习与协作 PRD | 会话存在删除实现，不能承诺永久保存；原始消息、文件、业务状态、审计与派生知识的生命周期和权限不同。知识 PRD 仍保持用户要求的实施暂停 |

这些是源码盘点，并非本轮全部重新运行验证。现有预算/QPS/并发控制不能宣称为自动调节策略；现有桌面 Tauri 配置也不能宣称 Android/iOS 已交付。OpenClaw、Hermes 等不在当前 `Runtime` 枚举里，不列为已支持。

## 本轮先修什么

1. 构建产物下载名：后端按存储的逻辑文件名输出 UTF-8 下载名；ZIP 不再被强制命名成 `.bin`。保留文件内容和权限逻辑。已完成 Rust 相关测试、构建、Chrome 下载及哈希比对，详见同日实测报告。
2. 视频示例的可交付性：脚本/提示词/YAML/工具锁定清单一起交付，checkout 绑定固定 SHA，脚本和输出用相对路径，准备依赖并核对完整性，独立探测和完整解码后再上传视频。
3. 失败闭环：下载慢不能无限推进而没有反馈；后端未返回视频必须明确失败，后续验收/成功产物上传必须跳过，存在的失败诊断独立归档，原始执行不可覆盖。

本轮 Windows 原生实测、失败记录和新链接写入 `ferry-vs-lobehub-real-scenarios-2026-11-12.md`。用户确认 Linux 缺环境，先交付 Windows 实测与 Linux 待验证项。工具下载成功不等同于真实视频生成成功，也不等同于视频内容已经由 human 验收。

## 后续验收顺序

先围绕同一任务验收现有链路，再扩大覆盖：

1. 用现有 Ferry 会话与 Agent 平台工具发起一个任务，核对产生的源码/工作流/执行与会话的关联，而非由测试夹具暗中完成关键步骤。
2. 对该任务分别走通成功、执行失败、验收失败、取消和断线恢复；重做产生新版本与新证据，不覆盖旧结果。
3. 下载交付物、核对版本与哈希，确认负责的 human 能找到内部验收结果并作必要裁决。
4. 再覆盖预览回收、Runner 重连和定时通知等现有模块的真实闭环，按实际阻断问题修复。

新增业务规则、权限含义、永久保留策略、自动调模策略和新业务模块仍按项目设计评审门槛处理，不因“完善成熟度”自行决定。没有把上述未执行步骤写成 PASS。

## 会话交付链接修复（2026-11-12）

范围：扩展现有 `ConversationResourceLink`，将同源、同工作空间的流水线与执行记录 API 详情链接转换为对应的本地化界面入口；产物 `/content` 链接使用现有下载接口及下载响应文件名。复用路由、Markdown 和链接组件，不新增页面、字段、接口、权限、依赖或数据库迁移。不转换列表、日志、带查询参数、其他源/空间或不存在的产物详情接口。原有工作流执行的授权查询、加载、错误和重试行为保持。

验收依据：现有导航中的 `pipelines`、`pipeline-runs`，以及产物列表的下载接口。相关测试覆盖界面地址、下载语义、同源/空间限制、非法子路径与原有拒绝/重试行为；`npm test -- src/features/conversations/ConversationResourceLink.test.tsx src/features/conversations/ConversationMarkdown.test.tsx src/app/router.test.ts src/features/pipelines/PipelineRunDetail.test.tsx`：44/44 PASS。

完整会话任务尚未通过：隔离 Runner 的 Agent probe 返回 CLI missing，会话创建返回 409。该记录位于忽略的 `data/maintenance/product-poc-20261112/conversation-delivery-latest.json`；不得将本次链接修复当作 Agent 已完成创建、执行、验收和交付的证据。

实际验证：`npm run build` PASS（含 TypeScript 构建，保留大包体积提示）。Chrome 加载真实 Markdown 组件，登录隔离服务后，实际点击流水线和执行链接打开本地化界面地址，并下载既有诊断产物，文件名为 `video-diagnostics.zip`；3 项检查 PASS，无页面运行异常。证据：忽略目录中的 `delivery-links-review.json`、`link-review-diagnostics.zip`。夹具最初因 HTML 转换入口不可用失败，改为真实 Vite HTML 入口；一次首次加载弹窗超时，保留未通过事实，后续完整运行通过。此验证覆盖链接和下载，不覆盖完整会话、页面全部操作或多端布局。未部署、未重启既有业务服务。

## 原生会话小任务成功链验证（2026-11-12，后续实测）

此前 CLI missing 的原因在测试环境：已有 Codex 安装的原生程序目录未进入隔离 Runner 的 PATH。采用与小说测试一致的原生路径配置，新测试 Runner probe 返回 `codex-cli 0.167.1`、`logged_in`。没有修改产品发现逻辑、全局 PATH、既有 Runner、账号配置或业务服务。

首次重测由 Agent 创建流水线后，夹具误用不支持的 `search` 筛选而返回 400，导致该测试失败；保留原始记录，修正夹具为现有 `q` 参数。该会话实际终态为 `interrupted`，没有遗留活动审批或冒充成功。

再次实测由会话里的 Agent 通过原生 MCP 工具发现接口、生成完整 YAML、请求创建并触发一次执行，测试夹具只准备 Runner/Agent、提交任务书及确认两次写操作，没有代写生成脚本或绕过原生审批。

| 证据 | 实际结果 |
|---|---|
| [会话](http://127.0.0.1:64992/zh-CN/workspaces/114241803829379072/conversations/114297340126298112) | Agent 返回真实流水线、执行和产物链接 |
| [流水线](http://127.0.0.1:64992/zh-CN/workspaces/114241803829379072/pipelines/114297490357878784) | 原生 Agent 生成相对路径的 sh/Python 脚本，包含独立写入、验收与成功门禁上传三步 |
| [执行](http://127.0.0.1:64992/zh-CN/workspaces/114241803829379072/pipeline-runs/114297542748930048) | 三步均 `succeeded`，实际 Runner 为任务指定节点 |
| [交付物](http://127.0.0.1:64992/api/workspaces/114241803829379072/pipeline-artifacts/114297548906168320/content) | ZIP 175 字节，唯一条目 `receipt.txt`，解压字节精确等于 `delivery-ready` 加一个换行 |
| ZIP SHA-256 | `78a6fc4c6c450683df7cbe7ce7c477cdaa324595a75da994a1e3d64cd96cde34`；实际下载后 Python 独立核验与平台记录一致 |
| 持久化事件 | 460 条事件，包含 MCP 工具调用、2 次审批、2 次审批发送与 finished；原始证据保存在忽略目录 `conversation-delivery-1794476208202/result.json` |

范围限制：这是 Windows 原生、指定 Runner、预装 Python/sh、有明确任务书的单 Agent 成功案例。没有验证自主选择执行器、自主安装依赖、多 human/多 Agent 群协作、任务生命周期统一关联、交付的 human 最终责任签收或 Linux。写操作审批与最终交付验收不是同一种审批。不能据此宣称整体协作产品成熟或优于 LobeHub。

用户确认 Ferry 沿用 raft.build 所代表的项目内人机协作方向；LobeHub 的模型、文件、知识和记忆能力丰富，但入口分散是用户当前体验，未作为本次独立验证的普遍产品结论。该方向不授权新增模块或改版导航。

## 原生会话验收失败拦截（同日）

第二个任务明确要求故意写入 `delivery-ready\n`、独立验收要求 `acceptance-ready\n`，不修正、不重试、不上传诊断产物。Agent 自行生成新流水线并经两次原生写审批后执行：

- [会话](http://127.0.0.1:64992/zh-CN/workspaces/114241803829379072/conversations/114297937109975040)：明确报告验收 FAILED、无交付物，未编造内容下载地址。
- [流水线](http://127.0.0.1:64992/zh-CN/workspaces/114241803829379072/pipelines/114298121629990912)、[失败执行](http://127.0.0.1:64992/zh-CN/workspaces/114241803829379072/pipeline-runs/114298180358635520)：写入 `succeeded`、验收 `failed`（退出码 1）、上传 `skipped`。
- [该执行的产物列表](http://127.0.0.1:64992/zh-CN/workspaces/114241803829379072/pipeline-artifacts?run_id=114298180358635520)：实际 API 查询数量为 0。

失败门禁验收 PASS，任务交付仍是失败，两者不能混淆。原始证据：忽略目录 `conversation-delivery-1794476350566/result.json`；额外 Python 检查步骤状态 PASS。两轮临时 Runner 结束后均停止，原始成功、失败与中断记录保留。

这轮实际 Agent 回复暴露了另一个入口：带 `run_id` 的产物列表 API 链接。对既有 `ConversationResourceLink` 作最小扩展，仅转换同源、同空间、唯一数字 `run_id` 的产物列表地址为现有产物页面并保留筛选。重复参数、额外参数、非数字、其他空间/源保持原行为；没有新增资源、字段或接口。更新后的四个相关测试套件 45/45 PASS。

最终验证：前端完整构建（含 TypeScript）PASS；Chrome 实际点击检查 4/4 PASS，包括产物列表打开界面后实际请求该失败执行的 `run_id`，返回 `total: 0`。同一浏览器检查保留流水线/执行入口及诊断 ZIP 下载验证。首次与完整构建同时运行时页面加载超时，未算通过；构建结束后以 60 秒导航上限完整重跑通过，无页面运行异常。临时 Vite 服务已停止，既有 64992/4291 服务未重启或替换。没有提交、推送或部署。

## 用户授权更新 4291 与停测清理（同日后续）

按用户明确要求，重新编译服务端并更新 `http://127.0.0.1:4291`，继续使用原 `data/dev-backend`、原环境和启动参数。备份在 `data/backups/pre-existing-capability-hardening-20261112-180735`。更新后数据库完整性、表结构、实例身份、密钥及相关业务记录保留检查 PASS。实际 HTML/JS 与最终 dist 字节一致，包含本次产物列表入口修复；Chrome 实际页面检查 PASS，两个原 4291 Runner 均重新在线。详细证据为忽略目录中的 `existing-capability-hardening-local-update.json`、`health-integration-local-browser.json` 与本次目录的 `live4291-review.json`。

64992 测试服务、测试 Runner、陈旧测试夹具进程与源码 Git daemon 已停止；5179/56167/57771/5678 无监听。上文 64992 测试链接因此停止访问，原测试数据库、报告与交付文件仍保留，不向 4291 导入测试业务数据。当前 4291 服务 PID 53400。

磁盘清理尚未完成：自动审批拒绝包含递归删除的批量命令，以及单独对本次临时 n8n 安装目录的删除命令，仅返回 `blocked by policy`。已核对 `data/maintenance/product-poc-20261112/n8n` 和 `node-v24.22.3-win-x64` 是工作区内普通目录；两个临时安装目录仍保留，没有冒充已删除。未改用其他工具绕过拒绝。更新与停服务验收 PASS，包含临时安装删除的完整清理验收 NOT READY。

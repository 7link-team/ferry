# Ferry 流水线全功能设计（Pipeline v2）

| 项 | 内容 |
|---|---|
| 版本 | v2.0-draft，2026-10-31 |
| 状态 | **草案，待评审确认**。第 27 节的待确认决策拍板前不进入实现。<br>**2026-11-02 更正**：D1–D17 已确认；其中 **D5、D6 已被已实现的方案取代**：旧格式 JSON 已迁移为 Actions 格式（提交 c32c4ee/02df78a），本文中关于旧格式执行器、流式改造（§18.2）、转换工具（P2-8）的内容作废；Windows 主机已由 act 主机模式支持（ADR 2026-11-01），§2.3 中“Windows 主机直接拒绝”的现状描述和 D6/P3 的原生执行器计划作废。产品需求以 `docs/prd/ferry-工作流平台PRD.md` 为准 |
| 读者 | 下一阶段实现者（server / runner / worker / 前端）、评审人 |
| 上位文档 | `docs/ferry-运维平台产品与技术方案.md`（REST 契约 §6、列表/表单 DSL §7/§8、HostChannel §9、构建最小闭环 §17）、`ADR.md`（T2/T4/T13/T14/T15、仓库事件、NATS 集群）、`docs/tasks/T14-actions-pipelines.md` |
| 不改变的既有决策 | GitHub Actions YAML 是定义的唯一事实来源；布局与工作空间资源绑定放在 YAML 之外（T14）；所有主机操作只经过 `HostChannel`（§3）；资源接口统一走 `/api/workspaces/{workspace}/{resource}`，列表用统一信封（§6）；页面是共享列表/表单组件的配置（§7/§8）；一条流水线可绑定多个环境，触发时选择（T14 已确认）；代码连接只含平台和授权，仓库与分支在流水线里选（T15 已确认） |

---

## 1. 结论摘要

**现状**：Ferry 已经有一条可用的执行链：GitHub Actions YAML 加 React Flow 画布，任务（job）按 `needs` 依赖组成 DAG，由固定版本的 act 执行器（`nektos/act v0.2.89`）在 Runner 或 SSH 主机上执行。它支持流式日志、取消、跨主机产物、多环境、人工卡点（`ferry-approval`）、变量快照与脱敏。Codeup 等 6 种代码平台的连接、仓库与分支浏览、事件监听（推送或轮询）也都已交付。

**核心缺口**：它还不是“全功能流水线”，因为缺了这几条主链路：

1. **代码事件触发不了流水线**。仓库事件只入库、转发到外部 Webhook（`repository_monitor.rs:615-682`）。YAML 里的 `on:` 从未被解析，`event_name` 写死为 `workflow_dispatch`（`actions_run.rs:636`）。入库的事件也只留下了事件类型、仓库、ref、操作者，触发需要的 commit、变更文件、MR 目标分支等都没有保存（§2.3）。
2. **调度器不可扩展，也不安全**：
   - 每个服务实例同一时间只执行 1 条流水线，因为 `dispatch` 内联等待 `execute`（`pipeline_runs.rs:621`）。
   - 执行记录没有租约。任一实例启动时，会把库里**所有** `running`/`cancelling` 的执行判为失败（`pipeline_runs.rs:574-583`），集群部署下会误杀其它实例上正在跑的构建。
3. **流水线无法联动部署和通知**：
   - 流水线不能触发部署。部署只能反向拉取“最近一次成功执行”的产物（`artifacts.rs:295-330`）。
   - 没有按次执行的通知。
4. **运维闭环操作缺失**：
   - 不能重跑，不能只重跑失败任务，不能从某个任务开始重跑。
   - 人工卡点没有审批人限定、拒绝、超时和意见。
   - 没有执行参数（inputs），没有并发策略，没有定义版本。
5. **质量与效率能力缺失**：测试报告、质量红线、缓存、矩阵跨主机并行，都没有。

**方案**：保留现有架构和 YAML 事实来源，分 4 期补齐，以阿里云云效 Flow 的研发运维闭环为功能基准：

- **P0：可靠性底座**。执行租约、实例内并发执行、主机任务槽、按租约恢复；触发来源的记录；定义版本。
- **P1：Flow 核心闭环**。代码事件 → 构建 → 审批 → 部署 → 通知。
  - YAML `on:` 子集：推送、合并请求、标签、手动输入参数，事件按规则选择环境。
  - 重跑三种形态；审批 v2；**环境保护**（生产审批与允许分支绑定在环境上，流水线编辑者无法绕过）；部署任务；通知；SSE 实时日志；执行 DAG 视图；画布节点面板。
- **P2：质量与效率**。测试报告与质量红线、缓存、矩阵展开、调用下游流水线、阶段泳道、Actions Schema 补全、模板参数、共享产物存储、旧格式转换。
- **P3：扩展**。Windows 执行、云效 Flow YAML 导入、步骤模板市场。

n8n 和 ComfyUI 不是 CI 产品，借鉴的是它们的画布与执行交互，具体有：

- 节点面板，触发器作为画布起点；
- 节点级执行状态实时上色；
- 从任意节点重跑，且复用上游结果；
- 以节点为单位的类型化配置面板；
- 定义历史版本。

它们的“节点间逐条数据流”模型不引入，原因见 §3.6。

---

## 2. 现状盘点（事实，2026-10-31 工作树）

> 本节只陈述代码事实，用来对齐差距。行号以当时工作树为准。

### 2.1 定义格式

| 格式 | 存储 | 能力 | 证据 |
|---|---|---|---|
| 旧格式 JSON（顺序步骤） | `pipelines.definition` | `workdir/env/steps/artifacts`；步骤为 `run` 或 `checkout`；只有 4 个模板变量 | `pipelines.rs:495-660` |
| Actions 信封（`definition_format = "github-actions"`） | 同上，JSON 信封 `{engine, workflow, executor_bindings, checkout_bindings, graph_layout}` | `needs` DAG、`if`、job outputs、矩阵（串行执行）、composite/JS/容器 action、产物 v4、`ferry-approval` 人工卡点 | `actions.rs:15-58, 113-485` |

Actions 格式里**明确拒绝**的有：工作流级和 job 级的 `concurrency`、job 级 `environment`、可复用工作流、动态 `runs-on`、`actions/cache`、非容器任务的 `docker://` action、动态 `secrets[...]`（`actions.rs:125-228, 440-452`）。

### 2.2 执行

- **创建入口唯一**：只有 `pipeline_runs::trigger`（HTTP handler，`pipeline_runs.rs:170-425`）。定时任务在进程内调用同一个 handler（`cron_jobs.rs:868-885`）。所有执行的 `trigger_kind` 都写成 `"manual"`（`pipeline_runs.rs:361`）。
- **调度**：每个实例跑一个 100 ms 循环，取最早一条 `queued`，用 CAS 置为 `running`，然后**内联等待执行结束**（`pipeline_runs.rs:567-641`）。
- **Actions 任务的执行方式**：
  - 同一次执行内，就绪的任务用 `FuturesUnordered` 并发执行（`actions_run.rs:246-318`）。
  - 每个任务的执行过程：上传 worker 二进制和请求文件，再走 `exec_stream` 流式执行，并解析 JSON 日志与 `kind:"result"` 记录（`actions_run.rs:520-645, 889-962`）。
- **主机组**：触发时选**在线成员中 ID 最小**的一台并固定下来（`pipeline_runs.rs:221-232`）。
- **取消**：
  - Actions：先置为 `cancelling`，执行器每 100 ms 检查一次，发送 `ExecCancel`；Runner 终止整个进程组（`exec_stream.rs:14-40`），流断开时 `ProcessGroup` 的 `Drop` 同样会终止它（`exec_stream.rs:38`）。
  - 旧格式：只停止服务端等待，远端命令可能继续运行（ADR T4）。
- **恢复**：启动时把所有 `running`/`cancelling` 判为 `failed`。
- **状态**：
  - 执行：`queued/running/cancelling/succeeded/failed/cancelled`。
  - 任务：`queued/running/waiting/approved/succeeded/failed/cancelled/timed_out/skipped`。
  - 步骤：`queued/running/succeeded/failed/timed_out/cancelled/skipped`。
- **日志**：存为 `pipeline_run_logs` 分片，每片 ≤16 KiB，前端每 1 秒按 `?after=seq` 轮询一次。旧格式的步骤结束后才落库，Runner 输出上限 1 MB（`session.rs:19`）。日志行没有 `job_key` 和 `attempt`，`seq` 由编排进程在内存中分配（`actions_run.rs` 的 `seq` 互斥锁），唯一键是 `(run_id, seq)`。
- **唯一索引**：
  - `pipeline_run_jobs` 唯一键为 `(run_id, job_key)`（`migration.rs:2647-2657`）；
  - `pipeline_run_steps` 唯一键为 `(run_id, step_index)`（`migration.rs:1697-1706`）。
- **主机保留**：Actions 执行开始时，对快照中的所有主机调用 `runner_updates::reserve_with`，作用是不让 Runner 升级打断执行中的任务（`actions_run.rs:43-69`、`runner_updates.rs:83-96`）。含审批任务的执行刻意跳过这一保留。
- **锁**：`lock::acquire` 在 `owner` 相同或锁已过期时都会成功（`lock.rs:20-30`）。因此对**同一 owner 可重入**，同 owner 的两个持有者互不排斥。轮询模块每次领取都用新的 `s.next_id()` 作 owner，以此规避（`polling.rs:517`）。时间由调用方传入（应用时钟）。
- **Actions 产物路径包含执行 ID**：形如 `artifacts/{run_id}/…zip`。恢复时只接受本执行前缀的产物，其它一律报错（`actions_run.rs:596-606`）。worker 回传的同路径产物会被 `fresh` 过滤掉（`actions_run.rs:677-681`）。部署按 `path` 精确匹配产物（`artifacts.rs:305`），而 Actions 产物的路径每次执行都不同，所以“按路径取最近成功产物”对 Actions 产物不成立。

### 2.3 周边能力

| 能力 | 现状 | 证据 |
|---|---|---|
| 环境 / 分组 | `pipeline_contexts`：环境带默认执行目标并作为变量作用域；一条流水线可绑多个环境，触发时选择 | `pipeline_contexts.rs` |
| 变量 | 作用域 global/build/deploy/environment；secret 加密；按执行生成快照（AAD 绑定执行 ID）；所有变量值脱敏 | `variables.rs:391-455`、`pipeline_runs.rs:279-337, 968-990` |
| 产物 | 存在执行实例的本地磁盘 `data_dir/artifacts/<run>/<id>`；按历史保留天数清理 | `artifacts.rs:63-68` |
| 部署联动 | 只能反向：部署清单声明 `artifact:{pipeline,path,run?}`，应用时取最近一次**成功执行**的产物 | `artifacts.rs:295-330`、`manifest.rs:55-61` |
| 告警 | `alert-rules` 的 `target_kind="pipeline"`，只看“最近一次执行”状态，投递渠道是单个 Webhook | `alerts.rs:380-408` |
| 仓库事件 | 推送模式：平台签名校验、5 家平台格式归一化、按请求体摘要去重。轮询模式：快照比对。两种模式都入库后单次转发，不重试 | `repository_monitor.rs:331-682`、`polling.rs:270,308` |
| 仓库事件的数据缺口 | `repository_events` 只存 `event/repository/reference/actor/action/delivery_key`，原始载荷被丢弃（`repository_monitor.rs:635-649`）。具体缺口：<br>① Webhook MR 的 `reference` 是**源分支**；<br>② 轮询事件不带 revision，MR 的 reference 是展示串（如 `#12 a → b`）（`polling.rs:393-405, 452-466`）；<br>③ Webhook 推送的 ref 是 `refs/heads/x`，轮询是裸 `x`；<br>④ 新分支的首次推送归一化为 `branch.created` 而不是 `push`（`repository_monitor.rs:413-427`）；<br>⑤ 轮询中 MR 任意指纹变化（标题、标签、评论数）都会产生 `pull_request`（`polling.rs:303`） | 同左 |
| 定时任务 | `cron_jobs` 的 `run_as` 是执行身份；可触发流水线并指定环境；有自己的重试机制 | `cron_jobs.rs:273, 868-943` |
| 模板 | `pipeline-templates` 增删改查与渲染；从编辑器“保存为模板”时 `parameters` 恒为 `[]` | `ResourceForm.tsx:275` |
| 执行器平台 | worker 只有 linux/darwin × amd64/arm64；遇到 Windows 主机直接拒绝 | `actions_run.rs:407-411` |
| 迁移约定 | 大文本列用 `migration::long_text()`（`migration.rs:45`），MySQL 下为 LONGTEXT。`schema_tests` 只覆盖手工加入列表的语句（`migration.rs:1104-1122`） | 同左 |

### 2.4 前端

- **画布**：只有一种节点类型 `job`，步骤是节点卡片内的行；连线只表达 `needs`。右侧面板可编辑：
  - 任务：名称、类型（执行或卡点）、执行目标、主机或容器、`runs-on`、前置任务、超时。
  - 步骤：命令、拉取代码、使用 action。
- **只能在 YAML 里编辑的**：`on`、`env`、`if`、矩阵、`services`、`outputs`、`continue-on-error` 等。
- **表单问题**：`environment_ids`、`application_id` 在 `blank` 里有默认值，但**没有表单字段**（`resources.tsx:688`）。
- **执行详情**：只有平铺的任务列表，没有 DAG；没有重跑按钮；产物只是跳转到列表的链接。
- **环境表单**（`pipeline-environments`，共享表单配置）：新增“环境保护”分组：必需审批人（成员和角色）、会签人数、允许的 ref 模式、超时、允许自审。
- **执行列表**：显示的是流水线和触发人的 ID，不是名称；状态筛选缺 `waiting`、`timed_out`、`skipped`。
- 没有撤销/重做，没有复制任务，没有节点面板，Monaco 没有 Actions Schema。

---

## 3. 成熟产品对标分析

> **2026-11-01 更正**：本节凭已有知识写成，没有核对实际产品。对标以 `docs/ferry-流水线对标功能清单.md` 为准（n8n 2.45.7、ComfyUI 0.43.0 为实际操作所得；云效 Flow 与 GitHub Actions 依据官方文档）。清单 §3 列出的修订建议和新增决策 D14–D17 确认后，再回写本文档。

### 3.1 阿里云云效 Flow（功能基准）

Flow 是国内团队最熟悉的“研发→交付”流水线，它的模型可以概括为：

```
流水线源（代码源×N / 制品源 / 上游流水线）+ 触发（代码提交 / MR / Tag / 定时 / Webhook / 手动）
  → 阶段 Stage（串行）
      → 任务 Job（同阶段并行，可设手动/自动运行）
          → 步骤 Step（插件化：构建、测试、扫描、上传制品、部署、通知……）
  + 流水线变量（通用变量 / 变量组 / 运行时变量）+ 缓存 + 构建集群
  + 人工卡点（指定审批人 / 会签或或签 / 超时）+ 红线（单测通过率、覆盖率、扫描问题数）
  + 主机组部署（分批、暂停、回滚）/ Kubernetes 部署（蓝绿、分批、金丝雀）
  + 运行历史：重试失败任务 / 跳过 / 取消 / 查看日志与报告 / 制品晋级
  + 模板、YAML 化、通知（钉钉 / 邮件 / Webhook）
```

值得借鉴的设计点：

1. **流水线源是一等对象**。一条流水线可有多个代码源；触发事件带来的 commit 决定这些源检出哪个版本，而不是去取分支最新提交。
2. **阶段与任务分层**。Flow 的阶段是串行执行边界，任务是调度单位。Ferry 只借鉴“阶段作为组织层”的部分：依赖仍由 `needs` 表达，阶段只用于展示，编辑器在新建任务时代为补全跨阶段的 `needs`（§7.1）。这样不会出现两套依赖语义。
7. **环境保护规则绑定在环境上**，而不是写在流水线里：能部署到生产的审批人、允许的分支等。流水线作者无法绕开（§13.0）。
3. **人工卡点和红线都是“任务”**，与构建任务在同一张图上，并继承失败语义。
4. **部署作为流水线里的任务**，消费本次构建的产物，而不是再去查“最近一次”。
5. **重试是对一次执行内的失败任务做的**，成功的部分不重跑。
6. **变量分三层**：定义时、环境、运行时输入。

**不借鉴的**：

- 构建集群托管。Ferry 的执行面是用户自己的主机。
- 自有制品仓库。Ferry 已有产物存储；外部制品库由步骤推送。
- 封闭的插件 SDK。Ferry 直接复用 Actions 生态（composite/JS/容器 action）。

### 3.2 GitHub Actions / GitLab CI（定义格式基准）

Ferry 已采用 Actions YAML，所以兼容语义以 Actions 为准。本方案用到的 Actions 语义：

- `on`：`push`（branches/tags/paths 及其 ignore）、`pull_request`（branches/types/paths）、`workflow_dispatch`（inputs 类型 string/choice/boolean/number/environment）；
- `concurrency`（group 加 cancel-in-progress）。注意 GitHub 的排队语义：同组**只保留一个**等待中的执行，新的替换旧的，并不是无限排队。Ferry 提供 `queue_latest` 策略对应这一语义，见 §8.3；
- `paths` 过滤在无法判定差异时，GitHub 会**执行**。Ferry 选择**跳过并记录原因**（§7.2），这是刻意的差异；
- `strategy.matrix`；
- 重跑：全部重跑、只重跑失败任务，每次重跑是同一编号下的新一次尝试（attempt）；
- 质量门禁参考 GitLab 的 `rules`/`needs`/`artifacts:reports:junit`；
- `environment` 的保护规则，即必需审批人和等待时间。

Ferry 把保护规则做成显式的审批任务，原因见 §13。

### 3.3 n8n（交互与运行模型借鉴）

n8n 的模型：节点（触发节点 / 动作节点）+ 连线，节点之间传递 item 列表；表达式；凭据；错误工作流；节点级的 retry 与 continue-on-fail；子工作流；暂停节点（等 Webhook 回调）；固定测试数据；部分执行；版本历史；队列模式（主进程 + Redis 上的 worker）。

借鉴：

1. **触发器是画布上的起点节点**，一条流水线可以有多个，并有启用开关。
2. **节点面板**：按类别拖入节点；每种节点类型有自己的配置表单。
3. **部分执行**：从某个节点重跑时复用上游上次的输出。对应 Ferry 的“从此任务重跑”。
4. **节点级重试策略**：`retry on fail`、最大次数、间隔。
5. **队列模式的思路**：编排与执行分离，worker 可横向扩展。对应 §8 的执行租约与主机任务槽。
6. **定义版本历史**，可对比、可恢复。

不借鉴：

- **节点间逐条数据流**。CI 的数据通道是文件和产物，再加少量 outputs，不是 JSON 记录流。引入它会造出第二套执行模型。
- **通用 HTTP/DB 集成节点市场**，超出运维平台范围。

### 3.4 ComfyUI（执行可视化借鉴）

ComfyUI 的模型：带类型端口的节点图；执行队列；**按输入哈希缓存节点输出**，只重算变化的部分；通过 WebSocket 推送节点级进度；工作流 JSON 内嵌在产物里，可复现；子图与分组。

借鉴：

1. **运行时图就是定义图**。执行详情直接在同一张 DAG 上给节点上色，可点开日志。
2. **可复现**：每次执行冻结定义版本、输入和解析后的绑定。Ferry 已有快照，本方案再补上 `revision_id`。
3. **增量重算的思想**落到两处：“从此任务重跑”和缓存（`actions/cache`）。不做按输入哈希自动跳过任务：构建步骤有副作用，自动跳过不安全。
4. **分组**对应阶段泳道。

### 3.5 其它参考

- **Jenkins**：stage 视图、`input` 审批步骤（指定提交人）、`post { always/failure }`。`post` 语义已由 `if: always()/failure()` 覆盖。
- **Tekton / Argo Workflows**：Pod 级隔离、`when` 表达式、`retryStrategy`（次数、退避、错误类别）。重试策略字段参考 Argo。
- **Drone / Woodpecker**：步骤即容器、`when` 过滤 `event/branch/path`。触发过滤语义与 Actions 一致，不另立格式。

### 3.6 能力矩阵

“目标”列为本方案的交付期：P0/P1/P2/P3 表示交付期，“—”表示不做。

| 能力 | 云效 Flow | GH Actions | n8n | ComfyUI | Ferry 现状 | 目标 |
|---|---|---|---|---|---|---|
| 可视化编排 | ✔ 阶段/任务 | ✘（只有运行图） | ✔ | ✔ | ✔ 任务 DAG | P1 节点面板 + P2 泳道 |
| YAML 定义 | ✔ | ✔ | JSON | JSON | ✔ | 保持 |
| 手动触发 + 参数 | ✔ | ✔ inputs | ✔ | ✔ | 只有分支/commit/环境 | P1 inputs |
| 代码推送 / MR / Tag 触发 | ✔ | ✔ | ✔ 触发节点 | — | ✘（事件不触发） | **P1** |
| 定时触发 | ✔ | ✔ schedule | ✔ | — | ✔ 定时任务 | 保持（P0 补触发来源） |
| 上游流水线触发 | ✔ | workflow_run | ✔ 子工作流 | — | ✘ | P2 |
| 并发控制 | ✔ | ✔ concurrency | 队列 | 队列 | ✘（每实例只跑 1 条） | **P0 + P1** |
| 人工卡点 | ✔ 审批人/会签 | 环境保护 | 暂停节点 | — | 基础（有 `pipelines.run` 即可批准，不能拒绝） | P1 审批 v2 + 环境保护 |
| 质量红线 | ✔ | 第三方 | — | — | ✘ | P2 |
| 测试报告 | ✔ | 第三方 | — | — | ✘ | P2 |
| 缓存 | ✔ | ✔ | — | ✔ 节点缓存 | ✘（显式拒绝） | P2 |
| 矩阵 | — | ✔ | — | — | 串行 | P2 跨主机展开 |
| 重跑失败任务 / 从节点重跑 | ✔ | ✔ | ✔ 部分执行 | ✔ | ✘ | **P1** |
| 节点级自动重试 | ✔ | ✘ | ✔ | — | ✘ | P1 |
| 部署任务 | ✔ | 第三方 | — | — | ✘（只能反向拉取） | **P1** |
| 通知 | ✔ | 第三方 | ✔ | — | 只看最近状态的告警 | P1 |
| 实时日志 | ✔ | ✔ | ✔ | ✔ WS | 1 秒轮询 | P1 SSE |
| 定义版本 | ✔ | Git | ✔ | — | ✘ | **P0** |
| 多实例安全 | — | — | 队列模式 | — | ✘（启动误杀） | **P0** |
| Windows 构建 | ✔ | ✔ | — | — | ✘ | P3 |

### 3.7 总取舍

1. **只引入一种定义模型**。触发、并发、审批、部署、通知都表达为 Actions YAML 的子集，加 `ferry-*` 扩展键。资源绑定、布局、阶段放进信封。不发明第二种 DSL。
2. **服务端任务与主机任务同图**。审批、部署、通知、红线、调用流水线是**服务端任务**，不占主机，不走 act，但与主机任务共用 `needs`/`if`/状态/重跑语义。这正是 Flow 的“卡点也是任务”，也是 n8n 的“所有东西都是节点”。
3. **先底座后功能**。P1 的事件触发会显著提高执行并发量，所以 P0 的租约和并发必须先交付，否则事件触发会让“每实例只跑 1 条”和“启动误杀”问题暴露得更快。

---

## 4. 差距清单

严重度：**S1** 指影响正确性或安全，**S2** 指缺失主链路功能，**S3** 指影响效率或体验。

| ID | 差距 | 严重度 | 期 |
|---|---|---|---|
| G1 | 执行无租约，启动时误杀其它实例的执行；集群下结果错误 | S1 | P0 |
| G2 | 每实例串行执行一条，没有主机容量控制 | S1（吞吐） | P0 |
| G3 | `trigger_kind` 恒为 manual，执行来源不可追溯 | S2 | P0 |
| G4 | 没有定义版本，执行不知道用的是哪一版定义 | S2 | P0 |
| G5 | 仓库事件（推送/MR/Tag）不触发流水线；`on:` 未解析 | S2 | P1 |
| G6 | 没有执行参数 inputs；事件上下文不传给 act | S2 | P1 |
| G7 | 没有重跑、只重跑失败、从任务重跑、任务自动重试 | S2 | P1 |
| G8 | 人工卡点没有审批人、拒绝、超时、意见 | S2 | P1 |
| G9 | 流水线不能部署；产物解析只认已结束且成功的执行 | S2 | P1 |
| G10 | 没有按次执行的通知 | S2 | P1 |
| G11 | 没有并发策略（排队 / 取消进行中 / 上限） | S2 | P1 |
| G12 | 日志靠轮询；旧格式不流式，且有 1 MB 上限 | S3 | P1 |
| G13 | 执行详情没有 DAG；执行列表显示 ID；状态筛选不全 | S3 | P1 |
| G14 | 画布只有一种节点；触发、环境、应用不能可视化编辑；没有撤销 | S3 | P1 |
| G15 | 没有测试报告和质量红线 | S2 | P2 |
| G16 | 不支持缓存 | S3 | P2 |
| G17 | 矩阵串行，不能跨主机 | S3 | P2 |
| G18 | 没有上游/下游流水线调用 | S3 | P2 |
| G19 | 集群下产物只在执行实例的本地磁盘上 | S1（集群） | P2，启用集群时提前 |
| G20 | Windows 主机不能执行 Actions 任务 | S3 | P3 |
| G21 | 模板不能定义参数；旧格式不能转换为新格式 | S3 | P2 |
| G22 | 没有环境保护：生产审批只能写在流水线里，编辑者可以去掉 | S1（安全） | P1 |
| G23 | 日志无总量上限（P1 去掉 1 MB 上限之后），主机工作目录与缓存没有清理和配额 | S2 | P1/P2 |

---

## 5. 目标、非目标与设计原则

### 5.1 目标

1. 在 Runner 或 SSH 主机上，跑通“代码推送 / MR / Tag → 自动构建 → 测试报告与红线 → 人工审批 → 部署到所选环境 → 通知”的完整闭环，Codeup 与另外 4 家平台一致。
2. 多实例集群下的执行正确：不重复执行，不误杀，编排者丢失后可接管。
3. 运维闭环操作完整：重跑、只重跑失败、从任务重跑、自动重试、取消、审批与拒绝、执行参数、并发策略、版本对比与恢复。
4. 画布可以编辑所有一等能力：触发、任务类型、环境、应用、阶段、重试、并发。YAML 与画布始终是同一份文档。

### 5.2 非目标（明确不做）

- 托管构建集群、自有制品仓库、代码托管。
- 完整的 GitHub 平台仿真：可复用工作流、`workflow_run`、`permissions` 令牌铸造、GitHub 环境保护对象。
- n8n 式的通用集成节点与逐条数据流。
- 按输入哈希自动跳过任务。
- 日志检索平台、指标时序库（总方案 §1 已排除）。

### 5.3 原则

- **P-1 单一事实来源**：定义 = YAML + 信封。派生出的索引（触发器表）在保存时由同一事务重建，不能单独编辑。
- **P-2 显式拒绝，不静默降级**：不支持的语法在保存时报错，错误是完整的中文句子，并指出第几个任务、哪个字段（沿用 T2 风格）。
- **P-3 可复现**：每次执行冻结定义版本、输入、变量、解析后的主机与检出 commit。
- **P-4 复用优先**：服务端任务复用已有模块：部署用 `deployments::scheduled` 的预览与应用路径，通知用告警的 Webhook 投递，身份用定时任务的 `run_as` 语义，锁用 `lock.rs`，凭据用 vault。
- **P-5 兼容**：旧格式 JSON 流水线和 v1 信封照常可读、可执行、可编辑。新能力只在 v2 信封上提供，另外提供一键转换。

---

## 6. 总体架构

```
                ┌──────────────────────── crates/server ─────────────────────────┐
 Git 平台 ──Webhook──▶ repository_monitor ──┐                                      │
 (Codeup/GH/GL/       (签名/归一化/去重)    │ 同一事务                              │
  Gitee/Gitea) ◀─轮询─ polling ─────────────┤                                      │
                                           ▼                                      │
 定时任务 cron_jobs ──────────▶  pipeline_trigger_deliveries（发件箱）            │
 手动 / API / 上游流水线 ─────┐            │ trigger_dispatcher（租约领取）        │
                              ▼            ▼                                      │
                    pipeline_runs::create_run（唯一创建入口，事务）               │
                              │  解析环境/变量/检出 commit/定义版本 → 快照        │
                              ▼                                                   │
                 pipeline_runs（queued）── run_scheduler（每实例 N 个槽）          │
                              │  领取 = 获取 lock `pipeline-run:{id}` + CAS        │
                              ▼                                                   │
                 RunOrchestrator（持有执行租约，心跳续约）                         │
                   ├─ 主机任务 → 获取主机槽 lock → HostChannel.exec_stream → act worker
                   ├─ 服务端任务 → approval / deploy / notify / gate / pipeline     │
                   └─ 状态写入（带 lease_epoch 栅栏）→ 日志分片 → SSE               │
                ── 恢复清扫器：租约过期的执行由任一实例接管 ──                       │
                └───────────────────────────────────────────────────────────────────┘
```

---

## 7. 定义模型 v2

### 7.1 信封 v2

`pipelines.definition` 仍是 JSON 字符串。`engine` 保持 `"github-actions"`，新增 `schema: 2`。缺省或为 1 时按 v1 读取（§23）。

```jsonc
{
  "engine": "github-actions",
  "schema": 2,
  "workflow": "<YAML 原文，事实来源>",
  "sources": [                          // 新：流水线源（Flow 概念），替代 checkout_bindings 中的仓库信息
    { "id": "app", "connection_id": "1234", "repository_url": "https://codeup.aliyun.com/org/app.git",
      "default_ref": "main", "trigger": true }
  ],
  "checkout_bindings": [                // 保留，新增 source_id；与 repository_url/connection_id 二选一
    { "job_id": "build", "step_id": "checkout_1", "source_id": "app", "path": "." }
  ],
  "executor_bindings": [ { "job_id": "build", "target_kind": "host-group", "target_ref": "88", "image": "host" } ],
  "task_bindings": {                    // 新：服务端任务引用的工作空间资源，不写进 YAML（T14：资源绑定在 YAML 之外）
    "approve": { "approvers": { "user_ids": ["1001"], "role_ids": ["31"] } },
    "deploy":  { "applications": { "<staging 环境ID>": "2001", "<prod 环境ID>": "2002" } },
    "call":    { "pipeline_id": "3001", "environment_id": "<环境ID>" }
  },
  "trigger_environments": [             // 新：事件触发时如何选择环境；按顺序首个匹配生效
    { "kinds": ["push", "tag"], "refs": ["refs/heads/main"], "environment_id": "<staging 环境ID>" },
    { "kinds": ["tag"], "refs": ["refs/tags/v*"], "environment_id": "<prod 环境ID>" }
  ],
  "stages": [ { "id": "s1", "name": "构建", "jobs": ["build", "test"] },
              { "id": "s2", "name": "发布", "jobs": ["approve", "deploy"] } ],
  "settings": {
    "concurrency": { "scope": "pipeline_ref", "policy": "queue", "max_parallel": 1 },
    "run_timeout_minutes": 0,           // 0 = 不限制整次执行
    "trigger_run_as": "5678"            // 事件触发的执行身份（§9.5），只能设为保存者本人
  },
  "notifications": [ { "events": ["failed", "waiting"], "channel": "dingtalk",
                       "url_variable": "DINGTALK_HOOK", "mention": [] } ],
  "reports": [ { "job_id": "test", "kind": "junit", "paths": ["target/surefire-reports/*.xml"] } ],
  "graph_layout": { "build": { "x": 0, "y": 0 } }
}
```

字段规则：

| 字段 | 规则 |
|---|---|
| `sources` | 0–10 个。`id` 匹配 `^[a-z][a-z0-9_-]{0,31}$` 且唯一。`connection_id` 必须是本工作空间可见的连接，`repository_url` 用现有仓库校验逻辑复核（T13/T15 的路径）。`trigger:true` 的源才参与事件匹配。plain git 连接不能 `trigger:true`，因为它没有事件能力（`repository_monitor.rs:63-73`）。 |
| `checkout_bindings[].source_id` | 必须指向已存在的源。设置 `source_id` 时，不能再写 `connection_id/repository_url/ref`；`ref` 由执行时解析（§9.6）。 |
| `stages` | 只用于展示：分组、泳道、顺序。每个任务最多属于一个阶段。不在任何阶段的任务归入“未分组”。**阶段不产生调度依赖**，依赖只由 `needs` 决定。编辑器在“新建阶段内任务”时，自动给它加上前一阶段全部任务的 `needs`，作为便捷操作。 |
| `task_bindings` | 键为带 `ferry-task` 的任务 ID。所有工作空间资源（用户、角色、应用、环境、流水线）都**只用 ID**，不用名称，以免改名后失效。保存时逐个校验：存在、属于本空间、类型正确。**这些字段不接受表达式**，因此执行参数无法改变审批人或部署目标。模板与导出中的 `task_bindings` 按跨空间资源处理，规则同现有的 `executor_bindings`。 |
| `trigger_environments` | 只在流水线绑定了环境时才必须提供。`kinds` 取 `push`/`pull_request`/`tag`；`refs` 用完全限定 ref 的 glob。事件触发时取首个匹配项；**没有匹配时投递记为 `skipped`，原因为“没有匹配的触发环境”**，不使用任何默认环境。`environment_id` 必须是本流水线已绑定的环境。未绑定环境的流水线忽略此字段（沿用 T14 的旧默认行为）。 |
| `settings.concurrency.scope` | `none`、`pipeline`（整条流水线一组）或 `pipeline_ref`（按流水线 + 触发 ref 分组）。 |
| `settings.concurrency.policy` | 三种取值：<br>• `queue`：全部排队；<br>• `queue_latest`：同组只保留最新一条等待中的执行，较早的等待执行置为 `cancelled(superseded)`，与 GitHub 语义一致；事件触发的流水线**默认使用此策略**，以抑制事件风暴；<br>• `cancel_in_progress`：新执行进组时，取消组内进行中和等待中的执行，记录 `end_reason=superseded`。 |
| `settings.concurrency.max_parallel` | 1–20。`scope=none` 时忽略。 |
| `settings.trigger_run_as` | 用户 ID，语义同定时任务的 `run_as`。**任何人对定义（信封或 YAML）做了修改，只要保存者不是当前的 `trigger_run_as`，该字段就改为保存者本人**。请求体必须带 `confirm_trigger_identity: true`，否则返回 400：“保存后事件触发将以你的身份执行，请确认。”这样可以防止 A 修改步骤、却以 B 的权限执行（§9.5）。缺省时事件触发不生效，编辑器给出提示。 |
| `notifications` | 0–10 条。`events` 取自 `succeeded/failed/cancelled/waiting/started`。`channel` 取 `webhook/dingtalk/feishu/wecom`。`url_variable` 必须是解析得到的 secret 类变量，URL 不进定义（§14）。 |
| `reports` | 0–20 条。`kind` 目前只有 `junit`。`paths` 是相对 workdir 的 glob，在 worker 内用 `os.Root` 限定范围（§11.3）。 |

### 7.2 `on:` 触发子集（新：解析并生效）

| YAML | 支持 | 映射 |
|---|---|---|
| `workflow_dispatch.inputs.<id>` | `type`：`string`/`choice`/`boolean`/`number`/`environment`；`required`/`default`/`description`/`options` | 触发表单（§19.4）；`${{ inputs.x }}` 和 `github.event.inputs.x` |
| `push.branches` / `branches-ignore` / `tags` / `tags-ignore` / `paths` / `paths-ignore` | ✔，glob 语义同 GitHub（`*`、`**`、`?`、`[]`、`!` 取反）。同一事件内 `branches` 和 `branches-ignore` 不能同时出现（GitHub 同样不允许） | 推送事件；新建 tag 走 `tags` 匹配 |
| `pull_request.branches`（目标分支） / `types` / `paths` | `types` 取 `opened`/`synchronize`/`reopened`/`closed`，缺省为前三者。合并事件 = `closed` + `merged`（归一化后为 `merge`，`repository_monitor.rs:452-457`） | 合并请求事件 |
| `schedule` | **拒绝**：“定时请在『定时任务』中配置，它支持执行身份、环境和重试。”（D3） | — |
| 其它事件（`release`、`issues`、`workflow_run`……） | 拒绝并指明事件名 | — |
| 字符串或数组简写，如 `on: push`、`on: [push, pull_request]` | ✔ 等价于不带过滤 | — |

**`paths` 过滤的判定**：依赖事件携带的变更文件列表。

- GitHub、GitLab、Gitea、Gitee 的推送载荷带 `commits[].added/modified/removed`。
- 轮询事件和缺少文件列表的载荷（包括可能的 Codeup 载荷，**需在 P1-1 用真实载荷核实**）改用连接适配器的 compare 接口取两个 commit 之间的差异。
- compare 失败时**不触发**，并把投递记录置为 `skipped`，原因写“无法判定变更路径”，在流水线的“触发记录”中可见（P-2：不静默放行）。这与 GitHub“无法判定时执行”不同，是刻意的选择。
- 新分支首次推送（`before` 全为 0）时，以该源的 `default_ref` 作为比较基准做 compare；`default_ref` 本身被新建时视为全部文件变更。
- 各平台、各模式的事件到触发类型的映射见 §9.3a。

### 7.3 Ferry 服务端任务 `ferry-task`

job 级扩展键，与 `runs-on/steps` 互斥。带 `ferry-task` 的任务**只允许** `name`、`needs`、`if`、`ferry-task`、`ferry-retry`、`timeout-minutes`。

**分工**：YAML 只写任务类型和与资源无关的参数；涉及工作空间资源的部分写在信封的 `task_bindings[job_id]` 里（§7.1）。这延续了 `executor_bindings` 的做法，也让模板和导出不绑死某个工作空间。

```yaml
jobs:
  approve:
    needs: [test]
    ferry-task:
      type: approval
      with:                         # 审批人在信封 task_bindings.approve 中（资源 ID 不进 YAML）
        min_approvals: 1
        timeout_minutes: 1440
        on_timeout: reject          # reject | approve
        allow_initiator: false      # 触发人能否审批自己的执行
        instructions: "确认测试报告后放行到生产"
  deploy:
    needs: [approve]
    ferry-task:
      type: deploy
      with:                         # 环境→应用映射在信封 task_bindings.deploy 中
        artifacts: [app]            # 部署要用的 upload-artifact 名称；缺省 = 本执行全部产物
        artifact_source: current-run  # current-run（默认）| latest-succeeded
        rollback_on_failure: true
  notify:
    needs: [deploy]
    if: always()
    ferry-task:
      type: notify
      with: { channel: feishu, url_variable: FEISHU_HOOK, template: "部署 ${{ needs.deploy.result }}" }
```

| type | 期 | 语义 | 复用 |
|---|---|---|---|
| `approval` | P1 | §13.1 | 现有 approvals 接口与 `waiting` 状态 |
| `deploy` | P1 | §12 | `deployments::scheduled` 的预览、版本与变量校验路径（`deployments.rs:914-947`） |
| `notify` | P1 | §14 | 告警 Webhook 投递 |
| `gate` | P2 | §13.2 质量红线 | 报告表 |
| `pipeline` | P2 | §9.8 调用其它流水线，可等待结果 | `create_run` |

**兼容**：`ferry-approval: true` 在解析时归一化为 `ferry-task: {type: approval}`，审批人缺省 = 拥有 `pipelines.run` 的成员，与现状一致。YAML 原文不改写。

### 7.4 任务重试 `ferry-retry`（P1）

```yaml
build:
  runs-on: self-hosted
  ferry-retry: { max_attempts: 3, on: [failure, timeout], delay_seconds: 30 }
```

- `max_attempts` 取 1–5，含首次执行。`on` 取自 `failure`、`timeout`、`host_lost`。`delay_seconds` 取 0–3600。
- 每次重试是同一任务的新一次尝试：步骤与日志带 `attempt`，UI 显示“第 2/3 次”。
- `approval`、`deploy` 类型**不允许**设置 `ferry-retry`：部署必须由人决定是否重来；审批重试没有意义。
- 与 T4 的“失败即停、不自动重试”**不冲突**：T4 指的是平台默认值。这里是用户显式按任务开启，默认关闭。

### 7.5 表达式求值

服务端在这几处需要求值：

- **所有任务的任务级 `if` 和状态函数**（`success()`/`failure()`/`always()`/`cancelled()`），包括主机任务。只有这样，被跳过的主机任务才不会占用主机槽、上传 worker。act 只负责步骤级的 `if`。现状是依赖满足即派发、由 act 判定（`actions_run.rs:273-283`），这一点在 P1-7 中改变；
- `ferry-task.with` 里的 `${{ }}`（`task_bindings` 不接受表达式）；
- 通知模板；
- 质量红线规则（§13.2）；
- P2 的静态矩阵展开。

**服务端求值的上下文**只包括：

| 上下文 | 内容 |
|---|---|
| `github` | 事件字段 |
| `inputs` | 执行参数 |
| `needs.*.result/outputs` | 上游结果与输出 |
| `env` | 工作流级 env 中的字面值 |
| `vars` | plain 类型变量 |
| `ferry` | 执行元数据 |
| `reports` | P2 |

`secrets` **不在其中**。引用 `secrets.*` 的服务端表达式在保存时被拒绝：“平台任务的表达式不能读取密钥。”这能防止通知模板把密钥发到外部 Webhook。上游 outputs 中被标记为密钥的值，在任务 outputs 里已经脱敏（现状）。

T14 的约束是“不做重复的 YAML 表达式求值器”。方案是 **worker 新增 `evaluate` 子命令**：

- 复用 act 的 `exprparser`（纯 Go，可交叉编译到 Windows）。
- 再为服务端所在平台多构建一份 worker：`windows-amd64`、`linux-*`、`darwin-*`。
- 服务端在本机以子进程调用它。Node 子进程规则不适用；Rust 侧用 `CREATE_NO_WINDOW` 标志，满足 §12“Windows 执行不弹窗”。

```
actions-worker evaluate --request <file>
请求  { "expressions": ["needs.test.result == 'success'", ...], "contexts": { "needs": {...}, "inputs": {...}, "env": {...}, "github": {...}, "ferry": {...} } }
响应  { "results": [ {"ok": true, "value": true}, {"ok": false, "error": "..."} ] }
```

- 一次执行的所有服务端求值批量合并成一次调用，单次超时 5 秒。
- 求值失败时任务失败，错误写入日志。**不当作 false**，避免静默跳过。
- 这是待确认决策 D1，替代方案是 Rust 子集求值器。

### 7.6 校验规则（新增部分）

错误文案示例沿用 T2 风格，指明任务和字段。

| 规则 | 错误文案示例 |
|---|---|
| `on` 含不支持的事件 | `触发事件「release」暂不支持；支持 push、pull_request、workflow_dispatch。` |
| `on.schedule` | `定时请在「定时任务」中配置，它支持执行身份、环境和重试。` |
| 有 `push`/`pull_request` 但没有 `trigger:true` 的源 | `已配置代码事件触发，但没有标记为触发源的代码源，请在「流水线源」中开启。` |
| `ferry-task` 与 `runs-on`/`steps` 同时出现 | `任务「deploy」是平台任务，不能同时设置 runs-on 或 steps。` |
| `ferry-task.type` 未知 | `任务「x」的平台任务类型「foo」不存在。` |
| `task_bindings.approve.approvers` 引用的用户不是本空间成员，或角色不存在 | `任务「approve」的审批人「1001」不是当前工作空间成员。` |
| `task_bindings.deploy.applications` 的键不是本流水线已绑定的环境，或值不是本空间的应用 | `任务「deploy」引用的环境「prod」未绑定到本流水线。` |
| 服务端任务缺少必需的 `task_bindings` | `任务「deploy」还没有配置各环境的部署应用。` |
| `task_bindings` 的值含 `${{` | `任务「approve」的审批人必须直接选择，不能使用表达式。` |
| 服务端表达式引用 `secrets.*` | `任务「notify」的表达式不能读取密钥。` |
| 绑定了环境且有事件触发，但 `trigger_environments` 为空 | `已配置代码事件触发，请为事件触发选择执行环境。` |
| `deploy` 不依赖任何主机任务 | 允许：可部署已有的最新成功产物，但 `artifact_source` 必须显式写 `latest-succeeded` |
| `ferry-retry` 用在 approval/deploy 上 | `任务「deploy」是部署任务，不能自动重试；失败后请人工重跑。` |
| `inputs` 的 id 不合法，或 `choice` 没有 `options` | `执行参数「env」是选项类型，必须提供 options。` |
| `paths` 与 `paths-ignore` 同时出现 | `push 触发不能同时设置 paths 和 paths-ignore。` |
| 通知的 `url_variable` 不存在或不是 secret | `通知地址变量「DINGTALK_HOOK」必须是密钥类型变量。` |

既有规则保持不变：128 KB、1–50 个任务、拒绝循环、拒绝 cache 等（cache 在 P2 放开，见 §11.2）。

### 7.7 完整示例

场景：Codeup 推送到 `main` 或 `release/*` 时自动构建测试；手动触发时可以选择版本号；审批后部署到所选环境，结束后通知飞书。实现 P1 时，这份示例同时作为真实验收夹具。

```yaml
name: app-ci
on:
  push:
    branches: [main, "release/**"]
    paths-ignore: ["docs/**"]
  pull_request:
    branches: [main]
  workflow_dispatch:
    inputs:
      version: { type: string, required: false, description: "留空则使用 commit 短号" }
      target:  { type: environment, required: true }
jobs:
  build:
    runs-on: self-hosted
    timeout-minutes: 30
    ferry-retry: { max_attempts: 2, on: [host_lost] }
    outputs:
      version: ${{ steps.meta.outputs.version }}
    steps:
      - id: checkout_1
        uses: actions/checkout@v4
      - id: meta
        run: echo "version=${{ inputs.version || github.sha }}" >> "$GITHUB_OUTPUT"
      - run: ./build.sh
      - uses: actions/upload-artifact@v4
        with: { name: app, path: dist/app.tar.gz }
  test:
    needs: [build]
    runs-on: self-hosted
    steps:
      - id: checkout_1
        uses: actions/checkout@v4
      - run: ./test.sh
  approve:
    needs: [test]
    if: github.event_name != 'pull_request'
    ferry-task:
      type: approval
      with: { min_approvals: 1, timeout_minutes: 1440, on_timeout: reject }
  deploy:
    needs: [approve]
    ferry-task:
      type: deploy
      with: { artifacts: [app], artifact_source: current-run, rollback_on_failure: true }
  notify:
    needs: [deploy]
    if: always()
    ferry-task:
      type: notify
      with: { channel: feishu, url_variable: FEISHU_HOOK, template: "app ${{ needs.build.outputs.version }} 部署结果：${{ needs.deploy.result }}" }
```

对应的信封要点：

- `sources`：一个 Codeup 源，`trigger: true`。
- `checkout_bindings`：两个 `checkout_1` 都指向该源。
- `executor_bindings`：`build`、`test` 绑定同一个主机组。
- `stages`：「构建」包含 build、test；「发布」包含 approve、deploy、notify。
- `notifications`：订阅 `failed`。
- `task_bindings`：
  - `approve` 的审批角色为 admin（角色 ID）；
  - `deploy` 的映射为 staging 环境 → 应用 2001、prod 环境 → 应用 2002。
- `trigger_environments`：推送到 `refs/heads/main` → staging。没有匹配项的推送（如 `release/*`）投递为 `skipped`；要让它们也部署，需再加一条规则。
- 环境保护：prod 环境本身配置了必需审批人（§13.0）。即使有人删掉 YAML 中的 approve 任务，部署到 prod 仍然需要审批。

`pull_request` 触发时，`approve` 因 `if` 为假而被跳过。按 GitHub 的语义，下游的 `deploy` 随之 skipped；`notify` 写了 `always()`，所以仍会执行。

---

## 8. 执行引擎 v2

### 8.1 状态机

**执行（pipeline_runs.state）**

```
queued ──领取──▶ running ⇄ waiting ──全部任务终态──▶ succeeded | failed
  │                 │ 取消 ─▶ cancelling ─▶ cancelled
  │ 并发策略取消 ──────────────────────────▶ cancelled(end_reason=superseded)
  └ 取消 ─▶ cancelled
```

- **`waiting`（新，落库）**：执行内没有可运行的主机任务和服务端任务，只剩等待人工或定时的任务（审批中、重试延迟中）。此时编排者**挂起**这次执行：释放执行级槽和执行锁，并把 `not_before` 设为最近的唤醒时刻（审批超时、重试延迟到期、整次执行超时）。
  - 审批决策接口把 `not_before` 置为当前时刻，执行随即以 `priority=1` 被重新领取，状态回到 `running`。
  - 挂起的执行**计入**并发组的 `max_parallel`，与 GitHub 中等待环境审批的执行占用并发一致。
  - 这避免了 4 个等待 30 天审批的执行占满一个实例。
- `end_reason` 取值：

| 值 | 含义 |
|---|---|
| `''` | 无 |
| `superseded` | 被并发策略取代 |
| `orchestrator_lost` | 编排者丢失 |
| `run_timeout` | 整次执行超时 |
| `rejected` | 审批拒绝 |
| `approval_timeout` | 审批超时 |
| `protection_denied` | 环境保护不允许该 ref |
| `cancelled_by_user` | 用户取消 |

  `create_run` 失败时不会产生执行行，失败原因记录在触发投递或 HTTP 错误中，所以没有 `trigger_invalid` 这一取值。
- `timed_out` 不作为执行级状态（保持现状），超时原因由 `end_reason` 表达。

**任务（pipeline_run_jobs.state）**

```
pending（等依赖）──依赖满足且服务端求值 if 为真──▶ queued（就绪待槽）──▶ running ──▶ succeeded | failed | timed_out
   │                                                                  └ ferry-retry 命中 → queued（attempt+1，同一行）
   ├─ if 为假 / 上游失败且无 always() ──▶ skipped
   └─（服务端任务）▶ waiting ─批准达到人数─▶ approved ──▶ succeeded
                         ├ 任一拒绝 / 超时且 on_timeout=reject ──▶ failed(end_reason=rejected|approval_timeout)
                         └ 超时且 on_timeout=approve ──▶ approved ──▶ succeeded（审批记录写入一条 user_id=0 的「超时自动通过」）
reused：重跑时从父执行复制的成功任务（只读，带 outputs）
```

- 新状态：`pending`、`reused`。兼容：v1 执行里未开始的 `queued` 任务，在调度器重算就绪性时等同于 `pending`（§23）。
- **尝试的存储模型（单行原地 + 子表按尝试分）**：
  - 任务行原地更新，`attempt` 递增，任务行只表示最新一次尝试；
  - 步骤和日志按尝试分开保存：`pipeline_run_steps` 新增 `job_key`、`attempt`，唯一键从 `(run_id, step_index)` 改为 `(run_id, job_key, attempt, step_index)`；
  - `pipeline_run_logs` 新增 `job_key`、`attempt`，唯一键仍是 `(run_id, seq)`；
  - 旧数据的 `job_key=''`、`attempt=1`，仍然唯一；
  - 矩阵子任务各自是独立的任务行（`job_key="build (a, b)"`），满足 `(run_id, job_key)` 唯一。
- 前端的尝试切换按 `(job_key, attempt)` 过滤步骤和日志。

### 8.2 调度与并发（P0）

替换 `pipeline_runs.rs:567-641` 的单槽循环，分两层：执行级槽 + 主机级槽。

1. **执行级槽**：每个实例 `FERRY_PIPELINE_MAX_RUNS` 个，默认 4，范围 1–64。调度循环在空闲槽 > 0 时领取，每个领取到的执行在独立的 `tokio::spawn` 里运行 `RunOrchestrator`。优雅关闭时等待所有编排者完成或挂起（复用现有 drain 通道）。
2. **锁 owner 必须每个持有者唯一**。`lock::acquire` 对同一 owner 可重入（§2.2），所以：
   - 执行锁 `pipeline-run:{id}` 的 owner = `{instance_boot}:{run_id}:{lease_epoch}`；
   - 主机槽锁 `pipeline-host:{host}:{i}` 的 owner = `{instance_boot}:{run_id}:{job_key}:{attempt}`；
   - 并发短锁的 owner = `s.next_id()`，做法同 `polling.rs:517`。
   - `instance_boot = {instance_id}:{boot_nonce}`，只写在 `pipeline_runs.lease_owner` 中用于启动恢复，**不直接作为锁 owner**。
   - 单元测试必须覆盖“同一进程内两个编排者争抢同一主机槽，只有一个成功”。
3. **领取**（防重复、防饥饿）：
   ```
   候选 = SELECT id, workspace_id FROM pipeline_runs r
          WHERE (state='queued' OR (state='waiting' AND not_before<=:now))
            AND (not_before IS NULL OR not_before<=:now)
            AND (concurrency_key='' OR state='waiting' OR concurrency_key NOT IN (已饱和的组))
          ORDER BY priority DESC,
                   (该工作空间当前 running 数) ASC,   -- 工作空间间轮转，防止一个空间饿死其它空间
                   created_at
          LIMIT 32
   ```
   - “已饱和的组”= `SELECT concurrency_key … WHERE state IN ('running','cancelling','waiting') GROUP BY concurrency_key HAVING COUNT(*) >= max_parallel`。`max_parallel` 存入 `pipeline_runs.concurrency_limit`，在创建时冻结，避免连表查询定义。
   - 对每个候选：
     ```
     lock::acquire("pipeline-run:{id}", owner, ttl=30s) 失败 → 跳过
     UPDATE pipeline_runs SET state='running', lease_owner=:instance_boot, lease_epoch=lease_epoch+1,
            started_at=COALESCE(started_at,:now)
       WHERE id=? AND state IN ('queued','waiting') AND lease_epoch=:读到的值
     rows_affected=0 → release 并跳过
     ```
   - 从 `waiting` 重新领取的执行，在编排者内重算任务状态后继续。
4. **心跳与自我熔断**：
   - 编排者每 10 秒 `lock::renew`。
   - 续约返回“锁已被他人持有”时，**立即**对本执行所有活动流发送 `ExecCancel`，停止一切写入并退出，不写终态。
   - 续约**出错**（例如数据库不可用）时继续重试。但如果按本地单调时钟，超过 `TTL − 8s`（即 22 秒）都没有一次成功续约，同样自我熔断：取消所有流后退出。这是为了避免锁过期、被他人接管之后，原编排者的远端任务仍在运行，造成重复执行。
   - 锁的过期时间基于各实例的应用时钟，所以**集群部署要求实例间时钟偏差 < 5 秒（NTP）**，写入部署文档。
5. **主机任务槽**：
   - `hosts` 新列 `pipeline_slots`，默认 2，范围 1–32，在主机编辑表单中暴露。
   - 主机任务开始前，按 `i = 0..slots` 依次尝试获取主机槽锁，同时对该主机调用 `runner_updates::reserve_with`，让 Runner 升级避开执行中的任务。这把现状的“整次执行保留”改为“按任务保留”。
   - 都失败时任务保持 `queued`，`wait_reason` 写“主机执行槽已满”或“主机正在升级”，UI 可见，1 秒后再试。
   - 槽锁随编排者心跳续约，任务结束时释放。接管者重新获取。
   - 主机进入升级 drain 时不再授予新的槽。
6. **主机组选机**（替换“ID 最小的在线成员”）：
   - 任务就绪时（不是触发时）在组的在线成员中选**空闲槽最多**的一台，同分取 ID 最小者，然后把选中的 `host_id` 写入任务行。
   - 触发时只校验“组非空且至少一台在线”，保持现有拒绝语义。
   - 执行快照记录的是**实际执行**的主机。
7. **整次执行超时**：`settings.run_timeout_minutes > 0` 时，编排者到点取消所有活动任务，`end_reason=run_timeout`。挂起期间同样计时，唤醒时刻取审批超时与整次执行超时中较早者。

### 8.3 并发组

- **组键**：
  - `scope=pipeline` → `p:{pipeline_id}`；
  - `scope=pipeline_ref` → `p:{pipeline_id}:{sha256(ref) 前 16 位}`，取哈希是为了避免过长的 ref 超出列宽；
  - 存入 `pipeline_runs.concurrency_key`，`concurrency_limit` 存 `max_parallel`。
- **`queue`**：由领取查询中的“已饱和的组”过滤实现。饱和组的执行不会占满候选窗口，所以不会造成队首阻塞。
- **`queue_latest`**：在 `create_run` 事务内，把同组 `queued` 的执行置为 `cancelled(superseded)`。
- **`cancel_in_progress`**：在 `create_run` 事务内，同组 `queued`、`waiting` 的执行直接置为 `cancelled(superseded)`，`running` 的置为 `cancelling(superseded)`。新执行要等组内的取消完成才会被领取，由饱和过滤保证。
- `create_run` 中的组内处理与领取之间的竞争，用 `lock::acquire("pipeline-concurrency:{key}", owner=next_id())` 短锁串行化，持锁时间仅限一次事务或一次领取。
- **P0-1 只交付领取与槽位机制，`concurrency_key` 为空即不分组；并发策略在 P1-8 交付。**

### 8.4 多实例恢复（P0，修复 G1）

1. **启动时**：只处理 `lease_owner` 的 `instance_id` 等于本实例、`boot_nonce` 不等于本次启动的执行。这些是本实例上次崩溃留下的，按第 2 条接管。**不再触碰其它实例的执行**。
2. **清扫器**：每个实例每 15 秒扫描 `state IN ('running','cancelling')`、`lease_owner<>''`，且锁 `pipeline-run:{id}` 不存在或已过期的执行，抢占锁并 `lease_epoch+1` 后接管：
   - **`running` 的主机任务**：原实例的流已断开。Runner 在流断开时终止进程组（`exec_stream.rs:38`）；跨实例转发时依赖 NATS 桥的脉冲超时（`cluster.rs:787-800`），P0-1 必须在集群用例中测试这一点。处理方式：
     - 置为 `failed(end_reason=orchestrator_lost)`；
     - 如果配置了 `ferry-retry` 且包含 `host_lost`，则按重试处理。
     - **SSH 通道断开时是否终止远端进程，需在 P0-1 用真实 sshd 核实。核实前，SSH 主机上的 `host_lost` 重试被禁用**，文档和 UI 都不得声称“已停止”。
   - **`waiting`、`pending`、`queued` 的任务**：原样续接。它们没有远端副作用，审批不会因服务重启而作废（修复现状里“重启后审批中的执行被判失败”的问题）。
   - **服务端任务**：按 §8.5 的意图记录续接。
   - **日志序号**：接管后从 `MAX(seq)+1` 重新开始分配。
3. **栅栏**（所有写入都要带）：
   - 编排者对 runs、jobs、steps、**logs** 的写入都使用条件写，形如 `INSERT … SELECT … WHERE EXISTS(SELECT 1 FROM pipeline_runs WHERE id=? AND lease_epoch=?)` 或 `UPDATE … WHERE … AND EXISTS(...)`，影响行数为 0 即视为失去租约并自我熔断。
   - 不在每条日志写入时更新 runs 行，避免热点行。
   - 每个执行**只有一个写入者**（当前编排者），因此日志序号分配与提交是串行的。SSE 的“无重复、无遗漏”依赖这一点。
   - `finish()` 必须带栅栏。
4. **滚动升级**：
   - 旧版本实例不会创建执行锁。新版本清扫器只接管 `lease_owner<>''` 的执行，因此**不会**误接管旧实例上的执行。
   - 但旧实例重启时仍会按旧规则判失败所有 `running` 执行，所以**包含 P0-1 的版本必须全停升级**：停止所有实例，再升级，再启动。这一点写入发布说明和升级脚本检查（§23）。

### 8.5 服务端任务执行器

- 在编排者内运行，不占主机槽。
- 审批等待通过 §8.1 的挂起与唤醒实现，不长期占用编排者。
- 每种类型实现同一个 trait（Rust 侧唯一的新抽象，因为有 5 个实现，满足 §5“有真实需要”）：

```rust
#[async_trait]
trait ServerTask {
    /// 校验在保存时完成；这里只执行。返回 outputs（会被加密存储）。
    async fn run(&self, ctx: &TaskContext, with: &serde_json::Value, binding: &serde_json::Value) -> Result<TaskOutcome>;
}
```

- **外部副作用的幂等**：部署、调用流水线、通知都会产生外部副作用。它们一律采用“先写意图，后执行”：
  1. 在独立事务中插入 `pipeline_task_intents(run_id, job_key, attempt, kind, state='pending', external_id='')`，唯一键为 `(run_id, job_key, attempt)`，插入同样带栅栏；
  2. 执行副作用时把意图 ID 传给下游，下游在**自己的事务里**记录它：
     - 部署执行新增唯一列 `pipeline_intent_id`，`deployments::scheduled` 的内部函数增加此参数；
     - 子流水线执行由独立的 `pipeline_run_parents(intent_id 唯一, run_id)` 表承载。这是部分唯一索引的可移植替代；
  3. 回写 `external_id` 和 `state='started'`。
  
  接管时：
  - 意图为 `pending` 时，先按 `pipeline_intent_id` 查下游：查到就补写 `external_id` 并继续等待；查不到才重新发起。由于唯一约束，重复发起会失败，不会产生第二次部署；
  - 通知的意图处于 `pending` 时**不重发**，按“至多一次”处理，标记为 `unknown` 并在执行详情中提示。

### 8.6 重跑（P1，修复 G7）

三种入口，全部是**新建一条执行**（D9）。**执行身份 = 发起重跑的用户**：重跑者自身需要 `pipelines.run`；包含部署任务时，还需要 `deployments.apply`。不继承父执行的身份，防止借重跑提升权限。

**复用集的形式化定义**：设父执行的任务集合为 J，重跑闭包为 C。

- `all`：C = J。
- `failed`：C = {父执行中状态 ∉ {succeeded, skipped} 的任务} ∪ 它们的全部下游。
- `from(j)`：C = {j} ∪ j 的全部下游。如果 j 的上游里有非 `succeeded` 的任务，拒绝：“上游任务「x」未成功，不能从此处重跑。”
- **审批传播**：审批任务进入 C 后要重新审批，它的全部下游也并入 C（不动点迭代）。另外，审批任务只要不在 C 中，且它的某个下游在 C 中，它也并入 C。这保证任何被重跑的任务都不会绕过它上游的审批。
- **复用集** R = {t ∈ J \ C | 父执行中 t.state = succeeded}，R 中的任务置为 `reused`，复制 outputs。
- J \ C 中的其它任务（如父执行中 `skipped` 的任务）在新执行中重新求值。
- 部署任务**没有特殊规则**：在 C 中就执行，在 R 中就复用。

**产物**：

- 产物表新增 `name`（upload-artifact 名称；旧格式为路径）、`job_key`、`attempt`、`state`（§16）。
- 创建重跑执行时，R 中任务的产物在新执行下**复制一份**：新行加文件复制，本地存储优先用硬链接，路径改写为 `artifacts/{new_run}/…`。这样现有恢复逻辑“只认本执行前缀”（`actions_run.rs:596-606`）保持不变，重跑链上的产物也互不依赖，父执行按保留策略清理不会破坏子执行。
- **同一执行内的任务重试（`ferry-retry`）**：新尝试开始前，把该任务上一尝试的产物行标记为 `superseded` 并删除文件。回收时 `fresh` 过滤改为按 `(job_key, attempt)` 判定，不再按路径。失败尝试的旧产物因此不会被下游或部署取到。

**其它字段**：

- 新执行的 `rerun_of_id` = 父执行，`root_run_id` = 最初那次执行，`attempt` = 父执行 `attempt + 1`；`number` 是新编号，保持 `(workspace_id, number)` 唯一，不改索引。
- UI 显示“#57（#52 的第 2 次尝试）”。
- 重跑沿用父执行的 `revision_id`、inputs、源 commit、环境；变量**重新解析**，因为变量可能已经被修正，这正是重跑的常见目的。
- 环境保护（§13.0）在重跑时重新检查。

### 8.7 取消与超时

- 不变：Actions 任务走 `ExecCancel`，Runner 终止进程组。
- 旧格式在 P1 改用 `exec_stream`（§18.2）后获得同样的取消语义。连接老版本 Runner 时回退到现状，UI 显示“该主机 Runner 版本较旧，取消后远端命令可能继续运行”。
- 取消时，服务端任务的处理：
  - 审批直接失败；
  - 部署**不中止**已发起的部署：部署有自己的状态与回滚语义，强行中止会造成半部署。任务标记为 `cancelled`，日志注明部署执行 ID 与当前状态，提示用户去应用页处理；
  - 通知直接取消。

### 8.8 矩阵跨主机展开（P2，修复 G17）

- **展开时机**：任务就绪时，用 `evaluate` 求出 `strategy.matrix` 的组合，覆盖 include、exclude 和 `fromJSON(needs.x.outputs.y)`。
- **展开形式**：组合 ≤ 64 时展开为子任务行，`job_key = "{job} ({v1}, {v2})"`，与 GitHub 显示一致；超过 64 时任务失败。
- **派发**：每个子任务独立领取主机槽，并给 worker 下发只含单一组合的请求。worker 侧需要把 `strategy.matrix` 固定为单值后再交给 act，见 §18.3。
- `max-parallel` 和 `fail-fast` 由编排者实现：`fail-fast` 为真时，一个组合失败就取消同任务的其它组合。worker 不再串行执行矩阵。
- 下游 `needs.job.result` 汇总规则与 GitHub 一致：全部成功才是 success；有失败为 failure；有取消且无失败为 cancelled。

---

## 9. 触发系统（P1，修复 G3/G5/G6）

### 9.1 唯一创建入口 `create_run`（P0）

- 把 `pipeline_runs::trigger`（HTTP handler）拆成两层：
  - `pub(crate) async fn create_run(s, tx: &DatabaseTransaction, actor: Actor, req: RunRequest) -> Result<Id>`：业务函数，包含全部校验，在调用方给定的事务内执行（发件箱需要把建执行和投递状态放在同一事务，§9.3b）；
  - HTTP handler 只负责解析请求、把 scope 转成 `Actor`。
- 定时任务、触发调度器、上游流水线、重跑都调用 `create_run`。定时任务不再伪造 HTTP 调用。

```rust
pub(crate) struct RunRequest {
    pub pipeline_id: Id,
    pub environment_id: Option<Id>,
    pub trigger: Trigger,                 // Manual{branch,commit} | Schedule{cron_run_id} | Event{delivery_id} | Pipeline{parent_run,parent_job} | Rerun{of, mode, from_job}
    pub inputs: BTreeMap<String, Value>,  // 已按 inputs 定义校验
    pub source_refs: BTreeMap<String, SourceRef>, // source_id → {ref, commit}
}
```

`pipeline_runs.trigger_kind` 取值：`manual`、`schedule`、`push`、`pull_request`、`tag`、`pipeline`、`rerun`。已有数据保持 `manual`，不回填，因为原始来源已不可知。另有新列 `trigger_ref`，存定时执行 ID、投递 ID、父执行 ID 等，供 UI 跳转。

### 9.2 触发器索引表

`pipeline_triggers` 是派生数据，在保存流水线的同一事务内**先删后插**重建：

| 列 | 说明 |
|---|---|
| `id`、`workspace_id`、`pipeline_id` | 雪花 ID，均为 BIGINT |
| `kind` | `push`、`pull_request` 或 `tag` |
| `connection_id` | 来自触发源 |
| `repository_key` | 规范化的仓库全路径，规则同 `repository_monitor` 的仓库过滤：小写主机，去掉 `.git`，路径只解码一次（沿用 alpha23 的 Codeup 路径规则）。用 `long_text`，只供展示和复核 |
| `repository_hash` | `sha256(repository_key)` 的十六进制，`VARCHAR(64)`，用于索引（MySQL 的组合索引不能包含 TEXT 列） |
| `source_id` | 信封中的源 ID |
| `filters` | JSON：branches、branches_ignore、tags、tags_ignore、paths、paths_ignore、types（LONG 列） |
| `created_at` | |

索引：`(workspace_id, connection_id, repository_hash, kind)`。

### 9.3 事件到执行的流程

**前置改造（P1-1 的第一项工作）**：现有事件记录缺少触发所需的数据（§2.3）。因此：

- 扩展 `repository_monitor::normalize()`（`repository_monitor.rs:382-516`）和 `polling::changes()`，在现有归一化结果之外，再产出一个类型化的 `TriggerEvent`；
- `TriggerEvent` 只存入发件箱，`repository_events` 的列与语义保持不变。

```rust
pub(crate) struct TriggerEvent {
    pub kind: TriggerKind,          // Push | Tag | PullRequest
    pub repository_key: String,     // §9.2 规范化
    pub git_ref: String,            // 完全限定：refs/heads/x、refs/tags/v1；MR 为 head ref
    pub before: String,             // 推送前 commit；新建为全 0；MR 为空
    pub after: String,              // 推送后 / tag 指向 / MR head 的 commit，必须非空
    pub base_ref: String,           // MR 目标分支（完全限定）；推送为空
    pub pr_number: Option<i64>,
    pub pr_action: String,          // opened | synchronize | reopened | closed
    pub pr_merged: bool,
    pub head_repository_key: String,// MR 来源仓库；与 repository_key 不同 = fork
    pub files: Option<Vec<String>>, // 载荷自带的变更文件；None 表示需要 compare
    pub message: String,            // 截断到 1 KiB，仅作数据
    pub author: String,             // 平台用户名，仅作展示
}
```

`after` 为空的事件（例如没有 commit 信息的载荷）**不产生 TriggerEvent**，这类事件只入库、照常转发，不触发流水线。

### 9.3a 各平台、各模式的映射

| 归一化事件（现有） | 产出的 TriggerEvent | 规则 |
|---|---|---|
| `push` | Push | ref 补全为 `refs/heads/…`（轮询给的是裸分支名） |
| `branch.created`（带 commit） | **Push**，before 为全 0 | 与 GitHub“新分支推送触发 on: push”一致 |
| `branch.deleted`、`tag.deleted` | 不产出 | 删除不触发构建 |
| `tag.created` | Tag | **去重键为 `(repository_key, ref, after)`**：GitHub 对同一个 tag 会同时发送 `push(created)` 和 `create` 两个请求，两者的请求体摘要不同 |
| `pull_request`（Webhook） | PullRequest | 平台动作映射：GitHub `opened/synchronize/reopened/closed`；GitLab `open/update/reopen/close/merge`；Gitea、Gitee、Codeup 按各自字段（P1-1 用真实载荷逐家核实并固化为测试夹具）。`update` 只有 head commit 变化时才算 `synchronize` |
| `pull_request`（轮询） | PullRequest | 只有快照中 `head` 变化时才产出 `synchronize`；新出现的 MR 产出 `opened`。只改了标题、标签、评论数的变化不产出（修正 `polling.rs:303` 的“任意指纹变化”在触发场景下的语义） |
| `merge` | PullRequest，`pr_action=closed`、`pr_merged=true` | |

- 轮询模式的 MR 需要额外字段：base_ref 和 head 仓库。这些从轮询快照的元数据中取（`polling.rs:308` 已包含 `head` 和 `reference`）。缺失时只能匹配不带 `branches` 过滤的 `pull_request` 触发，并且 fork 判定按“无法确认”处理，即**跳过**（§22.3）。

### 9.3b 发件箱与投递

```
repository_monitor::record / polling（已有：去重、入库 repository_events）
  └─ 同一事务内：对产出的 TriggerEvent 执行 pipeline_triggers::admit(tx, ev)
        查 (workspace, connection, repository_hash, kind) 的候选触发器 → 内存里做 ref/types 过滤
        对每条命中：INSERT pipeline_trigger_deliveries(state='pending', event=ev(加密 JSON), pipeline_id, trigger_id,
                                                        dedupe_key, redelivery_seq=0)
        唯一索引 (pipeline_id, dedupe_key, redelivery_seq)
        dedupe_key = sha256(repository_key | kind | git_ref | after | pr_action)  —— 跨请求体摘要的语义去重
trigger_dispatcher（每实例一个循环，租约领取，模式同 polling.rs:487-537 的共享租约，owner = next_id()）
  领取 pending → paths 判定（需要时调用 compare）→ 选择环境（§9.6）
     → 在同一个数据库事务里：create_run(tx, …) 且 UPDATE delivery SET state='created', run_id=… WHERE id=? AND state='pending'
     不触发 → state='skipped', reason=…    失败 → state='failed', reason=…
```

- **为什么要发件箱，而不是在入库事务里直接建执行**：`create_run` 要解析变量、主机等，失败原因很多。如果和事件入库共用事务，任何一个问题都会让事件本身丢失。发件箱让事件先可靠落库，投递结果可审计、可重放。
- **建执行与回写投递状态必须在同一事务**。因此 `create_run` 的签名接受外部事务（§9.1）。否则调度器在两步之间崩溃，投递会被重新领取，造成重复执行。
- **重放**：流水线“触发记录”页里 `failed` 或 `skipped` 的投递可以手动“重新投递”。这会插入一条 `redelivery_seq = max+1` 的新投递，原记录保持不变。系统**不自动重试**，原因同仓库事件的 ADR：避免对已部分执行的副作用重复触发。
- **前提**：连接必须开启监听，且勾选了对应事件：
  - 推送类：`push`、`branch.created`、`tag.created`；
  - 合并请求类：`pull_request`、`merge`。

  保存流水线时如果前提不满足，**允许保存**（用户可能稍后再开启），但在编辑器的触发器节点和流水线详情页显示“触发器未生效：连接「x」未开启 push 事件监听”。
- **触发人显示**：执行的 `triggered_by` 为 `trigger_run_as` 用户；事件 actor（平台用户名）另存 `trigger_actor`，UI 显示为“由 alice 推送触发（以 bob 身份执行）”。平台用户名不映射为 Ferry 用户，所以审批的“禁止自审”只约束执行身份，不约束推送人。这一限制在审批面板中注明。

### 9.4 各平台差异

| 平台 | 推送 | 合并请求 | Tag | 文件列表 | 说明 |
|---|---|---|---|---|---|
| GitHub / GitLab / Gitea / Gitee | Webhook 或轮询 | ✔ | ✔ | Webhook 载荷带；轮询走 compare | 归一化扩展见 §9.3a |
| Codeup | Webhook（新旧两种格式）或轮询 | ✔（`changeRequests`） | ✔ | **P1-1 用真实载荷核实**，缺失时走 compare（Codeup OpenAPI 的 compare 接口，同样在 P1-1 核实） | 没有仓库创建/删除事件，与本功能无关 |
| plain git | ✘ | ✘ | ✘ | — | 只能手动或定时触发 |

- 轮询模式的 commit 来自快照中的 `revision`（`polling.rs:270`）。
- 轮询**观察的是快照**：两次轮询之间的多次推送只会产生一次事件，最终 commit 是最新的那个。这一点在 UI 的触发器节点上注明。

### 9.5 执行身份 `trigger_run_as`

- 语义同定时任务的 `run_as`（`cron_jobs.rs:273`）。
- **绑定规则**：任何人保存了定义变更，只要不是当前的 `trigger_run_as`，身份就改为保存者，并需要显式确认（§7.1）。这样每一次事件执行的内容，都是执行身份本人最后认可的内容。
- **投递时实时检查**该用户：
  - 仍是工作空间成员；
  - 仍有 `pipelines.run`；
  - 定义含部署任务时还需 `deployments.apply`。
  
  不满足时投递 `failed`，原因写“执行身份已失去执行权限”。
- 待确认决策 D4。

### 9.6 环境与检出版本解析

**环境**：

| 触发方式 | 环境 |
|---|---|
| 手动、调用流水线 | 由请求指定（现状） |
| 事件 | 按 `trigger_environments` 首个匹配项选择；无匹配则投递 `skipped`（§7.1） |
| 定时 | 由定时任务配置（现状） |

选中的环境还要通过环境保护的 ref 检查（§13.0）。检查不通过时：

- 事件投递 `skipped`；
- 手动触发返回 400：“环境「prod」只允许从 refs/heads/main 部署。”

**检出版本**：`create_run` 为每个源计算 `{ref, commit}`，并冻结进快照：

| 触发 | 触发源 | 其它源 |
|---|---|---|
| 推送 / Tag | 事件的 ref + **事件 commit**（精确检出，不取分支最新） | 各自的 `default_ref`，执行时检出分支最新 |
| 合并请求 | head ref + head commit；`ferry.event.base_ref` 供脚本使用 | 同上 |
| 手动 | 用户选择的 ref 和可选 commit；缺省为 `default_ref` | 用户可逐个覆盖 |
| 定时 | `default_ref` | 同左 |

### 9.7 传给 act 的上下文

worker 请求的变化：

- **新增** `event_name`：`push`、`pull_request` 或 `workflow_dispatch`。定时和上游流水线触发也用 `workflow_dispatch`，并在 `ferry.trigger` 中区分。
- **新增** `event`：最小化的 GitHub 形状载荷，worker 写入临时文件后设为 act 的 `EventPath`：

```json
{ "ref": "refs/heads/main", "before": "…", "after": "<sha>",
  "repository": { "full_name": "org/app", "clone_url": "…", "default_branch": "main" },
  "head_commit": { "id": "<sha>", "message": "…", "author": { "name": "…" } },
  "pull_request": { "number": 12, "title": "…", "head": { "ref": "feat", "sha": "…" }, "base": { "ref": "main" } },
  "inputs": { "env": "prod" }, "sender": { "login": "alice" } }
```

- **新增** `inputs`。
- **新增**环境变量 `FERRY_RUN_ID`、`FERRY_RUN_NUMBER`、`FERRY_RUN_ATTEMPT`、`FERRY_JOB_ATTEMPT`、`FERRY_TRIGGER`、`FERRY_ENVIRONMENT`、`FERRY_SOURCE_<ID>_COMMIT`。
  - `FERRY_RUN_ATTEMPT`：重跑链上的执行尝试次数；
  - `FERRY_JOB_ATTEMPT`：本任务在本执行内的重试次数。
- `GITHUB_REF`、`GITHUB_SHA` 由 act 根据 event 与工作目录推导。**P1-1 的工作是验证 act v0.2.89 在 `EventPath` 下的 `github.ref/sha` 是否等于事件值**，不等时在 worker 里显式覆盖 `GitHubContext` 对应字段。
- 提交信息、作者等不受信任的文本只作为数据传入，绝不拼进命令。旧格式的 `{{…}}` 白名单不扩展。

### 9.8 调用流水线 `ferry-task: pipeline`（P2）

```yaml
ferry-task: { type: pipeline, with: { inputs: { version: "${{ needs.build.outputs.version }}" }, wait: true } }
# 目标流水线与环境在信封 task_bindings.call = { pipeline_id, environment_id } 中
```

- 以当前执行的 `triggered_by` 为身份调用 `create_run`，`Trigger::Pipeline`。幂等由 §8.5 的意图记录与 `pipeline_run_parents` 保证。
- 防环：沿 `parent_run` 链深度 ≤ 5，并且链上不能出现同一条流水线，否则任务失败。
- `wait: true` 时等待子执行结束，结果映射为任务结果，子执行的 outputs 暂不回传（P2 不做）；`wait: false` 时子执行入队后本任务即成功。
- 取消父执行时，**不级联**取消子执行，日志里给出子执行链接（同部署的理由）。

### 9.9 定时任务对接（P0）

- `cron_jobs.rs:868-885` 改为调用 `create_run`，`Trigger::Schedule{cron_run_id}`，执行的 `trigger_kind=schedule`。
- 定时任务的表单新增可选的“执行参数”（按目标流水线的 inputs 动态生成，§19.4），存入定时任务配置。

---

## 10. 变量、凭据与上下文

现有机制保持不变：作用域优先级、secret 加密、按执行生成快照、脱敏。新增内容：

1. **执行参数 inputs**：
   - 按 inputs 定义校验类型、必填、选项。
   - `environment` 类型的取值范围是本流水线已绑定的环境，选中后等价于 `environment_id`，两者不能冲突。
   - inputs 存入 `pipeline_runs.inputs`（加密，复用变量快照的封装，AAD 为 `pipeline-run-inputs:<run>`），因为用户可能在参数里传敏感值。
   - 日志脱敏：只对 inputs 中名字匹配 `(?i)(secret|token|password|key)` 的值脱敏。**这是显式的启发式规则**，UI 的参数说明里写明“敏感值请用密钥变量”。
   - inputs **不能**影响审批人、部署目标和被调用的流水线，因为 `task_bindings` 不接受表达式。
2. **服务端任务的 outputs**：
   - 部署任务输出 `deployment_run_id`、`revision`；
   - 审批任务输出 `approved_by`（逗号分隔的用户名）、`comment`；
   - 加密存储方式同 job outputs（`actions_run.rs:182-194`）。
3. **环境到应用的映射**放在 `task_bindings.deploy.applications` 里，不在环境上增加“应用”字段。原因：一个环境可能被多条流水线用来部署不同的应用，映射关系属于流水线定义。
4. **只在服务端使用的变量**：通知地址变量（`url_variable`、`sign_variable`）在快照中单独标记为 `server_only`，**不下发**到任何主机任务的 `secrets`。现状是快照里的变量名全部来自 YAML 引用，并整体下发给 worker；本项把通知变量从下发集合中排除。
5. **受保护环境的变量**：绑定到受保护环境的变量（作用域为 environment），只在环境保护门通过后才进入主机任务的下发集合（§13.0）。

---

## 11. 产物、缓存与报告

### 11.1 产物存储（P2，修复 G19）

- 抽出 `ArtifactStore` trait，有两个实现：`local`（现状）和 `s3`（S3 兼容，覆盖阿里云 OSS 和 MinIO）。
- 选择方式：`FERRY_ARTIFACT_STORE=local|s3`，以及 `FERRY_ARTIFACT_S3_ENDPOINT/BUCKET/REGION`；密钥只从环境变量或 vault 读取。
- **启用集群且存储为 `local` 时**：服务照常启动，但给出启动告警和平台设置页提示“集群模式下产物只保存在执行实例本地”。跨实例恢复产物（接管后的下游任务、其它实例上的部署）找不到文件时，任务**显式失败**：“产物位于实例 x 的本地存储，当前实例无法读取，请配置共享产物存储。”
  - 选择告警而不是拒绝启动，是为了不破坏已按 ADR“各实例独立数据目录”部署的集群。
- 待确认决策 D2，涉及新依赖。
- 如果集群在 P1 之前就要正式使用，D2 应提前到 P0 之后。

### 11.2 缓存（P2，修复 G16）

- 放开 `actions/cache` 的拒绝。worker 启用 act 自带的缓存服务（act v0.2.89 的 `pkg/artifactcache`）。**需在 P2-2 用真实 `actions/cache@v4` 验证协议版本兼容**，不兼容时本项降级为“明确拒绝并保留现状”。
- 缓存目录在执行主机本地：`<runner数据目录>/actions-cache/<workspace_id>/<pipeline_id>`。
  - 按流水线隔离，避免跨流水线投毒。
  - 主机组内不同主机的缓存互不共享，UI 和文档注明“缓存为主机本地”。
- 容量：
  - 每条流水线在每台主机上 ≤ 2 GiB；
  - **每台主机的全部缓存 ≤ 10 GiB**，在主机字段 `pipeline_cache_limit_mb` 中可调；
  - 超出时按 LRU 淘汰，由 worker 在任务结束时清理。
- 主机设置提供“清空流水线缓存”操作，通过 HostChannel 执行删除。
- 合并请求触发的执行**只读**缓存，不写入（§22.7）。

### 11.3 测试报告（P2，修复 G15）

- 信封里的 `reports` 声明任务和 glob。worker 在任务结束后（不论成功与否），在 `os.Root` 限定的 workdir 内匹配文件。
  - 单个文件 ≤ 10 MiB，合计 ≤ 50 MiB，超出时截断并告警。
  - 按 JUnit XML 解析，在 `result` 记录中新增 `reports: [{kind, totals:{tests,failures,errors,skipped,time}, cases:[{suite,name,status,time,message≤4KiB}]}]`，最多 5000 条用例。
- 解析在 worker 侧完成，原始 XML 不回传，只回传结构化结果，从而降低传输量与服务端解析面。
- 入库到 `pipeline_run_reports`，供红线和列表显示。

### 11.4 主机工作目录清理

- 现状：worker 的私有目录会在任务结束后删除。
- 本项新增：每个任务的工作目录 `<私有目录>/work` 在任务结束时一并删除，不论结果如何。
- 需要保留现场排查时，用户可以在任务上设置 `ferry-keep-workdir: true`，有效期仅限本次尝试；保留的目录在 24 小时后由下一次任务启动时清理。
- 旧格式流水线的 `workdir` 由用户显式指定，保持现状不删除（T2 的设计意图）。

---

## 12. 部署联动 `ferry-task: deploy`（P1，修复 G9）

执行流程：

1. 由执行的环境确定目标应用：`task_bindings.deploy.applications[environment_id]`。执行未选择环境，或映射里没有该环境时，任务失败：“当前环境「x」没有配置部署应用。”
2. **产物绑定**：
   - 产物按 **名称** 解析，而不是按路径。原因是 Actions 产物的路径包含执行 ID（§2.2）。
     - 产物表新增 `name` 列：Actions 产物取 upload-artifact 的名称；旧格式产物取路径（§16）。
     - 部署清单中的 `artifact.path`，在 Actions 流水线上解释为产物名称。
   - `artifact_source: current-run`：只取本执行（含重跑时复制过来的产物）中 `state='active'`、且所属任务已 `succeeded` 的产物。
   - `latest-succeeded`：取本流水线最近一次成功执行中同名的产物。这同时修复了“按路径取最近成功产物”对 Actions 产物不成立的问题。
   - 实现方式：扩展 `artifacts::resolve`（`artifacts.rs:295-330`），按 `name` 匹配，并新增参数 `within_run: Option<Id>`；给定时不要求执行已结束且成功，而是要求产物所属任务已经 `succeeded`。
3. **发起部署**：先写意图（§8.5），再调用部署的预览 → 应用路径。
   - 复用 `deployments::scheduled`（`deployments.rs:914-947`），抽出带“产物覆盖”和 `pipeline_intent_id` 参数的内部函数。
   - 权限检查改用执行身份的 `deployments.apply`。
   - 预览 token 与目标摘要由服务端生成并立即使用，语义与定时部署一致。
   - 部署执行表新增唯一列 `pipeline_intent_id`，作为幂等保证。
4. **等待结果**：每 2 秒轮询部署执行状态，日志里同步部署阶段。
   - 部署成功：任务成功。
   - 部署失败：如果 `rollback_on_failure` 为真，调用现有 `deployments::rollback`，回滚结果写入日志；任务仍记 `failed`。
5. 应用内的分批、暂停、主机组策略由应用的部署模式决定（`deployment_modes.rs`、`deployment_groups.rs`），流水线不重复定义。
6. **审计**：活动记录写“流水线 #n 的任务 deploy 发起部署 #m”。

---

## 13. 环境保护、审批与质量红线

### 13.0 环境保护（P1，新）

**问题**：如果生产审批只存在于流水线 YAML 中，任何能编辑流水线的人都可以删掉审批任务，直接部署到生产。Flow 和 GitHub 都把保护规则绑定在**环境**上，原因正在于此。

**方案**：`pipeline-environments` 资源新增 `protection` 字段（JSON，`long_text`），由拥有环境编辑权限的人维护，不出现在任何流水线定义中：

```json
{ "required_approvers": { "user_ids": ["1001"], "role_ids": ["31"] },
  "min_approvals": 1,
  "allowed_refs": ["refs/heads/main", "refs/tags/v*"],
  "timeout_minutes": 1440,
  "allow_initiator": false }
```

| 规则 | 生效位置 |
|---|---|
| `allowed_refs` | `create_run` 时检查触发源的 ref：手动触发返回 400，事件投递记 `skipped`，`end_reason=protection_denied` |
| `required_approvers` | 执行被领取后、**任何任务开始前**，平台插入一个隐式的环境保护门，任务键为 `__environment__`，状态机与审批任务相同。批准后才开始执行任务。重跑时重新审批 |
| 受保护环境的变量 | 只有保护门通过后才进入主机任务的下发集合 |

- 未配置 `protection` 的环境，行为与现状完全一致。
- 保护门与 YAML 中的审批任务**叠加**，而不是互相替代。
- 环境的 `protection` 修改记入活动记录。
- 待确认决策 D12。

### 13.1 审批 v2（P1，修复 G8）

| 能力 | 规则 |
|---|---|
| 审批人 | `task_bindings[job].approvers.user_ids` ∪ 持有 `role_ids` 中任一角色的成员。角色按**角色 ID** 引用，避免改名后失效；角色成员关系按审批时刻判定。两者都为空时，退回现状：拥有 `pipelines.run` 的成员。不新增权限点。 |
| 批准与拒绝 | `POST …/jobs/{key}/approvals`，请求体 `{decision:"approve"\|"reject", comment}`。`comment` ≤ 500 字；拒绝时必填。旧客户端不带请求体时等价于 approve（兼容）。 |
| 会签 | `min_approvals` 取 1–10，达到人数即通过；任意一人拒绝即失败（Flow“或签/会签”的最常用组合）。 |
| 自审 | `allow_initiator:false` 时，执行的触发人（手动触发者，或事件执行身份）不能审批。 |
| 超时 | `timeout_minutes` 取 0–43200（30 天），0 表示不超时。到期按 `on_timeout` 处理，状态转换见 §8.1。 |
| 记录 | 存入 `pipeline_run_approvals` 表，审批历史在执行详情中可见，同时写活动记录。 |
| 通知 | 进入 `waiting` 时触发 `notifications` 中订阅了 `waiting` 的通道，消息里附执行链接。 |
| 并发 | 同一用户对同一任务的同一尝试只能决策一次，由唯一索引 `(run_id, job_key, attempt, user_id)` 保证。决策和状态推进在同一事务内完成，用条件更新 `UPDATE pipeline_run_jobs SET state=… WHERE run_id=? AND job_key=? AND attempt=? AND state='waiting'` 防止重复推进，任务表不需要 `version` 列。事务提交后把执行的 `not_before` 置为当前时刻以唤醒执行（§8.1）。 |
| 失效的审批角色 | 引用的角色被删除后，按剩余审批人判定；审批人集合为空时，任务保持等待，日志提示“审批人已不存在，请修改流水线或环境保护配置”。**不**退回默认规则（`pipelines.run`），因为那会放宽权限。 |

### 13.2 质量红线 `ferry-task: gate`（P2）

```yaml
gate:
  needs: [test]
  ferry-task:
    type: gate
    with:
      rules:
        - "reports.test.failures == 0"
        - "reports.test.pass_rate >= 95"
        - "fromJSON(needs.scan.outputs.critical) <= 0"
```

- 规则就是 `evaluate` 表达式（§7.5），不另造一套规则语言。
- 可用的上下文：
  - `reports.<job>.tests/failures/errors/skipped/pass_rate`；
  - `needs.*`。
- 任一规则为假或求值出错，任务即失败，日志按规则逐条列出表达式与求值结果。
- 不在 P2 范围：覆盖率格式（Cobertura/JaCoCo）解析。它可以由脚本步骤输出 `coverage` 到 outputs，再写成规则，无需等待。

---

## 14. 通知（P1，修复 G10）

两种形式共用一个投递实现：

1. **流水线级 `notifications`**：执行进入 `started` / `waiting` / `succeeded` / `failed` / `cancelled` 时投递。
2. **任务级 `ferry-task: notify`**：在图中显式位置投递，可使用表达式模板（上下文不含 `secrets`，§7.5）。

投递规则：

- **渠道格式**：
  - `webhook`：通用 JSON；
  - `dingtalk`：markdown 消息，另有可选的 `sign_variable` 用于加签；
  - `feishu`：post 富文本；
  - `wecom`：markdown。
- **地址来源**：必须是 secret 变量，标记为 `server_only`（§10.4），**永不写入定义、日志，也不下发到主机**。
- **投递方式**：复用告警的出站 HTTP 客户端：
  - 超时；
  - 禁止跟随重定向转发凭据；
  - 响应体不入日志；
  - 单次尝试。
- **失败处理**：失败写入执行日志的“通知”段，并在执行详情中标注 ⚠。不阻塞执行，也不改变执行结果。
- **至多一次**：
  - 先插入 `pipeline_run_notifications`，唯一键为 `(run_id, job_key, event, channel_index)`，流水线级通知的 `job_key=''`，状态为 `sending`；
  - 再发送；
  - 最后更新为 `sent` 或 `failed`。
  
  接管后遇到 `sending` 记录，不重发，改为 `unknown`。
- 邮件通知不在本期：需要先有平台级 SMTP 配置（D7）。

---

## 15. 定义版本（P0，修复 G4）

- 新表 `pipeline_revisions`，每次创建或保存定义产生一行。
  - 定义未变时不产生新行。比较对象是**规范化 JSON**：信封按键排序后序列化；YAML 原文按字节比较，因为注释和格式也是用户内容。
  - 只修改元数据（名称、分组等）的保存也不产生新行。
  - 行内容：`number` 在流水线内递增；`definition`（`long_text`）；`message` 为可选保存说明，≤ 200 字；`created_by`、`created_at`。
- `pipeline_runs.revision_id` 记录执行所用的版本。重跑沿用该版本，即便流水线已修改；执行详情给出提示“该执行使用第 n 版定义（当前第 m 版）”。
- 接口：
  - `GET /pipelines/{id}/revisions`：统一信封；
  - `GET /pipelines/{id}/revisions/{rid}`：单个版本；
  - 恢复 = 前端取出旧定义，填入编辑器，用户再保存，从而产生新版本。不提供服务端“回滚”动词，避免绕过校验和版本冲突检查。
- 前端用 Monaco diff 对比任意两个版本，monaco 已在依赖中。
- 保留：同历史保留策略。被执行引用的版本不删除；只有在没有执行引用、且不是最近 50 版时才清理。
- 升级：迁移时为每条现存流水线生成第 1 版。

---

## 16. 数据模型变更汇总

**约定**：

- 用 SeaORM schema builder 追加迁移（T3d），不修改旧迁移。
- ID 都是 BIGINT 雪花 ID，所有整数用 BIGINT。
- JSON 或大文本列用 `migration::long_text()`（`migration.rs:45`），不带 DEFAULT，不进入索引；需要索引时另加 `*_hash VARCHAR(64)`。
- 新语句**必须手工加入** `schema_tests` 的列表（`migration.rs:1104-1122`），否则不会被测试覆盖。
- 三库（SQLite/MySQL/PostgreSQL）都要通过 `schema_tests` 和 `scripts/test-backends.sh`。
- 修改唯一索引的迁移要能重跑：MySQL 的 DDL 逐条提交，迁移中途失败后必须能再次执行（总方案 §3.1）。

| 迁移名 | 期 | 内容 |
|---|---|---|
| `pipeline_run_leases_v1` | P0 | `pipeline_runs` 加 `lease_owner VARCHAR(128) DEFAULT ''`、`lease_epoch BIGINT DEFAULT 0`、`priority BIGINT DEFAULT 0`、`not_before BIGINT NULL`、`end_reason VARCHAR(64) DEFAULT ''`、`concurrency_key VARCHAR(128) DEFAULT ''`（ref 部分取哈希，§8.3）、`concurrency_limit BIGINT DEFAULT 0`、`queue_reason VARCHAR(255) DEFAULT ''`；索引 `pipeline_runs_dispatch(state, priority, created_at)`、`pipeline_runs_concurrency(concurrency_key, state)`。`hosts` 加 `pipeline_slots BIGINT DEFAULT 2`。`pipeline_run_jobs` 加 `attempt BIGINT DEFAULT 1`、`wait_reason VARCHAR(255) DEFAULT ''`、`end_reason VARCHAR(64) DEFAULT ''`。`pipeline_run_steps` 加 `job_key VARCHAR(255) DEFAULT ''`、`attempt BIGINT DEFAULT 1`，唯一索引改为 `(run_id, job_key, attempt, step_index)`（先建新索引，再删旧索引）。`pipeline_run_logs` 加 `job_key VARCHAR(255) DEFAULT ''`、`attempt BIGINT DEFAULT 1`。 |
| `pipeline_run_provenance_v1` | P0 | `pipeline_runs` 加 `trigger_ref VARCHAR(64) DEFAULT ''`、`trigger_actor VARCHAR(255) DEFAULT ''`、`revision_id BIGINT DEFAULT 0`、`rerun_of_id BIGINT DEFAULT 0`、`root_run_id BIGINT DEFAULT 0`、`attempt BIGINT DEFAULT 1`、`inputs`（long_text，加密，可空）、`source_refs`（long_text，可空）；索引 `pipeline_runs_root(root_run_id)`、`pipeline_runs_pipeline(workspace_id, pipeline_id, created_at)`。 |
| `pipeline_revisions_v1` | P0 | 新表 `pipeline_revisions(id, workspace_id, pipeline_id, number, definition long_text, message, created_by, created_at)`；唯一索引 `(pipeline_id, number)`；为现存流水线回填第 1 版。 |
| `pipeline_triggers_v1` | P1 | 新表 `pipeline_triggers(id, workspace_id, pipeline_id, kind, connection_id, repository_key long_text, repository_hash VARCHAR(64), source_id, filters long_text, created_at)`；索引 `(workspace_id, connection_id, repository_hash, kind)`。新表 `pipeline_trigger_deliveries(id, workspace_id, pipeline_id, trigger_id, event_id, event long_text 加密, dedupe_key VARCHAR(64), redelivery_seq BIGINT DEFAULT 0, state, reason, run_id, created_at, updated_at)`；唯一索引 `(pipeline_id, dedupe_key, redelivery_seq)`，索引 `(state, created_at)`、`(workspace_id, pipeline_id, created_at)`。 |
| `pipeline_environment_protection_v1` | P1 | `pipeline_contexts` 加 `protection long_text`（可空；只对 `kind='environment'` 有意义）。 |
| `pipeline_approvals_v1` | P1 | 新表 `pipeline_run_approvals(id, workspace_id, run_id, job_key, attempt, user_id, user_name, decision, comment, created_at)`；唯一索引 `(run_id, job_key, attempt, user_id)`。 |
| `pipeline_task_intents_v1` | P1 | 新表 `pipeline_task_intents(id, workspace_id, run_id, job_key, attempt, kind, state, external_id, created_at, updated_at)`，唯一索引 `(run_id, job_key, attempt)`。部署执行表加 `pipeline_intent_id BIGINT NULL`，并建唯一索引（三库都允许唯一索引中出现多个 NULL）。新表 `pipeline_run_parents(intent_id 唯一, run_id, parent_run_id)`，P2 调用流水线时使用。 |
| `pipeline_notifications_v1` | P1 | 新表 `pipeline_run_notifications(id, workspace_id, run_id, job_key, event, channel_index, state, error, created_at, updated_at)`；唯一索引 `(run_id, job_key, event, channel_index)`。 |
| `pipeline_artifact_attempts_v1` | **P1-3**（重跑和部署都依赖） | `pipeline_artifacts` 加 `name long_text`、`name_hash VARCHAR(64)`、`job_key VARCHAR(255) DEFAULT ''`、`attempt BIGINT DEFAULT 1`、`state VARCHAR(16) DEFAULT 'active'`；回填：Actions 产物从路径中解析出名称，旧格式产物的 `name` 取路径；索引 `pipeline_artifacts_run(run_id)`、`pipeline_artifacts_lookup(workspace_id, pipeline_id, name_hash)`。 |
| `pipeline_reports_v1` | P2 | 新表 `pipeline_run_reports(id, workspace_id, run_id, job_key, attempt, kind, totals long_text, cases long_text, created_at)`；索引 `(run_id)`。 |

**状态值说明**：`state` 列仍是字符串，新枚举值（执行的 `waiting`，任务的 `pending`、`reused`）无需迁移。前端 `pipeline.ts:54-71` 的状态徽标需要同步更新。

---

## 17. REST API 变更

全部位于 `/api/workspaces/{workspace}` 下，遵守统一信封、`version` 冲突检查、中文错误句子（§6）。

| 方法与路径 | 期 | 说明 | 权限 |
|---|---|---|---|
| `POST /pipelines/{id}/runs` | P0 扩展 | 请求体新增 `inputs`、`sources: {source_id: {ref, commit}}`；保留 `branch`、`commit`（等价于唯一触发源的覆盖值） | `pipelines.run` |
| `PATCH`/`POST /pipelines` | P1 扩展 | 请求体新增 `confirm_trigger_identity`（§7.1） | 现有 |
| `GET /pipelines/{id}/revisions`、`GET /pipelines/{id}/revisions/{rid}` | P0 | 版本列表与详情 | `pipelines.read` |
| `PATCH /pipeline-environments/{id}` | P1 扩展 | 新增 `protection` 字段 | 现有环境编辑权限 |
| `POST /pipeline-runs/{id}/reruns` | P1 | 请求体 `{mode: "all"\|"failed"\|"from", job_key?}` → 201 `{id}` | `pipelines.run`；含部署时另需 `deployments.apply` |
| `POST /pipeline-runs/{id}/jobs/{key}/approvals` | P1 扩展 | 请求体 `{decision, comment}`；`key=__environment__` 表示环境保护门 | 审批人规则（§13） |
| `GET /pipeline-runs/{id}/approvals` | P1 | 审批历史 | `pipelines.read` |
| `GET /pipeline-runs/{id}/log-streams?after={seq}` | P1 | `text/event-stream`。事件 `log`（与现有日志条目同形，新增 `job_key`、`attempt`）、`state`（执行、任务、步骤状态变化）、`end`。心跳 15 秒；断线后客户端带 `after` 续接。不可用时前端回退到现有轮询 | `pipelines.read` |
| `GET /pipeline-runs/{id}/graph` | P1 | DAG 视图所需数据：任务、needs、阶段、状态、尝试次数、主机名，避免前端重新解析快照 | `pipelines.read` |
| `GET /pipeline-trigger-deliveries?pipeline_id=` | P1 | 触发记录，统一信封 | `pipelines.read` |
| `POST /pipeline-trigger-deliveries/{id}/redeliveries` | P1 | 手动重新投递失败或跳过的投递 | `pipelines.run` |
| `GET /code-connections/{id}/tags?url=&page=` | P1 | 标签建议，与现有 branches 接口同形；如适配器已有能力则直接复用 | 现有 |
| `GET /pipeline-runs/{id}/reports` | P2 | 测试报告 | `pipelines.read` |
| `GET /pipeline-runs?…` | P1 扩展 | 返回字段新增 `pipeline_name`、`triggered_by_name`、`trigger_kind`、`waiting_jobs`；筛选新增 `trigger_kind`、`awaiting_me=true` | `pipelines.read` |
| `GET /pipelines?…` | P1 扩展 | 返回字段新增 `last_run: {id, number, state, finished_at}`、`trigger_kinds: [...]` | `pipelines.read` |

**SSE 与集群**：

- SSE 由任意实例提供，数据源是数据库。
- 每个实例对每个**执行**（而不是每个连接）维护一个查询循环：活动时每 500 ms 查询一次增量日志与状态；NATS 可用时改为由编排者发布“有新数据”通知，收到后再查询。
- 同一执行的多个连接共享一次查询（进程内广播）。
- 每个实例最多保持 500 个 SSE 连接，超出时返回 503，前端回退到轮询。

---

## 18. 协议、Runner 与 worker 变更

### 18.1 协议（`crates/protocol`）

- **P0–P2 不需要新帧**。流式执行、取消、文件读写已经覆盖全部需求。
- P3 的 Windows 执行如果走 PowerShell 原生执行，也复用 `ExecStreamStart`。
- 结论：`VERSION` 保持为 1。

### 18.2 旧格式执行改为流式（P1）

- `pipeline_runs.rs` 的旧格式执行器改用 `exec_stream`，日志按分片实时落库。Runner 的 1 MB 捕获上限不再适用，改由 §21 的日志字节上限约束。
- 老 Runner 兼容：老版本 Runner 收到 `ExecStreamStart` 时的实际行为（返回 `Failed`、断开会话或忽略），**需在 P1-9 用当前最老的受支持 Runner 版本实测**，再据此实现回退到 `exec` 的检测。回退后在日志首行提示升级 Runner。Actions 执行已经依赖 `exec_stream`，可参考其现有处理方式。

### 18.3 worker（`workers/actions`）

| 变更 | 期 | 说明 |
|---|---|---|
| `evaluate` 子命令 | P1 | §7.5；构建目标新增 `windows-amd64`，仅用于服务端求值（执行仍拒绝 Windows） |
| 请求新增 `event_name`/`event`/`inputs`/额外环境变量 | P1 | §9.7 |
| `reports` 收集 | P2 | §11.3 |
| 单一矩阵组合执行 | P2 | 请求新增 `matrix: {k: v}`。worker 在交给 act 前把该任务的 `strategy.matrix` 改写为单值组合，以副本方式进行，不修改原 YAML 文本。删除 `MaxParallelString="1"` 的串行依赖。 |
| 缓存服务 | P2 | §11.2 |

所有 worker 变更都要满足：沿用 `go.sh` 的固定补丁机制、`-race` 测试、可复现构建（两次独立构建的 SHA-256 相同，与 T14 相同）。

---

## 19. 前端设计

总原则：所有列表和表单仍是 `ResourceTable`/`ResourceForm`（AutoTable/AutoForm）的配置。画布和执行 DAG 属于“专用编辑器”，作为表单字段的自定义渲染器，延续 `actions-editor` 的做法（总方案 §7/§8、ADR 统一 UI 迁移）。**不新增第二套表格或表单实现**。

### 19.1 编辑器（`ActionsEditor.tsx` 演进）

```
┌ 顶栏：名称 · 可视化/YAML · 版本(第 n 版 ▾ 对比) · 流水线设置 · 撤销/重做 · 保存 · 保存并执行 ┐
├──────────┬──────────────────────────────────────────────────┬──────────────────────┤
│ 节点面板 │  画布（阶段泳道，可折叠）                        │ 配置面板（按节点类型）│
│ 触发器   │  [触发器] ─▶ ┃构建┃ build ─▶ test ─▶ ┃发布┃ 审批 ─▶ 部署 ─▶ 通知 │                      │
│  手动    │                                                  │  AutoForm 配置驱动    │
│  推送    │                                                  │                      │
│  合并请求│                                                  │                      │
│  标签    │                                                  │                      │
│  定时→   │                                                  │                      │
│ 任务     │                                                  │                      │
│  主机任务│                                                  │                      │
│  容器任务│                                                  │                      │
│  人工审批│                                                  │                      │
│  部署    │                                                  │                      │
│  通知    │                                                  │                      │
│  质量红线│(P2)                                              │                      │
│  调用流水线(P2)                                             │                      │
└──────────┴──────────────────────────────────────────────────┴──────────────────────┘
```

1. **节点类型**：
   - `trigger`：可视化的 `on`，每种事件一个节点，固定在最左侧，不可连线；
   - `job`：现有类型；
   - `server-task`：按 `type` 显示不同图标和摘要；
   - `stage`：React Flow 的 group 节点。

   节点类型注册表 `nodeTypes` 是一张配置表：类型 → {图标、面板配置、YAML 读写函数}。新增类型只需加表项。
2. **触发器面板**：事件类型、分支/标签/路径过滤（标签输入，支持 `!` 取反）、MR types；绑定源（下拉选择 `sources` 中 `trigger:true` 的源）；监听状态提示（§9.3），并给出“去开启”链接到连接的监听页签；执行身份显示及“改为我”。
   - “定时”节点不写入 YAML，点击后跳转到新建定时任务，预填本流水线。这与 D3 对应：给用户一个入口，但不制造第二套定时机制。
3. **流水线源面板**（“流水线设置”抽屉中）：增删源，复用 `PipelineRepository` 的连接 → 仓库 → 分支选择器，标记触发源。
4. **流水线设置**：分组、标签（已有）；**环境（多选）、应用（单选）**，补上 G14 指出的缺失字段；**事件触发环境规则**（`trigger_environments`，行编辑：事件类型 + ref 模式 + 环境）；并发策略；整次执行超时；通知；测试报告（P2）。
5. **任务面板新增**：`if` 条件（文本框，下方给常用快捷项：总是执行、上游失败时、仅主分支）；`ferry-retry`；任务级 `env` 键值对；`continue-on-error`；`outputs` 键值对。
6. **服务端任务面板**：
   - 审批：成员和角色选择器（复用成员下拉，写入 `task_bindings`）、会签人数、超时、超时动作、允许自审、说明；
   - 部署：按已绑定环境逐行选择应用（写入 `task_bindings`）、产物名称（从本流水线 upload-artifact 的名称中选择）、产物来源、失败回滚；
   - 通知：渠道、地址变量（secret 变量下拉）、模板；
   - 红线（P2）：规则行编辑。
7. **编辑操作**：
   - 撤销/重做：文档快照栈，元素是 YAML 文本 + 信封，最多 100 步；快捷键 Ctrl/⌘+Z 和 Shift+Z；
   - 复制/粘贴任务：自动重命名 `build_2`，清理 `needs`；
   - 启用 Delete 键，删除前确认；
   - 自动排版：沿用现有分层算法，增加“按阶段排版”。
8. **校验呈现**：节点右上角的红点显示错误数，配置面板逐字段显示错误。服务端保存返回的 `fields` 映射到节点，键形如 `jobs.deploy.ferry-task.with.artifacts` 或 `task_bindings.deploy.applications`。
9. **YAML 模式**：Monaco 注册 Actions JSON Schema（P2），以本地打包文件提供，不发起网络请求，并补充 `ferry-*` 扩展键的 schema。

### 19.2 执行详情（`PipelineRunDetail.tsx` 演进）

- **顶部**：编号与尝试次数链、状态、触发说明（事件 · 提交信息 · 作者 · MR 链接 · 执行身份）、环境、定义版本、耗时；操作按钮：取消、重新运行、重跑失败任务。
- **主区域**：只读 DAG，复用编辑器的节点渲染，带阶段泳道，节点按状态着色。
  - 运行中节点脉动；`waiting` 节点高亮并给出“审批”按钮；`reused` 节点使用淡色并标注“复用自 #52”。
  - 节点右键或菜单提供“从此任务重跑”。
- **点击节点**打开底部抽屉（移动端为全屏）：步骤与日志（现有组件）、尝试次数切换、outputs（secret 已脱敏）、审批历史、部署执行链接。
- **页签**：
  - 日志（现有）；
  - 产物（内联列表与下载，统一表格配置，`fixed: run_id`）；
  - 测试报告（P2，汇总与失败用例表）；
  - 变量快照（现有）；
  - 触发详情（事件载荷摘要）。
- **实时性**：用 SSE 订阅日志和状态，失败时回退到现有 1 秒轮询。

### 19.3 列表

- **流水线列表**新增列：最近执行（状态徽标、编号、相对时间）、触发方式图标；行操作“触发”和“查看触发记录”。
- **执行列表**：
  - 显示流水线名称、触发人名称、触发方式，并可筛选触发方式；
  - 状态筛选补全 `waiting`、`timed_out`、`skipped`；
  - 新增“等待我审批”快捷筛选：`awaiting_me=true`，由服务端按审批人规则判定（含环境保护门）。
- **触发记录**：流水线详情的新页签，统一列表，显示时间、事件、ref、commit、结果（已创建 → 执行链接 / 已跳过 → 原因 / 失败 → 原因、重新投递）。

### 19.4 触发表单（动态参数）

- `pipelineRunAction`（`resources.tsx:156-163`）当前的参数是静态的。需要扩展 DSL：`RowAction.parameters` 支持 `(row) => Promise<FormField[]>`，即异步生成。用于：
  - 按 inputs 生成字段；
  - 每个源的 ref 选择（分支建议复用现有接口，并增加 tag 建议，复用连接的 tags 接口；如没有该接口，在 P1 补充 `GET code-connections/{id}/tags`）；
  - 环境选择。
- 这是对共享组件配置能力的扩展，符合 §12“扩展配置而非分叉”。
- 定时任务表单复用同一个异步参数生成函数。

### 19.5 可访问性与响应式

- 沿用现有验收：375/768/1280/1920 四种宽度，亮色与暗色主题。
- 画布节点可用键盘聚焦，方向键移动选择，Enter 打开面板。
- DAG 在移动端改为纵向列表视图，复用现有任务列表。

---

## 20. 权限与审计

- **不新增权限点**：
  - 审批由 §13 的规则决定；
  - 重跑需要 `pipelines.run`，包含部署任务时还需要 `deployments.apply`，身份为重跑者本人（§8.6）；
  - 部署任务在执行时检查执行身份的 `deployments.apply`；
  - 修改触发配置需要流水线的编辑权限，并受 `trigger_run_as` 的重绑规则约束（§7.1、§9.5）；
  - 环境保护由环境编辑权限维护。
- **审批人引用**：用户和角色都按 ID 引用，改名不影响。角色被删除时按 §13.1 的“失效的审批角色”处理，不放宽权限。
- **按流水线的访问控制**（谁能执行哪条流水线）不在本期：现有权限模型是工作空间级的扁平权限（总方案 §4.2）。对生产的保护由环境保护（§13.0）承担。如需按流水线授权，另行设计。
- **活动记录**新增以下事件，只记录 ID 和编号，不记录变量或参数值：
  - 触发配置变更；
  - 执行身份变更；
  - 环境保护变更；
  - 重跑；
  - 审批决策；
  - 部署发起；
  - 手动重新投递。

---

## 21. 可观测性与限额

| 项 | 默认 | 配置 |
|---|---|---|
| 每实例并发执行（不含挂起的执行） | 4 | `FERRY_PIPELINE_MAX_RUNS` |
| 每主机任务槽 | 2 | 主机字段 `pipeline_slots` |
| 每次执行最多任务数（含矩阵展开） | 200 | 常量 |
| **每个任务的日志** | 32 MiB | 常量；超出后不再落库，追加一条截断标记，任务继续执行 |
| **每次执行的日志** | 128 MiB | 常量；规则同上。日志存放在业务数据库中，这一上限保护数据库 |
| 单个事件最多触发的流水线数 | 50 | 常量；超出部分的投递记为 `skipped`，原因写明 |
| 触发调度器并发 | 4 | 常量 |
| SSE 连接数 | 500/实例 | 常量 |
| 每主机缓存总量 | 10 GiB | 主机字段 `pipeline_cache_limit_mb` |

- 结构化日志（tracing）字段：`run_id`、`job_key`、`attempt`、`lease_epoch`。**禁止**输出远端错误原文（现有规则，`pipeline_runs.rs:623`）。
- 仪表盘（总方案 §13）新增指标：今日执行数、成功率、平均耗时、等待审批数、排队中执行数。它们由 `pipeline_runs` 聚合得出，不引入时序库。
- 保留策略：执行、日志、产物、报告、投递记录、审批记录、通知记录都遵循现有的 `history_retention_days`（`maintenance.rs:570-586`），作为同一个清理任务的一部分。重跑时产物已经复制（§8.6），父执行被清理不会影响子执行。

---

## 22. 安全

1. **事件伪造**：沿用平台签名或 token 校验与请求体摘要去重；另加语义去重键（§9.3b）。没有签名能力的平台必须配置 secret（现状）。
2. **事件载荷注入**：提交信息、分支名等只作为数据进入 event JSON 和环境变量。旧格式模板白名单不扩展。checkout 的 ref 和 commit 沿用 T13 的安全引用方式。
3. **合并请求来自 fork**：
   - 只有 `head_repository_key == repository_key` 的 MR 才会触发；
   - fork 来源，或来源无法确认（轮询缺少 head 仓库字段）的投递，一律 `skipped`，原因写“来自外部仓库的合并请求不自动执行”。
   - 原因：外部 fork 可能执行不受信任的代码，并读取密钥变量。如果有需求，再以“需审批后执行”的形式作为后续选项（D11）。
4. **同仓库 MR 的密钥暴露**：同仓库的任意分支都能通过 MR 触发执行并读取流水线变量，与 GitHub 对同仓库 PR 的行为一致。
   - 受保护环境用 `allowed_refs` 限制分支（§13.0），MR 的 head ref 通常不在允许范围内，因此拿不到生产变量。
   - 文档中明确提示：“能推送分支的人，就能读取未受保护环境的变量。”
5. **执行身份提升**：§9.5 的重绑规则，加上投递时实时检查权限；重跑使用重跑者的身份（§8.6）。
6. **服务端表达式**：上下文不含 `secrets`，`task_bindings` 不接受表达式（§7.5）。
7. **缓存投毒**：按流水线隔离目录；缓存不跨工作空间；合并请求触发的执行**只读**缓存，不写入。
8. **通知地址**：只能来自 secret 变量，标记为仅服务端使用，不下发到主机；响应体不入日志；不跟随重定向转发凭据。
9. **服务端求值子进程**：
   - 使用固定路径的二进制，校验其 SHA-256 与打包清单一致；
   - 请求文件放在 0600 临时目录；
   - 超时 5 秒后 kill；
   - Windows 下不弹窗。

---

## 23. 兼容与迁移

| 对象 | 处理 |
|---|---|
| 旧格式 JSON 流水线 | 照常编辑和执行。**获得**：执行租约与并发、触发来源、版本、流式日志、重跑（只有“重新运行”，因为没有任务粒度）。**不获得**：事件触发、服务端任务、inputs、环境保护门之外的新能力；受保护环境的 ref 和审批规则对旧格式同样生效。另提供“转换为可视化流水线”（P2）：生成单任务 Actions 定义，`workdir`→`defaults.run.working-directory`，`env`→`env`，`valueFrom: variable/X`→`${{ vars.X }}` 或 `${{ secrets.X }}`（按变量类型），`checkout`→`actions/checkout` + 源，`artifacts`→`actions/upload-artifact`；转换结果在保存前展示 diff，由用户确认（D5）。 |
| v1 信封 | 读取时视为 `schema: 1`。**P1-1 起前端写出 v2**：下次保存时，前端把 `checkout_bindings` 中的 `connection_id/repository_url` 提取为 `sources`（`trigger:false`），再写为 v2。服务端同时接受 v1 和 v2。**升级迁移不改写已存储的定义**，避免一次性批量重写造成风险。 |
| `ferry-approval: true` | 照常有效，解析时等价于没有 `task_bindings` 的审批任务（§7.3） |
| 历史执行 | 新列取默认值；UI 对 `revision_id=0` 显示“升级前执行”。 |
| **升级方式** | **包含 P0-1 的版本必须全停升级**（§8.4.4）。升级脚本在启动新版本前检查：同一数据库上不能有旧版本实例的心跳（集群），并且 `lease_owner=''` 的 `running` 执行数为 0；否则拒绝启动并说明原因。单实例和桌面形态沿用现有的“升级前 drain”。 |
| 升级时遗留的执行 | 全停升级后，`lease_owner=''` 的遗留 `running` 执行由新版本首次启动时按现状判为失败，一次性行为，在发布说明中注明。 |
| 定时任务 | 配置不变；新增的 inputs 字段可选。 |
| 前端旧客户端 | 审批接口不带请求体时视为 approve；触发接口不带 inputs 时用默认值，必填而无默认值时返回 400 中文提示。 |

---

## 24. 分期计划

每个任务给出：目标、主要文件、验收（必须是真实主机、真实仓库，沿用 T4/T13/T14 的验收标准，并在三种数据库上运行）。任务文档在开发时落到 `docs/tasks/T18-*.md`。

### P0 可靠性底座

| 任务 | 内容 | 主要文件 | 验收 |
|---|---|---|---|
| P0-1 执行租约与并发 | §8.1（尝试的存储模型与 `waiting` 挂起）、§8.2、§8.4；迁移 `pipeline_run_leases_v1`。并发策略不在此任务（P1-8） | `pipeline_runs.rs`、`actions_run.rs`、`lock.rs`（只复用）、`runner_updates.rs`、`migration.rs`、`entities.rs`、`hosts.rs` | 见下方列表 |
| P0-2 唯一创建入口与触发来源 | §9.1、§9.9；`pipeline_run_provenance_v1` 中的触发来源部分 | `pipeline_runs.rs`、`cron_jobs.rs` | 定时触发的执行 `trigger_kind=schedule` 且可跳转到定时执行；HTTP 触发行为不变（现有 154 项回归通过）；`create_run` 可以在外部事务中调用 |
| P0-3 定义版本 | §15；`pipeline_revisions_v1` | `pipelines.rs`、`migration.rs`、前端版本下拉和对比 | 保存产生版本，规范化后相同的定义不产生版本；执行记录 `revision_id`；对比与恢复；升级回填第 1 版且三库一致 |

P0-1 的验收：

1. 单实例同时运行 4 条执行，第 5 条排队；4 条审批中的执行被挂起，不占执行槽。
2. 主机槽为 1 时，**同一实例内**同主机的两个任务串行，`wait_reason` 可见（覆盖锁重入问题）。
3. 双实例共享数据库：kill 实例 A，实例 B 在 45 秒内接管：
   - A 上的主机任务记 `orchestrator_lost`；
   - 审批中的执行继续等待，批准后执行完成；
   - **本用例不涉及跨实例产物**（共享产物存储见 P2-7）。
4. 实例重启不影响另一实例上运行中的执行（G1 的回归用例）。
5. 栅栏：
   - 注入延迟让租约过期，原编排者的状态写入和日志写入都被拒绝，并自我熔断（取消远端流）；
   - 接管后日志 `seq` 连续、无冲突。
6. 数据库短暂不可用超过 22 秒时，编排者自我熔断并取消远端流。
7. SSH 断流后远端进程是否终止，跨实例转发时 NATS 桥是否断开，**按真实结果记录**，不预设结论。
8. Runner 升级 drain 期间不授予新槽。
9. 全停升级检查生效。

### P1 Flow 核心闭环

| 任务 | 内容 | 验收要点 |
|---|---|---|
| P1-1 事件触发 | 第一步：§9.3 的 `TriggerEvent`，扩展 normalize 和 polling，并为 5 家平台各录制真实载荷作为夹具（Codeup 需要用户提供测试组织与 PAT）。第二步：§7.1 `sources` 与 `trigger_environments`、§7.2、§9.2–9.7，worker 的 event/inputs；`pipeline_triggers_v1` | 推送到匹配分支 → 执行，检出 commit 精确等于事件 commit；**新分支首推触发**；不匹配的分支和路径不触发；tag 触发且同一 tag 只建一条执行；MR opened/synchronize 触发，轮询中只改标题不触发；fork MR 被跳过；轮询模式触发；按 `trigger_environments` 选择环境，无匹配时 `skipped`；compare 失败时 `skipped`；执行身份失权时 `failed`；调度器在 `create_run` 后崩溃不产生重复执行；**act 中 `github.sha/ref` 等于事件值** |
| P1-2 inputs 与触发表单 | §7.2 dispatch、§10.1、§19.4 | 5 种类型的校验；必填缺失返回 400；定时任务传参；敏感值脱敏规则 |
| P1-3 重跑与自动重试 | §8.6、§7.4；**`pipeline_artifact_attempts_v1`** | 三种重跑的复用集与 §8.6 定义一致（单元测试穷举小 DAG）；复用任务的 outputs 正确；产物复制到新执行后可恢复；父执行被清理后子执行的产物仍在；审批传播；`ferry-retry` 的失败/超时/主机丢失三种触发；重试后旧尝试的产物不被下游取到；尝试切换显示 |
| P1-4 审批 v2 与环境保护 | §13.0、§13.1；`pipeline_approvals_v1`、`pipeline_environment_protection_v1` | 指定用户/角色 ID、会签 2 人、拒绝必填意见、超时 reject/approve、禁止自审、重启后继续等待、并发重复提交只计一次；**删除 YAML 审批任务后，部署到受保护环境仍需审批**；`allowed_refs` 拒绝；受保护环境的变量在保护门通过前不下发 |
| P1-5 部署任务 | §12；`pipeline_task_intents_v1` | 构建 → 部署到所选环境，产物来自本次执行、按名称解析（不是“最近一次”）；用**真实部署模式**（Docker/Native）验证；部署失败时回滚；执行身份无 `deployments.apply` 时失败；取消执行不中止部署；**在发起部署后、回写意图前 kill 编排者，接管后不产生第二次部署** |
| P1-6 通知 | §14；`pipeline_notifications_v1` | 本地 HTTP 夹具接收 4 种渠道格式；地址不出现在定义、日志、活动记录和主机的 worker 请求中；接管后不重发 |
| P1-7 服务端求值 | §7.5、§8.5 | 所有任务级 `if` 由服务端求值，被跳过的主机任务不占槽；`if` 覆盖 success/failure/always/cancelled 及 needs/outputs/inputs 比较；引用 `secrets` 的服务端表达式在保存时被拒绝；求值错误使任务失败；Windows 桌面服务端可用且不弹窗 |
| P1-8 并发策略 | §8.3 | `queue` 上限且无队首阻塞（一个饱和组不阻塞其它流水线）；`queue_latest` 只保留最新；`cancel_in_progress` 取代；`pipeline_ref` 分组；挂起的执行计入上限；工作空间轮转 |
| P1-9 实时日志与旧格式流式 | §17 SSE、§18.2、§21 日志上限 | SSE 断线续接无重复无遗漏；集群下任意实例可订阅；旧格式日志实时可见；超过 32 MiB 时截断；老 Runner 回退（按实测行为） |
| P1-10 前端 | §19.1–19.3 | 浏览器真实验收：节点面板拖入 5 种节点 → 保存 → 重开一致；YAML 注释保留；撤销/重做；环境/应用字段；触发环境规则；环境保护表单；触发器提示；执行 DAG 实时着色；审批/拒绝；三种重跑；执行列表名称与筛选；375/768/1280/1920 宽度及暗色主题 |

### P2 质量与效率

| 任务 | 内容 |
|---|---|
| P2-1 测试报告与红线 | §11.3、§13.2 |
| P2-2 缓存 | §11.2（含协议兼容核实，失败则降级） |
| P2-3 矩阵展开 | §8.8 |
| P2-4 调用流水线 | §9.8 |
| P2-5 阶段泳道 | §7.1 `stages`、§19.1 |
| P2-6 Actions Schema、模板参数 | §19.1 第 9 点；模板“保存为模板”时可编辑参数 |
| P2-7 共享产物存储 | §11.1（启用集群时提前） |
| P2-8 旧格式转换 | §23 |
| P2-9 主机工作目录清理 | §11.4 |

### P3 扩展（另行立项）

- Windows 执行（D6）；
- 云效 Flow YAML 导入（T14 已延期）；
- 步骤模板市场：复用 `marketplace.rs` 的模式，提供可复用的任务片段，类似 n8n 社区节点，但只是 YAML 片段。

**依赖顺序**：

```
P0-1 → P0-2 → (P0-3 ∥ P1-7)
→ P1-1 → P1-3
→ (P1-2 ∥ P1-4 ∥ P1-6 ∥ P1-8 ∥ P1-9)
→ P1-5 → P1-10 收尾
```

P1-5 依赖 P1-3 的产物迁移和 P1-4 的环境保护。前端可以与后端并行，前提是接口契约（§17）先冻结。

**委派建议**（按 CLAUDE.md §6）：

- P0-1 是核心的调度与栅栏逻辑，属于紧耦合的架构决策，**由主代理直接实现**。
- P1 中 P1-4、P1-6、P1-9，以及 worker 侧改动（P1-1 的 worker 部分、P1-7）彼此独立，可以在契约冻结后交给子代理并行实现。
- 集成与验收由主代理负责。

---

## 25. 测试策略

| 层 | 内容 |
|---|---|
| Rust 单元测试 | 定义校验（每条新规则都有正例和反例，校验错误文案）、触发过滤 glob（对照 GitHub 文档示例）、事件映射（§9.3a，每家平台、每种模式都用录制的夹具）、并发组键与饱和过滤、重跑复用集（穷举小 DAG）、审批判定、栅栏条件、锁 owner 唯一性 |
| Go 单元测试 | `evaluate`、事件上下文、矩阵单组合改写、报告解析、缓存（`-race`） |
| 真实 API（Python 脚本，三库） | 扩展 `scripts/test-pipelines.py`、`test-actions-execution.py`，新增 `test-pipeline-triggers.py`、`test-pipeline-cluster.py`（双实例，含故障注入） |
| 真实主机 | Runner 和 SSH 两种通道，所有主机任务用例都在两种通道上运行 |
| 浏览器 | 扩展 `test-actions-canvas.mjs`，新增 `test-pipeline-run-graph.mjs` |
| 回归 | 现有 154 项执行回归、分类 29 项、仓库事件 120 项全部保持通过；旧格式 JSON 流水线逐项回归 |
| 不可伪造 | Codeup 真实账号与真实 Webhook 验收需要用户提供环境。夹具通过**不能**表述为“真实云验收”（沿用 Codeup ADR） |

---

## 26. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| act v0.2.89 在 EventPath、缓存、单组合矩阵上的行为与预期不符 | P1-1、P2-2、P2-3 受阻 | 每项第一步都是可行性测试（T14 的做法）；失败时显式降级并记录，不硬做 |
| 租约与栅栏改造影响面大 | 回归风险最高 | P0-1 单独交付，包含双实例故障注入测试和完整的现有回归；全停升级 |
| 集群时钟偏差或数据库停顿导致租约误判 | 重复执行 | 本地单调时钟自我熔断（§8.2.4）；要求 NTP；SSH 主机在核实前禁用 `host_lost` 重试 |
| SSH 断流不终止远端进程 | 接管后同一任务可能在远端重复运行（仅限配置了 `host_lost` 重试的场景） | 核实之前，SSH 主机上的 `host_lost` 重试默认禁用，UI 说明原因 |
| 事件风暴（批量推送或轮询首轮） | 大量执行 | 轮询首轮只建立基线（现状）；语义去重；事件触发默认 `queue_latest`；单事件最多 50 条流水线；执行级槽 |
| 5 家平台的载荷差异超出预期（尤其 Codeup 的 MR 动作、文件列表） | P1-1 范围膨胀 | P1-1 第一步先录制真实载荷并固化夹具，再写映射；无法确定的字段按 §9.3a 的保守规则处理（跳过而不是放行） |
| 服务端求值子进程在 Windows 桌面上被安全软件拦截 | `if` 求值失败 | 显式报错（任务失败并给出原因），不静默放行 |
| 修改 `pipeline_run_steps` 唯一索引 | 迁移失败或中途中断 | 先建新索引、再删旧索引，可重跑；三库迁移测试 |

---

## 27. 待确认决策（需要你拍板）

每项给出推荐与理由。未确认前，相应任务不开工。

| # | 决策 | 推荐 | 理由 | 备选 |
|---|---|---|---|---|
| D1 | 服务端表达式求值方式 | **worker `evaluate` 子命令**（复用 act exprparser，多打一个 Windows 构建） | 与主机任务语义完全一致，满足 T14“不做重复求值器” | 用 Rust 写一个子集求值器：不依赖子进程，但两套语义可能出现差异 |
| D2 | 集群产物存储 | **S3 兼容（OSS/MinIO）**，引入 `object_store` crate | 集群必须共享存储；对象存储是运维常规设施 | 只支持共享文件系统（NFS）：无新依赖，但部署要求更高，且一致性依赖 NFS 配置 |
| D3 | YAML `on.schedule` | **拒绝，统一使用定时任务** | 定时任务已有执行身份、环境、重试、历史；避免两套定时机制 | 把 YAML 中的 schedule 物化为定时任务：两处可编辑，容易不一致 |
| D4 | 事件触发的执行身份 | **流水线级 `trigger_run_as`，谁最后修改定义就绑定谁（需确认）** | 复用定时任务已验证的语义；每次事件执行的内容都由执行身份本人认可 | 用一个“系统身份”：需要新增服务账号概念，范围扩大 |
| D5 | 旧格式 JSON 流水线 | **冻结新功能，提供转换** | 避免两种格式都实现服务端任务和触发 | 功能对齐：工作量翻倍 |
| D6 | Windows 执行（P3） | **PowerShell/cmd 原生任务（不走 act）**，只支持 `run` 步骤 | act 对 Windows 主机模式支持有限；Windows 构建多数是脚本 | 尝试 act 的 Windows 主机模式：兼容性未知 |
| D7 | 通知渠道 | **Webhook + 钉钉 / 飞书 / 企业微信机器人**；邮件延后 | 不需要新的平台配置 | 同时做邮件：需要平台级 SMTP 配置与发件验证 |
| D8 | 分期与优先级 | **P0 → P1 → P2**，P3 另行立项 | P1 依赖 P0 的正确性 | 先做 P1 事件触发：会放大 G1/G2 风险 |
| D9 | 重跑形态 | **新建执行（新编号 + 尝试链）**，产物复制到新执行 | 不改 `(workspace_id, number)` 唯一索引；历史不被改写；父执行清理不影响子执行 | 同一编号多次尝试（GitHub 风格）：要改唯一索引和日志的键 |
| D10 | 并发配置位置与默认策略 | **信封 `settings.concurrency`**，YAML `concurrency` 继续拒绝；事件触发默认 `queue_latest` | 与“绑定与设置在 YAML 外”（T14）一致；不需要在服务端求值 group 表达式；默认抑制事件风暴 | 接受 YAML `concurrency`：需要求值 `${{ github.ref }}` 等表达式，兼容面更大 |
| D11 | fork 来源的 MR | **一律不自动执行** | 防止不受信任的代码读取密钥 | 允许但需审批：P1 范围扩大 |
| D12 | 环境保护 | **在环境上配置必需审批人与允许的 ref，平台插入隐式保护门；与 YAML 审批任务叠加** | 流水线编辑者无法绕过生产审批；与 Flow、GitHub 一致 | 只靠 YAML 审批任务：任何能编辑流水线的人都能去掉审批 |
| D13 | 事件触发的环境选择 | **`trigger_environments` 规则，按 ref 匹配；无匹配则跳过** | 现有规则要求绑定环境的流水线必须显式选择环境（T14），不能凭空选默认环境 | 为流水线指定一个“事件默认环境”：更简单，但 tag 发生产、分支发测试这类常见需求无法表达 |

---

## 28. 评审记录

### 28.1 自评（对照 CLAUDE.md 与原始需求）

| 检查项 | 结论 | 说明 |
|---|---|---|
| 需求：深度对标 Flow / n8n / ComfyUI 等 | **撤回（2026-11-01）** | 原对标没有核对实际产品；已由 `docs/ferry-流水线对标功能清单.md` 替代 |
| 需求：生产级全功能 | PASS | 覆盖触发、并发、环境保护、审批、红线、部署、通知、重跑、缓存、矩阵、版本、集群、安全、限额、保留 |
| 需求：精细到可直接开发 | PASS | 字段级 schema（§7）、状态机（§8.1）、事件模型与映射（§9.3）、DDL（§16）、API（§17）、文件与验收（§24） |
| 范围：不替换既有决策 | PASS | YAML 事实来源、资源绑定在 YAML 外（`task_bindings`）、HostChannel、REST 契约、共享组件、多环境、T15 连接模型均保留；新增约束列为 D1–D13 |
| 复用 | PASS | `lock.rs`（按实际的可重入语义使用）、`runner_updates::reserve_with`、`deployments::scheduled`、`artifacts::resolve`、定时任务的 `run_as`、告警投递、vault、`PipelineRepository`、AutoForm |
| 复杂度 | PASS | Rust 侧只新增一个 `ServerTask` trait（5 个实现）；红线复用 `evaluate`，没有新规则语言；没有新 DSL；拒绝了 n8n 数据流与按输入哈希跳过 |
| 事实与推断分离 | PASS | 未核实项明确标注“需在 P*-* 核实”：act EventPath、5 家平台的 MR 动作与文件列表、Codeup compare、SSH 断流、NATS 桥断开、act 缓存协议、老 Runner 对流式帧的反应 |
| 未经授权的产品决策 | PASS | 审批语义、环境保护、执行身份、fork MR、存储、通知渠道、事件环境选择都列入 §27 |

### 28.2 独立评审

- **方式**：由一个未参与撰写的评审代理，只读核对文档与代码。
  - 抽查了 31 处代码引用：27 处准确，2 处行号偏移，2 处引用了不存在的机制。
  - 对状态机、租约、发件箱、重跑、审批、部署、密钥、SSE 做了失败场景推演。
- **首轮结论**：NOT READY，共 6 个 Critical、17 个 Important、13 个 Minor。
- 以下所有 Critical 与 Important 均已修订，Minor 已逐项处理。修订后的事实声明已由主代理对照代码复核：
  - `lock.rs:20-30` 的可重入；
  - `actions_run.rs:596-606, 677-681` 的产物路径；
  - `repository_monitor.rs:413-427` 的 `branch.created`。

### 28.3 评审问题与处理

| # | 级别 | 问题 | 处理 |
|---|---|---|---|
| C1 | Critical | 仓库事件不保存 commit、before、文件、MR 目标分支、fork 来源，触发设计无数据可用 | 新增 §2.3 事实；§9.3 定义类型化的 `TriggerEvent`，由扩展后的 normalize 和 polling 产出，并存入发件箱；§9.3a 给出各平台、各模式映射；P1-1 第一步为录制真实载荷 |
| C2 | Critical | 绑定环境的流水线被事件触发时无法选择环境，必然失败 | §7.1 新增 `trigger_environments`；§9.6 定义选择规则，无匹配则跳过；D13 |
| C3 | Critical | `lock::acquire` 对同一 owner 可重入，同一实例内主机槽失效 | §2.2 记录事实；§8.2.2 规定每个持有者使用唯一 owner；P0-1 验收第 2 条覆盖 |
| C4 | Critical | 产物路径包含执行 ID，“按引用复用”“同路径”“重试”都不成立 | §8.6 改为把产物复制到新执行并改写路径；同一执行内的重试标记旧产物为 `superseded`、按尝试回收；§12 改为按名称解析；迁移提前到 P1-3 |
| C5 | Critical | 唯一索引 `(run_id, job_key)`、`(run_id, step_index)` 阻碍尝试和矩阵 | §8.1 确定“任务行原地更新 + 步骤和日志按尝试分开”的模型；§16 修改步骤唯一索引，并给日志加 `job_key/attempt` |
| C6 | Critical | 生产审批只写在可编辑的 YAML 中，可被流水线编辑者去掉 | 新增 §13.0 环境保护（必需审批人、允许 ref、受保护变量延迟下发，与 YAML 审批叠加）；D12；P1-4 验收 |
| I1 | Important | 工作空间资源 ID 写进了 YAML，违反 T14 | 新增信封 `task_bindings`；YAML 只保留类型与非资源参数；`task_bindings` 不接受表达式 |
| I2 | Important | 执行身份可被提权；重跑身份未定义 | §7.1、§9.5：任何人修改定义都重绑为本人，并需确认；§8.6：重跑使用重跑者身份并检查 `deployments.apply` |
| I3 | Important | 等待审批的执行占用执行级槽 | §8.1 新增落库的 `waiting` 挂起状态，用 `not_before` 唤醒；挂起的执行计入并发上限 |
| I4 | Important | 队首阻塞，工作空间间不公平 | §8.2.3 用 SQL 排除饱和组，候选窗口 32，按工作空间的 running 数轮转 |
| I5 | Important | 时钟偏差与数据库停顿导致的自我熔断缺失 | §8.2.4 按本地单调时钟熔断并取消远端流；要求 NTP；风险表新增一行 |
| I6 | Important | 日志写入不受栅栏保护；序号冲突 | §8.4.3 日志写入同样带栅栏；单写入者；接管后从 `MAX(seq)+1` 开始；不更新热点行 |
| I7 | Important | 部署和通知接管不幂等 | §8.5 新增“先写意图”和下游唯一的 `pipeline_intent_id`；通知先插入再发送；P1-5 验收覆盖崩溃窗口 |
| I8 | Important | 发件箱唯一键与重新投递冲突；建执行与回写不原子 | §9.3b 唯一键改为 `(pipeline_id, dedupe_key, redelivery_seq)`；`create_run` 接受外部事务，与投递状态同事务提交 |
| I9 | Important | 新分支首推、tag 双事件、轮询 MR 指纹、动作映射、`before` 全 0 | §9.3a 映射表；§7.2 规定新分支以 `default_ref` 为比较基准 |
| I10 | Important | 忽略了现有的主机升级保留 | §2.2 记录事实；§8.2.5 改为按任务 `reserve_with`，drain 期间不授予槽 |
| I11 | Important | 主机任务的 `if` 由谁求值未定义 | §7.5 规定所有任务级 `if` 由服务端求值，act 只负责步骤级 |
| I12 | Important | 服务端表达式可读取密钥；inputs 可改审批人；通知变量会下发到主机 | §7.5 上下文排除 `secrets`；`task_bindings` 不接受表达式；§10.4 通知变量标记为 `server_only` 不下发 |
| I13 | Important | 重跑规则自相矛盾 | §8.6 给出复用集的形式化定义与审批传播；删除部署任务的特殊规则 |
| I14 | Important | “集群 + 本地存储即拒绝启动”违反已批准的集群 ADR | §11.1 改为告警，跨实例读取不到时显式失败；P0-1 验收第 3 条明确不涉及跨实例产物 |
| I15 | Important | 滚动升级不安全 | §8.4.4：清扫器只接管 `lease_owner<>''` 的执行；P0 版本必须全停升级，并由升级脚本检查（§23） |
| I16 | Important | 迁移事实错误：`LONG_COLUMNS` 不存在、`schema_tests` 需手工登记、任务表缺 `version` 列、索引长度问题 | §2.3、§16 改为 `long_text()`、手工登记、`*_hash` 索引；审批改用条件更新；`not_before` 已用于挂起唤醒；通知表补 `workspace_id` 和 `job_key` |
| I17 | Important | 缺少生产能力：日志上限、主机缓存总量、工作目录清理、按流水线授权、同仓库 MR 的密钥暴露、事件风暴、重跑链的产物保留 | §21 日志上限与保留策略；§11.2 主机缓存总量；§11.4 工作目录清理；§20 说明按流水线授权不在本期、由环境保护承担；§22.4 同仓库 MR；`queue_latest`；产物复制 |
| M1 | Minor | 行号偏移（`actions_run.rs:636`、`407-411`） | 已更正 |
| M2 | Minor | 能力矩阵写成“任何人可批” | 改为“有 `pipelines.run` 即可批准” |
| M3 | Minor | Flow 的阶段是执行边界，不只是展示 | §3.1 改写为“只借鉴组织层，并说明差异” |
| M4 | Minor | `trigger_invalid` 永远不会出现 | 已删除 |
| M5 | Minor | `on_timeout: approve` 的状态转换未定义 | §8.1 已补充 |
| M6 | Minor | §8.3 在 P0-1 与 P1-8 重复；`schema: 2` 何时引入不明 | P0-1 不含并发策略；v2 从 P1-1 开始写出（§23） |
| M7 | Minor | `FERRY_RUN_ATTEMPT` 有两种含义 | 拆分为 `FERRY_RUN_ATTEMPT` 和 `FERRY_JOB_ATTEMPT` |
| M8 | Minor | 角色按名称引用 | 改为角色 ID |
| M9 | Minor | “字节相同”判定会产生多余版本 | 信封规范化后比较 |
| M10 | Minor | SSE 每个连接每 500 ms 查询，成本高 | 改为按执行聚合，NATS 可用时用通知触发 |
| M11 | Minor | 跨实例时依赖 NATS 桥脉冲断流 | 纳入 P0-1 验收第 7 条 |
| M12 | Minor | 红线规则是第二种表达式语言 | 改为复用 `evaluate` 表达式 |
| M13 | Minor | 推送人未映射到 Ferry 用户，禁止自审不覆盖推送人 | §9.3b 明确此限制并在 UI 注明 |

**修订后结论**：可以进入用户评审。§27 的 D1–D13 确认后，可从 P0-1 开工；P1 的事件契约以 §9.3 为准。

# 预览穿透自动托管设计（流水线内置步骤 + 工作流穿透节点）

状态：IMPLEMENTATION。用户确认（2026-11-09）：流水线与工作流一起做；验收可由人工或 AI 完成；驳回语义、审批权限、等待超时按推荐；去掉左侧菜单。
前置：`docs/preview-holes-design.md`（基础穿透、代理、沙箱、连接释放）。本文只写增量。

## 1. 目标与范围

预览主要发生在流水线或工作流的验收、产物阶段。要做到：由流程自己启动服务、自动开预览、交给人或 AI 验收、流程结束自动回收，不需要人工维护端口和链接。

**本期做：**
- 流水线内置步骤 `ferry/preview@v1`：给当前 job 所在 Runner 的端口开预览，输出 `url`，可选择停在这一步等人工通过或驳回。
- 工作流节点 `host.preview`（穿透节点）：在指定主机上启动服务命令，等待端口可访问，开预览，输出 `url`。验收方式有两种：`human`（出口为“通过 / 驳回”）和 `none`（不等待，下游节点包括 AI 节点直接引用 `url`）。
- 自动回收：
  - 流水线：job 结束时回收该 job 的预览，运行进入终态时再兜一遍底。
  - 工作流：执行进入终态时回收预览，并停止节点启动的服务。
  - 后台监控兜底：来源已结束或已不存在的预览会被回收。
- 验收接口：`POST /api/workspaces/{ws}/holes/{id}/reviews`，供运行详情页、执行详情页和主机 tab 共用。
- 手动预览：去掉左侧菜单，改为主机详情新增“预览穿透”tab；列表显示来源和验收状态。
- 修复：“不可访问”原因文字的英文前缀问题，改为本地化存储。

**不做：**
- 会话中的 agent 自主开预览。
- 消息通知（流水线目前没有通知机制）。
- 审批人名单、多人会签、驳回意见。
- 工作流 `human.approval` 通用节点。
- 预览访问需要登录。
- 在 job 或执行之外保活服务。

## 2. 关键事实约束（项目内依据）

| 事实 | 依据 | 设计结论 |
|---|---|---|
| Runner 执行的命令结束后，其进程组会被 SIGKILL（Windows 用 Job Object） | `crates/runner/src/exec_stream.rs:13-55`、`process_job.rs` | 被预览的服务只能活在仍在运行的 job 或会话里 |
| 流水线 worker 只通过 stdout 单向发送事件，没有 stdin | `pipelines/actions_run/events.rs`、`workers/actions/main.go:366` | 服务端到步骤的回传通过 job 私有目录里的文件实现 |
| worker 会把内置 action 改写成调用自身子命令 | `workers/actions/artifact_steps.go:15` | `ferry/preview@v1` 改写为 `<worker> preview` |
| 流水线所有终态写入都经过 `runs.rs::finish_in` | `pipelines/runs.rs:1116` | 在这里兜底回收 |
| 工作流终态写入在 `run.rs::finish` 和 `request_cancel_in` | `workflows/run.rs:768, 458` | 在这两处回收预览并停止服务 |
| 工作流 `host.command` 单次最长 120 秒，结束即杀掉进程 | `workflows/host.rs:262` | 穿透节点自己持有一个长期运行的执行会话 |
| 工作流支持 park/wake 等待 | `workflows/run.rs:1189, 932` | `human` 验收复用 park/wake |
| 现有审批只有通过，没有驳回和超时 | `pipelines/runs.rs:1065` | 预览验收自带通过、驳回、超时三种结果 |

## 3. 数据模型（迁移 `holes_v2`，只新增列）

| 字段 | 类型 | 必填 | 默认 | 约束和说明 |
|---|---|---|---|---|
| source_kind | VARCHAR(16) | 是 | 'manual' | manual / pipeline / workflow |
| source_id | BIGINT | 是 | 0 | pipeline 为运行 id，workflow 为执行 id，manual 为 0 |
| source_ref | VARCHAR(128) | 是 | '' | pipeline 为 job key，workflow 为节点 id |
| review_state | VARCHAR(16) | 是 | 'none' | none / waiting / approved / rejected / timed_out |
| reviewed_at | BIGINT | 是 | 0 | |
| reviewed_by | BIGINT | 是 | 0 | 0 表示系统（超时） |

`probe_error` 改为存本地化消息（沿用 `i18n::render_stored` 的存储格式），按查看者的语言渲染。已有的纯文本记录原样显示。

**索引：** `holes_source (source_kind, source_id)`，用于终态回收和运行详情查询。

**DDL 示例（PostgreSQL；实际由 SeaQuery 生成，MySQL/SQLite 差异由现有 helper 处理）：**

```sql
ALTER TABLE holes ADD COLUMN source_kind VARCHAR(16) NOT NULL DEFAULT 'manual';
ALTER TABLE holes ADD COLUMN source_id BIGINT NOT NULL DEFAULT 0;
ALTER TABLE holes ADD COLUMN source_ref VARCHAR(128) NOT NULL DEFAULT '';
ALTER TABLE holes ADD COLUMN review_state VARCHAR(16) NOT NULL DEFAULT 'none';
ALTER TABLE holes ADD COLUMN reviewed_at BIGINT NOT NULL DEFAULT 0;
ALTER TABLE holes ADD COLUMN reviewed_by BIGINT NOT NULL DEFAULT 0;
CREATE INDEX holes_source ON holes (source_kind, source_id);
```

**影响历史数据：** 已有记录全部落到 `manual / 0 / '' / none`，语义不变。down 迁移按项目惯例为空操作，手工回滚时删除这些列和索引即可。

## 4. 生命周期与状态

**自动创建的预览**：
- `expires_at` 是硬性上限。等待验收时取 `now + 验收超时`；不等待时取 `now + 24h`。
- 来源结束时，不论是否已过期都会被撤销。
- 不能续期，不能手动编辑名称、端口或路径模式，可以手动撤销。

**验收状态流转：**

| 操作 | 前置条件 | 结果 | 非法时 |
|---|---|---|---|
| 创建（需要验收） | — | waiting | — |
| 创建（不需要验收） | — | none | — |
| 通过 / 驳回 `POST /holes/{id}/reviews {decision}` | waiting，且预览未撤销、未过期 | approved / rejected，记录 reviewed_at 和 reviewed_by | 不是 waiting 时返回 409 `holes.reviewClosed`；手动预览返回 409 |
| 超时（监控或步骤自己发现） | waiting 且 `expires_at <= now` | timed_out，reviewed_by = 0 | — |

**验收结果对流程的影响：**
- **流水线：**
  - approved：步骤成功，执行该 job 的后续步骤。
  - rejected 或 timed_out：步骤失败，job 失败，依赖它的 job 跳过（`always()` 除外）。
- **工作流：** approved 走“通过”出口；rejected 或 timed_out 走“驳回”出口，输出里带 `review_state`。

**验收权限：**
- 来源是 pipeline：需要 `pipelines.run`，与现有审批一致。
- 来源是 workflow：需要 `workflows.run`。
- 审批动作写入 activity 审计。

## 5. 流水线：`ferry/preview@v1`

```yaml
- id: preview
  uses: ferry/preview@v1
  with:
    port: 5173              # 必填，1-65535
    name: 前端原型           # 选填，默认取步骤名
    strip-prefix: false     # 选填，默认 true
    hold: approval          # approval | none，默认 none
    ready-timeout-seconds: 60   # 等待端口可访问，1-600，默认 60
    timeout-minutes: 60     # hold=approval 时等待验收，1-1440，默认 60
```

**worker 侧：**
- `rewriteBuiltinPreviewStep` 把这个步骤改写为 `<worker> preview`。参数通过 `INPUT_FERRY_PREVIEW_*` 传入，不拼进 shell 源码。
- 子命令生成请求 id，打印一行标记 `::ferry-preview::<nonce>::<json>` 来提交请求，然后轮询 `<job 私有目录>/ferry-preview/<id>.json`：
  - 读到 `{"url"}`：写入 `$GITHUB_OUTPUT` 的 `url`，并把链接打到步骤日志。
  - 读到 `{"error"}`：步骤失败。
  - hold=approval 时继续轮询 `{"decision"}`：approved 退出码 0，其它为 1。
  - 自身也有兜底超时，为验收超时加 30 秒。
- nonce 由 worker 启动时随机生成，只通过环境变量传给改写后的步骤。服务端只接受带正确 nonce 的标记，防止日志里的任意文本伪造请求。

**服务端侧：**
- `events.rs` 在步骤日志中识别标记并校验 nonce，然后在后台处理请求：
  1. 以运行发起人的身份（`Scope::for_actor(triggered_by)`）在当前 job 的主机上创建预览。来源记为 `pipeline / run_id / job_key`；需要 `holes.create`，主机必须是 runner 主机。
  2. 等待端口可访问，超时就写入 error。
  3. 通过主机文件通道把结果写回私有目录。写入路径只能是 `<该 job 私有目录>/ferry-preview/<id>.json`，id 只能由 `[a-z0-9-]` 组成。
  4. 验收出结果后，再写一次 decision。
- **回收：** job 进入终态时回收 `source_ref = job_key` 的预览；`finish_in` 内回收整个运行的预览（同一事务，提交后断开连接）。
- **运行详情 API：** `GET .../pipeline-runs/{id}` 新增 `previews` 字段，包含 id、job、name、url、state、review_state、probe_state、expires_at。

## 6. 工作流：`host.preview` 节点

**参数：**

| 参数 | 说明 | 取值 |
|---|---|---|
| host_id | 主机，必须是 runner 主机 | 必填，或用表达式引用上游 `execution_host.id` |
| command | 启动服务的命令，选填；为空时表示端口上的服务已由其它方式启动 | 最长 4000 字符；与 `host.command` 一样禁止 `{{` 插值，输入通过 `input_env` 传入 |
| port | 端口 | 1-65535 |
| name | 预览名称 | 选填 |
| strip_prefix | 是否去掉路径前缀 | 默认 true |
| ready_timeout_seconds | 等待端口可访问 | 1-600，默认 60 |
| review | 验收方式 | `human` / `none`，默认 none |
| review_timeout_minutes | 等待验收超时 | 1-1440，默认 60 |

**出口：**
- review=none：`main`。
- review=human：`approved` 和 `rejected`。

**输出：**
```json
{"url","hole_id","review_state","execution_host":{"id","name"}}
```

**执行过程：**
1. 有 command 时，以执行身份打开一个流式执行会话，不受 120 秒限制。会话登记到“执行 → 服务”表，节点完成后会话继续存活。
2. 等待端口可访问。
3. 创建预览，来源记为 `workflow / execution_id / node_id`。
4. review=none 时直接完成；review=human 时 park，等待验收结果唤醒，超时按 timed_out 唤醒。

**回收：** 执行进入终态（`finish` 和 `request_cancel_in`）时，撤销该执行的预览，并取消登记的服务会话（发送 ExecCancel，由 Runner 杀掉整个进程组）。服务端重启后，会话随连接断开，进程被 Runner 回收；残留的预览由监控兜底撤销。

**执行详情页：** 节点数据视图显示预览链接；review=human 且仍在等待时显示“通过 / 驳回”按钮。

**权限：** 节点执行需要 `workflows.run`、`hosts.terminal` 和 `holes.create`。

## 7. 手动入口调整（前端）

- 删除 `nav.ts` 里的 holes 入口。资源配置保留，用作主机 tab 和直接链接。
- 主机详情页新增 tab `holes`，可见条件是 `holes.read`。tab 内是 `ResourceTable`，固定 `host_id` 过滤；新建时主机预填并锁定。
- 列表新增“来源”列：手动显示“手动”；流水线显示“流水线运行 #编号 · job”并链接到运行；工作流显示“工作流执行 · 节点”并链接到执行。
- 列表新增“验收”列。状态为 waiting 时，行操作出现“通过 / 驳回”，需要 `pipelines.run` 或 `workflows.run`。
- 自动创建的预览隐藏编辑和续期操作。
- 运行详情页（流水线）新增“预览”区。执行详情（工作流）在节点数据里显示链接和验收按钮。

## 8. 异常与极端值

| 情况 | 处理 |
|---|---|
| 端口在 ready 超时内不可访问 | 不创建预览，或者立即撤销；步骤或节点失败，错误是本地化的“本地服务未就绪” |
| Runner 版本过旧，不支持 Forward | 同上，错误为 `holes.runnerUpgrade` |
| 主机不是 runner 主机 | 拒绝 |
| 同一 job 或节点多次开预览 | 允许，各自独立回收 |
| 运行或执行被取消 | 预览立即撤销，服务被杀掉，等待中的验收不再改变状态 |
| 写回文件失败（主机断线） | 记日志；步骤靠自身兜底超时失败，预览由回收逻辑处理 |
| 运行历史被清理 | 监控发现来源不存在时撤销预览 |

## 9. 测试与验收

**Rust：**
- 迁移和已有数据的默认值。
- 验收状态流转，包括 409 和权限。
- 按来源回收，包括连接被断开。
- 监控兜底：超时和来源丢失。
- 标记解析和 nonce 校验。
- 写回路径校验，拒绝路径穿越。
- `finish_in` 和工作流 `finish` 时回收。
- 工作流节点：服务会话随执行结束被取消，human 验收的两个出口。

**Go：** `ferry/preview@v1` 改写；子命令轮询，覆盖 url、error、decision、超时四种结果；参数校验。

**前端：** 去掉导航；主机 tab；来源列和验收操作的权限、状态条件；运行详情预览区；工作流执行的验收按钮；六语言文案。

**端到端（真实 server、Runner、Chrome）：**
1. 流水线 job 起服务 → `ferry/preview@v1` 设为 hold → 运行详情能打开预览 → 通过后 job 成功、预览被撤销。另跑一次驳回，job 失败。
2. 工作流穿透节点起服务 → human 验收 → 通过后执行结束，服务进程消失，预览被撤销。
3. 主机 tab 手动创建预览。

**交付检查：** `cargo test --workspace`、`go test ./...`（workers/actions）、`npm run typecheck`、vitest、`npm run build`、`cargo fmt --check` 全部通过。

**设计稿：** Figma 暂缺，以截图为准。

## 10. 评审维度摘要

| 维度 | 结论 |
|---|---|
| 灰度策略 | 无。新步骤和新节点只有显式使用才生效 |
| 操作记录 | 创建、验收、撤销写入 activity；系统操作人为 0 |
| 删除保留 | 自动预览不提供续期；记录随来源回收后保留，可以手动删除 |
| 计算逻辑 | 无 |
| 配置来源 | 默认值与上下限是硬编码常量；无新增环境变量 |
| 影响核心流程 | 流水线 `finish_in`、job 终态，工作流 `finish` 和 `request_cancel_in` 各新增一次回收调用；不改变原有状态流转 |
| 版本兼容 | 兼容升级。worker 随 server 构建；使用新步骤需要新 worker；旧 Runner 开预览时会得到明确错误 |
| 是否依赖三方 | 否 |
| 逻辑冲突 | 现有 `ferry-approval` 门控保持不变；预览验收是步骤或节点级别的独立机制 |

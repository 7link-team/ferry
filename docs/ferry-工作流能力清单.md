# Ferry 工作流能力清单

| 项 | 内容 |
|---|---|
| 版本 | 2026-11-02 |
| 依据 | 《ferry-工作流平台v3总体设计.md》全文，尤其 §2、§5–§10、§15、§18；《ferry-流水线对标功能清单.md》§2 及其付费限制说明；流水线外壳沿用 v2 |
| 用途 | 履行 v3 §18.5；逐期逐项演示、记录证据的验收库存，不是当前已交付功能声明 |
| 版本基线 | n8n 2.45.7；其 n8n-nodes-base 与 @n8n/n8n-nodes-langchain 均为 2.45.6。ComfyUI v0.43.0，按用户提供的本机清点快照核对 |
| 节点证据位置 | D:\bench\inv\n8n-codex.json；n8n-integrations.md；n8n-n8n-nodes-langchain-2.45.6/package/package.json 的 n8n.nodes；comfy-nodes.md；comfy/comfy_api_nodes/nodes_*.py |
| 截图证据位置 | D:\bench\shots\n8n\ 与 D:\bench\shots\comfyui\；本文仅引用相对文件名，不复制第三方界面到公开仓库 |
| 与 v3 的关系 | 本清单是验收依据，分期以 v3 为准；§18 是规范性补充，覆盖前文冲突。ADR 2026-11-02后续条目已确认V1～V15推荐方向；V6/V7具体模型家族/服务商仍在S4/S3前选择，不将方向确认等同具体选型或实现 |
| 可机读设计映射 | [节点契约说明](prd/schemas/README.md)：46个S1核心+3个S2内置数据+4个S2文件/归档节点（共53个）；B001～B075逐成员映射，B008及B005/B020/B021/B041/B047/B057已补参数契约；design_only，其余后续成员保留scheduled，不改变本清单“未开始/已交付”的验收状态 |

## 使用方式

每期验收时，将本期承接的行逐项演示；同一行列出多个原产品成员时，每个成员的业务能力都必须核对，不能只演示代表节点。验收记录按下表逐行展开，状态列只取“未开始”或“已交付”，交付时附验证证据（Ferry 版本、输入、操作步骤、输出或录像/日志、结论）。本文初始全部为“未开始”，原产品截图不等于 Ferry 交付证据。

| 清单行/成员 | 状态 | 交付验证证据 |
|---|---|---|
| 所有 A–G 行及其成员（初始值） | 未开始 | 待交付时逐项填写；不得以原产品截图代替 |

期列只使用 S0–S5、v2 P0/P1/P2 或“不对标”。按功能承接而非文件格式兼容验收，不复制 n8n/ComfyUI 代码，不承诺加载它们的工作流或社区节点。核心节点保留 75 个源标识各占一行，旧版合并到同一 Ferry 能力，不重复建设。C 保留 127 个文件成员各一行；D 分类附录保留 363 个节点各一行。F 的 691 是目录条目出现次数（145 组），不是去重节点数：原样精确 ID 去重为 680；replacement 与其它组有交叉，且一个 ID 有尾空格，均不静默改写。G 按 46 个文件计，文件内全部字面 node_id 一并列出。

S1 的代码事件、上游完成依赖 v2 P1-1 发件箱，人工会签/自审依赖 P1-4；顺序按 §18.16：v2 P0 → S0 → P1-1/P1-4 核心规则 → S1。S0 的表结构不能当成 S1 引擎已经交付。所有“不对标”都需说明原因；待选服务商、模型族与长尾批次仍保留在验收范围内，不以“不对标”消除。

## A. n8n 平台能力（非节点）

依据标记：“实测”指对标清单 §2 或 v3 §2.2/§9.2；“仅入口/锁定”不表示验证过付费能力；“目录/设计记载”无可对应截图时明确记为未补测。

| 编号 | 能力 | n8n 实际表现（依据） | Ferry 承接（v3 章节） | 期 | 证据截图（本机） |
|---|---|---|---|---|---|
| A001 | 画布编辑 | 实测：节点/连线、拖动、复制粘贴、撤销重做、整理布局 | §9.1–§9.2 | S1 | n8n/04-canvas-if-merge-unexecuted.png；n8n/16-after-tidy-up.png |
| A002 | 便签、分组、停用、命名 | 实测：便签及组、停用/重命名入口 | §6.1、§9.2 | S1 | n8n/14-canvas-group-node.png；n8n/15-sticky-deactivate-rename.png |
| A003 | 快捷键、命令面板、小地图、聚焦 | 实测：右键标快捷键、命令栏、聚焦与小地图 | §9.1–§9.2 | S1 | n8n/13-node-context-menu.png；n8n/62-command-bar.png；n8n/63-focus-panel.png；n8n/64-minimap-while-panning.png |
| A004 | 节点库与触发选择 | 实测：分类节点面板、运行时选择触发器 | §5.2、§7.1、§9.2 | S1 | n8n/03-node-panel-triggers.png；n8n/28-execute-trigger-choice.png |
| A005 | NDV 三栏与数据视图 | 实测：输入、参数/设置、输出；Schema/表格/JSON | §9.2 | S1 | n8n/07-ndv-if-input-output.png；n8n/41-ndv-code-node.png |
| A006 | 表达式补全与实时求值 | 实测：语法错误与数值结果预览、字段补全 | §6.4、§18.12 | S1 | n8n/08-expression-inline-autocomplete.png；n8n/09-expression-result-preview.png |
| A007 | 节点通用设置 | 实测：重试次数/间隔；错误停止、继续、错误口；始终输出、一次、备注 | §6.6、§18.7、§18.9 | S1 | n8n/10-ndv-node-settings.png；n8n/11-node-settings-retry-onerror.png；n8n/12-canvas-error-output-branch.png |
| A008 | 固定数据与编辑固定 JSON | 实测：固定输出可编辑，单步复用固定数据；生产忽略固定数据 | §6.1、§6.6、§18.14 | S1 | n8n/17-ndv-pinned-output.png；n8n/18-edit-pinned-json.png；n8n/19-partial-exec-pinned-false-branch.png |
| A009 | 参数变化后的过期标记 | 实测：下游橙色过期 | §9.2 | S1 | n8n/08-expression-inline-autocomplete.png；n8n/13-node-context-menu.png |
| A010 | 完整/单步执行与分支状态 | 实测：画布 ✓/✗、连线条数、未命中分支灰色、Execute step | §6.6–§6.7、§9.3 | S1 | n8n/05-canvas-executed-status.png；n8n/19-partial-exec-pinned-false-branch.png |
| A011 | 实时执行日志树 | 实测：左侧节点树与耗时，右侧输入输出，运行转圈 | §9.2–§9.3、§12.2 | S1 | n8n/06-logs-panel.png |
| A012 | 停止、失败诊断 | 实测：可停止；失败画布与详情；Ferry 区分取消/失败 | §6.7、§18.4 | S1 | n8n/30-stopped-execution.png；n8n/42-execution-failed-canvas.png；n8n/44-failed-execution-detail.png |
| A013 | 重试版本与重试关联 | 实测：当前保存定义/原定义；Retry of execution；调试到编辑器入口锁定 | §9.3、§18.7 | S1 | n8n/45-execution-retry-menu.png；n8n/46-debug-in-editor.png |
| A014 | 执行列表、筛选与版本 | 实测：工作流/全局列表、状态时间版本过滤、详情版本 | §5.7、§9.3、§12.2 | S1 | n8n/37-workflow-executions-tab.png；n8n/38-executions-filter.png；n8n/43-global-executions-list.png |
| A015 | 执行自定义数据、标注搜索 | v3 §18.3 记载；截图未单独覆盖 | §18.3、§12.2 | S1 | 未有专门截图；依据文字记录，待补测 |
| A016 | 工作流设置 | 实测：错误工作流、时区、保存成功/失败/手动执行、超时 | §6.1、§18.7、§18.13 | S1 | n8n/21-workflow-settings.png；n8n/21b-workflow-settings-timeout.png |
| A017 | 草稿、发布、取消发布 | 实测：发布必填版本名、橙点、生产只用已发布版本 | §5.1、§18.6 | S0 | n8n/33-publish-dialog.png；n8n/34-published-state.png；n8n/35-published-with-unpublished-changes.png；n8n/36-publish-dropdown.png |
| A018 | 版本历史与预览 | 实测：列表和版本预览；无对比，社区版只保留一天；Ferry 增加对比 | §5.1、§9.3 | S0 | n8n/22-version-history.png；n8n/23-version-history-expanded-preview.png |
| A019 | 手动/定时/错误/子工作流触发 | 实测：多规则定时、子工作流参数；上游完成及代码事件按外壳发件箱 | §5.2、§6.6、§18.16 | S1 | n8n/24-triggers-workflow.png；n8n/31-ndv-schedule.png；n8n/58-ndv-execute-workflow-trigger.png；n8n/60-ndv-execute-subworkflow.png |
| A020 | 测试/生产 Webhook 与同步回应 | 实测：双地址、监听测试事件；同步模式按 v3 记载 | §5.2、§18.3、§18.10 | S1 | n8n/25-ndv-webhook-urls.png；n8n/26-webhook-listening.png；n8n/27-webhook-test-output.png |
| A021 | Webhook 安全与响应选项 | v3 §18.3：basic/header/JWT、CORS、IP、原始/二进制体、状态码/头；未单项补测 | §18.3、§18.10 | S1 | n8n/25-ndv-webhook-urls.png |
| A022 | 等待与多页表单 | 实测：时长/指定时间/Webhook/表单等待；多页与结束页由 §18.3 补齐 | §5.4、§18.3、§18.7 | S1 | n8n/29-running-wait-node.png；n8n/32-ndv-wait-resume-modes.png；n8n/59-ndv-form-trigger.png |
| A023 | 聊天、应用轮询与触发静态数据 | 目录/设计记载：聊天消息、轮询去重状态；没有单独截图 | §2.2、§5.2、§18.3 | S2 | 未有专门截图；依据文字记录，待补测 |
| A024 | 生产激活与调用策略 | 实测：发布状态；子工作流调用入口可见，“谁能调用”设置未找到，不能当成已验证 | §5.1–§5.2、§6.6、§18.6 | S1 | n8n/34-published-state.png；n8n/61-workflow-settings-caller-policy.png |
| A025 | 凭据与 OAuth2 | 实测：凭据创建入口；不声称跑通过全部 OAuth2 授权流程 | §2.2、§7.5、§14 | S0 | n8n/57-credential-create.png |
| A026 | 变量 | 实测：Variables 为锁定/升级入口，值管理未验证 | §2.2、§6.4；复用已有工作空间变量并接入工作流 | S0 | n8n/56-variables.png |
| A027 | 数据表 | v3 §2.2/§12.1 记载内置表；无单独截图 | §7.4、§12.1 | S2 | 未有专门截图；依据文字记录，待补测 |
| A028 | 项目、文件夹、角色与分享基础 | 实测：分享对话框；文件夹为许可证锁定入口，团队/细粒度权限未验证，不推断授权行为 | §2.2、§5、§12.1 group_id；工作空间权限与分组接入 | S0 | n8n/02-folder-license-gate.png；n8n/50-share-dialog.png |
| A029 | 定义级角色、共享权限细分 | 仅入口/锁定；Ferry 依 D14 扩展工作流级权限 | §2.2；v2 D14 | v2 P2 | n8n/50-share-dialog.png |
| A030 | 列表检索、标签、收藏与归档 | 实测：列表过滤、卡片菜单、描述标签、归档入口 | §18.3、§18.2 | S1 | n8n/47-workflow-list.png；n8n/48-workflow-list-filters.png；n8n/49-workflow-card-menu.png；n8n/51-description-tags.png；n8n/67-workflow-list-archived-tags.png |
| A031 | Insights 统计 | 实测：执行数、失败率、节省时间；按工作流细分需付费，未验证 | §2.2、§5.6 | S1 | n8n/55-insights.png |
| A032 | 模板与导入导出 | 记录：外部模板站、JSON 导入导出；不兼容其 JSON 格式 | §5.1、§9.1；Ferry 自有图模板 | S1 | n8n/20-workflow-more-menu.png |
| A033 | MCP 服务端/客户端 | v3 §2.2 记载：工作流暴露工具、Agent 调用服务；无独立实测截图 | §2.2、§7.4 | S2 | 未有专门截图；依据文字记录，待补测 |
| A034 | Evaluations 评测 | 实测：评测页入口；没有完整数据集/评分跑通证据 | §18.3；数据表+评测运行+规则/LLM评分 | S2 | n8n/65-evaluations-tab.png |
| A035 | 日志外送 | 仅付费/锁定：Log streaming；未验证实际投递 | §18.3；Webhook/OTLP | S5 | n8n/52-settings.png |
| A036 | 外部密钥库 | 仅设置/付费入口；未验证 Vault/KMS 取值 | §18.3 | S5 | n8n/52-settings.png |
| A037 | 源码管理、环境与 Git 同步 | 仅设置/锁定入口；没有同步成功证据 | §2.2、§15 | S5 | n8n/53-settings-environments.png |
| A038 | 社区节点与自定义 SDK | 实测：社区节点设置入口；不代表安装验证；Ferry 用自有声明规范/WASM，不加载 npm 节点 | §7.1、V12、§18.3；S2 定规范、S5 交付 SDK | S5 | n8n/54-settings-community-nodes.png |
| A039 | AI 辅助编排入口 | 实测：Assistant 入口，生成/修改图的具体行为未验证；作用待核实 | §15 S5 场景完善；由 LLM/Agent+自有图规范承接，需补测验收边界 | S5 | n8n/66-assistant.png |
| A040 | 全局设置与迁移检查入口 | 实测设置菜单；3.0 兼容报告只见入口。Ferry 验收自身图 schema/type_version 升级校验，不输出 n8n 迁移报告 | §6.1、§7.1、§9.1 | S1 | n8n/52-settings.png；n8n/20-workflow-more-menu.png |

## B. n8n 核心节点（75 个）

名称取 n8n-codex.json 的 Core Nodes 分类，表内省略统一前缀 n8n-nodes-base.。旧版行仍保留源 ID 以保证可追溯，“去重”指对应同一个 Ferry 节点；不把旧版实现另算一项 Ferry 产品。作用未从实测核实的项目明确保留核实标记。

| 编号 | n8n 节点 | 作用（一句） | Ferry 对应节点 / 能力 | 期 | 备注 |
|---|---|---|---|---|---|
| B001 | toolExecutor | 执行工具调用 | Agent 工具执行器（§7.4） | S2 | 不沿用其内部实现 |
| B002 | aiTransform | AI 辅助数据转换 | LLM 结构化输出+数据转换（§7.4） | S2 | 提示生成的转换需用户检查 |
| B003 | code | 运行 JS/Python 数据代码 | JS 沙箱/Python 主机节点（§7.2、§18.12） | S1 | JS 无 npm/文件/进程；Python 只在主机 |
| B004 | compareDatasets | 比较两组条目 | Compare（§7.4） | S1 |  |
| B005 | compression | 压缩与解压归档 | 文件归档节点（§18.3） | S2 |  |
| B006 | cron | 按 cron 规则触发 | 定时触发（§5.2、§7.4） | S1 | 旧版，与 scheduleTrigger 合并 |
| B007 | crypto | 加密、摘要等密码学转换 | 加密与哈希（§7.4） | S1 |  |
| B008 | dataTable | 读写内置数据表 | 数据表 CRUD（§12.1） | S2 |  |
| B009 | dateTime | 处理日期时间 | 日期时间（§7.4） | S1 |  |
| B010 | e2eTest | 端到端测试夹具 | 不对标 | 不对标 | 上游测试专用，不是业务节点 |
| B011 | e2eTestPollingTrigger | 轮询端到端测试夹具 | 不对标 | 不对标 | 上游测试专用，不是业务触发器 |
| B012 | editImage | 缩放裁剪合成图像 | 服务端图像节点（§18.3） | S3 |  |
| B013 | emailReadImap | 接收 IMAP 邮件 | 读取邮件（§7.4） | S2 |  |
| B014 | emailSend | 通过 SMTP 发邮件 | 发送邮件（§7.4） | S1 | S2 批次邮件能力复用；本基础 SMTP 节点 S1 |
| B015 | errorTrigger | 工作流失败触发 | 错误触发（§5.2） | S1 |  |
| B016 | executeCommand | 执行系统命令 | 主机命令（§7.4、§18.11） | S1 | 不在服务端执行命令 |
| B017 | executeWorkflow | 调用子工作流 | 执行子工作流（§6.6、§18.15） | S1 | 逐条/全部、调用深度与权限 |
| B018 | executeWorkflowTrigger | 作为子工作流接收输入 | 执行子工作流时触发（§7.4） | S1 |  |
| B019 | executionData | 写入执行自定义数据 | execution_annotations（§18.3） | S1 |  |
| B020 | convertToFile | 把条目转换为文件 | 文件↔条目（§18.3） | S2 |  |
| B021 | extractFromFile | 从文件提取条目 | 文件↔条目（§18.3） | S2 |  |
| B022 | readWriteFile | 读取或写入主机文件 | 主机文件读写（§7.4、§18.11） | S1 | 路径按主机权限，不暴露服务端文件系统 |
| B023 | filter | 筛选条目 | Filter（§7.4） | S1 |  |
| B024 | form | 后续表单页与结束页 | 多页表单/人工表单（§18.3） | S1 |  |
| B025 | formTrigger | 表单提交触发 | 表单触发（§5.2、§9.6） | S1 | 稳定生产地址，默认关闭公开访问 |
| B026 | ftp | 传输 FTP 文件 | 声明式/主机脚本/WASM 集成（§7.1、§18.3） | S5 | 协议细项待核实；不以 HTTP 模板冒充 FTP 客户端 |
| B027 | function | 批量运行旧版 JS | JS 代码 batch（§7.2） | S1 | 旧版，与 code 合并 |
| B028 | functionItem | 逐条运行旧版 JS | JS 代码 per_item（§7.2） | S1 | 旧版，与 code 合并 |
| B029 | git | 执行 Git 仓库操作 | 主机命令/检出代码（§18.11） | S1 | 仓库 Git 操作与 S5 平台源码同步不同 |
| B030 | html | 解析与生成 HTML | HTML 数据节点（§7.4） | S1 |  |
| B031 | htmlExtract | 从 HTML 提取内容 | HTML 数据节点（§7.4） | S1 | 旧版，与 html 合并 |
| B032 | httpRequest | 发送 HTTP 请求 | HTTP 请求（§7.3–§7.4） | S1 | 分页、批量、认证、重试；统一出站策略 |
| B033 | iCal | 处理日历文件；具体操作待核实 | 文件转换/声明式扩展（§18.3、§15） | S5 | 不能仅凭名称承诺全部 iCalendar 操作 |
| B034 | if | 按条件分为两路 | If（§7.4、§18.7） | S1 |  |
| B035 | interval | 按间隔触发 | 定时触发（§5.2） | S1 | 旧版，与 scheduleTrigger 合并 |
| B036 | itemLists | 旧版条目列表转换 | Split/Aggregate/Sort/Limit/去重（§7.4） | S1 | 旧版，与独立转换节点合并 |
| B037 | localFileTrigger | 监听本地文件事件 | 主机脚本/扩展触发器（§7.1、§18.3、§15） | S5 | 不监听服务端租户任意目录；细项待核实 |
| B038 | manualTrigger | 手动启动 | 手动触发（§5.2） | S1 |  |
| B039 | markdown | 转换 Markdown | Markdown 数据节点（§7.4） | S1 | 展示按 §9.6 净化 |
| B040 | merge | 合并多路条目 | Merge（§7.4、§18.7） | S1 | 全部前驱终态后求值，按端口次序 |
| B041 | moveBinaryData | 在 JSON 与二进制间转换 | 文件↔条目/资产引用（§18.3） | S2 | 旧版，与 convertToFile/extractFromFile 合并 |
| B042 | n8n | 调用 n8n 平台资源 API | Ferry 自身集成（§7.4、§18.6） | S2 | 映射自有工作流/流水线资源；不兼容 n8n API，不直接部署 |
| B043 | n8nTrigger | 平台事件触发；具体事件待核实 | 执行/定义事件触发（§5.2、§18.16） | S1 | 按 Ferry 事件承接，不照搬产品内部事件 |
| B044 | noOp | 原样透传数据 | 无操作（§7.4） | S1 |  |
| B045 | readBinaryFile | 读取单个文件 | 主机文件读写（§7.4） | S1 | 旧版，与 readWriteFile 合并 |
| B046 | readBinaryFiles | 读取多个文件 | 主机文件读写（§7.4） | S1 | 旧版，与 readWriteFile 合并 |
| B047 | readPDF | 提取 PDF 文本 | 文件提取（§18.3） | S2 | 旧版，与 extractFromFile 合并 |
| B048 | renameKeys | 重命名字段 | 重命名键（§7.4） | S1 |  |
| B049 | respondToWebhook | 指定 Webhook 响应 | 回应 Webhook（§18.3、§18.10） | S1 | SSE 流式扩展 S2，见 C 与 A 流式说明 |
| B050 | rssFeedRead | 读取 RSS 内容 | HTTP+解析/声明规范（§7.1、§15） | S5 | 长尾批次按需 |
| B051 | rssFeedReadTrigger | RSS 更新触发 | 轮询触发+持久去重（§18.3、§15） | S5 | 长尾批次按需 |
| B052 | scheduleTrigger | 按时间规则触发 | 定时触发（§5.2） | S1 | 派生 cron_jobs 行只读 |
| B053 | set | 设置/编辑条目字段 | 编辑字段（§7.4） | S1 |  |
| B054 | simulate | 模拟执行测试数据 | 不对标 | 不对标 | 上游模拟/测试专用；Ferry 用固定数据验收测试，不克隆此节点 |
| B055 | simulateTrigger | 模拟触发测试数据 | 不对标 | 不对标 | 上游模拟/测试专用；Ferry 用测试执行与固定数据 |
| B056 | splitInBatches | 分批循环处理条目 | 结构化循环容器（§6.6、§18.7） | S1 | 不是允许画有向环 |
| B057 | spreadsheetFile | 电子表格文件与条目互转 | 文件↔条目（§18.3） | S2 | 旧版，与 extractFromFile/convertToFile 合并 |
| B058 | sseTrigger | 接收 SSE 事件触发 | 扩展触发器（§18.3、§15） | S5 | SSE 客户端触发不同于 S2 服务端流式响应 |
| B059 | ssh | 远程主机执行/传输 | HostChannel 主机命令/文件节点（§6.5、§18.11） | S1 | 复用已管理主机凭据与授权 |
| B060 | stickyNote | 画布便签 | layout.notes（§6.1、§9.2） | S1 | 布局能力，无运行副作用 |
| B061 | stopAndError | 停止并抛错 | 停止并报错（§7.4） | S1 |  |
| B062 | switch | 多路条件分支 | Switch（§7.4） | S1 |  |
| B063 | timeSaved | 记录节省时间；节点操作细项待核实 | 执行统计/标注（§2.2、§18.3） | S1 | 不猜测上游内部计量算法 |
| B064 | totp | 生成基于时间的一次性口令 | 代码+加密数据能力（§7.4） | S1 | 算法/编码参数在交付时核实 |
| B065 | aggregate | 聚合条目 | Aggregate（§7.4） | S1 |  |
| B066 | limit | 限制条目数量 | Limit（§7.4） | S1 |  |
| B067 | removeDuplicates | 去除重复条目 | 去重（§7.4） | S1 | 跨执行去重状态细项需核实 |
| B068 | sort | 排序条目 | Sort（§7.4） | S1 |  |
| B069 | splitOut | 拆出列表元素 | Split（§7.4） | S1 |  |
| B070 | summarize | 汇总条目字段 | Aggregate/汇总转换（§7.4） | S1 | 这是数据汇总，不是 LLM 摘要 |
| B071 | wait | 按时间或回调等待 | Wait/表单挂起（§6.6、§18.7） | S1 | 释放槽位，恢复已完成输出 |
| B072 | webhook | HTTP 请求触发 | Webhook（§5.2、§18.10） | S1 | 必须实测并发满503、人工202、超时504 |
| B073 | WorkflowTrigger | 工作流事件触发；事件集待核实 | 执行/定义事件（§5.2、§18.16） | S1 | 保留源 ID 大小写 |
| B074 | writeBinaryFile | 写出二进制文件 | 主机文件读写（§7.4） | S1 | 旧版，与 readWriteFile 合并 |
| B075 | xml | XML 与条目转换 | XML 数据节点（§7.4） | S1 |  |

## C. n8n AI 节点（127 个）

成员完整取自 package.json 的 n8n.nodes；每行列出完整相对路径，按文件而非展示名去重。下列各族均在 S2 交付其业务能力（§7.4/§7.5），供应商专用操作和外部后端协议按 S5 批次按需补齐，不能以一个通用 LLM 调用宣称专用 API 全部完成。列出“作用待核实”的成员须在该批验收前补充行为证据。

服务商凭据复用 vault：llm.openai_compatible 配置 base_url、key、模型列表和用户单价；另有 llm.anthropic、llm.dashscope（§7.5）。OpenAI 标准对话/embeddings 由 llm.openai_compatible 承接。DeepSeek、Groq、Mistral Chat、MiniMax Chat、Moonshot Chat、Lemonade、NVIDIA Chat、OpenRouter、Vercel AI Gateway、xAI Chat、Ollama 在下表仅列为兼容适配候选：所给 package.json 只能证明节点存在，不能证明其端点协议；只有端点通过 OpenAI 协议实测后才计入覆盖。embeddings、工具、流式、结构化输出分别验证；原生或私有操作转 S5 服务商适配，不凭记忆认定兼容。Azure 的认证/API 版本、Bedrock 的签名、Google/Cohere 等原生协议不能仅改 base_url 即声称覆盖。

V10：向量库统一写入/检索接口，SQLite/MySQL 内置检索每集合 ≤2 万条，进程内缓存暴力检索；PostgreSQL 可选 pgvector。外部向量库成员映射同一能力接口但其原生后端适配单独列 S5，不能用内置库通过来验收全部外部连接器。MCP 按 §2.2 同时承接工作流作为工具服务端及 Agent 客户端。所有调用还需验收 §18.13 预算预留/结算、无单价拒绝、max_cost、凭据限流与最多20轮（默认10轮）。

### C.Agent/Chains（9 个）

Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C001 | dist/nodes/agents/Agent/Agent.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |
| C002 | dist/nodes/agents/Agent/AgentTool.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |
| C003 | dist/nodes/agents/OpenAiAssistant/OpenAiAssistant.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |
| C004 | dist/nodes/chains/ChainSummarization/ChainSummarization.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |
| C005 | dist/nodes/chains/ChainLLM/ChainLlm.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |
| C006 | dist/nodes/chains/ChainRetrievalQA/ChainRetrievalQa.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |
| C007 | dist/nodes/chains/SentimentAnalysis/SentimentAnalysis.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |
| C008 | dist/nodes/chains/InformationExtractor/InformationExtractor.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |
| C009 | dist/nodes/chains/TextClassifier/TextClassifier.node.js | Agent+节点/子工作流工具、分类、摘要、抽取与检索链（§7.4）；Assistant 用 Agent+记忆承接 | S2 |

### C.Chat models（25 个）

统一 LLM 对话及服务商凭据（§7.5）；标准兼容协议与原生协议分开验收。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C010 | dist/nodes/llms/LmChatAlibabaCloud/LmChatAlibabaCloud.node.js | llm.dashscope → LLM 对话（§7.5） | S2 |
| C011 | dist/nodes/llms/LMChatAnthropic/LmChatAnthropic.node.js | llm.anthropic → LLM 对话（§7.5） | S2 |
| C012 | dist/nodes/llms/LmChatAzureOpenAi/LmChatAzureOpenAi.node.js | 原生协议凭据适配器 → 统一 LLM 能力；原生供应商连接 S5 批次按需（§7.5、§15） | S5 |
| C013 | dist/nodes/llms/LmChatAwsBedrock/LmChatAwsBedrock.node.js | 原生协议凭据适配器 → 统一 LLM 能力；原生供应商连接 S5 批次按需（§7.5、§15） | S5 |
| C014 | dist/nodes/llms/LmChatCohere/LmChatCohere.node.js | 原生协议凭据适配器 → 统一 LLM 能力；原生供应商连接 S5 批次按需（§7.5、§15） | S5 |
| C015 | dist/nodes/llms/LmChatDatabricks/LmChatDatabricks.node.js | 原生协议凭据适配器 → 统一 LLM 能力；原生供应商连接 S5 批次按需（§7.5、§15） | S5 |
| C016 | dist/nodes/llms/LmChatDeepSeek/LmChatDeepSeek.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C017 | dist/nodes/llms/LmChatGoogleGemini/LmChatGoogleGemini.node.js | 原生协议凭据适配器 → 统一 LLM 能力；原生供应商连接 S5 批次按需（§7.5、§15） | S5 |
| C018 | dist/nodes/llms/LmChatGoogleVertex/LmChatGoogleVertex.node.js | 原生协议凭据适配器 → 统一 LLM 能力；原生供应商连接 S5 批次按需（§7.5、§15） | S5 |
| C019 | dist/nodes/llms/LmChatGroq/LmChatGroq.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C020 | dist/nodes/llms/LmChatMistralCloud/LmChatMistralCloud.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C021 | dist/nodes/llms/LmChatMinimax/LmChatMinimax.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C022 | dist/nodes/llms/LmChatMoonshot/LmChatMoonshot.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C023 | dist/nodes/llms/LMChatLemonade/LmChatLemonade.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C024 | dist/nodes/llms/LmChatNvidia/LmChatNvidia.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C025 | dist/nodes/llms/LMChatOllama/LmChatOllama.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C026 | dist/nodes/llms/LmChatOpenRouter/LmChatOpenRouter.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C027 | dist/nodes/llms/LmChatVercelAiGateway/LmChatVercelAiGateway.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C028 | dist/nodes/llms/LmChatXAiGrok/LmChatXAiGrok.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C029 | dist/nodes/llms/LMChatOpenAi/LmChatOpenAi.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C030 | dist/nodes/llms/LMOpenAi/LmOpenAi.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C031 | dist/nodes/llms/LMCohere/LmCohere.node.js | 原生协议凭据适配器 → 统一 LLM 能力；原生供应商连接 S5 批次按需（§7.5、§15） | S5 |
| C032 | dist/nodes/llms/LMLemonade/LmLemonade.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C033 | dist/nodes/llms/LMOllama/LmOllama.node.js | llm.openai_compatible → 标准对话/补全子集；端点差异需验证（§7.5） | S2 |
| C034 | dist/nodes/llms/LMOpenHuggingFaceInference/LmOpenHuggingFaceInference.node.js | 原生协议凭据适配器 → 统一 LLM 能力；原生供应商连接 S5 批次按需（§7.5、§15） | S5 |

### C.Embeddings（13 个）

文本嵌入+凭据适配器（§7.4/§7.5）；不把 chat 兼容等同 embeddings 兼容。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C035 | dist/nodes/embeddings/EmbeddingsCohere/EmbeddingsCohere.node.js | 统一嵌入接口；原生服务商/数据库 embedding 协议适配 S5（§7.5、§15） | S5 |
| C036 | dist/nodes/embeddings/EmbeddingsDatabricks/EmbeddingsDatabricks.node.js | 统一嵌入接口；原生服务商/数据库 embedding 协议适配 S5（§7.5、§15） | S5 |
| C037 | dist/nodes/embeddings/EmbeddingsAwsBedrock/EmbeddingsAwsBedrock.node.js | 统一嵌入接口；原生服务商/数据库 embedding 协议适配 S5（§7.5、§15） | S5 |
| C038 | dist/nodes/embeddings/EmbeddingsAzureOpenAi/EmbeddingsAzureOpenAi.node.js | 统一嵌入接口；原生服务商/数据库 embedding 协议适配 S5（§7.5、§15） | S5 |
| C039 | dist/nodes/embeddings/EmbeddingsGoogleGemini/EmbeddingsGoogleGemini.node.js | 统一嵌入接口；原生服务商/数据库 embedding 协议适配 S5（§7.5、§15） | S5 |
| C040 | dist/nodes/embeddings/EmbeddingsGoogleVertex/EmbeddingsGoogleVertex.node.js | 统一嵌入接口；原生服务商/数据库 embedding 协议适配 S5（§7.5、§15） | S5 |
| C041 | dist/nodes/embeddings/EmbeddingsHuggingFaceInference/EmbeddingsHuggingFaceInference.node.js | 统一嵌入接口；原生服务商/数据库 embedding 协议适配 S5（§7.5、§15） | S5 |
| C042 | dist/nodes/embeddings/EmbeddingsMistralCloud/EmbeddingsMistralCloud.node.js | llm.openai_compatible 的 embeddings 端点（须验证）；统一嵌入能力 §7.4 | S2 |
| C043 | dist/nodes/embeddings/EmbeddingsOpenAI/EmbeddingsOpenAi.node.js | llm.openai_compatible 的 embeddings 端点（须验证）；统一嵌入能力 §7.4 | S2 |
| C044 | dist/nodes/embeddings/EmbeddingsLemonade/EmbeddingsLemonade.node.js | llm.openai_compatible 的 embeddings 端点（须验证）；统一嵌入能力 §7.4 | S2 |
| C045 | dist/nodes/embeddings/EmbeddingsOllama/EmbeddingsOllama.node.js | llm.openai_compatible 的 embeddings 端点（须验证）；统一嵌入能力 §7.4 | S2 |
| C046 | dist/nodes/embeddings/EmbeddingsOracleDB/EmbeddingsOracleDb.node.js | 统一嵌入接口；原生服务商/数据库 embedding 协议适配 S5（§7.5、§15） | S5 |
| C047 | dist/nodes/embeddings/EmbeddingsNvidia/EmbeddingsNvidia.node.js | llm.openai_compatible 的 embeddings 端点（须验证）；统一嵌入能力 §7.4 | S2 |

### C.Memory（10 个）

Agent 会话记忆（§7.4），可落数据表；外部记忆后端单独适配。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C048 | dist/nodes/memory/MemoryBufferWindow/MemoryBufferWindow.node.js | Agent 会话记忆（§7.4），可落数据表；外部记忆后端单独适配 | S2 |
| C049 | dist/nodes/memory/MemoryLakebaseChat/MemoryLakebaseChat.node.js | 统一 Agent 记忆 S2；本外部记忆后端适配 S5 批次按需 | S5 |
| C050 | dist/nodes/memory/MemoryMotorhead/MemoryMotorhead.node.js | 统一 Agent 记忆 S2；本外部记忆后端适配 S5 批次按需 | S5 |
| C051 | dist/nodes/memory/MemoryPostgresChat/MemoryPostgresChat.node.js | Agent 会话记忆（§7.4），可落数据表；外部记忆后端单独适配 | S2 |
| C052 | dist/nodes/memory/MemoryMongoDbChat/MemoryMongoDbChat.node.js | Agent 会话记忆（§7.4），可落数据表；外部记忆后端单独适配 | S2 |
| C053 | dist/nodes/memory/MemoryRedisChat/MemoryRedisChat.node.js | Agent 会话记忆（§7.4），可落数据表；外部记忆后端单独适配 | S2 |
| C054 | dist/nodes/memory/MemoryManager/MemoryManager.node.js | Agent 会话记忆（§7.4），可落数据表；外部记忆后端单独适配 | S2 |
| C055 | dist/nodes/memory/MemoryChatRetriever/MemoryChatRetriever.node.js | Agent 会话记忆（§7.4），可落数据表；外部记忆后端单独适配 | S2 |
| C056 | dist/nodes/memory/MemoryXata/MemoryXata.node.js | 统一 Agent 记忆 S2；本外部记忆后端适配 S5 批次按需 | S5 |
| C057 | dist/nodes/memory/MemoryZep/MemoryZep.node.js | 统一 Agent 记忆 S2；本外部记忆后端适配 S5 批次按需 | S5 |

### C.Vector stores（25 个）

V10 统一写入/检索；内置/pgvector 为 S2，外部后端适配 S5。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C058 | dist/nodes/vector_store/VectorStoreAzureAISearch/VectorStoreAzureAISearch.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C059 | dist/nodes/vector_store/VectorStoreDatabricks/VectorStoreDatabricks.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C060 | dist/nodes/vector_store/VectorStoreInMemory/VectorStoreInMemory.node.js | V10 统一写入/检索；内置/pgvector 为 S2，外部后端适配 S5 | S2 |
| C061 | dist/nodes/vector_store/VectorStoreInMemoryInsert/VectorStoreInMemoryInsert.node.js | V10 统一写入/检索；内置/pgvector 为 S2，外部后端适配 S5 | S2 |
| C062 | dist/nodes/vector_store/VectorStoreInMemoryLoad/VectorStoreInMemoryLoad.node.js | V10 统一写入/检索；内置/pgvector 为 S2，外部后端适配 S5 | S2 |
| C063 | dist/nodes/vector_store/VectorStoreMilvus/VectorStoreMilvus.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C064 | dist/nodes/vector_store/VectorStoreMongoDBAtlas/VectorStoreMongoDBAtlas.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C065 | dist/nodes/vector_store/VectorStoreOracleDB/VectorStoreOracleDB.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C066 | dist/nodes/vector_store/VectorStorePGVector/VectorStorePGVector.node.js | V10 统一写入/检索；内置/pgvector 为 S2，外部后端适配 S5 | S2 |
| C067 | dist/nodes/vector_store/ChatHubVectorStorePGVector/ChatHubVectorStorePGVector.node.js | V10 统一写入/检索；内置/pgvector 为 S2，外部后端适配 S5 | S2 |
| C068 | dist/nodes/vector_store/ChatHubVectorStoreQdrant/ChatHubVectorStoreQdrant.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C069 | dist/nodes/vector_store/VectorStorePinecone/VectorStorePinecone.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C070 | dist/nodes/vector_store/ChatHubVectorStorePinecone/ChatHubVectorStorePinecone.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C071 | dist/nodes/vector_store/VectorStorePineconeInsert/VectorStorePineconeInsert.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C072 | dist/nodes/vector_store/VectorStorePineconeLoad/VectorStorePineconeLoad.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C073 | dist/nodes/vector_store/VectorStoreRedis/VectorStoreRedis.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C074 | dist/nodes/vector_store/VectorStoreQdrant/VectorStoreQdrant.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C075 | dist/nodes/vector_store/VectorStoreSupabase/VectorStoreSupabase.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C076 | dist/nodes/vector_store/VectorStoreSupabaseInsert/VectorStoreSupabaseInsert.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C077 | dist/nodes/vector_store/VectorStoreSupabaseLoad/VectorStoreSupabaseLoad.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C078 | dist/nodes/vector_store/VectorStoreWeaviate/VectorStoreWeaviate.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C079 | dist/nodes/vector_store/VectorStoreZep/VectorStoreZep.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C080 | dist/nodes/vector_store/VectorStoreZepInsert/VectorStoreZepInsert.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C081 | dist/nodes/vector_store/VectorStoreZepLoad/VectorStoreZepLoad.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |
| C082 | dist/nodes/vector_store/VectorStoreChromaDB/VectorStoreChromaDB.node.js | V10 写入/检索能力 S2；本成员外部后端原生连接器 S5 批次按需 | S5 |

### C.Retrievers（4 个）

检索链+子工作流+重排（§7.4、V10）。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C083 | dist/nodes/retrievers/RetrieverContextualCompression/RetrieverContextualCompression.node.js | 检索链+子工作流+重排（§7.4、V10） | S2 |
| C084 | dist/nodes/retrievers/RetrieverVectorStore/RetrieverVectorStore.node.js | 检索链+子工作流+重排（§7.4、V10） | S2 |
| C085 | dist/nodes/retrievers/RetrieverMultiQuery/RetrieverMultiQuery.node.js | 检索链+子工作流+重排（§7.4、V10） | S2 |
| C086 | dist/nodes/retrievers/RetrieverWorkflow/RetrieverWorkflow.node.js | 检索链+子工作流+重排（§7.4、V10） | S2 |

### C.Text splitters（3 个）

文本拆分/代码节点，作为嵌入前处理（§7.4）。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C087 | dist/nodes/text_splitters/TextSplitterCharacterTextSplitter/TextSplitterCharacterTextSplitter.node.js | 文本拆分/代码节点，作为嵌入前处理（§7.4） | S2 |
| C088 | dist/nodes/text_splitters/TextSplitterRecursiveCharacterTextSplitter/TextSplitterRecursiveCharacterTextSplitter.node.js | 文本拆分/代码节点，作为嵌入前处理（§7.4） | S2 |
| C089 | dist/nodes/text_splitters/TextSplitterTokenSplitter/TextSplitterTokenSplitter.node.js | 文本拆分/代码节点，作为嵌入前处理（§7.4） | S2 |

### C.Output parsers（3 个）

JSON Schema 结构化输出+条目列表+LLM修复（§7.4）。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C090 | dist/nodes/output_parser/OutputParserAutofixing/OutputParserAutofixing.node.js | JSON Schema 结构化输出+条目列表+LLM修复（§7.4） | S2 |
| C091 | dist/nodes/output_parser/OutputParserItemList/OutputParserItemList.node.js | JSON Schema 结构化输出+条目列表+LLM修复（§7.4） | S2 |
| C092 | dist/nodes/output_parser/OutputParserStructured/OutputParserStructured.node.js | JSON Schema 结构化输出+条目列表+LLM修复（§7.4） | S2 |

### C.Tools（10 个）

Agent 工具=节点/子工作流（§7.4）；外部搜索等服务由 HTTP/声明规范承接。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C093 | dist/nodes/tools/ToolCalculator/ToolCalculator.node.js | Agent 工具=节点/子工作流（§7.4）；外部搜索等服务由 HTTP/声明规范承接 | S2 |
| C094 | dist/nodes/tools/ToolCode/ToolCode.node.js | Agent 工具=节点/子工作流（§7.4）；外部搜索等服务由 HTTP/声明规范承接 | S2 |
| C095 | dist/nodes/tools/ToolHttpRequest/ToolHttpRequest.node.js | Agent 工具=节点/子工作流（§7.4）；外部搜索等服务由 HTTP/声明规范承接 | S2 |
| C096 | dist/nodes/tools/ToolSearXng/ToolSearXng.node.js | 工具调用框架 S2；本外部服务的 HTTP/声明规范适配 S5 批次按需（§7.1、§15） | S5 |
| C097 | dist/nodes/tools/ToolSerpApi/ToolSerpApi.node.js | 工具调用框架 S2；本外部服务的 HTTP/声明规范适配 S5 批次按需（§7.1、§15） | S5 |
| C098 | dist/nodes/tools/ToolThink/ToolThink.node.js | Agent 工具=节点/子工作流（§7.4）；外部搜索等服务由 HTTP/声明规范承接 | S2 |
| C099 | dist/nodes/tools/ToolVectorStore/ToolVectorStore.node.js | Agent 工具=节点/子工作流（§7.4）；外部搜索等服务由 HTTP/声明规范承接 | S2 |
| C100 | dist/nodes/tools/ToolWikipedia/ToolWikipedia.node.js | 工具调用框架 S2；本外部服务的 HTTP/声明规范适配 S5 批次按需（§7.1、§15） | S5 |
| C101 | dist/nodes/tools/ToolWolframAlpha/ToolWolframAlpha.node.js | 工具调用框架 S2；本外部服务的 HTTP/声明规范适配 S5 批次按需（§7.1、§15） | S5 |
| C102 | dist/nodes/tools/ToolWorkflow/ToolWorkflow.node.js | Agent 工具=节点/子工作流（§7.4）；外部搜索等服务由 HTTP/声明规范承接 | S2 |

### C.MCP（4 个）

客户端/工具调用/注册发现与工作流工具服务端（§2.2）。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C103 | dist/nodes/mcp/McpClient/McpClient.node.js | 客户端/工具调用/注册发现与工作流工具服务端（§2.2） | S2 |
| C104 | dist/nodes/mcp/McpClientTool/McpClientTool.node.js | 客户端/工具调用/注册发现与工作流工具服务端（§2.2） | S2 |
| C105 | dist/nodes/mcp/McpRegistryClientTool/McpRegistryClientTool.node.js | 客户端/工具调用/注册发现与工作流工具服务端（§2.2） | S2 |
| C106 | dist/nodes/mcp/McpTrigger/McpTrigger.node.js | 客户端/工具调用/注册发现与工作流工具服务端（§2.2） | S2 |

### C.Triggers（3 个）

聊天触发、会话回应及流式输出（§5.2、§18.3）。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C107 | dist/nodes/trigger/ManualChatTrigger/ManualChatTrigger.node.js | 聊天触发、会话回应及流式输出（§5.2、§18.3） | S2 |
| C108 | dist/nodes/trigger/ChatTrigger/ChatTrigger.node.js | 聊天触发、会话回应及流式输出（§5.2、§18.3） | S2 |
| C109 | dist/nodes/trigger/ChatTrigger/Chat.node.js | 聊天触发、会话回应及流式输出（§5.2、§18.3） | S2 |

### C.Others（18 个）

服务商原生操作、文档加载、代码、重排、模型选择与安全检查；按实际业务能力承接。

| 编号 | 成员文件（全部） | Ferry 对应能力 / 服务商方式 | 期 |
|---|---|---|---|
| C110 | dist/nodes/vendors/AlibabaCloud/AlibabaCloud.node.js | llm.dashscope；标准对话 S2，供应商专属操作逐项核实（§7.5） | S2 |
| C111 | dist/nodes/vendors/Anthropic/Anthropic.node.js | llm.anthropic；标准对话 S2，供应商专属操作逐项核实（§7.5） | S2 |
| C112 | dist/nodes/vendors/GoogleGemini/GoogleGemini.node.js | 原生供应商节点，作用待核实；凭据+声明规范/适配器 S5 批次按需（§7.1、§7.5、§15） | S5 |
| C113 | dist/nodes/vendors/MiniMax/MiniMax.node.js | llm.openai_compatible 标准接口 S2；非兼容的供应商专属操作 S5 批次按需 | S2 |
| C114 | dist/nodes/vendors/Moonshot/Moonshot.node.js | llm.openai_compatible 标准接口 S2；非兼容的供应商专属操作 S5 批次按需 | S2 |
| C115 | dist/nodes/vendors/Ollama/Ollama.node.js | llm.openai_compatible 标准接口 S2；非兼容的供应商专属操作 S5 批次按需 | S2 |
| C116 | dist/nodes/vendors/OpenAi/OpenAi.node.js | llm.openai_compatible 标准接口 S2；非兼容的供应商专属操作 S5 批次按需 | S2 |
| C117 | dist/nodes/vendors/Microsoft/MicrosoftAgent365Trigger.node.js | 原生供应商节点，作用待核实；凭据+声明规范/适配器 S5 批次按需（§7.1、§7.5、§15） | S5 |
| C118 | dist/nodes/vendors/Databricks/DatabricksAiAgent.node.js | 原生供应商节点，作用待核实；凭据+声明规范/适配器 S5 批次按需（§7.1、§7.5、§15） | S5 |
| C119 | dist/nodes/code/Code.node.js | 代码作为 AI 扩展工具；JS 沙箱/Python 主机（§7.2），AI 工具接入 S2，不承诺 LangChain/npm | S2 |
| C120 | dist/nodes/document_loaders/DocumentDefaultDataLoader/DocumentDefaultDataLoader.node.js | 文件/JSON/条目/代码仓库加载→文本条目（§18.3、§7.4） | S2 |
| C121 | dist/nodes/document_loaders/DocumentBinaryInputLoader/DocumentBinaryInputLoader.node.js | 文件/JSON/条目/代码仓库加载→文本条目（§18.3、§7.4） | S2 |
| C122 | dist/nodes/document_loaders/DocumentGithubLoader/DocumentGithubLoader.node.js | 文件/JSON/条目/代码仓库加载→文本条目（§18.3、§7.4） | S2 |
| C123 | dist/nodes/document_loaders/DocumentJSONInputLoader/DocumentJsonInputLoader.node.js | 文件/JSON/条目/代码仓库加载→文本条目（§18.3、§7.4） | S2 |
| C124 | dist/nodes/rerankers/RerankerCohere/RerankerCohere.node.js | 检索后重排；Cohere 原生后端适配 S5（§7.5、§15） | S5 |
| C125 | dist/nodes/ToolExecutor/ToolExecutor.node.js | Agent 工具执行器（§7.4） | S2 |
| C126 | dist/nodes/ModelSelector/ModelSelector.node.js | 按条件选择凭据可用模型（§7.5、§7.4） | S2 |
| C127 | dist/nodes/Guardrails/Guardrails.node.js | 规则校验/LLM 评分+If（§7.4、§18.3）；作用细项待核实 | S2 |

## D. n8n 集成节点（363 个）

分类与名称完全沿用 n8n-integrations.md（每节点只列一次；不再按多分类标签重复计数）。接入按 §7.1、V12：声明式节点规范分批交付，复杂签名/分页/二进制由 §18.3 WASM SDK 或主机脚本补齐。HTTP+凭据模板是长尾过渡途径，不代表有对应产品节点或覆盖全部操作。每个节点的作用/操作清单由其交付批次核实，未核实的不凭名称补写。

第一批 = §7.4 S2 行：钉钉、飞书、企业微信、邮件、MySQL、PostgreSQL、Redis、MongoDB、S3/OSS、Kafka（生产）、GitHub、GitLab、Gitee、Codeup、Slack、Telegram，另含 v3 明列的 SQLite 与 Ferry 自身。钉钉、飞书、企业微信、Codeup 是 Ferry 新增的中国生态集成，本 363 名单中没有这些节点；Gitee/SQLite/Ferry 自身同样不在此集成附录中，不计入 n8n 节点基数。邮件 SMTP/IMAP 已在 B 中；不把 Gmail/Outlook 等专用 API 误列邮件第一批。Kafka 消费触发没有被 v3 的“生产”行承诺，列 S5。

| 类别 | 数量 | 代表节点 | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| Communication | 85 | awsSes、ciscoWebex、ciscoWebexTrigger | 声明式规范批次+WASM/主机扩展；第一批成员：redisTrigger、slack、slackTrigger、telegram、telegramTrigger | S2（5）；S5（80） |
| Development | 74 | amqp、amqpTrigger、awsCertificateManager | 声明式规范批次+WASM/主机扩展；第一批成员：awsS3、github、githubTrigger、gitlab、gitlabTrigger、kafka、mongoDb、mySql、postgres、postgresTrigger、redis、s3 | S2（12）；S5（62） |
| Data & Storage | 41 | adalo、airtable、airtableTrigger | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| Productivity | 38 | acuitySchedulingTrigger、airtop、asana | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| Marketing | 36 | activeCampaign、activeCampaignTrigger、agileCrm | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| Sales | 31 | actionNetwork、affinity、affinityTrigger | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| Miscellaneous | 16 | bambooHr、dhl、figmaTrigger | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| Utility | 14 | awsTextract、bitly、Brandfetch | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| Finance & Accounting | 13 | chargebee、chargebeeTrigger、erpNext | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| Analytics | 11 | googleAds、googleAnalytics、googleCloudNaturalLanguage | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| AI | 2 | messageAnAgent、perplexity | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |
| ECM | 2 | microsoftSharePoint、microsoftSharePointTrigger | 声明式规范批次+WASM/主机扩展；第一批成员：无；全为长尾 | S5 |

### D. 完整分类附录

#### Communication（85 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D001 | awsSes | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D002 | ciscoWebex | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D003 | ciscoWebexTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D004 | customerIo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D005 | customerIoTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D006 | demio | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D007 | discord | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D008 | discourse | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D009 | disqus | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D010 | egoi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D011 | emelia | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D012 | emeliaTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D013 | formIoTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D014 | formstackTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D015 | freshdesk | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D016 | getResponse | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D017 | getResponseTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D018 | gmail | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D019 | gmailTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D020 | googleBusinessProfileTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D021 | googleChat | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D022 | gotify | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D023 | goToWebinar | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D024 | hackerNews | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D025 | haloPSA | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D026 | helpScout | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D027 | helpScoutTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D028 | intercom | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D029 | iterable | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D030 | jotFormTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D031 | koBoToolbox | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D032 | koBoToolboxTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D033 | lemlist | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D034 | lemlistTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D035 | line | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D036 | mailerLite | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D037 | mailerLiteTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D038 | mailgun | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D039 | mailjet | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D040 | mailjetTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D041 | mandrill | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D042 | matrix | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D043 | mattermost | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D044 | messageBird | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D045 | microsoftOutlook | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D046 | microsoftOutlookTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D047 | microsoftTeams | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D048 | microsoftTeamsTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D049 | mocean | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D050 | monicaCrm | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D051 | mqttTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D052 | msg91 | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D053 | pagerDuty | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D054 | plivo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D055 | postmarkTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D056 | pushbullet | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D057 | pushcut | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D058 | pushcutTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D059 | pushover | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D060 | reddit | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D061 | redisTrigger | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D062 | rocketchat | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D063 | rundeck | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D064 | sendy | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D065 | signl4 | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D066 | slack | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D067 | slackTrigger | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D068 | sms77 | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D069 | telegram | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D070 | telegramTrigger | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D071 | twilio | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D072 | twilioTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D073 | twist | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D074 | typeformTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D075 | vero | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D076 | vonage | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D077 | whatsApp | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D078 | whatsAppTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D079 | wufooTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D080 | zammad | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D081 | zendesk | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D082 | zendeskTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D083 | zohoCrm | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D084 | zoom | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D085 | zulip | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Development（74 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D086 | amqp | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D087 | amqpTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D088 | awsCertificateManager | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D089 | awsCognito | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D090 | awsComprehend | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D091 | awsElb | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D092 | awsiam | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D093 | awsLambda | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D094 | awsRekognition | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D095 | awsS3 | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D096 | awsSns | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D097 | awsSnsTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D098 | awsSqs | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D099 | awsTranscribe | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D100 | bitbucketTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D101 | bubble | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D102 | circleCi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D103 | cloudflare | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D104 | cortex | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D105 | crateDb | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D106 | debughelper | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D107 | elasticsearch | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D108 | elasticSecurity | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D109 | facebookGraphApi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D110 | filemaker | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D111 | github | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D112 | githubTrigger | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D113 | gitlab | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D114 | gitlabTrigger | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D115 | gong | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D116 | googleCloudStorage | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D117 | grafana | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D118 | jenkins | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D119 | jira | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D120 | jiraTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D121 | jwt | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D122 | kafka | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D123 | kafkaTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D124 | ldap | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D125 | metabase | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D126 | microsoftEntra | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D127 | microsoftGraphSecurity | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D128 | microsoftSql | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D129 | misp | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D130 | mongoDb | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D131 | mqtt | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D132 | mySql | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D133 | netlify | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D134 | netlifyTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D135 | netscalerAdc | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D136 | npm | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D137 | okta | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D138 | peekalink | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D139 | postbin | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D140 | postgres | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D141 | postgresTrigger | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D142 | rabbitmq | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D143 | rabbitmqTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D144 | redis | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D145 | s3 | 第一批：§7.4 对应服务，声明式/原生连接器；动作与触发分别演示 | S2 |
| D146 | sentryIo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D147 | splunk | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D148 | taiga | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D149 | taigaTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D150 | theHive | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D151 | theHiveProject | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D152 | theHiveProjectTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D153 | theHiveTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D154 | travisCi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D155 | uptimeRobot | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D156 | urlScanIo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D157 | venafiTlsProtectCloud | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D158 | venafiTlsProtectCloudTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D159 | venafiTlsProtectDatacenter | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Data & Storage（41 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D160 | adalo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D161 | airtable | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D162 | airtableTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D163 | awsDynamoDb | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D164 | azureCosmosDb | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D165 | azureStorage | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D166 | baserow | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D167 | bitwarden | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D168 | box | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D169 | boxTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D170 | databricks | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D171 | databricksTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D172 | dropbox | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D173 | googleBigQuery | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D174 | googleDrive | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D175 | googleDriveTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D176 | googleFirebaseCloudFirestore | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D177 | googleFirebaseRealtimeDatabase | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D178 | googleSheets | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D179 | googleSheetsTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D180 | graphql | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D181 | grist | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D182 | microsoftDataverse | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D183 | microsoftExcel | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D184 | microsoftExcelSharePoint | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D185 | microsoftOneDrive | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D186 | microsoftOneDriveTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D187 | nextCloud | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D188 | nocoDb | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D189 | odoo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D190 | questDb | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D191 | quickbase | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D192 | seaTable | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D193 | seaTableTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D194 | snowflake | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D195 | stackby | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D196 | storyblok | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D197 | strapi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D198 | supabase | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D199 | timescaleDb | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D200 | uproc | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Productivity（38 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D201 | acuitySchedulingTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D202 | airtop | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D203 | asana | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D204 | asanaTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D205 | beeminder | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D206 | calendlyTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D207 | calTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D208 | clickUp | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D209 | clickUpTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D210 | clockify | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D211 | coda | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D212 | coinGecko | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D213 | confluence | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D214 | flow | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D215 | flowTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D216 | freshservice | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D217 | googleCalendar | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D218 | googleCalendarTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D219 | googleTasks | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D220 | harvest | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D221 | linear | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D222 | linearTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D223 | microsoftToDo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D224 | mondayCom | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D225 | notion | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D226 | notionTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D227 | oura | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D228 | raindrop | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D229 | serviceNow | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D230 | strava | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D231 | stravaTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D232 | syncroMsp | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D233 | todoist | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D234 | togglTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D235 | trello | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D236 | trelloTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D237 | twake | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D238 | wekan | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Marketing（36 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D239 | activeCampaign | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D240 | activeCampaignTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D241 | agileCrm | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D242 | apiTemplateIo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D243 | autopilot | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D244 | autopilotTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D245 | bannerbear | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D246 | brevo | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D247 | brevoTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D248 | cockpit | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D249 | contentful | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D250 | convertKit | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D251 | convertKitTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D252 | copper | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D253 | facebookLeadAdsTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D254 | facebookTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D255 | freshworksCrm | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D256 | ghost | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D257 | googleBusinessProfile | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D258 | googleSlides | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D259 | highLevel | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D260 | linkedIn | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D261 | mailchimp | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D262 | mailchimpTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D263 | mautic | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D264 | mauticTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D265 | medium | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D266 | microsoftDynamicsCrm | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D267 | quickChart | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D268 | sendGrid | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D269 | surveyMonkeyTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D270 | twitter | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D271 | webflow | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D272 | webflowTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D273 | wordpress | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D274 | youTube | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Sales（31 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D275 | actionNetwork | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D276 | affinity | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D277 | affinityTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D278 | clearbit | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D279 | copperTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D280 | drift | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D281 | dropcontact | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D282 | eventbriteTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D283 | gumroadTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D284 | hubspot | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D285 | hubspotTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D286 | hunter | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D287 | keap | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D288 | keapTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D289 | lonescale | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D290 | lonescaleTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D291 | magento2 | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D292 | paddle | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D293 | phantombuster | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D294 | pipedrive | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D295 | pipedriveTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D296 | salesforce | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D297 | salesforceTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D298 | salesmate | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D299 | shopify | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D300 | shopifyTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D301 | tapfiliate | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D302 | unleashedSoftware | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D303 | uplead | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D304 | wooCommerce | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D305 | wooCommerceTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Miscellaneous（16 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D306 | bambooHr | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D307 | dhl | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D308 | figmaTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D309 | googleBooks | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D310 | googleContacts | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D311 | googleDocs | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D312 | homeAssistant | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D313 | jinaAi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D314 | lingvaNex | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D315 | nasa | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D316 | onfleet | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D317 | onfleetTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D318 | openWeatherMap | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D319 | philipsHue | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D320 | spotify | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D321 | workableTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Utility（14 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D322 | awsTextract | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D323 | bitly | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D324 | Brandfetch | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D325 | clockifyTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D326 | deepL | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D327 | googleTranslate | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D328 | gSuiteAdmin | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D329 | mailcheck | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D330 | mindee | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D331 | mistralAi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D332 | oneSimpleApi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D333 | openAi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D334 | openThesaurus | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D335 | yourls | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Finance & Accounting（13 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D336 | chargebee | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D337 | chargebeeTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D338 | erpNext | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D339 | invoiceNinja | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D340 | invoiceNinjaTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D341 | payPal | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D342 | payPalTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D343 | quickbooks | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D344 | stripe | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D345 | stripeTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D346 | wise | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D347 | wiseTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D348 | xero | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### Analytics（11 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D349 | googleAds | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D350 | googleAnalytics | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D351 | googleCloudNaturalLanguage | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D352 | googlePerspective | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D353 | humanticAi | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D354 | marketstack | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D355 | orbit | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D356 | postHog | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D357 | profitWell | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D358 | securityScorecard | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D359 | segment | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### AI（2 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D360 | messageAnAgent | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D361 | perplexity | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

#### ECM（2 个）

| 编号 | 节点（原名） | 批次与承接方式 | 期 |
|---|---|---|---|
| D362 | microsoftSharePoint | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |
| D363 | microsoftSharePointTrigger | S5 批次按需：声明式规范；超出规范的用 §18.3 WASM SDK/主机脚本 | S5 |

## E. ComfyUI 平台能力（非节点）

实测依据为对标清单 §2、v3 §2.3、§9.2、§18.4；没有独立截图的行为仅按文字记录列入，不假称补测过。不同阶段的基础与扩展分行，保证各期可验收。

| 编号 | 能力 | ComfyUI 实际表现（依据） | Ferry 承接（v3 章节） | 期 | 证据截图（本机） |
|---|---|---|---|---|---|
| E001 | 画布、选择与编辑 | 实测：节点图、多选工具箱、右键、移动、缩放 | §9.1–§9.2 | S1 | comfyui/05-default-sd-workflow.png；comfyui/36-multi-select-toolbox.png；comfyui/45-node-context-menu.png |
| E002 | 搜索与分类库 | 实测：双击搜索、输入/输出 IMAGE 类型筛选 | §6.3、§9.2 | S1 | comfyui/10-node-search-dblclick.png；comfyui/11b-search-filtered-IMAGE.png |
| E003 | 端口兼容与连线校验 | 实测：只高亮兼容端口；不兼容连线不能接上；松手空白按类型搜索 | §6.3；Ferry 额外显示不兼容原因 | S1 | comfyui/13-link-incompatible-drag.png；comfyui/14-link-release-empty.png |
| E004 | 媒体端口类型 | v3 记载：图像/视频/音频端口与媒体展示 | §6.3、§15 | S3 | comfyui/11b-search-filtered-IMAGE.png |
| E005 | 中转点 reroute | v3 §2.3 记载；没有单独截图 | §6.1 layout.reroutes、§9.2 | S1 | 未有专门截图；依据文字记录，待补测 |
| E006 | 组与移动组内容 | 实测：组移动包含节点 | §6.1、§9.2 | S1 | comfyui/39-group-moved-with-contents.png |
| E007 | 子图、端口、面包屑 | 实测：创建子图，进入子图；Ferry 用保存时内联组件 | §9.2、§18.4 | S3 | comfyui/48-subgraph-created.png；comfyui/49-inside-subgraph.png |
| E008 | 可复用子图蓝图与版本 | v3 §18.4 补充；组件内部可含 gpu:*，保存内联 | §18.4；组件库 | S3 | comfyui/48-subgraph-created.png |
| E009 | 组件批量升级 | v3 §18.2 组件库按版本引用、先显示影响范围再升级 | §18.2 | S5 | 未有专门截图；依据文字记录，待补测 |
| E010 | bypass 与 mute | 实测：透传/跳过语义不同、颜色不同 | §6.1、§6.6、§18.7 | S1 | comfyui/42-run-with-bypass.png |
| E011 | 固定、折叠、颜色 | v3 §2.3/§9.2 记载菜单；不把 UI 固定等同 n8n 固定输出数据 | §9.2 | S1 | comfyui/45-node-context-menu.png |
| E012 | 运行与更改时运行 | 记录：普通运行、参数变动后自动运行；Ferry 自动试运行限纯/缓存友好图 | §2.3、§8.5 | S3 | comfyui/16-run-progress-5.png |
| E013 | instant 模式 | 运行模式入口/文字记载，独立触发时机作用待核实；不声称已验证语义 | §2.3、§8.5 自动试运行；补测后核对模式差异 | S3 | 无独立截图 |
| E014 | 批次数与批量入队 | 实测记录/v3 §2.3：可设置批次数 | §2.3、§15 S3 执行交互 | S3 | comfyui/19-queue-panel.png |
| E015 | 插队到队首 | v3 §2.3 记载；具体权限与优先级在交付时演示 | §2.3、§5.3 | S3 | comfyui/19-queue-panel.png |
| E016 | 节点与整体进度、标题进度 | 实测：执行节点绿框、进度、标题百分比 | §2.3、§9.2、§12.2 SSE | S1 | comfyui/16-run-progress-5.png；comfyui/22-slow-run-1.png |
| E017 | 采样实时预览 | v3 §2.3 记载；本地中间帧不写持久日志 | §8.2、§8.5 | S4 | comfyui/16-run-progress-5.png |
| E018 | 中断 | 实测：节点边界生效且标失败；Ferry 单独标 cancelled | §6.7、§8.2 | S1 | comfyui/30-after-interrupt.png |
| E019 | 队列与排队项取消 | 实测：运行/排队/失败、悬停取消 | §5.7、§9.4 | S1 | comfyui/19-queue-panel.png；comfyui/20-queue-item-hover.png；comfyui/24b-pending-row-hover.png；comfyui/73-failed-job-hover.png |
| E020 | 缓存框架与手动清除 | 实测：未变6节点0.00s，改参只重算下游，无命中标记；Ferry 显式⚡及来源 | §6.6、§18.4、§18.9 | S1 | 无专门截图；对标清单 §2 第12条 |
| E021 | 媒体按输入签名缓存 | 同上；增加模型版本、资产哈希、凭据非敏感配置、workspace、重做轮次 | §15 S3、§18.9 | S3 | 无专门截图；对标清单 §2 第12条 |
| E022 | 执行到选定输出 | 实测：只算目标的上游闭包 | §6.6、§9.2 | S1 | comfyui/45-node-context-menu.png |
| E023 | 问题面板、定位、修复 | 实测：发现3个问题、红框、警告按钮、按类型分组 | §9.2、§18.4；连接与凭据校验 | S1 | comfyui/05-default-sd-workflow.png；comfyui/08c-issues-after-run.png；comfyui/09-locate-missing-node.png；comfyui/32-validation-error-run.png |
| E024 | 缺模型检查与下载 | 实测：缺模型详情；Ferry 检查目标主机模型就绪，提供下载 | §8.3 | S4 | comfyui/06-missing-models-details.png |
| E025 | 错误报告、底部日志 | 实测：Markdown 报告可复制、底部控制台 | §18.4、§9.2 | S1 | comfyui/35-runtime-error-details.png；comfyui/63-bottom-panel-logs.png |
| E026 | 资产面板、媒体预览 | 实测：资产列表；Ferry 筛选、下载、来源、插入工作流 | §5.5、§9.4；S0 基础库，S3 媒体验收 | S3 | comfyui/27-assets-panel.png |
| E027 | 从历史恢复精确参数 | 实测：历史项在新标签页读取当时工作流 | §9.3；以本次定义打开 | S3 | comfyui/21-reload-workflow-from-history.png |
| E028 | 模型库与主机模型同步 | v3 §2.3 记载按目录分类、缺失模型下载 | §8.3、§9.4；许可证门禁 | S4 | comfyui/06-missing-models-details.png |
| E029 | 模板、工作流侧栏与导入导出 | 实测记录：652模板；侧栏可见；Ferry 自有图模板不兼容 JSON | §5.1、§9.1；场景模板 S5 另验收 | S1 | comfyui/58-workflows-sidebar.png |
| E030 | 四类场景官方模板 | v3 §15 规定的最终场景回归，不把上游652模板视作 Ferry 已有 | §10、§15 | S5 | comfyui/58-workflows-sidebar.png |
| E031 | 设置、快捷键 | 实测：设置/About；键位细项未单项核验 | §9.1、§9.2 | S1 | comfyui/62-settings-about.png；comfyui/45-node-context-menu.png |
| E032 | 应用模式 | v3 §2.3 记载：简化表单界面；无独立截图 | §9.5–§9.6、V9/V15 | S3 | 未有专门截图；依据文字记录，待补测 |
| E033 | 遮罩编辑器 | v3 §18.4 补齐：绘制并保存新遮罩资产；无专门截图 | §18.4 | S3 | 未有专门截图；依据文字记录，待补测 |
| E034 | 图像上传、粘贴与资产选择 | v3 §18.4 补齐媒体参数控件；无专门截图 | §18.4 | S3 | 未有专门截图；依据文字记录，待补测 |
| E035 | 任意控件转输入端口 | 实测：widget-to-input，含 GPU 类型的组件参数 | §18.4 | S3 | comfyui/71-widget-to-input-link.png |
| E036 | 种子控制 | v3 §18.4：固定、递增、执行后随机；Ferry 执行开始时确定种子并写记录 | §18.4、§18.7、§18.9 | S3 | 未有专门截图；依据文字记录，待补测 |
| E037 | 分块 VAE、低显存模式 | v3 §18.4 补齐 worker 运行选项 | §8.2、§18.4 | S4 | 未有专门截图；依据文字记录，待补测 |
| E038 | 3D 加载、预览与生成 | v3 §18.4：model3d 类型、GLB 预览、云端/本地生成 | §18.4 | S5 | 未有专门截图；依据文字记录，待补测 |

## F. ComfyUI 核心节点（691 个，145 组）

组名、数量与成员逐项抄录 comfy-nodes.md。混合组拆成子行（仍共145组）；子行数量相加恢复原组数量。每个子行的成员全部列出，按其本行业务能力逐个验收。代码格式保留原始 ID，包括 replacements 中带尾空格的 ConditioningAverage 。

承接规则：通用图像处理以 S3 服务端 image 节点提供等效操作；模型加载、文本编码、采样、潜空间、条件、LoRA、ControlNet、放大等 S4 ferry-infer。hunyuan、wan、LTXV、flux、sd3、stable_cascade、cosmos 等是模型族专属：依 V6 在 S4 以许可证为第一条件选择首批图像/视频/TTS 族，再逐族扩展；表列 S4 表示本地推理承接面，不代表首批克隆每个节点或支持全部模型族。选型未覆盖的成员必须保留未开始，不能以另一模型族演示通过。音频广泛处理/生成与3D扩充列 S5，S3 云端 TTS/音乐/ASR、S4 首批本地 TTS 不因此后移（见 §7.4、V6）。

训练与模型合并分开验收：模型合并、保存 checkpoint 为 §18.4 明列的 S4 本地节点。训练目录 S4 只承接主机脚本编排、数据集输入、进度/损失资产和 LoRA 保存（§7.4 主机能力、§8.2、V6 的许可证审核），不擅自承诺另建完整训练平台；具体训练库/配方在 S4 核实。GPU 内部类型只在同一分段内，组件保存时内联，不能用普通跨工作流传张量替代。

### F001. nodes.py(core)（65 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F001.1 nodes.py(core) | 54 | `KSampler`、`CheckpointLoaderSimple`、`CLIPTextEncode`、`CLIPSetLastLayer`、`VAEDecode`、`VAEEncode`、`VAEEncodeForInpaint`、`VAELoader`、`EmptyLatentImage`、`LatentUpscale`、`LatentUpscaleBy`、`LatentFromBatch`、`RepeatLatentBatch`、`ConditioningAverage`、`ConditioningCombine`、`ConditioningConcat`、`ConditioningMultiply`、`ConditioningSetArea`、`ConditioningSetAreaPercentage`、`ConditioningSetAreaStrength`、`ConditioningSetMask`、`KSamplerAdvanced`、`SetLatentNoiseMask`、`LatentComposite`、`LatentBlend`、`LatentRotate`、`LatentFlip`、`LatentCrop`、`LoraLoader`、`CLIPLoader`、`UNETLoader`、`DualCLIPLoader`、`CLIPVisionEncode`、`StyleModelApply`、`unCLIPConditioning`、`ControlNetApply`、`ControlNetApplyAdvanced`、`ControlNetLoader`、`DiffControlNetLoader`、`StyleModelLoader`、`CLIPVisionLoader`、`VAEDecodeTiled`、`VAEEncodeTiled`、`unCLIPCheckpointLoader`、`GLIGENLoader`、`GLIGENTextBoxApply`、`InpaintModelConditioning`、`CheckpointLoader`、`DiffusersLoader`、`LoadLatent`、`SaveLatent`、`ConditioningZeroOut`、`ConditioningSetTimestepRange`、`LoraLoaderModelOnly` | 采样与潜空间、模型加载、条件、LoRA、ControlNet/VAE；ferry-infer（§8.2–§8.3），具体模型族按V6选型 | S4 |
| F001.2 nodes.py(core) | 11 | `SaveImage`、`PreviewImage`、`LoadImage`、`LoadImageMask`、`LoadImageOutput`、`ImageScale`、`ImageScaleBy`、`ImageInvert`、`ImageBatch`、`ImagePadForOutpaint`、`EmptyImage` | 通用图像输入/保存/预览/缩放等；服务端 image+资产节点（§7.4、§18.3–§18.4） | S3 |

### F002. ace（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F002 ace | 5 | `EmptyAceStep1.5LatentAudio`、`EmptyAceStepLatentAudio`、`ReferenceTimbreAudio`、`TextEncodeAceStepAudio`、`TextEncodeAceStepAudio1.5` | 音频扩充：文件/声道/处理/生成或音频编码模型；§7.4、§15。模型族 ace 按V6许可证审核；不是首批TTS全覆盖承诺 | S5 |

### F003. advanced_samplers（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F003 advanced_samplers | 3 | `SamplerEulerCFGpp`、`SamplerLCM`、`SamplerLCMUpscale` | 本地模型/采样/条件/推理优化扩展：advanced_samplers；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F004. align_your_steps（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F004 align_your_steps | 1 | `AlignYourStepsScheduler` | 本地模型/采样/条件/推理优化扩展：align_your_steps；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F005. apg（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F005 apg | 1 | `APG` | 本地模型/采样/条件/推理优化扩展：apg；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F006. ar_video（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F006 ar_video | 3 | `ARVideoI2V`、`EmptyARVideoLatent`、`SamplerARVideo` | 视频模型族/采样或时序控制：ar_video；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F007. attention_multiply（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F007 attention_multiply | 4 | `CLIPAttentionMultiply`、`UNetCrossAttentionMultiply`、`UNetSelfAttentionMultiply`、`UNetTemporalAttentionMultiply` | 本地模型/采样/条件/推理优化扩展：attention_multiply；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F008. audio（21 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F008 audio | 21 | `AudioAdjustVolume`、`AudioConcat`、`AudioEqualizer3Band`、`AudioMerge`、`ConditioningStableAudio`、`EmptyAudio`、`EmptyLatentAudio`、`GetAudioLatentSize`、`JoinAudioChannels`、`LoadAudio`、`PreviewAudio`、`RecordAudio`、`SaveAudio`、`SaveAudioAdvanced`、`SaveAudioMP3`、`SaveAudioOpus`、`SplitAudioChannels`、`TrimAudioDuration`、`VAEDecodeAudio`、`VAEDecodeAudioTiled`、`VAEEncodeAudio` | 音频扩充：文件/声道/处理/生成或音频编码模型；§7.4、§15。模型族 audio 按V6许可证审核；不是首批TTS全覆盖承诺 | S5 |

### F009. audio_encoder（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F009 audio_encoder | 3 | `AudioEncoderEncode`、`AudioEncoderLoader`、`SheetSage2AudioToABC` | 音频扩充：文件/声道/处理/生成或音频编码模型；§7.4、§15。模型族 audio_encoder 按V6许可证审核；不是首批TTS全覆盖承诺 | S5 |

### F010. bernini（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F010 bernini | 1 | `BerniniConditioning` | 模型族专属：bernini；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F011. bg_removal（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F011 bg_removal | 2 | `LoadBackgroundRemovalModel`、`RemoveBackground` | 本地模型/采样/条件/推理优化扩展：bg_removal；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F012. boogu（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F012 boogu | 1 | `TextEncodeBooguEdit` | 模型族专属：boogu；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F013. bounding_boxes（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F013 bounding_boxes | 1 | `CreateBoundingBoxes` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F014. camera（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F014 camera | 1 | `CreateCameraInfo` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 camera 的模型与依赖许可证另审，细项待核实 | S5 |

### F015. camera_angle（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F015 camera_angle | 1 | `CameraAngle` | 视频模型族/采样或时序控制：camera_angle；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F016. camera_trajectory（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F016 camera_trajectory | 1 | `WanCameraEmbedding` | 视频模型族/采样或时序控制：camera_trajectory；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F017. canny（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F017 canny | 1 | `Canny` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F018. cfg（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F018 cfg | 2 | `CFGNorm`、`CFGZeroStar` | 本地模型/采样/条件/推理优化扩展：cfg；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F019. chroma_radiance（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F019 chroma_radiance | 2 | `ChromaRadianceOptions`、`EmptyChromaRadianceLatentImage` | 模型族专属：chroma_radiance；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F020. clip_sdxl（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F020 clip_sdxl | 2 | `CLIPTextEncodeSDXL`、`CLIPTextEncodeSDXLRefiner` | 模型族专属：clip_sdxl；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F021. color（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F021 color | 1 | `ColorToRGBInt` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F022. compositing（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F022 compositing | 3 | `JoinImageWithAlpha`、`PorterDuffImageComposite`、`SplitImageWithAlpha` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F023. compositor（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F023 compositor | 3 | `AddLayer`、`ImageCompositor`、`LayersFromBoundingBoxes` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F024. cond（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F024 cond | 4 | `CLIPTextEncodeControlnet`、`ConditioningLoader`、`SaveConditioning`、`T5TokenizerOptions` | 本地模型/采样/条件/推理优化扩展：cond；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F025. context_windows（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F025 context_windows | 3 | `ContextWindowsManual`、`LTXVContextWindows`、`WanContextWindowsManual` | 视频模型族/采样或时序控制：context_windows；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F026. controlnet（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F026 controlnet | 2 | `ControlNetInpaintingAliMamaApply`、`SetUnionControlNetType` | 本地模型/采样/条件/推理优化扩展：controlnet；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F027. cosmos（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F027 cosmos | 3 | `CosmosImageToVideoLatent`、`CosmosPredict2ImageToVideoLatent`、`EmptyCosmosLatentVideo` | 视频模型族/采样或时序控制：cosmos；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F028. curve（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F028 curve | 2 | `CurveEditor`、`ImageHistogram` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F029. custom_sampler（37 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F029 custom_sampler | 37 | `AddNoise`、`BasicGuider`、`BasicScheduler`、`BetaSamplingScheduler`、`CFGGuider`、`CFGOverride`、`DisableNoise`、`DualCFGGuider`、`DualModelGuider`、`ExponentialScheduler`、`ExtendIntermediateSigmas`、`FlipSigmas`、`KSamplerSelect`、`KarrasScheduler`、`LaplaceScheduler`、`ManualSigmas`、`PolyexponentialScheduler`、`RandomNoise`、`SDTurboScheduler`、`SamplerCustom`、`SamplerCustomAdvanced`、`SamplerDPMAdaptative`、`SamplerDPMPP_2M_SDE`、`SamplerDPMPP_2S_Ancestral`、`SamplerDPMPP_3M_SDE`、`SamplerDPMPP_SDE`、`SamplerER_SDE`、`SamplerEulerAncestral`、`SamplerEulerAncestralCFGPP`、`SamplerLMS`、`SamplerSASolver`、`SamplerSEEDS2`、`SamplingPercentToSigma`、`SetFirstSigma`、`SplitSigmas`、`SplitSigmasDenoise`、`VPScheduler` | 本地模型/采样/条件/推理优化扩展：custom_sampler；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F030. dataset（35 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F030 dataset | 35 | `AddTextPrefix`、`AddTextSuffix`、`AdjustBrightness`、`AdjustContrast`、`CenterCropImages`、`ImageDeduplication`、`ImageGrid`、`LoadImageDataSetFromFolder`、`LoadImageTextDataSetFromFolder`、`LoadTrainingDataset`、`LoadVideoDataSetFromFolder`、`LoadVideoTextDataSetFromFolder`、`MakeTrainingDataset`、`MergeImageLists`、`MergeTextLists`、`NormalizeImages`、`RandomCropImages`、`ReplaceText`、`ResizeImagesByLongerEdge`、`ResizeImagesByShorterEdge`、`ResolutionBucket`、`SaveImageDataSetToFolder`、`SaveImageTextDataSetToFolder`、`SaveTrainingDataset`、`ShuffleDataset`、`ShuffleImageTextDataset`、`ShuffleVideoDataset`、`ShuffleVideoTextDataset`、`StripWhitespace`、`TextToLowercase`、`TextToUppercase`、`TruncateText`、`VideoFrameSample`、`VideoRandomTemporalCrop`、`VideoTemporalCrop` | 训练/数据集主机脚本编排、进度与产出资产；数据集加载/清洗/分桶/采帧与保存，通用文本/图像子操作可复用S1/S3；§7.4、§8.2、V6。不是新增训练平台承诺 | S4 |

### F031. depth_anything_3（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F031 depth_anything_3 | 5 | `DA3GeometryToMesh`、`DA3GeometryToPointCloud`、`DA3Inference`、`DA3Render`、`LoadDA3Model` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 depth_anything_3 的模型与依赖许可证另审，细项待核实 | S5 |

### F032. differential_diffusion（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F032 differential_diffusion | 1 | `DifferentialDiffusion` | 本地模型/采样/条件/推理优化扩展：differential_diffusion；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F033. easycache（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F033 easycache | 2 | `EasyCache`、`LazyCache` | 本地模型/采样/条件/推理优化扩展：easycache；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F034. edit_model（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F034 edit_model | 2 | `ReferenceLatent`、`ReferenceMultiLatent` | 本地模型/采样/条件/推理优化扩展：edit_model；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F035. eps（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F035 eps | 2 | `Epsilon Scaling`、`TemporalScoreRescaling` | 本地模型/采样/条件/推理优化扩展：eps；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F036. fibo2（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F036 fibo2 | 1 | `TextEncodeFibo2` | 模型族专属：fibo2；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F037. flux（9 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F037 flux | 9 | `CLIPTextEncodeFlux`、`EmptyFlux2LatentImage`、`Flux2Scheduler`、`Flux3LayoutPrompt`、`FluxDisableGuidance`、`FluxGuidance`、`FluxKVCache`、`FluxKontextImageScale`、`FluxKontextMultiReferenceLatentMethod` | 模型族专属：flux；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F038. frame_interpolation（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F038 frame_interpolation | 2 | `FrameInterpolate`、`FrameInterpolationModelLoader` | 视频模型族/采样或时序控制：frame_interpolation；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F039. freelunch（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F039 freelunch | 2 | `FreeU`、`FreeU_V2` | 本地模型/采样/条件/推理优化扩展：freelunch；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F040. fresca（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F040 fresca | 1 | `FreSca` | 本地模型/采样/条件/推理优化扩展：fresca；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F041. gaussian_splat（7 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F041 gaussian_splat | 7 | `File3DToSplat`、`GetSplatCount`、`MergeSplat`、`RenderSplat`、`SplatToFile3D`、`SplatToMesh`、`TransformSplat` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 gaussian_splat 的模型与依赖许可证另审，细项待核实 | S5 |

### F042. gits（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F042 gits | 1 | `GITSScheduler` | 本地模型/采样/条件/推理优化扩展：gits；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F043. glsl（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F043 glsl | 1 | `GLSLShader` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F044. hidream（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F044 hidream | 2 | `CLIPTextEncodeHiDream`、`QuadrupleCLIPLoader` | 模型族专属：hidream；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F045. hidream_o1（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F045 hidream_o1 | 3 | `EmptyHiDreamO1LatentImage`、`HiDreamO1PatchSeamSmoothing`、`HiDreamO1ReferenceImages` | 模型族专属：hidream_o1；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F046. hunyuan（11 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F046 hunyuan | 11 | `CLIPTextEncodeHunyuanDiT`、`EmptyHunyuanImageLatent`、`EmptyHunyuanLatentVideo`、`EmptyHunyuanVideo15Latent`、`HunyuanImageToVideo`、`HunyuanRefinerLatent`、`HunyuanVideo15ImageToVideo`、`HunyuanVideo15LatentUpscaleWithModel`、`HunyuanVideo15SuperResolution`、`LatentUpscaleModelLoader`、`TextEncodeHunyuanVideo_ImageToVideo` | 视频模型族/采样或时序控制：hunyuan；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F047. hunyuan3d（6 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F047 hunyuan3d | 6 | `EmptyLatentHunyuan3Dv2`、`Hunyuan3Dv2Conditioning`、`Hunyuan3Dv2ConditioningMultiView`、`VAEDecodeHunyuan3D`、`VoxelToMesh`、`VoxelToMeshBasic` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 hunyuan3d 的模型与依赖许可证另审，细项待核实 | S5 |

### F048. hypernetwork（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F048 hypernetwork | 1 | `HypernetworkLoader` | 本地模型/采样/条件/推理优化扩展：hypernetwork；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F049. hypertile（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F049 hypertile | 1 | `HyperTile` | 本地模型/采样/条件/推理优化扩展：hypertile；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F050. ideogram4（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F050 ideogram4 | 1 | `Ideogram4Scheduler` | 模型族专属：ideogram4；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F051. image_compare（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F051 image_compare | 1 | `ImageCompare` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F052. images（23 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F052 images | 23 | `GetImageSize`、`GetLatentSize`、`ImageAddNoise`、`ImageColorSpace`、`ImageCrop`、`ImageCropToMask`、`ImageCropV2`、`ImageFlip`、`ImageFromBatch`、`ImageMergeTileList`、`ImageRotate`、`ImageSDRToHDR`、`ImageScaleToMaxDimension`、`ImageStitch`、`PadImageNode`、`PrimitiveBoundingBox`、`RepeatImageBatch`、`ResizeAndPadImage`、`SaveAnimatedPNG`、`SaveAnimatedWEBP`、`SaveImageAdvanced`、`SaveSVGNode`、`SplitImageToTileList` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F053. ip2p（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F053 ip2p | 1 | `InstructPixToPixConditioning` | 模型族专属：ip2p；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F054. joyimage（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F054 joyimage | 1 | `TextEncodeJoyImageEdit` | 模型族专属：joyimage；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F055. json_prompt（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F055 json_prompt | 1 | `BuildJsonPromptIdeogram` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F056. kandinsky5（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F056 kandinsky5 | 3 | `CLIPTextEncodeKandinsky5`、`Kandinsky5ImageToVideo`、`NormalizeVideoLatentStart` | 视频模型族/采样或时序控制：kandinsky5；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F057. latent（15 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F057 latent | 15 | `LatentAdd`、`LatentApplyOperation`、`LatentApplyOperationCFG`、`LatentBatch`、`LatentBatchSeedBehavior`、`LatentConcat`、`LatentCut`、`LatentCutToBatch`、`LatentInterpolate`、`LatentMultiply`、`LatentOperationBlend`、`LatentOperationSharpen`、`LatentOperationTonemapReinhard`、`LatentSubtract`、`ReplaceVideoLatentFrames` | 本地模型/采样/条件/推理优化扩展：latent；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F058. load_3d（6 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F058 load_3d | 6 | `Load3D`、`Load3DAdvanced`、`Preview3D`、`Preview3DAdvanced`、`PreviewGaussianSplat`、`PreviewPointCloud` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 load_3d 的模型与依赖许可证另审，细项待核实 | S5 |

### F059. logic（13 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F059.1 logic | 4 | `AutogrowNamesTestNode`、`AutogrowPrefixTestNode`、`ComboOptionTestNode`、`DCTestNode` | 上游 UI/动态端口测试夹具；不是业务逻辑节点 | 不对标 |
| F059.2 logic | 9 | `BranchNode`、`ComfyAndNode`、`ComfyNotNode`、`ComfyOrNode`、`ComfySoftSwitchNode`、`ComfySwitchNode`、`ConvertStringToComboNode`、`CustomCombo`、`InvertBooleanNode` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F060. loop（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F060 loop | 5 | `EndLoop`、`LoopIteration`、`LoopProgress`、`LoopResult`、`StartLoop` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F061. lora_debug（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F061 lora_debug | 2 | `LoraLoaderBypass`、`LoraLoaderBypassModelOnly` | 本地模型/采样/条件/推理优化扩展：lora_debug；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F062. lora_extract（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F062 lora_extract | 1 | `LoraSave` | 本地模型/CLIP/VAE/LoRA 合并或保存（§18.4）；模型与依赖许可证核实后交付 | S4 |

### F063. lora_stack（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F063 lora_stack | 2 | `LoadLoraModel`、`LoadLoraTextEncoder` | 本地模型/采样/条件/推理优化扩展：lora_stack；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F064. lotus（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F064 lotus | 1 | `LotusConditioning` | 模型族专属：lotus；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F065. lt（18 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F065 lt | 18 | `EmptyLTXVLatentVideo`、`GetICLoRAParameters`、`LTXVAddGuide`、`LTXVAddLatentGuide`、`LTXVConcatAVLatent`、`LTXVConditioning`、`LTXVCropGuides`、`LTXVDualCFGGuider`、`LTXVDurationPredictor`、`LTXVImgToVideo`、`LTXVImgToVideoInplace`、`LTXVModalityGuidance`、`LTXVPreprocess`、`LTXVReferenceAudio`、`LTXVScheduler`、`LTXVSeparateAVLatent`、`LTXVSpatioTemporalGuidance`、`ModelSamplingLTXV` | 视频模型族/采样或时序控制：lt；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F066. lt_audio（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F066 lt_audio | 5 | `LTXAVTextEncoderLoader`、`LTXVAudioVAEDecode`、`LTXVAudioVAEEncode`、`LTXVAudioVAELoader`、`LTXVEmptyLatentAudio` | 音频扩充：文件/声道/处理/生成或音频编码模型；§7.4、§15。模型族 lt_audio 按V6许可证审核；不是首批TTS全覆盖承诺 | S5 |

### F067. lt_keyframes（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F067 lt_keyframes | 4 | `LTXVAddGeneratedKeyframes`、`LTXVFreezeLatent`、`LTXVGeneratedKeyframesToGuides`、`LTXVSeparateGeneratedKeyframes` | 视频模型族/采样或时序控制：lt_keyframes；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F068. lt_upsampler（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F068 lt_upsampler | 1 | `LTXVLatentUpsampler` | 视频模型族/采样或时序控制：lt_upsampler；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F069. lumina2（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F069 lumina2 | 2 | `CLIPTextEncodeLumina2`、`RenormCFG` | 模型族专属：lumina2；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F070. mage（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F070 mage | 1 | `TextEncodeMageFlowEdit` | 模型族专属：mage；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F071. mahiro（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F071 mahiro | 1 | `Mahiro` | 本地模型/采样/条件/推理优化扩展：mahiro；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F072. marigold（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F072 marigold | 1 | `MarigoldV2PostProcess` | 模型族专属：marigold；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F073. mask（13 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F073.1 mask | 12 | `CropMask`、`FeatherMask`、`GrowMask`、`ImageColorToMask`、`ImageCompositeMasked`、`ImageToMask`、`InvertMask`、`MaskComposite`、`MaskPreview`、`MaskToImage`、`SolidMask`、`ThresholdMask` | 通用遮罩处理与预览；服务端 image+遮罩资产（§18.3–§18.4） | S3 |
| F073.2 mask | 1 | `LatentCompositeMasked` | 潜空间遮罩合成；ferry-infer（§8.2），不是CPU图像遮罩 | S4 |

### F074. math（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F074 math | 1 | `ComfyMathExpression` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F075. mediapipe（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F075 mediapipe | 4 | `LoadMediaPipeFaceLandmarker`、`MediaPipeFaceLandmarker`、`MediaPipeFaceMask`、`MediaPipeFaceMeshVisualize` | 本地模型/采样/条件/推理优化扩展：mediapipe；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F076. mesh_io（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F076 mesh_io | 1 | `Get3DComponents` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 mesh_io 的模型与依赖许可证另审，细项待核实 | S5 |

### F077. mesh_postprocess（14 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F077 mesh_postprocess | 14 | `ApplyTextureToMesh`、`BakeAmbientOcclusion`、`BakeNormalMapFromMesh`、`BakeTextureFromVoxel`、`DecimateMesh`、`FillHoles`、`MeshSmoothNormals`、`MeshTextureToImage`、`PaintMesh`、`RemeshMesh`、`RenderMesh`、`RenderUVAtlas`、`UnwrapMesh`、`WeldVertices` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 mesh_postprocess 的模型与依赖许可证另审，细项待核实 | S5 |

### F078. ming（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F078 ming | 1 | `TextEncodeMingImageEdit` | 模型族专属：ming；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F079. minimax_h3（6 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F079 minimax_h3 | 6 | `EmptyMiniMaxH3LatentAV`、`MiniMaxH3AddGuide`、`MiniMaxH3FunControlNetApply`、`MiniMaxH3ImageToVideo`、`MiniMaxH3ReferenceToVideo`、`MiniMaxH3SigmaShift` | 视频模型族/采样或时序控制：minimax_h3；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F080. minimax_music（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F080 minimax_music | 2 | `EmptyMiniMaxMusic3LatentAudio`、`MiniMaxMusic3TextEncode` | 音频扩充：文件/声道/处理/生成或音频编码模型；§7.4、§15。模型族 minimax_music 按V6许可证审核；不是首批TTS全覆盖承诺 | S5 |

### F081. mochi（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F081 mochi | 1 | `EmptyMochiLatentVideo` | 视频模型族/采样或时序控制：mochi；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F082. model_advanced（11 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F082 model_advanced | 11 | `ModelAttentionBackend`、`ModelComputeDtype`、`ModelNoiseScale`、`ModelSamplingAuraFlow`、`ModelSamplingContinuousEDM`、`ModelSamplingContinuousV`、`ModelSamplingDiscrete`、`ModelSamplingFlux`、`ModelSamplingSD3`、`ModelSamplingStableCascade`、`RescaleCFG` | 本地模型/采样/条件/推理优化扩展：model_advanced；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F083. model_downscale（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F083 model_downscale | 1 | `PatchModelAddDownscale` | 本地模型/采样/条件/推理优化扩展：model_downscale；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F084. model_merging（11 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F084 model_merging | 11 | `CLIPMergeAdd`、`CLIPMergeSimple`、`CLIPMergeSubtract`、`CLIPSave`、`CheckpointSave`、`ModelMergeAdd`、`ModelMergeBlocks`、`ModelMergeSimple`、`ModelMergeSubtract`、`ModelSave`、`VAESave` | 本地模型/CLIP/VAE/LoRA 合并或保存（§18.4）；模型与依赖许可证核实后交付 | S4 |

### F085. model_merging_model_specific（17 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F085 model_merging_model_specific | 17 | `ModelMergeAuraflow`、`ModelMergeCosmos14B`、`ModelMergeCosmos7B`、`ModelMergeCosmosPredict2_14B`、`ModelMergeCosmosPredict2_2B`、`ModelMergeFlux1`、`ModelMergeKrea2`、`ModelMergeLTXV`、`ModelMergeMOSSImage`、`ModelMergeMochiPreview`、`ModelMergeQwenImage`、`ModelMergeSD1`、`ModelMergeSD2`、`ModelMergeSD35_Large`、`ModelMergeSD3_2B`、`ModelMergeSDXL`、`ModelMergeWAN2_1` | 本地模型/CLIP/VAE/LoRA 合并或保存（§18.4）；按成员名称对应模型族，V6选族而非逐个克隆 | S4 |

### F086. model_patch（7 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F086 model_patch | 7 | `AnimaLLLiteApply`、`ModelPatchLoader`、`QwenImageDiffsynthControlnet`、`SUPIRApply`、`USOStyleReference`、`WanUni3CControlnetApply`、`ZImageFunControlnet` | 本地模型/采样/条件/推理优化扩展：model_patch；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F087. moge（6 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F087 moge | 6 | `LoadMoGeModel`、`MoGeGeometryToFOV`、`MoGeInference`、`MoGePanoramaInference`、`MoGePointMapToMesh`、`MoGeRender` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 moge 的模型与依赖许可证另审，细项待核实 | S5 |

### F088. morphology（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F088 morphology | 3 | `ImageRGBToYUV`、`ImageYUVToRGB`、`Morphology` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F089. moss_image（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F089 moss_image | 3 | `MOSSImageControl`、`MOSSImageLayoutPrompt`、`TextEncodeMOSSImageEdit` | 模型族专属：moss_image；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F090. multigpu（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F090 multigpu | 5 | `MultiGPU_Options`、`MultiGPU_WorkUnits`、`SelectCLIPDevice`、`SelectModelDevice`、`SelectVAEDevice` | GPU设备选择/分段调度（§8.2、§8.4）；原多卡分片细项待核实，不能以多主机槽位冒充单模型跨卡 | S4 |

### F091. nag（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F091 nag | 1 | `NAGuidance` | 本地模型/采样/条件/推理优化扩展：nag；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F092. nop（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F092 nop | 1 | `wanBlockSwap` | 视频模型族/采样或时序控制：nop；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F093. number_convert（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F093 number_convert | 1 | `ComfyNumberConvert` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F094. optimalsteps（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F094 optimalsteps | 1 | `OptimalStepsScheduler` | 本地模型/采样/条件/推理优化扩展：optimalsteps；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F095. pag（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F095 pag | 1 | `PerturbedAttentionGuidance` | 本地模型/采样/条件/推理优化扩展：pag；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F096. painter（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F096 painter | 1 | `Painter` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F097. perpneg（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F097 perpneg | 2 | `PerpNeg`、`PerpNegGuider` | 本地模型/采样/条件/推理优化扩展：perpneg；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F098. photomaker（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F098 photomaker | 2 | `PhotoMakerEncode`、`PhotoMakerLoader` | 模型族专属：photomaker；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F099. pid（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F099 pid | 1 | `PiDConditioning` | 模型族专属：pid；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F100. pixart（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F100 pixart | 1 | `CLIPTextEncodePixArtAlpha` | 模型族专属：pixart；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F101. post_processing（11 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F101.1 post_processing | 2 | `BatchImagesMasksLatentsNode`、`BatchLatentsNode` | 潜空间批处理；ferry-infer（§8.2） | S4 |
| F101.2 post_processing | 9 | `BatchImagesNode`、`BatchMasksNode`、`ColorTransfer`、`ImageBlend`、`ImageBlur`、`ImageQuantize`、`ImageScaleToTotalPixels`、`ImageSharpen`、`ResizeImageMaskNode` | 通用图像/遮罩批处理与后处理；服务端 image（§18.3） | S3 |

### F102. preview_any（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F102 preview_any | 1 | `PreviewAny` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F103. primitive（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F103 primitive | 5 | `PrimitiveBoolean`、`PrimitiveFloat`、`PrimitiveInt`、`PrimitiveString`、`PrimitiveStringMultiline` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F104. qwen（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F104 qwen | 5 | `EmptyQwenImageLayeredLatentImage`、`QwenImage21Cache`、`TextEncodeQwenImage21`、`TextEncodeQwenImageEdit`、`TextEncodeQwenImageEditPlus` | 模型族专属：qwen；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F105. rebatch（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F105.1 rebatch | 1 | `RebatchImages` | 图像条目重分批；服务端图像/条目能力（§7.4） | S3 |
| F105.2 rebatch | 1 | `RebatchLatents` | 潜空间重分批；ferry-infer（§8.2） | S4 |

### F106. replacements（16 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F106.1 replacements | 6 | `BatchImagesNode`、`ImageBatch`、`ImageScaleBy`、`ImageScaleToMaxDimension`、`ResizeImageMaskNode`、`ResizeImagesByLongerEdge` | 图像批处理/尺寸/遮罩替代名；服务端 image（§18.3），保留原ID | S3 |
| F106.2 replacements | 6 | `ConditioningAverage`、`ConditioningAverage `、`ControlNetLoader`、`SDV_img2vid_Conditioning`、`SVD_img2vid_Conditioning`、`T2IAdapterLoader` | 条件/模型/视频替代名；ferry-infer，按V6选族；含原样尾空格ID，旧名称映射同一能力 | S4 |
| F106.3 replacements | 4 | `Load3D`、`Load3DAnimation`、`Preview3D`、`Preview3DAnimation` | 3D 加载/动画预览替代名；model3d（§18.4），保留旧/替代ID，不重复实现 | S5 |

### F107. resolution（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F107 resolution | 1 | `ResolutionSelector` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F108. rope（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F108 rope | 1 | `ScaleROPE` | 本地模型/采样/条件/推理优化扩展：rope；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F109. rtdetr（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F109 rtdetr | 2 | `DrawBBoxes`、`RTDETR_detect` | 本地模型/采样/条件/推理优化扩展：rtdetr；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F110. sag（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F110 sag | 1 | `SelfAttentionGuidance` | 本地模型/采样/条件/推理优化扩展：sag；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F111. sam3（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F111 sam3 | 4 | `SAM3_Detect`、`SAM3_TrackPreview`、`SAM3_TrackToMask`、`SAM3_VideoTrack` | 本地模型/采样/条件/推理优化扩展：sam3；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F112. sam3d_body（6 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F112 sam3d_body | 6 | `BuildPoseFile`、`SAM3DBody_FaceExpression`、`SAM3DBody_Loader`、`SAM3DBody_Predict`、`SAM3DBody_Render`、`SAM3DBody_Smooth` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 sam3d_body 的模型与依赖许可证另审，细项待核实 | S5 |

### F113. save_3d（8 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F113 save_3d | 8 | `GetMeshInfo`、`MergeMeshes`、`MeshToFile3D`、`RotateMesh`、`Save3DAdvanced`、`SaveGLB`、`SaveGaussianSplat`、`SavePointCloud` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 save_3d 的模型与依赖许可证另审，细项待核实 | S5 |

### F114. scail（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F114 scail | 2 | `SCAIL2ColoredMask`、`WanSCAILToVideo` | 视频模型族/采样或时序控制：scail；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F115. sd3（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F115 sd3 | 5 | `CLIPTextEncodeSD3`、`ControlNetApplySD3`、`EmptySD3LatentImage`、`SkipLayerGuidanceSD3`、`TripleCLIPLoader` | 模型族专属：sd3；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F116. sdpose（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F116 sdpose | 4 | `CropByBBoxes`、`SDPoseDrawKeypoints`、`SDPoseFaceBBoxes`、`SDPoseKeypointExtractor` | 本地模型/采样/条件/推理优化扩展：sdpose；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F117. sdupscale（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F117 sdupscale | 1 | `SD_4XUpscale_Conditioning` | 模型族专属：sdupscale；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F118. seed（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F118 seed | 1 | `SeedNode` | 生成参数的固定/递增/执行后随机种子（§18.4）；执行开始时记录确定值 | S3 |

### F119. seedvr（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F119 seedvr | 5 | `SeedVR2Conditioning`、`SeedVR2PostProcessing`、`SeedVR2Preprocess`、`SeedVR2TemporalChunk`、`SeedVR2TemporalMerge` | 视频模型族/采样或时序控制：seedvr；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F120. sensenova（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F120 sensenova | 1 | `SenseNovaSamplingOptions` | 模型族专属：sensenova；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F121. slg（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F121 slg | 2 | `SkipLayerGuidanceDiT`、`SkipLayerGuidanceDiTSimple` | 本地模型/采样/条件/推理优化扩展：slg；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F122. sparse_attention（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F122 sparse_attention | 1 | `BlockSparseAttention` | 本地模型/采样/条件/推理优化扩展：sparse_attention；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F123. stable3d（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F123 stable3d | 3 | `SV3D_Conditioning`、`StableZero123_Conditioning`、`StableZero123_Conditioning_Batched` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 stable3d 的模型与依赖许可证另审，细项待核实 | S5 |

### F124. stable_cascade（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F124 stable_cascade | 4 | `StableCascade_EmptyLatentImage`、`StableCascade_StageB_Conditioning`、`StableCascade_StageC_VAEEncode`、`StableCascade_SuperResolutionControlnet` | 模型族专属：stable_cascade；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F125. string（15 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F125 string | 15 | `CaseConverter`、`ConvertArrayToString`、`ConvertDictionaryToString`、`JsonExtractString`、`RegexExtract`、`RegexMatch`、`RegexReplace`、`StringCompare`、`StringConcatenate`、`StringContains`、`StringFormat`、`StringLength`、`StringReplace`、`StringSubstring`、`StringTrim` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F126. tcfg（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F126 tcfg | 1 | `TCFG` | 本地模型/采样/条件/推理优化扩展：tcfg；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F127. text（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F127 text | 1 | `SaveText` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F128. text_overlay（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F128 text_overlay | 1 | `TextOverlay` | 通用图像处理/图像参数控件；服务端 image 与资产操作（§7.4、§18.3–§18.4），细项作用待核实，GLSL只验收等效图像操作 | S3 |

### F129. textgen（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F129 textgen | 2 | `TextGenerate`、`TextGenerateLTX2Prompt` | 本地文本生成/提示扩展，具体作用待核实；S2 LLM能力已单列，本地额外模型按§15扩充及V6审核 | S5 |

### F130. tomesd（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F130 tomesd | 1 | `TomePatchModel` | 本地模型/采样/条件/推理优化扩展：tomesd；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F131. toolkit（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F131 toolkit | 2 | `CreateList`、`GetItemFromList` | 工具与逻辑：控制/循环/表达式/文本/列表/值或通用输出查看（§6、§7.4、§9.2），按成员验收；不承诺原文件兼容 | S1 |

### F132. torch_compile（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F132 torch_compile | 1 | `TorchCompileModel` | 本地模型/采样/条件/推理优化扩展：torch_compile；ferry-infer（§8.2–§8.3、§18.4），模型族按V6审核；成员具体作用待核实 | S4 |

### F133. train（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F133 train | 4 | `LoraModelLoader`、`LossGraphNode`、`SaveLoRA`、`TrainLoraNode` | 训练/数据集主机脚本编排、进度与产出资产；LoRA训练与保存，具体配方待核实；§7.4、§8.2、V6。不是新增训练平台承诺 | S4 |

### F134. trellis2（10 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F134 trellis2 | 10 | `EmptyTrellis2LatentStructure`、`Pixal3DConditioning`、`Pixal3DMultiViewConditioning`、`Trellis2Conditioning`、`Trellis2ShapeStage`、`Trellis2TextureStage`、`Trellis2UpsampleStage`、`VaeDecodeShapeTrellis`、`VaeDecodeStructureTrellis2`、`VaeDecodeTextureTrellis` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 trellis2 的模型与依赖许可证另审，细项待核实 | S5 |

### F135. triposplat（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F135 triposplat | 4 | `TripoSplatConditioning`、`TripoSplatPreprocessImage`、`TripoSplatSamplingPreview`、`VAEDecodeTripoSplat` | 3D/几何/点云/网格处理、加载预览或生成；model3d（§18.4）。组 triposplat 的模型与依赖许可证另审，细项待核实 | S5 |

### F136. upscale_model（2 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F136 upscale_model | 2 | `ImageUpscaleWithModel`、`UpscaleModelLoader` | 模型族专属：upscale_model；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

### F137. video（9 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F137 video | 9 | `ConcatenateVideo`、`CreateVideo`、`GetVideoComponents`、`LoadVideo`、`SaveVideo`、`SaveWEBM`、`Video Slice`、`VideoCrop`、`VideoTrim` | 视频文件载入、拆分、拼接、裁剪与保存；资产+主机ffmpeg（§7.4、V8），不是视频模型推理 | S3 |

### F138. video_model（6 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F138 video_model | 6 | `ConditioningSetAreaPercentageVideo`、`ImageOnlyCheckpointLoader`、`ImageOnlyCheckpointSave`、`SVD_img2vid_Conditioning`、`VideoLinearCFGGuidance`、`VideoTriangleCFGGuidance` | 视频模型族/采样或时序控制：video_model；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F139. void（6 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F139 void | 6 | `OpticalFlowLoader`、`VOIDInpaintConditioning`、`VOIDQuadmaskPreprocess`、`VOIDSampler`、`VOIDWarpedNoise`、`VOIDWarpedNoiseSource` | 视频模型族/采样或时序控制：void；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F140. wan（18 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F140 wan | 18 | `TrimVideoLatent`、`Wan22FunControlToVideo`、`Wan22ImageToVideoLatent`、`WanAnimate2Cache`、`WanAnimate2ToVideo`、`WanAnimateToVideo`、`WanCameraImageToVideo`、`WanFirstLastFrameToVideo`、`WanFunControlToVideo`、`WanFunInpaintToVideo`、`WanHuMoImageToVideo`、`WanImageToVideo`、`WanInfiniteTalkToVideo`、`WanPhantomSubjectToVideo`、`WanSoundImageToVideo`、`WanSoundImageToVideoExtend`、`WanTrackToVideo`、`WanVaceToVideo` | 视频模型族/采样或时序控制：wan；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F141. wandancer（4 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F141 wandancer | 4 | `WanDancerEncodeAudio`、`WanDancerPadKeyframes`、`WanDancerPadKeyframesList`、`WanDancerVideo` | 视频模型族/采样或时序控制：wandancer；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F142. wanmove（5 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F142 wanmove | 5 | `GenerateTracks`、`WanMoveConcatTrack`、`WanMoveTrackToVideo`、`WanMoveTracksFromCoords`、`WanMoveVisualizeTracks` | 视频模型族/采样或时序控制：wanmove；ferry-infer（§8.2–§8.3），按V6许可证优先选族，非全部节点克隆 | S4 |

### F143. webcam（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F143 webcam | 1 | `WebcamCapture` | 媒体输入控件/采集资产（§18.4）；摄像头权限与具体行为待核实 | S3 |

### F144. yue2（3 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F144 yue2 | 3 | `EmptyYuE2LatentAudio`、`YuE2GenerateABC`、`YuE2GenerateMusic` | 音频扩充：文件/声道/处理/生成或音频编码模型；§7.4、§15。模型族 yue2 按V6许可证审核；不是首批TTS全覆盖承诺 | S5 |

### F145. zimage（1 个）

| 组 | 数量 | 节点（全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| F145 zimage | 1 | `TextEncodeZImageOmni` | 模型族专属：zimage；加载/编码/条件/采样等按成员核实，ferry-infer（§8.2），V6选族，非全部节点克隆 | S4 |

## G. ComfyUI 合作伙伴 API 节点（46 个文件）

按本机 comfy/comfy_api_nodes/nodes_*.py 的46个文件列出，辅助目录 apis、util 和 __init__.py 不计入。以下 node_id 仅用于来源对照，由源码字面 node_id 提取，不复制其实现；不同文件、同服务商仍分别保留一行。服务商名称按文件名与 ID 识别，具体商业名称/功能未核实的标记“待核实”。

统一映射到 v3 §8.1 的 media.<provider> 云端媒体能力适配器，凭据由 §7.5 管理。S3 是首批能力的交付期，V7 要求国内外各1–2家，名单须在 S3 开工前由用户确认。名单未确认前，本表媒体服务商保守记 S5 批次按需；入选者对应的媒体能力再按 v3 前移 S3 并更新计数。这样不把全部候选记成首批承诺。未入选服务商继续“未开始”，不自动验收通过。3D 专用服务文件归 S5（§18.4），纯 LLM 文件归 S2/S5，不能把文本 API 硬称为媒体 API。

| 编号 | 服务商 / 文件 | node_id（字面值全部列出） | Ferry 承接方式 | 期 |
|---|---|---|---|---|
| G001 | Anthropic Claude / nodes_anthropic.py | `ClaudeNode` | §7.4/§7.5 LLM凭据与文本能力；非媒体，原生差异单独核实 | S2 |
| G002 | Beeble / nodes_beeble.py | `BeebleSwitchXVideoEdit`、`BeebleSwitchXImageEdit` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G003 | Black Forest Labs / nodes_bfl.py | `FluxProUltraImageNode`、`FluxProExpandNode`、`FluxProFillNode`、`FluxEraseNode`、`FluxVTONode`、`Flux2ImageNode`、`Flux3TextToVideoNode`、`Flux3ImageToVideoNode`、`Flux3VideoContinuationNode`、`FluxVideoUpscaleNode`、`FluxVideoEditNode`、`Flux3ImageNode`、`Flux3MaxTextToVideoNode`、`Flux3MaxImageToVideoNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G004 | Bria / nodes_bria.py | `BriaImageEditNode`、`BriaRemoveImageBackground`、`BriaGenFill`、`BriaEraser`、`BriaExpandImage`、`BriaIncreaseResolution`、`BriaRemoveVideoBackground`、`BriaVideoGreenScreen`、`BriaVideoReplaceBackground`、`BriaTransparentVideoBackground`、`BriaImageGenerate`、`BriaEraseByText`、`BriaAddObject`、`BriaReplaceObject`、`BriaRelight`、`BriaRestorePhoto`、`BriaReseason`、`BriaEraseForeground`、`BriaReplaceImageBackground`、`BriaGuidedRemoveBackground`、`BriaProductHolding`、`BriaVirtualTryOn`、`BriaVideoEraser` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G005 | ByteDance / nodes_bytedance.py | `ByteDanceSeedreamNode`、`ByteDanceSeedreamNodeV3`、`ByteDanceSeedreamNodeV2`、`ByteDanceSeedreamLayerSeparationNodeV2`、`ByteDanceSeedreamLayerSeparationNode`、`ByteDanceTextToVideoNode`、`ByteDanceImageToVideoNode`、`ByteDanceFirstLastFrameNode`、`ByteDance2TextToVideoNode`、`ByteDance2FirstLastFrameNode`、`ByteDance2ReferenceNodeV2`、`ByteDance2ReferenceNode`、`ByteDance2DraftToFinalVideoNode`、`ByteDanceCreateImageAsset`、`ByteDanceCreateVideoAsset`、`ByteDanceSeedAudio`、`ByteDanceVideoEnhanceNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G006 | ByteDance（LLM） / nodes_bytedance_llm.py | `ByteDanceSeedNode` | §7.4/§7.5 LLM凭据与文本能力；非媒体，原生差异单独核实 | S5 |
| G007 | Clarity AI / nodes_clarityai.py | `ClarityCrystalUpscaleNode`、`ClarityCrystalVideoUpscaleNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G008 | Comfy Cloud / nodes_comfy_cloud.py | `ComfyCloudFlux2TextToImageNode`、`ComfyCloudZImageTurboNode`、`ComfyCloudMageFlowTextToImageNode`、`ComfyCloudMageFlowTurboTextToImageNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G009 | ElevenLabs / nodes_elevenlabs.py | `ElevenLabsSpeechToText`、`ElevenLabsVoiceSelector`、`ElevenLabsTextToSpeech`、`ElevenLabsAudioIsolation`、`ElevenLabsTextToSoundEffects`、`ElevenLabsInstantVoiceClone`、`ElevenLabsSpeechToSpeech`、`ElevenLabsTextToDialogue` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G010 | Fish Audio / nodes_fishaudio.py | `FishAudioVoiceSelector`、`FishAudioTextToSpeech`、`FishAudioSpeechToText`、`FishAudioInstantVoiceClone` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G011 | Google Gemini / nodes_gemini.py | `GeminiNode`、`GeminiNodeV2`、`GeminiNodeV3`、`GeminiInputFiles`、`GeminiImageNode`、`GeminiImage2Node`、`GeminiNanoBanana2`、`GeminiNanoBanana2V2`、`GeminiVideoOmni`、`GeminiVideoOmniV2` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实；文件内LLM辅助ID复用§7.5文本适配，媒体生成仍按V7 | S5 |
| G012 | xAI Grok / nodes_grok.py | `GrokImageNode`、`GrokImageEditNode`、`GrokImageEditNodeV2`、`GrokVideoNode`、`GrokVideoEditNode`、`GrokVideoReferenceNode`、`GrokVideoExtendNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实；文件内LLM辅助ID复用§7.5文本适配，媒体生成仍按V7 | S5 |
| G013 | HeyGen / nodes_heygen.py | `HeyGenTalkingPhotoNode`、`HeyGenAvatarVideoNode`、`HeyGenCreateAvatarNode`、`HeyGenVideoTranslateNode`、`HeyGenTextToSpeechNode`、`HeyGenReferenceToVideoNode`、`HeyGenImageToVideoNode`、`HeyGenVoiceCloneNode`、`HeyGenVoiceTextToSpeechNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G014 | Higgsfield / nodes_higgsfield.py | `HiggsfieldMotionTransferNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G015 | HitPaw / nodes_hitpaw.py | `HitPawGeneralImageEnhance`、`HitPawVideoEnhance` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G016 | 腾讯混元3D / nodes_hunyuan3d.py | `TencentTextToModelNode`、`TencentImageToModelNode`、`TencentModelTo3DUVNode`、`Tencent3DTextureEditNode`、`Tencent3DPartNode`、`TencentSmartTopologyNode` | §18.4 3D云端适配（复用§8.1异步/资产/意图机制），S5按需 | S5 |
| G017 | 腾讯混元图像 / nodes_hunyuan_image.py | `HunyuanImageTextToImageApi`、`HunyuanImageEditApi` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G018 | Ideogram / nodes_ideogram.py | `IdeogramV3`、`IdeogramV4`、`IdeogramPImage`、`IdeogramTextToImageApi`、`IdeogramEditApi`、`IdeogramPreciseEditApi`、`IdeogramVectorGenerateApi`、`IdeogramVectorImageToSVGApi` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G019 | Kling / nodes_kling.py | `KlingTextToVideoNode`、`KlingOmniProTextToVideoNode`、`KlingOmniProFirstLastFrameNode`、`KlingOmniProImageToVideoNode`、`KlingOmniProVideoToVideoNode`、`KlingOmniProEditVideoNode`、`KlingOmniProImageNode`、`KlingImage2VideoNode`、`KlingStartEndFrameNode`、`KlingLipSyncAudioToVideoNode`、`KlingLipSyncTextToVideoNode`、`KlingImageGenerationNode`、`KlingTextToVideoWithAudio`、`KlingImageToVideoWithAudio`、`KlingMotionControl`、`KlingVideoNode`、`KlingFirstLastFrameNode`、`KlingAvatarNode`、`KlingTryOnNode`、`Kling4TextToVideoNode`、`Kling4ImageToVideoNode`、`Kling4ReferenceToVideoNode`、`Kling4EditVideoNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G020 | Krea / nodes_krea.py | `Krea2ImageNode`、`Krea2StyleReferenceNode`、`Krea3ImageNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G021 | LTX / nodes_ltxv.py | `LtxApi25TextToVideo`、`LtxApi25ImageToVideo`、`LtxApi25AudioToVideo` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G022 | Luma / nodes_luma.py | `LumaReferenceNode`、`LumaConceptsNode`、`LumaImageNode`、`LumaImageModifyNode`、`LumaVideoNode`、`LumaImageToVideoNode`、`LumaImageNode2`、`LumaImageEditNode2`、`LumaRay32TextToVideoNode`、`LumaRay32ImageToVideoNode`、`LumaRay32KeyframeNode`、`LumaRay32KeyframesToVideoNode`、`LumaRay32VideoEditNode`、`LumaRay32VideoReframeNode`、`LumaRay32ExtendVideoNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G023 | Magnific / nodes_magnific.py | `MagnificImageUpscalerCreativeNode`、`MagnificImageUpscalerPreciseV2Node`、`MagnificImageStyleTransferNode`、`MagnificImageRelightNode`、`MagnificImageSkinEnhancerNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G024 | Meshy / nodes_meshy.py | `MeshyTextToModelNode`、`MeshyRefineNode`、`MeshyImageToModelNode`、`MeshyMultiImageToModelNode`、`MeshyTextToModelSmartTopologyNode`、`MeshyImageToModelSmartTopologyNode`、`MeshyRigModelNode`、`MeshyAnimateModelNode`、`MeshyTextureNode`、`MeshyTextureMultiViewNode` | §18.4 3D云端适配（复用§8.1异步/资产/意图机制），S5按需 | S5 |
| G025 | Meta / nodes_meta.py | `MetaMuseImageTextToImageApi`、`MetaMuseImageEditApi` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G026 | MiniMax / nodes_minimax.py | `MinimaxTextToVideoNode`、`MinimaxImageToVideoNode`、`MinimaxSubjectToVideoNode`、`MinimaxHailuoVideoNode`、`MinimaxHailuo03TextToVideoNode`、`MinimaxHailuo03FirstLastFrameNode`、`MinimaxHailuo03ReferenceNode`、`MinimaxHailuo03ContextIRNode`、`MinimaxHailuo03RegenerateNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G027 | Mirelo / nodes_mirelo.py | `MireloVideoToSFX`、`MireloTextToSFX` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G028 | OpenAI / nodes_openai.py | `OpenAIGPTImage1`、`OpenAIGPTImageNodeV2`、`OpenAIChatNode`、`OpenAIInputFiles`、`OpenAIChatConfig` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实；文件内LLM辅助ID复用§7.5文本适配，媒体生成仍按V7 | S5 |
| G029 | OpenRouter / nodes_openrouter.py | `OpenRouterLLMNode`、`OpenRouterImageNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实；文件内LLM辅助ID复用§7.5文本适配，媒体生成仍按V7 | S5 |
| G030 | PixVerse / nodes_pixverse.py | `PixverseTemplateNode`、`PixverseTextToVideoNode`、`PixverseImageToVideoNode`、`PixverseTransitionVideoNode`、`PixverseV6TextToVideoNode`、`PixverseV6ImageToVideoNode`、`PixverseV6FirstLastFrameNode`、`PixverseV6ExtendVideoNode`、`PixverseV6FusionVideoNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G031 | Pruna / nodes_pruna.py | `PrunaTextToVideoNode`、`PrunaImageToVideoNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G032 | Quiver（名称待核实） / nodes_quiver.py | `QuiverTextToSVGNode`、`QuiverImageToSVGNode`、`QuiverTextToSVGNodeV2`、`QuiverImageToSVGNodeV2` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G033 | Qwen / nodes_qwen.py | `QwenImageTextToImageApi`、`QwenImageEditApi` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G034 | Recraft / nodes_recraft.py | `RecraftColorRGB`、`RecraftControls`、`RecraftStyleV3RealisticImage`、`RecraftStyleV3DigitalIllustration`、`RecraftStyleV3VectorIllustrationNode`、`RecraftStyleV3LogoRaster`、`RecraftStyleV3InfiniteStyleLibrary`、`RecraftCreateStyleNode`、`RecraftV4CreateStyleNode`、`RecraftTextToImageNode`、`RecraftImageToImageNode`、`RecraftImageInpaintingNode`、`RecraftTextToVectorNode`、`RecraftVectorizeImageNode`、`RecraftReplaceBackgroundNode`、`RecraftRemoveBackgroundNode`、`RecraftCrispUpscaleNode`、`RecraftCreativeUpscaleNode`、`RecraftV4TextToImageNode`、`RecraftV4TextToVectorNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G035 | Reve / nodes_reve.py | `ReveImageCreateNode`、`ReveImageEditNode`、`ReveImageRemixNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G036 | Rodin / nodes_rodin.py | `Rodin3D_Regular`、`Rodin3D_Detail`、`Rodin3D_Smooth`、`Rodin3D_Sketch`、`Rodin3D_Gen2`、`Rodin3D_Gen25_Image`、`Rodin3D_Gen25_Text` | §18.4 3D云端适配（复用§8.1异步/资产/意图机制），S5按需 | S5 |
| G037 | Runway / nodes_runway.py | `RunwayImageToVideoNodeGen3a`、`RunwayImageToVideoNodeGen4`、`RunwayFirstLastFrameNode`、`RunwayTextToImageNode`、`RunwayAleph2KeyframeNode`、`RunwayAleph2PromptImageNode`、`RunwayAleph2VideoToVideoNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G038 | Sonilo（名称待核实） / nodes_sonilo.py | `SoniloVideoToMusic`、`SoniloTextToMusic` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G039 | Sync / nodes_sync_so.py | `SyncLipSyncNode`、`SyncTalkingImageNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G040 | Topaz / nodes_topaz.py | `TopazImageEnhance`、`TopazImageEnhanceV2`、`TopazVideoEnhance`、`TopazVideoEnhanceV2` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G041 | Tripo / nodes_tripo.py | `TripoTextToModelNode`、`TripoTextToModelNodeV2`、`TripoImageToModelNode`、`TripoImageToModelNodeV2`、`TripoMultiviewToModelNode`、`TripoImageToMultiviewNode`、`TripoEditMultiviewNode`、`TripoTextureNode`、`TripoTextureNodeV2`、`TripoRigNode`、`TripoRetargetNode`、`TripoRigCheckNode`、`TripoSegmentNode`、`TripoMeshCompleteNode`、`TripoRetopologyNode`、`TripoSmartSegmentNode`、`TripoConversionNode`、`TripoImportModelNode`、`TripoP1TextToModelNode`、`TripoP1ImageToModelNode`、`TripoP1MultiviewToModelNode`、`TripoPSeriesTextToModelNode`、`TripoPSeriesImageToModelNode`、`TripoPSeriesMultiviewToModelNode` | §18.4 3D云端适配（复用§8.1异步/资产/意图机制），S5按需 | S5 |
| G042 | VEED / nodes_veed.py | `VeedFabricNode`、`VeedLipsyncNode`、`VeedRemoveVideoBackgroundNode`、`VeedCleanAudioNode`、`VeedCleanVideoAudioNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G043 | Google Veo / nodes_veo2.py | `Veo3VideoGenerationNode`、`Veo3FirstLastFrameNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G044 | Vidu / nodes_vidu.py | `ViduTextToVideoNode`、`ViduImageToVideoNode`、`ViduReferenceVideoNode`、`ViduStartEndToVideoNode`、`Vidu2TextToVideoNode`、`Vidu2ImageToVideoNode`、`Vidu2ReferenceVideoNode`、`Vidu2StartEndToVideoNode`、`ViduExtendVideoNode`、`ViduMultiFrameVideoNode`、`Vidu3TextToVideoNode`、`Vidu3ImageToVideoNode`、`Vidu3StartEndToVideoNode`、`Vidu4ImageToVideoNode`、`Vidu4ReferenceVideoNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G045 | Wan / nodes_wan.py | `WanTextToImageApi`、`WanImageToImageApi`、`WanTextToVideoApi`、`WanImageToVideoApi`、`WanReferenceVideoApi`、`Wan2TextToVideoApi`、`Wan2ImageToVideoApi`、`Wan2VideoContinuationApi`、`Wan2VideoEditApi`、`Wan2ReferenceVideoApi`、`Wan3ReferenceToVideoApi`、`Wan3ImageToVideoApi`、`HappyHorseTextToVideoApi`、`HappyHorseImageToVideoApi`、`HappyHorseVideoEditApi`、`HappyHorseReferenceVideoApi` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |
| G046 | WaveSpeed / nodes_wavespeed.py | `WavespeedFlashVSRNode`、`WavespeedImageUpscaleNode` | §8.1 media.<provider> 能力适配，S5批次按需；如经V7确认入选，首批相应能力在S3交付，更新本行。各ID逐项核实 | S5 |

## H. 汇总

统计口径一：A–G 带编号的实际验收行数，B/C/D各源节点一行，F按混合组子行计，G每文件一行；D分类概览与H统计表不是新增能力，不重复计数。统计口径二：节点/文件单位，F按本行成员数加权为691，其余每行权重1。平台行与节点行之间可能承接同一 Ferry 实现，两个产品之间也有重叠，计数是来源覆盖量，不能理解为需要新增同样多的 Ferry 节点。

S4 行按 V6 模型族选择承接，未选成员仍未开始；G 在 V7 未确认前按 S5 统计，入选首批能力前移 S3 后须重算。本表是当前分期库存，不是已交付量。S0、v2 P2 另列，避免遗漏外壳项；S0 不并入 S1。

### n8n 分期数量

| 口径/部分 | S0 | S1 | S2 | S3 | S4 | S5 | v2 P0 | v2 P1 | v2 P2 | 不对标 | 合计 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| A 验收行 | 5 | 25 | 4 | 0 | 0 | 5 | 0 | 0 | 1 | 0 | 40 |
| B 验收行 | 0 | 53 | 11 | 1 | 0 | 6 | 0 | 0 | 0 | 4 | 75 |
| C 验收行 | 0 | 0 | 79 | 0 | 0 | 48 | 0 | 0 | 0 | 0 | 127 |
| D 验收行 | 0 | 0 | 17 | 0 | 0 | 346 | 0 | 0 | 0 | 0 | 363 |
| 验收行合计 | 5 | 78 | 111 | 1 | 0 | 405 | 0 | 0 | 1 | 4 | 605 |
| 节点/文件单位（不含平台行） | 0 | 53 | 107 | 1 | 0 | 400 | 0 | 0 | 0 | 4 | 565 |

### ComfyUI 分期数量

| 口径/部分 | S0 | S1 | S2 | S3 | S4 | S5 | v2 P0 | v2 P1 | v2 P2 | 不对标 | 合计 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| E 验收行 | 0 | 16 | 0 | 15 | 4 | 3 | 0 | 0 | 0 | 0 | 38 |
| F 验收行 | 0 | 9 | 0 | 22 | 99 | 21 | 0 | 0 | 0 | 1 | 152 |
| G 验收行 | 0 | 0 | 1 | 0 | 0 | 45 | 0 | 0 | 0 | 0 | 46 |
| 验收行合计 | 0 | 25 | 1 | 37 | 103 | 69 | 0 | 0 | 0 | 1 | 236 |
| 节点/文件单位（不含平台行） | 0 | 40 | 1 | 93 | 432 | 167 | 0 | 0 | 0 | 4 | 737 |

### 不对标项及边界

| 来源/编号 | 能力或成员 | 原因/承接边界 | 期 |
|---|---|---|---|
| B010 | e2eTest | 上游测试专用，不是业务节点 | 不对标 |
| B011 | e2eTestPollingTrigger | 上游测试专用，不是业务触发器 | 不对标 |
| B054 | simulate | 上游模拟/测试专用；Ferry 用固定数据验收测试，不克隆此节点 | 不对标 |
| B055 | simulateTrigger | 上游模拟/测试专用；Ferry 用测试执行与固定数据 | 不对标 |
| F logic 测试子行 | AutogrowNamesTestNode、AutogrowPrefixTestNode、ComboOptionTestNode、DCTestNode | 上游 UI/动态端口测试夹具，不是业务能力；其它逻辑成员仍由 S1 承接 | 不对标 |
| v3 §18.2 / V14 | 托管构建集群 | 自托管执行面为用户主机；使用用户主机组与弹性扩容脚本替代，不建设公共构建云；V14 已确认 | 不对标 |
| v3 §18.2 / V14 | 自有制品包仓库（Maven/npm等协议） | 资产与产物存储只承接保存/分发；不建设独立包管理协议仓库；V14 已确认 | 不对标 |
| v3 §18.2 / V14 | Go template 式 YAML 预渲染 | 工作流表达式/循环与 v2 P2 模板参数承接动态需求，不提供这种预渲染；V14 已确认 | 不对标 |

上表的三个 Flow 范围边界是规范性说明，不在 n8n/ComfyUI 的源节点/平台计数中追加。K8s 分批/蓝绿/灰度按 §18.2 随应用模块计划交付，不能列“不对标”；声明式长尾、WASM 自定义 SDK、音频、3D、训练编排、模型合并均已有归属，没有以生态不兼容为由删去目录。

### 证据缺口与验收前置

要求的全部目录、package.json、46个合作伙伴文件及本机截图文件名均已读取。源目录枚举证明成员存在，不证明全部操作行为；未单独实测的节点/功能须在交付前补测。第三方付费/锁定部分仅验证入口可见，未验证权限、日志外送、环境同步或密钥读取的实际行为。package.json 列举的 AI 节点实现文件在所给解包目录中未提供，因此不能据此确认所有服务商协议、专属操作与兼容性，已保留待核实项。

V6 本地模型族（含传递依赖/权重许可证）、V7 首批云端服务商/账号、V10 向量实现等仍依 v3 决策；本清单不替用户确认。对未核实的 instant 时机、复杂原生 API、多卡分片、训练配方等，不用通用框架演示冒充完整验收。验收时逐项补充操作与证据，任何范围或分期调整先以 v3 为依据，再同步本清单。

本次库存：841 条编号验收行；n8n 605 条，ComfyUI 236 条。节点基线：n8n 75+127+363=565；ComfyUI 核心691条目录出现记录/145组，加46个合作伙伴文件。

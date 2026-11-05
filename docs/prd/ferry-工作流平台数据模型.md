# Ferry 工作流平台数据模型

> v1.11-review（2026-11-03）：本轮按用户要求补设计，**§10～14为新增技术目标，明确覆盖处优先于§4/§9的v1.4来源投影**。旧309列是历史基线，不含本次设计。当前完整目标26表/300字段；§13补长文本二进制前缀/全文复核及向量hash桶碰撞槽，替代旧VARCHAR键投影；§14复用context补Merge输入冻结与写栅栏，不增列。不是已实现迁移，不得直接用于生产；产品默认仍依PRD §15.10 PDR，实际三库迁移另验。

> 评审基线：2026-11-02。设计文档，不执行迁移。所有限定来源缺口均标「源未定义，待补」。

## 1. 来源与约定

仅使用v2《ferry-流水线全功能设计》§16及§8/9/13/14/15，v3《ferry-工作流平台v3总体设计》§11/12.1（含其资产/成本/模型字段引用）/18，ADR最新2026-11-02条目，PRD字段FR/AC，以及migration.rs/entities.rs现状。覆盖顺序：ADR/PRD确认→v3 §18→§11/12.1→v2。D5/D6已被实现决定取代，不保留旧JSON执行引擎；Windows沿用act，不新增执行器表。

全部整数BIGINT，id为雪花主键不自增；新业务表version BIGINT NOT NULL DEFAULT 1，不能借此改写不可变历史。存量不顺便补version；v2 §13.1明确jobs靠条件写，无需version。无外键，依应用范围校验、事务与引用保护。

DDL为SQLite已知字段目标投影，与字段表同源生成。源未定默认值不编造DEFAULT。非空/可空按源语义作评审映射，未给精确长度时VARCHAR(16/32/255)仅是SQL映射草案，不新增产品长度限制；最终长度/可空性源未定义，待补。未命名字段不生成SQL。data/vector四表仅有version的占位投影不具业务能力，禁止部署。long_text()在SQLite/PG为TEXT，MySQL为LONGTEXT，大文本/JSON无DEFAULT、不进索引。

## 2. 时间审计与存量边界

每个新表中的每个xx_at及存量新增xx_at均配同名前缀xx_by BIGINT NOT NULL DEFAULT 0，0=system，包括created/updated/started/finished/expires/verified/last_used。triggered_by或user_id不能代替。expires_by记录设定截止的人；自动处理为0。无时间列不臆造created_at。not_before不匹配xx_at但仍是调度时间。

不回填存量审计：原操作者未必可恢复，补列/伪造操作者会影响历史数据及旧写入契约。以下由当前entities.rs提取的缺配对时间列全部「存量，未改」，不因本表另有ALTER就补xx_by。

| 存量表 | 缺匹配xx_by的xx_at | 状态 |
|---|---|---|
| users | created_at、updated_at | 存量，未改 |
| sessions | created_at、seen_at、expires_at | 存量，未改 |
| workspaces | created_at、updated_at | 存量，未改 |
| workspace_roles | created_at、updated_at | 存量，未改 |
| workspace_members | created_at、updated_at | 存量，未改 |
| activity | created_at | 存量，未改 |
| host_groups | created_at、updated_at | 存量，未改 |
| hosts | seen_at、created_at、updated_at | 存量，未改 |
| tags | created_at、updated_at | 存量，未改 |
| runner_tokens | expires_at、updated_at | 存量，未改 |
| credentials | created_at、updated_at | 存量，未改 |
| repositories | created_at、updated_at | 存量，未改 |
| pipelines | created_at、updated_at | 存量，未改 |
| locks | acquired_at、expires_at | 存量，未改 |
| worker_ids | heartbeat_at | 存量，未改 |
| pipeline_runs | created_at、started_at、finished_at | 存量，未改 |
| pipeline_run_steps | started_at、finished_at | 存量，未改 |
| deployments | created_at、updated_at | 存量，未改 |
| deployment_revisions | started_at、finished_at、created_at、updated_at | 存量，未改 |
| variables | created_at、updated_at | 存量，未改 |
| app_templates | created_at、updated_at | 存量，未改 |
| pipeline_templates | created_at、updated_at | 存量，未改 |
| settings | created_at、updated_at | 存量，未改 |
| dictionaries | created_at、updated_at | 存量，未改 |
| dictionary_items | created_at、updated_at | 存量，未改 |
| cron_jobs | next_run_at、created_at、updated_at | 存量，未改 |
| cron_runs | scheduled_at、started_at、finished_at、created_at、updated_at | 存量，未改 |
| alert_rules | created_at、updated_at | 存量，未改 |
| alert_events | triggered_at、recovered_at、created_at、updated_at | 存量，未改 |
| marketplace_sources | synced_at、created_at、updated_at | 存量，未改 |
| marketplace_entries | created_at、updated_at | 存量，未改 |
| repository_oauth_flows | expires_at、created_at、updated_at | 存量，未改 |
| repository_connections | expires_at、created_at、updated_at | 存量，未改 |
| repository_monitors | created_at、updated_at | 存量，未改 |
| repository_poll_states | last_polled_at、next_poll_at | 存量，未改 |
| repository_events | created_at | 存量，未改 |
| pipeline_artifacts | created_at、expires_at | 存量，未改 |
| config_templates | created_at、updated_at | 存量，未改 |
| deployment_revision_targets | started_at、finished_at | 存量，未改 |
| resource_imports | created_at、committed_at | 存量，未改 |
| pipeline_run_jobs | started_at、finished_at | 存量，未改 |
| pipeline_contexts | created_at、updated_at | 存量，未改 |
| repository_analysis_configs | next_reconcile_at、created_at、updated_at | 存量，未改 |
| repository_analysis_repositories | created_at | 存量，未改 |
| repository_analysis_sources | synced_at | 存量，未改 |
| repository_analysis_refs | updated_at | 存量，未改 |
| repository_analysis_jobs | retry_at、updated_at | 存量，未改 |
| code_insights | next_run_at、created_at、updated_at | 存量，未改 |

时间戳单位沿用项目契约；指定来源未统一声明则为「源未定义，待补」，不擅自换算历史时间。

## 3. ER概览（逻辑关联，无数据库外键）

- pipelines/workflows → definition_revisions → pipeline_runs/workflow_executions；定义选择草稿/发布指针，执行revision_id冻结。
- pipeline_runs → pipeline_run_jobs → steps/logs/reports，任务按run_id+job_key+attempt关联。
- workflow_executions → workflow_node_runs/workflow_execution_logs/cost_records。
- trigger_bindings → trigger_deliveries → 执行；webhook_endpoints → 定义；执行 → human_tasks/human_responses（通用键缺口见逐表）。
- 执行 → execution_intents → execution_parents → 子执行；deployment_revisions.pipeline_intent_id唯一关联意图。
- assets ← asset_refs → execution/node_run/cache/pin/user_pin；assets → asset_tags；node_cache.source_execution_id → 执行。
- models ← host_models → hosts；data_tables→data_table_rows、vector_collections→vector_items为概念父子关系，物理键待补。

## 4. 逐表模型

索引源只给列时，本文件提供评审名称；未擅自增加组合。主键与二级索引均列出。各表统一适用§5跨库说明。

### definition_revisions（新表；v2 P0）

用途：不可变定义快照。来源：v2 §15/16；v3 §11/12.1。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v2 §15/16；v3 §11/12.1 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v2 §15/16；v3 §11/12.1 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | pipeline/workflow | v2 §15/16；v3 §11/12.1 |
| `subject_id` | BIGINT | 是 | 无 | 见说明/枚举 | 定义ID | v2 §15/16；v3 §11/12.1 |
| `number` | BIGINT | 是 | 无 | 见说明/枚举 | 同定义递增 | v2 §15/16；v3 §11/12.1 |
| `definition` | TEXT | 是 | 无 | 见说明/枚举 | 规范化信封；YAML原文字节比较 | v2 §15/16；v3 §11/12.1 |
| `message` | VARCHAR(200) | 否 | 无 | length(message)<=200 | 可选说明≤200字符 | v2 §15/16；v3 §11/12.1 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 0=system | 用户时间审计清单；v2 §15/16；v3 §11/12.1 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §15/16；v3 §11/12.1 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| definition_revisions_number | subject_kind, subject_id, number | 是 | 唯一键/外键替代 |

删除政策与保留：随历史保留期清理；被执行引用或最近50版时不删除。

限制与缺口：发布版本名物理列/唯一键、发布说明与message关系：源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "definition_revisions" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "subject_kind" VARCHAR(16) NOT NULL,
  "subject_id" BIGINT NOT NULL,
  "number" BIGINT NOT NULL,
  "definition" TEXT NOT NULL,
  "message" VARCHAR(200) CHECK (length(message)<=200),
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "created_at" BIGINT NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "definition_revisions_number" ON "definition_revisions" ("subject_kind", "subject_id", "number");
```

### workflows（新表；S0）

用途：定义及草稿/发布指针。来源：v3 §12.1；ADR Q12/Q14。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v3 §12.1；ADR Q12/Q14 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v3 §12.1；ADR Q12/Q14 |
| `name` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；ADR Q12/Q14 |
| `group_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；ADR Q12/Q14 |
| `description` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；ADR Q12/Q14 |
| `definition` | TEXT | 是 | 无 | 见说明/枚举 | 草稿图JSON | v3 §12.1；ADR Q12/Q14 |
| `draft_revision_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；ADR Q12/Q14 |
| `published_revision_id` | BIGINT | 否 | 无 | 见说明/枚举 | 未发布空值编码待补 | v3 §12.1；ADR Q12/Q14 |
| `settings` | TEXT | 是 | 无 | 见说明/枚举 | save_data/max_cost等设置JSON | v3 §12.1；ADR Q12/Q14 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | created_*展开 | 用户时间审计清单；v3 §12.1；ADR Q12/Q14 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1；ADR Q12/Q14 |
| `updated_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1；ADR Q12/Q14 |
| `updated_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1；ADR Q12/Q14 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：物理删 + 引用保护；仅归档后删除，归档保留历史，不是soft delete。

限制与缺口：归档列/值、group_id目标、trigger_run_as正式列声明、名称长度：源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "workflows" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "name" VARCHAR(255) NOT NULL,
  "group_id" BIGINT NOT NULL,
  "description" TEXT NOT NULL,
  "definition" TEXT NOT NULL,
  "draft_revision_id" BIGINT NOT NULL,
  "published_revision_id" BIGINT,
  "settings" TEXT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "created_at" BIGINT NOT NULL,
  "updated_at" BIGINT NOT NULL,
  "updated_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### workflow_executions（新表；S0）

用途：实例、队列及谱系。来源：v3 §12.1；v2 §8/16。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v3 §12.1；v2 §8/16 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v3 §12.1；v2 §8/16 |
| `workflow_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `revision_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `number` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `mode` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | manual/test/production/retry/webhook_sync | v3 §12.1；v2 §8/16 |
| `state` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `trigger_kind` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | 扩展字符串码待补 | v3 §12.1；v2 §8/16 |
| `trigger_ref` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `triggered_by` | BIGINT | 是 | 无 | 见说明/枚举 | 公开提交者不是执行身份 | 用户时间审计清单；v3 §12.1；v2 §8/16 |
| `parent_execution_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `retry_of_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `root_execution_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `cost_micros` | BIGINT | 是 | 无 | 见说明/枚举 | 不同币种不能相加 | v3 §12.1；v2 §8/16 |
| `data_bytes` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1；v2 §8/16 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1；v2 §8/16 |
| `started_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1；v2 §8/16 |
| `started_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1；v2 §8/16 |
| `finished_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1；v2 §8/16 |
| `finished_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1；v2 §8/16 |
| `lease_owner` | VARCHAR(128) | 是 | '' | 见说明/枚举 | instance_id:boot_nonce | v3 §12.1；v2 §8/16 |
| `lease_epoch` | BIGINT | 是 | 0 | 见说明/枚举 | 写栅栏，接管递增 | v3 §12.1；v2 §8/16 |
| `priority` | BIGINT | 是 | 0 | 见说明/枚举 | 普通0，唤醒1 | v3 §12.1；v2 §8/16 |
| `not_before` | BIGINT | 否 | 无 | 见说明/枚举 | 调度下界 | v3 §12.1；v2 §8/16 |
| `end_reason` | VARCHAR(64) | 是 | '' | 见说明/枚举 | 见枚举 | v3 §12.1；v2 §8/16 |
| `concurrency_key` | VARCHAR(128) | 是 | '' | 见说明/枚举 | ref部分取哈希 | v3 §12.1；v2 §8/16 |
| `concurrency_limit` | BIGINT | 是 | 0 | 见说明/枚举 | 创建时冻结 | v3 §12.1；v2 §8/16 |
| `queue_reason` | VARCHAR(255) | 是 | '' | 见说明/枚举 | 排队原因 | v3 §12.1；v2 §8/16 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| workflow_executions_dispatch | state, priority, created_at | 否 | 高频查询 |
| workflow_executions_workflow | workspace_id, workflow_id, created_at | 否 | 高频查询 |
| workflow_executions_number | workspace_id, number | 是 | 唯一键/外键替代 |

删除政策与保留：随执行历史保留期清理；运行中输出持久化，终态才按save_data清理。

限制与缺口：等待累计/seed/清理标记/同步回应未送达/预算预留、父链空值码源未定义，待补。created_at由索引确认，开始结束依现有命名映射。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "workflow_executions" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "workflow_id" BIGINT NOT NULL,
  "revision_id" BIGINT NOT NULL,
  "number" BIGINT NOT NULL,
  "mode" VARCHAR(32) NOT NULL,
  "state" VARCHAR(32) NOT NULL,
  "trigger_kind" VARCHAR(32) NOT NULL,
  "trigger_ref" VARCHAR(64) NOT NULL,
  "triggered_by" BIGINT NOT NULL,
  "parent_execution_id" BIGINT NOT NULL,
  "retry_of_id" BIGINT NOT NULL,
  "root_execution_id" BIGINT NOT NULL,
  "cost_micros" BIGINT NOT NULL,
  "data_bytes" BIGINT NOT NULL,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "started_at" BIGINT NOT NULL,
  "started_by" BIGINT NOT NULL DEFAULT 0,
  "finished_at" BIGINT NOT NULL,
  "finished_by" BIGINT NOT NULL DEFAULT 0,
  "lease_owner" VARCHAR(128) NOT NULL DEFAULT '',
  "lease_epoch" BIGINT NOT NULL DEFAULT 0,
  "priority" BIGINT NOT NULL DEFAULT 0,
  "not_before" BIGINT,
  "end_reason" VARCHAR(64) NOT NULL DEFAULT '',
  "concurrency_key" VARCHAR(128) NOT NULL DEFAULT '',
  "concurrency_limit" BIGINT NOT NULL DEFAULT 0,
  "queue_reason" VARCHAR(255) NOT NULL DEFAULT '',
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX "workflow_executions_dispatch" ON "workflow_executions" ("state", "priority", "created_at");
CREATE INDEX "workflow_executions_workflow" ON "workflow_executions" ("workspace_id", "workflow_id", "created_at");
CREATE UNIQUE INDEX "workflow_executions_number" ON "workflow_executions" ("workspace_id", "number");
```

### workflow_node_runs（新表；S1）

用途：节点轮次尝试结果。来源：v3 §12.1、§18.7/18.9/18.14。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v3 §12.1、§18.7/18.9/18.14 |
| `execution_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `node_id` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `iteration_path` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `iteration_hash` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | iteration_path哈希 | v3 §12.1、§18.7/18.9/18.14 |
| `attempt` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `state` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `items_in` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `items_out` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `output` | TEXT | 是 | 无 | 见说明/枚举 | vault加密，AAD绑定执行；≤64KiB内联，超过转资产 | v3 §12.1、§18.7/18.9/18.14 |
| `output_asset` | BIGINT | 否 | 无 | 见说明/枚举 | 资产引用 | v3 §12.1、§18.7/18.9/18.14 |
| `error` | TEXT | 是 | 无 | 见说明/枚举 | 脱敏 | v3 §12.1、§18.7/18.9/18.14 |
| `cache_key` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `cache_hit_from` | BIGINT | 否 | 无 | 见说明/枚举 | 命中来源 | v3 §12.1、§18.7/18.9/18.14 |
| `cost_micros` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.7/18.9/18.14 |
| `started_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1、§18.7/18.9/18.14 |
| `started_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1、§18.7/18.9/18.14 |
| `finished_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1、§18.7/18.9/18.14 |
| `finished_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1、§18.7/18.9/18.14 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| workflow_node_runs_attempt | execution_id, node_id, iteration_hash, attempt | 是 | 唯一键/外键替代 |

删除政策与保留：随执行数据策略清理；过程不可清；清理后仅全部重跑。

限制与缺口：完整状态码、redo_round/seed具体列源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "workflow_node_runs" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "execution_id" BIGINT NOT NULL,
  "node_id" VARCHAR(255) NOT NULL,
  "iteration_path" TEXT NOT NULL,
  "iteration_hash" VARCHAR(64) NOT NULL,
  "attempt" BIGINT NOT NULL,
  "state" VARCHAR(32) NOT NULL,
  "items_in" BIGINT NOT NULL,
  "items_out" BIGINT NOT NULL,
  "output" TEXT NOT NULL,
  "output_asset" BIGINT,
  "error" TEXT NOT NULL,
  "cache_key" VARCHAR(64) NOT NULL,
  "cache_hit_from" BIGINT,
  "cost_micros" BIGINT NOT NULL,
  "started_at" BIGINT NOT NULL,
  "started_by" BIGINT NOT NULL DEFAULT 0,
  "finished_at" BIGINT NOT NULL,
  "finished_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "workflow_node_runs_attempt" ON "workflow_node_runs" ("execution_id", "node_id", "iteration_hash", "attempt");
```

### workflow_execution_logs（新表；S1）

用途：图日志分片。来源：v3 §12.1；v2 §8/16；entities.rs。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v3 §12.1；v2 §8/16；entities.rs |
| `execution_id` | BIGINT | 是 | 无 | 见说明/枚举 | v3执行ID映射 | v3 §12.1；v2 §8/16；entities.rs |
| `seq` | BIGINT | 是 | 无 | 见说明/枚举 | 接管MAX(seq)+1 | v3 §12.1；v2 §8/16；entities.rs |
| `step_index` | BIGINT | 是 | 无 | 见说明/枚举 | 沿用v2日志结构 | v3 §12.1；v2 §8/16；entities.rs |
| `stream` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | 流枚举源未定义，待补 | v3 §12.1；v2 §8/16；entities.rs |
| `chunk` | TEXT | 是 | 无 | 见说明/枚举 | 加密存储；展示/导出脱敏 | v3 §12.1；v2 §8/16；entities.rs |
| `job_key` | VARCHAR(255) | 是 | '' | 见说明/枚举 | v2新结构 | v3 §12.1；v2 §8/16；entities.rs |
| `attempt` | BIGINT | 是 | 1 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16；entities.rs |
| `node_id` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16；entities.rs |
| `iteration_hash` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1；v2 §8/16；entities.rs |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| workflow_execution_logs_seq | execution_id, seq | 是 | 唯一键/外键替代 |

删除政策与保留：随执行日志保留期清理。

限制与缺口：v3称同v2结构；是否裁剪step_index/job_key源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "workflow_execution_logs" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "execution_id" BIGINT NOT NULL,
  "seq" BIGINT NOT NULL,
  "step_index" BIGINT NOT NULL,
  "stream" VARCHAR(32) NOT NULL,
  "chunk" TEXT NOT NULL,
  "job_key" VARCHAR(255) NOT NULL DEFAULT '',
  "attempt" BIGINT NOT NULL DEFAULT 1,
  "node_id" VARCHAR(255) NOT NULL,
  "iteration_hash" VARCHAR(64) NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "workflow_execution_logs_seq" ON "workflow_execution_logs" ("execution_id", "seq");
```

### node_cache（新表；S1）

用途：逐条成功输出缓存。来源：v3 §12.1、§18.9。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `key_hash` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.9 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v3 §12.1、§18.9 |
| `node_type` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.9 |
| `outputs` | TEXT | 是 | 无 | 见说明/枚举 | 成功输出或资产引用；失败不缓存 | v3 §12.1、§18.9 |
| `size` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.9 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1、§18.9 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1、§18.9 |
| `last_used_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1、§18.9 |
| `last_used_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1、§18.9 |
| `source_execution_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.9 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| node_cache_key | key_hash | 是 | 唯一键/外键替代 |

删除政策与保留：物理删 + 引用保护；手动清缓存释放引用；自动保留期源未定义，待补。

限制与缺口：按工作流清缓存的归属查询字段源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "node_cache" (
  "key_hash" VARCHAR(64) NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "node_type" VARCHAR(255) NOT NULL,
  "outputs" TEXT NOT NULL,
  "size" BIGINT NOT NULL,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "last_used_at" BIGINT NOT NULL,
  "last_used_by" BIGINT NOT NULL DEFAULT 0,
  "source_execution_id" BIGINT NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "node_cache_key" ON "node_cache" ("key_hash");
```

### assets（新表；S0）

用途：内容寻址资产库。来源：v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `sha256` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `size` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `mime` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `kind` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | image/video/audio/text/file/model；S5 model3d | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `width` | BIGINT | 否 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `height` | BIGINT | 否 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `duration_ms` | BIGINT | 否 | 无 | 见说明/枚举 | 产出时回传 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `thumbnail_asset_id` | BIGINT | 否 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `execution_id` | BIGINT | 否 | 无 | 见说明/枚举 | 来源执行 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `node_id` | VARCHAR(255) | 否 | 无 | 见说明/枚举 | 来源节点 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `pinned` | BIGINT | 是 | 无 | 见说明/枚举 | 布尔0/1 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1资产字段引用；PRD FR-ASSET-01/02/03/04 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| assets_sha256 | workspace_id, sha256 | 是 | 唯一键/外键替代 |

删除政策与保留：物理删 + 引用保护；执行引用随history_retention_days释放；固定不清；无引用标记后默认24小时复查再删。

限制与缺口：GC标记列/表、来源执行清理后固定资产参数追溯载体源未定义，待补，不造gc_at。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "assets" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "sha256" VARCHAR(64) NOT NULL,
  "size" BIGINT NOT NULL,
  "mime" VARCHAR(255) NOT NULL,
  "kind" VARCHAR(32) NOT NULL,
  "width" BIGINT,
  "height" BIGINT,
  "duration_ms" BIGINT,
  "thumbnail_asset_id" BIGINT,
  "execution_id" BIGINT,
  "node_id" VARCHAR(255),
  "pinned" BIGINT NOT NULL,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "assets_sha256" ON "assets" ("workspace_id", "sha256");
```

### asset_refs（新表；S0）

用途：资产引用保护。来源：v3 §12.1规范性唯一键/两阶段GC。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `asset_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1规范性唯一键/两阶段GC |
| `owner_kind` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | execution/node_run/cache/pin/user_pin | v3 §12.1规范性唯一键/两阶段GC |
| `owner_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1规范性唯一键/两阶段GC |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| asset_refs_owner | asset_id, owner_kind, owner_id | 是 | 唯一键/外键替代 |

删除政策与保留：物理删 + 引用保护；拥有者释放引用，不按资产年龄盲删；无独立保留期。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "asset_refs" (
  "asset_id" BIGINT NOT NULL,
  "owner_kind" VARCHAR(32) NOT NULL,
  "owner_id" BIGINT NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "asset_refs_owner" ON "asset_refs" ("asset_id", "owner_kind", "owner_id");
```

### asset_tags（新表；S0）

用途：资产标签。来源：v3 §12.1资产字段引用。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `asset_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1资产字段引用 |
| `tag` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 产品长度源未定义，待补 | v3 §12.1资产字段引用 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| asset_tags_tag | tag | 否 | 高频查询 |

删除政策与保留：物理删 + 引用保护；随资产清理；无独立保留期。

限制与缺口：源仅说可索引，未声明(asset_id,tag)唯一，不擅加唯一键。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "asset_tags" (
  "asset_id" BIGINT NOT NULL,
  "tag" VARCHAR(255) NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX "asset_tags_tag" ON "asset_tags" ("tag");
```

### trigger_bindings（新表；S0结构/P1规则）

用途：保存事务内重建的派生索引。来源：v2 §9.2/16；v3 §11/12.1/18.16。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v2 §9.2/16；v3 §11/12.1/18.16 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v2 §9.2/16；v3 §11/12.1/18.16 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | pipeline/workflow | v2 §9.2/16；v3 §11/12.1/18.16 |
| `pipeline_id` | BIGINT | 是 | 无 | 见说明/枚举 | 泛化见缺口 | v2 §9.2/16；v3 §11/12.1/18.16 |
| `kind` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | push/pull_request/tag | v2 §9.2/16；v3 §11/12.1/18.16 |
| `connection_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §9.2/16；v3 §11/12.1/18.16 |
| `repository_key` | TEXT | 是 | 无 | 见说明/枚举 | 展示复核 | v2 §9.2/16；v3 §11/12.1/18.16 |
| `repository_hash` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | sha256(repository_key) | v2 §9.2/16；v3 §11/12.1/18.16 |
| `source_id` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §9.2/16；v3 §11/12.1/18.16 |
| `filters` | TEXT | 是 | 无 | 见说明/枚举 | branches/tags/paths/types JSON | v2 §9.2/16；v3 §11/12.1/18.16 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §9.2/16；v3 §11/12.1/18.16 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §9.2/16；v3 §11/12.1/18.16 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| trigger_bindings_lookup | workspace_id, connection_id, repository_hash, kind | 否 | 高频查询 |

删除政策与保留：物理删 + 引用保护；定义保存先删后插；生产检查已发布未归档。

限制与缺口：v3仅声明增subject_kind，未明确pipeline_id改subject_id，工作流键承载源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "trigger_bindings" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "subject_kind" VARCHAR(16) NOT NULL,
  "pipeline_id" BIGINT NOT NULL,
  "kind" VARCHAR(32) NOT NULL,
  "connection_id" BIGINT NOT NULL,
  "repository_key" TEXT NOT NULL,
  "repository_hash" VARCHAR(64) NOT NULL,
  "source_id" VARCHAR(255) NOT NULL,
  "filters" TEXT NOT NULL,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX "trigger_bindings_lookup" ON "trigger_bindings" ("workspace_id", "connection_id", "repository_hash", "kind");
```

### trigger_deliveries（新表；S0结构/P1规则）

用途：发件箱及手工重投。来源：v2 §9.3b/16；v3 §11/12.1/18.16。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `pipeline_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `trigger_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `event_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `event` | TEXT | 是 | 无 | 见说明/枚举 | 加密事件；冻结merge commit | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `dedupe_key` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | sha256(repository_key\|kind\|git_ref\|after\|pr_action) | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `redelivery_seq` | BIGINT | 是 | 0 | 见说明/枚举 | 重投MAX+1 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `state` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | pending/created/skipped/failed | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `reason` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `run_id` | BIGINT | 否 | 无 | 见说明/枚举 | 结果执行 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §9.3b/16；v3 §11/12.1/18.16 |
| `updated_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §9.3b/16；v3 §11/12.1/18.16 |
| `updated_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §9.3b/16；v3 §11/12.1/18.16 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| trigger_deliveries_dedupe | pipeline_id, dedupe_key, redelivery_seq | 是 | 唯一键/外键替代 |
| trigger_deliveries_dispatch | state, created_at | 否 | 高频查询 |
| trigger_deliveries_history | workspace_id, pipeline_id, created_at | 否 | 高频查询 |

删除政策与保留：随保留期清理；重投新行原记录不变，不自动重试。

限制与缺口：pipeline_id/run_id泛化、唯一键是否含subject_kind及期限源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "trigger_deliveries" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "subject_kind" VARCHAR(16) NOT NULL,
  "pipeline_id" BIGINT NOT NULL,
  "trigger_id" BIGINT NOT NULL,
  "event_id" BIGINT NOT NULL,
  "event" TEXT NOT NULL,
  "dedupe_key" VARCHAR(64) NOT NULL,
  "redelivery_seq" BIGINT NOT NULL DEFAULT 0,
  "state" VARCHAR(32) NOT NULL,
  "reason" TEXT NOT NULL,
  "run_id" BIGINT,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "updated_at" BIGINT NOT NULL,
  "updated_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "trigger_deliveries_dedupe" ON "trigger_deliveries" ("pipeline_id", "dedupe_key", "redelivery_seq");
CREATE INDEX "trigger_deliveries_dispatch" ON "trigger_deliveries" ("state", "created_at");
CREATE INDEX "trigger_deliveries_history" ON "trigger_deliveries" ("workspace_id", "pipeline_id", "created_at");
```

### human_tasks（新表；S0结构/P1核心/S1交互）

用途：人工任务表单与外审入口。来源：v2 §13/16；v3 §11/12.1/18.16；ADR Q08。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `run_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `job_key` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `attempt` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `kind` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | 审批/表单/挑选/编辑；英文码待补 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `form_schema` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `external_token_hash` | VARCHAR(64) | 否 | 无 | 见说明/枚举 | 只存摘要 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `expires_at` | BIGINT | 否 | 无 | 见说明/枚举 | 外审默认72小时；任务默认7天超时另属规则 | v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `expires_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §13/16；v3 §11/12.1/18.16；ADR Q08 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：随执行人工历史保留期清理；过期/撤销/已用不可再提交。

限制与缺口：状态/指派/截止/撤销已用/编辑草稿/回应关联键/轮次键/外审幂等键源未定义，待补；拆表字段分配需确认。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "human_tasks" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "subject_kind" VARCHAR(16) NOT NULL,
  "run_id" BIGINT NOT NULL,
  "job_key" VARCHAR(255) NOT NULL,
  "attempt" BIGINT NOT NULL,
  "kind" VARCHAR(32) NOT NULL,
  "form_schema" TEXT NOT NULL,
  "external_token_hash" VARCHAR(64),
  "expires_at" BIGINT,
  "expires_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### human_responses（新表；S0结构/P1核心/S1交互）

用途：人工决策及加密回应。来源：v2 §13.1/16；v3 §11/12.1；ADR review defaults。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `run_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `job_key` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `attempt` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `user_id` | BIGINT | 是 | 无 | 见说明/枚举 | 超时自动通过0 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `user_name` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `decision` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | approve/reject/returned；其它码待补 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `comment` | VARCHAR(500) | 否 | 无 | (comment IS NULL OR length(comment)<=500) AND (decision NOT IN ('reject','returned') OR (comment IS NOT NULL AND length(comment)>=1)) | ≤500字符；reject/returned必填 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `response` | TEXT | 否 | 无 | 见说明/枚举 | 加密；不覆盖原稿 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §13.1/16；v3 §11/12.1；ADR review defaults |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| human_responses_once | run_id, job_key, attempt, user_id | 是 | 唯一键/外键替代 |

删除政策与保留：随执行人工历史保留期清理；回应与状态推进同事务，提交后唤醒。

限制与缺口：拆分关联键未明确，原唯一键缺subject_kind/iteration，外审统一user_id=0不能保证一次性；通用键源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "human_responses" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "subject_kind" VARCHAR(16) NOT NULL,
  "run_id" BIGINT NOT NULL,
  "job_key" VARCHAR(255) NOT NULL,
  "attempt" BIGINT NOT NULL,
  "user_id" BIGINT NOT NULL,
  "user_name" VARCHAR(255) NOT NULL,
  "decision" VARCHAR(32) NOT NULL,
  "comment" VARCHAR(500) CHECK ((comment IS NULL OR length(comment)<=500) AND (decision NOT IN ('reject','returned') OR (comment IS NOT NULL AND length(comment)>=1))),
  "response" TEXT,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "human_responses_once" ON "human_responses" ("run_id", "job_key", "attempt", "user_id");
```

### execution_intents（新表；S0结构/P1核心/S1使用）

用途：逐节点逐轮次逐条目的副作用意图。来源：v2 §8.5/16；v3 §18.8。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v2 §8.5/16；v3 §18.8 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v2 §8.5/16；v3 §18.8 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8.5/16；v3 §18.8 |
| `execution_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8.5/16；v3 §18.8 |
| `node_key` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 流水线job_key或图node_id | v2 §8.5/16；v3 §18.8 |
| `iteration_hash` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8.5/16；v3 §18.8 |
| `item_index` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8.5/16；v3 §18.8 |
| `attempt` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8.5/16；v3 §18.8 |
| `kind` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 完整副作用类型码源未定义，待补 | v2 §8.5/16；v3 §18.8 |
| `state` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | pending/started/done/unknown | v2 §8.5/16；v3 §18.8 |
| `external_id` | VARCHAR(255) | 是 | '' | 见说明/枚举 | 下游ID | v2 §8.5/16；v3 §18.8 |
| `idempotency_key` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 构造及上限待补 | v2 §8.5/16；v3 §18.8 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §8.5/16；v3 §18.8 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §8.5/16；v3 §18.8 |
| `updated_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §8.5/16；v3 §18.8 |
| `updated_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §8.5/16；v3 §18.8 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| execution_intents_once | subject_kind, execution_id, node_key, iteration_hash, item_index, attempt | 是 | 唯一键/外键替代 |

删除政策与保留：随执行意图历史保留期清理；不得早于下游幂等保护期；期限源未定义，待补。

限制与缺口：非循环/非逐条的iteration_hash/item_index空值编码、kind全集、idempotency_key构造源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "execution_intents" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "subject_kind" VARCHAR(16) NOT NULL,
  "execution_id" BIGINT NOT NULL,
  "node_key" VARCHAR(255) NOT NULL,
  "iteration_hash" VARCHAR(64) NOT NULL,
  "item_index" BIGINT NOT NULL,
  "attempt" BIGINT NOT NULL,
  "kind" VARCHAR(64) NOT NULL,
  "state" VARCHAR(16) NOT NULL,
  "external_id" VARCHAR(255) NOT NULL DEFAULT '',
  "idempotency_key" VARCHAR(255) NOT NULL,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "updated_at" BIGINT NOT NULL,
  "updated_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "execution_intents_once" ON "execution_intents" ("subject_kind", "execution_id", "node_key", "iteration_hash", "item_index", "attempt");
```

### execution_parents（新表；P1结构/P2与S1使用）

用途：子执行幂等关联，替代部分唯一索引。来源：v2 §8.5/16；v3 §18.8。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `intent_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8.5/16；v3 §18.8 |
| `child_execution_id` | BIGINT | 是 | 无 | 见说明/枚举 | v3明确替代run_id | v2 §8.5/16；v3 §18.8 |
| `parent_run_id` | BIGINT | 是 | 无 | 见说明/枚举 | 继承v2字段，通用化待补 | v2 §8.5/16；v3 §18.8 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| execution_parents_intent | intent_id | 是 | 唯一键/外键替代 |

删除政策与保留：物理删 + 引用保护；不得早于意图幂等保护期；独立期限源未定义，待补。

限制与缺口：parent_run_id的工作流泛化、父子类型判别字段源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "execution_parents" (
  "intent_id" BIGINT NOT NULL,
  "child_execution_id" BIGINT NOT NULL,
  "parent_run_id" BIGINT NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "execution_parents_intent" ON "execution_parents" ("intent_id");
```

### pipeline_run_notifications（新表；P1）

用途：通知至多一次账本。来源：v2 §14/16；v3 §11共用服务。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v2 §14/16；v3 §11共用服务 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v2 §14/16；v3 §11共用服务 |
| `run_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §14/16；v3 §11共用服务 |
| `job_key` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 执行级为空串 | v2 §14/16；v3 §11共用服务 |
| `event` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | started/waiting/succeeded/failed/cancelled | v2 §14/16；v3 §11共用服务 |
| `channel_index` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §14/16；v3 §11共用服务 |
| `state` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | sending/sent/failed/unknown | v2 §14/16；v3 §11共用服务 |
| `error` | TEXT | 是 | 无 | 见说明/枚举 | 不存响应体/密钥/地址 | v2 §14/16；v3 §11共用服务 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §14/16；v3 §11共用服务 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §14/16；v3 §11共用服务 |
| `updated_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §14/16；v3 §11共用服务 |
| `updated_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §14/16；v3 §11共用服务 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| pipeline_run_notifications_once | run_id, job_key, event, channel_index | 是 | 唯一键/外键替代 |

删除政策与保留：随执行通知历史保留期清理；接管sending改unknown不重发。

限制与缺口：源仅说共用服务，未定义工作流通知表改名或subject_kind，工作流投递归属源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "pipeline_run_notifications" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "run_id" BIGINT NOT NULL,
  "job_key" VARCHAR(255) NOT NULL,
  "event" VARCHAR(32) NOT NULL,
  "channel_index" BIGINT NOT NULL,
  "state" VARCHAR(16) NOT NULL,
  "error" TEXT NOT NULL,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "updated_at" BIGINT NOT NULL,
  "updated_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "pipeline_run_notifications_once" ON "pipeline_run_notifications" ("run_id", "job_key", "event", "channel_index");
```

### pipeline_run_reports（新表；v2 P2）

用途：任务尝试报告。来源：v2 §16。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v2 §16 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v2 §16 |
| `run_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §16 |
| `job_key` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §16 |
| `attempt` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v2 §16 |
| `kind` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | 具体报告类型码源未定义，待补 | v2 §16 |
| `totals` | TEXT | 是 | 无 | 见说明/枚举 | 聚合JSON | v2 §16 |
| `cases` | TEXT | 是 | 无 | 见说明/枚举 | 用例JSON | v2 §16 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v2 §16 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v2 §16 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| pipeline_run_reports_run | run_id | 否 | 高频查询 |

删除政策与保留：随执行报告保留期清理；独立期限源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "pipeline_run_reports" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "run_id" BIGINT NOT NULL,
  "job_key" VARCHAR(255) NOT NULL,
  "attempt" BIGINT NOT NULL,
  "kind" VARCHAR(32) NOT NULL,
  "totals" TEXT NOT NULL,
  "cases" TEXT NOT NULL,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX "pipeline_run_reports_run" ON "pipeline_run_reports" ("run_id");
```

### cost_records（新表；S2）

用途：逐节点分币种成本明细。来源：v3 §12.1成本字段引用；§18.13；ADR Q11。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `execution_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `node_id` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `provider` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `model` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `unit` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | input_tokens/output_tokens/seconds/images/video_seconds/characters等 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `quantity` | BIGINT | 是 | 无 | 见说明/枚举 | 小数计量尺度源未定义，待补 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `amount_micros` | BIGINT | 是 | 无 | 见说明/枚举 | 金额微单位 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `currency` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | 凭据配置币种，不换汇 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `created_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `created_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1成本字段引用；§18.13；ADR Q11 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| cost_records_workspace | workspace_id, created_at | 否 | 高频查询 |
| cost_records_execution | execution_id | 否 | 高频查询 |

删除政策与保留：随成本历史保留期清理；预算跨保留期边界源未定义，待补。

限制与缺口：独立主键/credential关联/价格表/预算预留结算释放、quantity小数尺度源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "cost_records" (
  "workspace_id" BIGINT NOT NULL,
  "execution_id" BIGINT NOT NULL,
  "node_id" VARCHAR(255) NOT NULL,
  "provider" VARCHAR(255) NOT NULL,
  "model" VARCHAR(255) NOT NULL,
  "unit" VARCHAR(32) NOT NULL,
  "quantity" BIGINT NOT NULL,
  "amount_micros" BIGINT NOT NULL,
  "currency" VARCHAR(32) NOT NULL,
  "created_at" BIGINT NOT NULL,
  "created_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX "cost_records_workspace" ON "cost_records" ("workspace_id", "created_at");
CREATE INDEX "cost_records_execution" ON "cost_records" ("execution_id");
```

### data_tables（新表；S2）

用途：内置表列定义。来源：v3 §12.1。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：删除方式、引用保护与期限源未定义，待补。

限制与缺口：列定义JSON的物理列、ID/工作空间键/名称、索引槽类型元数据源未定义，待补；version占位DDL不是完整业务表。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "data_tables" (
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### data_table_rows（新表；S2）

用途：行数据与最多5个类型化索引槽。来源：v3 §12.1。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |
| `i1..i5`（5列，未定型，不计已定字段） | 字符串或数字，源未定义，待补 | 源未定义，待补 | 源未定义，待补 | 最多5个类型化索引槽 | 每槽类型确定后才能给DDL | v3 §12.1 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：删除方式、引用保护与期限源未定义，待补。

限制与缺口：i1..i5已命名但槽类型/类型切换/JSON列名/行ID/所属表键源未定义，待补，不能全部TEXT冒充数字索引；每表≤100000行。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "data_table_rows" (
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### vector_collections（新表；S2）

用途：向量集合。来源：v3 §12.1。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：删除方式、引用保护与期限源未定义，待补。

限制与缺口：集合ID/工作空间键/维度/距离度量/名称等字段源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "vector_collections" (
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### vector_items（新表；S2）

用途：集合嵌入向量。来源：v3 §12.1。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：删除方式、引用保护与期限源未定义，待补。

限制与缺口：二进制long存储的列名/编码/集合键/主键/维度/元数据源未定义，待补；SQLite/MySQL每集合≤20000条，进程缓存暴力计算；PG可选pgvector无此上限。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "vector_items" (
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### models（新表；S4）

用途：模型来源与许可证。来源：v3 §12.1模型字段引用；PRD FR-GPU-01/03。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | PRIMARY KEY | 雪花ID主键 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `name` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `family` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 待V6选型，不封闭枚举 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `kind` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | checkpoint/lora/vae/controlnet/upscaler/tts/llm | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `source_url` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `sha256` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `size` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `license_id` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | SPDX或custom | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `license_url` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `commercial_use` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | allowed/restricted/forbidden；必填 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `restrictions` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `notes` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-01/03 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| PRIMARY KEY | id | 是 | 唯一键 |
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：物理删 + 引用保护；权重由主机直连上游；记录/本地权重删除顺序与保留期源未定义，待补。

限制与缺口：非allowed首次使用确认具体许可并写审计；审计载荷结构源未定义，待补，不造许可确认表。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "models" (
  "id" BIGINT PRIMARY KEY NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "name" VARCHAR(255) NOT NULL,
  "family" VARCHAR(255) NOT NULL,
  "kind" VARCHAR(32) NOT NULL,
  "source_url" TEXT NOT NULL,
  "sha256" VARCHAR(64) NOT NULL,
  "size" BIGINT NOT NULL,
  "license_id" VARCHAR(255) NOT NULL,
  "license_url" TEXT NOT NULL,
  "commercial_use" VARCHAR(16) NOT NULL,
  "restrictions" TEXT NOT NULL,
  "notes" TEXT NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### host_models（新表；S4）

用途：主机模型状态。来源：v3 §12.1模型字段引用；PRD FR-GPU-02/04。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `host_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-02/04 |
| `model_id` | BIGINT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-02/04 |
| `state` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | absent/downloading/ready/failed | v3 §12.1模型字段引用；PRD FR-GPU-02/04 |
| `path` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1模型字段引用；PRD FR-GPU-02/04 |
| `verified_at` | BIGINT | 是 | 无 | 见说明/枚举 | 时间戳；单位沿用项目契约 | v3 §12.1模型字段引用；PRD FR-GPU-02/04 |
| `verified_by` | BIGINT | 是 | 0 | 见说明/枚举 | 时间审计，0=system | 用户时间审计清单；v3 §12.1模型字段引用；PRD FR-GPU-02/04 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：物理删 + 引用保护；解绑/本地文件清理与期限源未定义，待补。

限制与缺口：来源未声明(host_id,model_id)主键或唯一键，不擅加；进度/失败原因列源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "host_models" (
  "host_id" BIGINT NOT NULL,
  "model_id" BIGINT NOT NULL,
  "state" VARCHAR(16) NOT NULL,
  "path" TEXT NOT NULL,
  "verified_at" BIGINT NOT NULL,
  "verified_by" BIGINT NOT NULL DEFAULT 0,
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### execution_annotations（新表；S1）

用途：执行标注及搜索。来源：v3 §18.3。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `key` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | SQL引用保留字 | v3 §18.3 |
| `value` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §18.3 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 来源未指定；不能据此宣称查询已优化 |

删除政策与保留：删除方式与期限源未定义，待补。

限制与缺口：执行/工作空间归属键、唯一语义及value可索引表示源未定义，待补；不对TEXT建索引伪称搜索完成。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "execution_annotations" (
  "key" VARCHAR(255) NOT NULL,
  "value" TEXT NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
```

### webhook_endpoints（新表；S1）

用途：稳定随机入口与安全配置。来源：v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `endpoint_id` | VARCHAR(255) | 是 | 无 | 见说明/枚举 | 随机串唯一，长度源未定义，待补 | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `workspace_id` | BIGINT | 是 | 无 | 见说明/枚举 | 工作空间隔离 | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | pipeline/workflow | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `subject_id` | BIGINT | 是 | 无 | 见说明/枚举 | 定义ID | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `mode` | VARCHAR(16) | 是 | 无 | 见说明/枚举 | test/production | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `method` | VARCHAR(32) | 是 | 无 | 见说明/枚举 | HTTP方法允许集合待补 | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `path` | TEXT | 是 | 无 | 见说明/枚举 | 自定义路径不索引 | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `auth` | TEXT | 是 | 无 | 见说明/枚举 | none/basic/header/jwt及凭据引用配置 | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `cors` | TEXT | 是 | 无 | 见说明/枚举 | CORS配置 | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `ip_allowlist` | TEXT | 是 | 无 | 见说明/枚举 | 来源字段，无已定默认 | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `respond_mode` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | 立即/最后节点/回应节点；存储码待补 | v3 §12.1、§18.3/18.10；PRD FR-TRIG-02/07 |
| `version` | BIGINT | 是 | 1 | 见说明/枚举 | 项目业务行版本；不允许覆盖不可变历史 | migration.rs约定 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| webhook_endpoints_id | endpoint_id | 是 | 唯一键/外键替代 |
| webhook_endpoints_route | endpoint_id, method, mode | 是 | 唯一键/外键替代 |

删除政策与保留：物理删 + 引用保护；归档/撤销发布仅停生产，不删除入口，重新发布地址稳定；测试仅调试窗口；独立期限源未定义，待补。

限制与缺口：来源同时说endpoint_id唯一及组合唯一；前者使组合在约束上冗余并限制同ID多模式/方法，冲突待确认，不能擅自放宽。认证JSON结构、窗口/超时/同步未送达字段源未定义，待补。

SQLite DDL（已知字段目标投影）：

```sql
CREATE TABLE "webhook_endpoints" (
  "endpoint_id" VARCHAR(255) NOT NULL,
  "workspace_id" BIGINT NOT NULL,
  "subject_kind" VARCHAR(16) NOT NULL,
  "subject_id" BIGINT NOT NULL,
  "mode" VARCHAR(16) NOT NULL,
  "method" VARCHAR(32) NOT NULL,
  "path" TEXT NOT NULL,
  "auth" TEXT NOT NULL,
  "cors" TEXT NOT NULL,
  "ip_allowlist" TEXT NOT NULL,
  "respond_mode" VARCHAR(64) NOT NULL,
  "version" BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX "webhook_endpoints_id" ON "webhook_endpoints" ("endpoint_id");
CREATE UNIQUE INDEX "webhook_endpoints_route" ON "webhook_endpoints" ("endpoint_id", "method", "mode");
```

### pipeline_runs（存量ALTER；v2 P0）

用途：租约、来源与重跑追溯增量。来源：v2 §8/9/16；entities.rs。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `lease_owner` | VARCHAR(128) | 是 | '' | 见说明/枚举 | instance_id:boot_nonce | v2 §8/9/16；entities.rs |
| `lease_epoch` | BIGINT | 是 | 0 | 见说明/枚举 | 写栅栏，接管递增 | v2 §8/9/16；entities.rs |
| `priority` | BIGINT | 是 | 0 | 见说明/枚举 | 普通0，唤醒1 | v2 §8/9/16；entities.rs |
| `not_before` | BIGINT | 否 | 无 | 见说明/枚举 | 调度下界 | v2 §8/9/16；entities.rs |
| `end_reason` | VARCHAR(64) | 是 | '' | 见说明/枚举 | 见枚举 | v2 §8/9/16；entities.rs |
| `concurrency_key` | VARCHAR(128) | 是 | '' | 见说明/枚举 | ref部分取哈希 | v2 §8/9/16；entities.rs |
| `concurrency_limit` | BIGINT | 是 | 0 | 见说明/枚举 | 创建时冻结 | v2 §8/9/16；entities.rs |
| `queue_reason` | VARCHAR(255) | 是 | '' | 见说明/枚举 | 排队原因 | v2 §8/9/16；entities.rs |
| `trigger_ref` | VARCHAR(64) | 是 | '' | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/9/16；entities.rs |
| `trigger_actor` | VARCHAR(255) | 是 | '' | 见说明/枚举 | 平台用户名，不映射Ferry用户 | v2 §8/9/16；entities.rs |
| `revision_id` | BIGINT | 是 | 0 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/9/16；entities.rs |
| `rerun_of_id` | BIGINT | 是 | 0 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/9/16；entities.rs |
| `root_run_id` | BIGINT | 是 | 0 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/9/16；entities.rs |
| `attempt` | BIGINT | 是 | 1 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/9/16；entities.rs |
| `source_refs` | TEXT | 否 | 无 | 见说明/枚举 | 源ref/commit快照 | v2 §8/9/16；entities.rs |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| pipeline_runs_dispatch | state, priority, created_at | 否 | 高频查询 |
| pipeline_runs_concurrency | concurrency_key, state | 否 | 高频查询 |
| pipeline_runs_root | root_run_id | 否 | 高频查询 |
| pipeline_runs_pipeline | workspace_id, pipeline_id, created_at | 否 | 高频查询 |

删除政策与保留：沿用执行历史保留期；重跑复制产物后父执行可独立清理。

限制与缺口：inputs已在entities.rs存在Option<String>，不能重复ADD；加密迁移须核对旧格式，不能重复加密；旧trigger_kind仍manual不猜来源。存量不补version或时间审计。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "pipeline_runs" ADD COLUMN "lease_owner" VARCHAR(128) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_runs" ADD COLUMN "lease_epoch" BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "pipeline_runs" ADD COLUMN "priority" BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "pipeline_runs" ADD COLUMN "not_before" BIGINT;
ALTER TABLE "pipeline_runs" ADD COLUMN "end_reason" VARCHAR(64) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_runs" ADD COLUMN "concurrency_key" VARCHAR(128) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_runs" ADD COLUMN "concurrency_limit" BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "pipeline_runs" ADD COLUMN "queue_reason" VARCHAR(255) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_runs" ADD COLUMN "trigger_ref" VARCHAR(64) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_runs" ADD COLUMN "trigger_actor" VARCHAR(255) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_runs" ADD COLUMN "revision_id" BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "pipeline_runs" ADD COLUMN "rerun_of_id" BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "pipeline_runs" ADD COLUMN "root_run_id" BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "pipeline_runs" ADD COLUMN "attempt" BIGINT NOT NULL DEFAULT 1;
ALTER TABLE "pipeline_runs" ADD COLUMN "source_refs" TEXT;
CREATE INDEX "pipeline_runs_dispatch" ON "pipeline_runs" ("state", "priority", "created_at");
CREATE INDEX "pipeline_runs_concurrency" ON "pipeline_runs" ("concurrency_key", "state");
CREATE INDEX "pipeline_runs_root" ON "pipeline_runs" ("root_run_id");
CREATE INDEX "pipeline_runs_pipeline" ON "pipeline_runs" ("workspace_id", "pipeline_id", "created_at");
```

### hosts（存量ALTER；P0/S4）

用途：执行槽增量。来源：v2 §8.2/16；v3 §12.1。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `pipeline_slots` | BIGINT | 是 | 2 | 见说明/枚举 | 1–32 | v2 §8.2/16；v3 §12.1 |
| `gpu_slots` | BIGINT | 是 | 0 | 见说明/枚举 | S4首报GPU数后设定，用户可改 | v2 §8.2/16；v3 §12.1 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 沿用已有索引 |

删除政策与保留：沿用存量主机物理删及引用保护；无新增保留期。

限制与缺口：GPU槽S4单独迁移，老Runner不报GPU不影响既有行为。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "hosts" ADD COLUMN "pipeline_slots" BIGINT NOT NULL DEFAULT 2;
ALTER TABLE "hosts" ADD COLUMN "gpu_slots" BIGINT NOT NULL DEFAULT 0;
```

### pipeline_run_jobs（存量ALTER；P0/P2）

用途：最新尝试与失败跳过。来源：v2 §8/16；ADR review defaults Q04。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `attempt` | BIGINT | 是 | 1 | 见说明/枚举 | 同一行递增 | v2 §8/16；ADR review defaults Q04 |
| `wait_reason` | VARCHAR(255) | 是 | '' | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/16；ADR review defaults Q04 |
| `end_reason` | VARCHAR(64) | 是 | '' | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/16；ADR review defaults Q04 |
| `skipped_by` | BIGINT | 否 | 无 | 见说明/枚举 | P2跳过操作者；存量无跳过用NULL | 用户时间审计清单；v2 §8/16；ADR review defaults Q04 |
| `reason` | TEXT | 否 | 无 | 见说明/枚举 | P2跳过原因；必填/长度源未定义，待补 | v2 §8/16；ADR review defaults Q04 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 沿用已有索引 |

删除政策与保留：随执行历史保留；skip不增attempt，原失败日志保留。

限制与缺口：skipped_by/reason源于ADR；旧行可空兼容表示仅为评审映射；§13.1条件更新无需version。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "pipeline_run_jobs" ADD COLUMN "attempt" BIGINT NOT NULL DEFAULT 1;
ALTER TABLE "pipeline_run_jobs" ADD COLUMN "wait_reason" VARCHAR(255) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_run_jobs" ADD COLUMN "end_reason" VARCHAR(64) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_run_jobs" ADD COLUMN "skipped_by" BIGINT;
ALTER TABLE "pipeline_run_jobs" ADD COLUMN "reason" TEXT;
```

### pipeline_run_steps（存量ALTER；P0）

用途：步骤按任务尝试隔离。来源：v2 §8/16；migration.rs/entities.rs。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `job_key` | VARCHAR(255) | 是 | '' | 见说明/枚举 | 旧行空串 | v2 §8/16；migration.rs/entities.rs |
| `attempt` | BIGINT | 是 | 1 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/16；migration.rs/entities.rs |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| pipeline_run_steps_attempt_order | run_id, job_key, attempt, step_index | 是 | 唯一键/外键替代 |

删除政策与保留：随执行步骤历史保留期清理。

限制与缺口：先建新唯一键再删旧pipeline_run_steps_order(run_id,step_index)，不推断旧job_key。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "pipeline_run_steps" ADD COLUMN "job_key" VARCHAR(255) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_run_steps" ADD COLUMN "attempt" BIGINT NOT NULL DEFAULT 1;
CREATE UNIQUE INDEX "pipeline_run_steps_attempt_order" ON "pipeline_run_steps" ("run_id", "job_key", "attempt", "step_index");
-- 先确认新唯一键成功，再删除旧键。
DROP INDEX "pipeline_run_steps_order";
```

### pipeline_run_logs（存量ALTER；P0）

用途：尝试过滤增量。来源：v2 §8/16；migration.rs/entities.rs。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `job_key` | VARCHAR(255) | 是 | '' | 见说明/枚举 | 旧行空串 | v2 §8/16；migration.rs/entities.rs |
| `attempt` | BIGINT | 是 | 1 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8/16；migration.rs/entities.rs |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 沿用已有索引 |

删除政策与保留：沿用执行日志保留期。

限制与缺口：保留现有pipeline_run_logs_seq(run_id,seq)唯一键，历史不重编号。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "pipeline_run_logs" ADD COLUMN "job_key" VARCHAR(255) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_run_logs" ADD COLUMN "attempt" BIGINT NOT NULL DEFAULT 1;
```

### pipeline_contexts（存量ALTER；P1）

用途：环境保护。来源：v2 §13/16。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `protection` | TEXT | 否 | 无 | 见说明/枚举 | 仅kind=environment有效；未配置保持旧行为 | v2 §13/16 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 沿用已有索引 |

删除政策与保留：沿用环境物理删 + 引用保护；变更写活动记录。

限制与缺口：保护JSON校验见追溯，不造环境保护新表。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "pipeline_contexts" ADD COLUMN "protection" TEXT;
```

### pipeline_artifacts（存量ALTER；P1-3）

用途：产物尝试与失效。来源：v2 §8.6/16。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `name` | TEXT | 是 | 无 | 见说明/枚举 | Actions从路径解析名称，旧格式取路径 | v2 §8.6/16 |
| `name_hash` | VARCHAR(64) | 是 | 无 | 见说明/枚举 | sha256(name) | v2 §8.6/16 |
| `job_key` | VARCHAR(255) | 是 | '' | 见说明/枚举 | 来源字段，无已定默认 | v2 §8.6/16 |
| `attempt` | BIGINT | 是 | 1 | 见说明/枚举 | 来源字段，无已定默认 | v2 §8.6/16 |
| `state` | VARCHAR(16) | 是 | 'active' | 见说明/枚举 | active/superseded | v2 §8.6/16 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| pipeline_artifacts_run | run_id | 否 | 高频查询 |
| pipeline_artifacts_lookup | workspace_id, pipeline_id, name_hash | 否 | 高频查询 |

删除政策与保留：随产物保留期；重试标superseded并删除上一尝试文件；重跑新行及文件复制/硬链接互不依赖。

限制与缺口：name/hash无DEFAULT且须回填，目标DDL不可对非空表直接执行：先可空ADD，回填校验后收紧；解析失败策略源未定义，待补。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "pipeline_artifacts" ADD COLUMN "name" TEXT NOT NULL;
ALTER TABLE "pipeline_artifacts" ADD COLUMN "name_hash" VARCHAR(64) NOT NULL;
ALTER TABLE "pipeline_artifacts" ADD COLUMN "job_key" VARCHAR(255) NOT NULL DEFAULT '';
ALTER TABLE "pipeline_artifacts" ADD COLUMN "attempt" BIGINT NOT NULL DEFAULT 1;
ALTER TABLE "pipeline_artifacts" ADD COLUMN "state" VARCHAR(16) NOT NULL DEFAULT 'active';
CREATE INDEX "pipeline_artifacts_run" ON "pipeline_artifacts" ("run_id");
CREATE INDEX "pipeline_artifacts_lookup" ON "pipeline_artifacts" ("workspace_id", "pipeline_id", "name_hash");
```

### deployment_revisions（存量ALTER；P1）

用途：部署执行意图关联。来源：v2 §8.5/16；entities.rs。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `pipeline_intent_id` | BIGINT | 否 | 无 | 见说明/枚举 | 可空唯一，三库允许多NULL | v2 §8.5/16；entities.rs |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| deployment_revisions_pipeline_intent | pipeline_intent_id | 是 | 唯一键/外键替代 |

删除政策与保留：沿用部署历史/引用保护；不得早于意图恢复期清理。

限制与缺口：现有部署执行在deployment_revisions，不能误加到配置表deployments；沿用源pipeline_intent_id名称。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "deployment_revisions" ADD COLUMN "pipeline_intent_id" BIGINT;
CREATE UNIQUE INDEX "deployment_revisions_pipeline_intent" ON "deployment_revisions" ("pipeline_intent_id");
```

### pipelines（存量ALTER；S0/P1）

用途：草稿、发布指针及自动身份。来源：v3 §11的D16共用字段；v2 §9.5；ADR Q14。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `draft_revision_id` | BIGINT | 是 | 无 | 见说明/枚举 | 首版回填 | v3 §11的D16共用字段；v2 §9.5；ADR Q14 |
| `published_revision_id` | BIGINT | 否 | 无 | 见说明/枚举 | 未发布编码待补 | v3 §11的D16共用字段；v2 §9.5；ADR Q14 |
| `trigger_run_as` | BIGINT | 否 | 无 | 见说明/枚举 | 仅发布确认绑定，不随保存改变 | v3 §11的D16共用字段；v2 §9.5；ADR Q14 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 沿用已有索引 |

删除政策与保留：物理删 + 引用保护；仅归档后删除；归档保留历史。

限制与缺口：归档列/值及存量自动发布授权源未定义，待补，不能把当前编辑人伪造成历史发布人。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "pipelines" ADD COLUMN "draft_revision_id" BIGINT NOT NULL;
ALTER TABLE "pipelines" ADD COLUMN "published_revision_id" BIGINT;
ALTER TABLE "pipelines" ADD COLUMN "trigger_run_as" BIGINT;
```

### workspaces（存量ALTER；S0）

用途：工作流执行编号计数器。来源：v3 §12.1。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| `workflow_execution_number` | BIGINT | 否 | 无 | 见说明/枚举 | 初值及事务源未定义，待补 | v3 §12.1 |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 沿用已有索引 |

删除政策与保留：沿用存量工作空间删除保护及保留规则。

限制与缺口：须对齐现有计数器分配机制，不能猜初值0或1。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
ALTER TABLE "workspaces" ADD COLUMN "workflow_execution_number" BIGINT;
```

### cron_jobs（存量ALTER；S1）

用途：工作流定时的只读派生行。来源：v3 §11/12.1；ADR Q14；entities.rs。

| 字段 | 类型 | 必填 | 默认 | 取值/约束 | 说明 | 来源 |
|---|---|---|---|---|---|---|
| — | — | — | — | — | 无已定义新增列 | v3 §11/12.1；ADR Q14；entities.rs |

| 名称 | 列 | 唯一 | 用途(高频查询/唯一键/外键替代) |
|---|---|---|---|
| 无新增二级索引 | — | — | 沿用已有索引 |

删除政策与保留：物理删 + 引用保护；派生配置随节点同步，历史保留；细节源未定义，待补。

限制与缺口：可复用config/run_as；“由工作流管理”标记、工作流/节点物理键源未定义，待补，无确切新列不造ALTER。

SQLite DDL（仅新增列；既有列由存量schema提供）：

```sql
-- 源未定义，待补；没有可执行新增列DDL。
```

## 5. 跨库与迁移规范

### 5.1 三库差异与可移植方案

| 项目 | SQLite（§4 DDL） | MySQL | PostgreSQL |
|---|---|---|---|
| 整数、金额微单位、ID、version | BIGINT，不用INTEGER PRIMARY KEY隐式分配 | BIGINT | BIGINT |
| JSON、大文本、密文 | TEXT，无DEFAULT | long_text()→LONGTEXT，无DEFAULT，不能索引TEXT | TEXT，无DEFAULT |
| 索引字符串 | VARCHAR(n)，应用校验字符长度 | VARCHAR(n)，utf8mb4_bin，核算组合键总字节宽度 | VARCHAR(n)或TEXT，应用保持长度校验 |
| 哈希 | VARCHAR(64) | VARCHAR(64)，不索引原大文本 | VARCHAR(64)或TEXT，验证64位十六进制 |
| 幂等创建索引 | 先检查存在 | 不支持CREATE INDEX IF NOT EXISTS；has_index/has_column后执行，DDL逐条提交 | 同项目存在性检查 |
| 可空唯一键 | 多NULL合法 | 多NULL合法 | 默认多NULL合法，不用NULLS NOT DISTINCT |
| 标识符 | 双引号 | builder生成反引号，不机械复制SQLite双引号 | 双引号 |

迁移采用SeaORM schema builder追加，不修改旧迁移。long_text()约在migration.rs第45行；MySQL普通TEXT容量不等于LONGTEXT。未给最终长度的索引列须实现前确认，尤其endpoint_id和execution_intents复合键；不能凭SQLite成功承诺所有MySQL版本可用。仅用来源已定义的repository_hash/name_hash/iteration_hash，不臆造annotations.value_hash。MySQL统一utf8mb4_bin，保留大小写/重音敏感唯一性。

SQLite/PostgreSQL支持条件/部分唯一索引，MySQL没有等价WHERE唯一索引。采用源指定的execution_parents独立表唯一intent_id、deployment_revisions.pipeline_intent_id可空唯一（三库多NULL），不填0后加部分唯一。发布版本名还缺正式字段/结构，不写猜测的“仅发布时唯一”索引。SQLite用length()表达字符长度；MySQL转换为CHAR_LENGTH()（LENGTH是字节数），PostgreSQL用length()/char_length()。CHECK表达已明确的字符长度与reject/returned意见必填；应用也须按字符校验，不能只靠旧MySQL版本可能不执行的CHECK。

### 5.2 迁移顺序与历史兼容

v2设计迁移直接采用v3通用名，不先建旧表再改名。v3未给精确注册名的标“建议注册名”。每行down均按项目规则返回错误，业务数据永不自动移除；手工回退须全停、备份、查新状态/格式/引用，默认保留schema只退二进制。若旧版不能读取新数据，应隔离新资源或恢复升级前备份，并明确升级后业务数据损失，不能DROP表冒充无损回滚。

| 顺序/阶段 | 迁移名 | up：增量DDL、回填 | down：人工回退影响 | 历史数据与版本兼容 |
|---|---|---|---|---|
| 1 / v2 P0-1 | pipeline_run_leases_v1 | runs租约/队列列及索引；hosts.pipeline_slots；jobs尝试/原因；steps/logs键，先建新步骤唯一键再删旧键 | 不自动down；退旧唯一键前验证新行仍满足run_id+step_index，不满足禁止强退 | 历史attempt=1、job_key=''、lease_owner=''；旧queued任务按pending重算。含P0-1版本全停升级，不能滚动混跑（v2 §8.4/§23） |
| 2 / v2 P0 | definition_revisions_v1（原pipeline_revisions_v1） | 通用快照；现存流水线生成number=1，created_by=0除非可靠可溯源 | 不自动down；删除将丢不可变历史并破坏引用，默认保留 | 当前定义不能冒充过去执行原版本；无实质变化不增版；回填可重入 |
| 3 / v2 P0 | pipeline_run_provenance_v1 | 来源/重跑列及索引；inputs已有不ADD；source_refs可空 | 不自动down；旧版不识别谱系/密文时禁止直接恢复生产写入 | revision_id=0标历史未知，不指向当前第1版；旧trigger_kind保持manual；root/rerun=0、attempt=1，不猜来源 |
| 4 / S0 | workflows_v1、workflow_executions_v1（建议注册名） | 新定义/执行、工作空间计数器、流水线草稿发布指针 | 不自动down；保留新数据，旧版不能执行图 | 无工作流历史回填；草稿可指首版，但存量首次生产发布授权源未定义，待补 |
| 5 / S0 | content_assets_v1（建议注册名） | assets/asset_refs/asset_tags，共用local/S3；GC载体补齐后再启物理GC | 不自动down；不连库行与对象一起强删 | 不迁pipeline_artifacts为内容寻址；不跨空间共享权限，旧路径产物并存 |
| 6 / S0结构→P1-1 | trigger_bindings_v1（原pipeline_triggers_v1，含trigger_deliveries） | 通用结构、补泛化键；按已发布定义派生，事件事务同写发件箱 | 不自动down；先停dispatcher再保留pending/已投递账本防重复 | repository_events列不变，不从旧事件伪造TriggerEvent/补投历史，不自动重试 |
| 7 / S0结构→P1-4核心 | human_tasks_v1（原pipeline_approvals_v1，拆human_tasks/human_responses） | 结构与会签/资格/超时/条件推进；补关联、轮次/外审唯一键 | 不自动down；停接回应，保留未完任务，不一律判失败 | requires_approval不删；旧审批转新任务策略源未定义，待补；S1依赖此核心 |
| 8 / P1 | pipeline_environment_protection_v1 | contexts.protection可空及保护门 | 不自动down；旧版不执行保护，不能无隔离退旧版 | NULL保持原行为，不编造审批人，保护修改审计 |
| 9 / P1-3 | pipeline_artifact_attempts_v1 | name/hash先可空ADD，路径解析回填/算哈希，校验后收紧NOT NULL；attempt=1/state=active及索引 | 不自动down；停重跑/部署，复制产物保留，不为旧版兼容重命名文件 | 解析失败策略源未定义，待补；SQLite收紧须保留全部旧列索引重建表；MySQL各步可重跑 |
| 10 / P1 | execution_intents_v1（原pipeline_task_intents_v1） | 通用意图/子执行表；deployment_revisions可空唯一意图ID | 不自动down；删幂等账本/下游关联可能重复副作用 | 旧部署多NULL合法，不补造历史意图；新意图先落库，恢复pending先查下游 |
| 11 / P1 | pipeline_notifications_v1 | 通知账本与唯一键，发前sending | 不自动down；退旧实现可能重复发，停发并保留账本 | 不补发旧事件；接管sending→unknown不重试 |
| 12 / S1 | workflow_graph_v1、webhook_endpoints_v1、execution_annotations_v1（建议注册名） | 节点/图日志/缓存/入口/标注；归档列待补；新时间配xx_by | 不自动down；保留恢复输出，先停新入口、隔离旧版 | 无图历史回填；发布不改地址，仅缓存成功纯节点输出 |
| 13 / v2 P2 | pipeline_reports_v1；失败跳过迁移精确名源未定义，待补 | 报告表及索引；jobs.skipped_by/reason，随D14 all权限上线 | 不自动down；报告与失败日志保留 | 不补造旧报告；skip不增attempt，仅failed/timed_out主机任务，不跳审批/部署/环境门 |
| 14 / S2 | data_tables_v1、vector_collections_v1、cost_records_v1（建议注册名） | 补全后建数据/向量/成本；trigger_state载体待补 | 不自动down；保留轮询游标/预算账本，不退为零游标 | 无向量/价格历史回填；缺价不是0；SQLite/MySQL集合限额，不冒称provider选型完成 |
| 15 / S3 | 精确迁移名源未定义，待补 | 复用资产/成本/节点表；seed/redo_round/组件版本持久化载体待补 | 不自动down；保留来源、成本与固定资产 | 无确切新列不造DDL，不重写冻结参数/资产哈希 |
| 16 / S4 | models_v1、host_models_v1、host_gpu_slots_v1（建议注册名） | 模型/主机模型；gpu_slots=0，首报后设GPU数，之后可手改 | 不自动down；停GPU调度，记录/权重保留 | 老Runner不报GPU不影响原行为；哈希失败不ready，不批量重分主机槽 |

规范顺序为v2 P0→S0→P1-1/P1-4通用核心→S1→{P1其余∥S2}→S3→S4（v3 §18.16），不是简单旧v2顺排。v2 P2调用工作流依赖S1。P0必须停止全部实例：新清扫只接管lease_owner非空的新执行，旧实例重启清扫仍可能误伤；“增加列兼容旧查询”不等于允许混跑。SSH host_lost重试须先验证真实远端终止。

新增builder语句必须手工登记schema_tests，运行三库schema_tests及scripts/test-backends.sh。execute_schema先has_column/has_index，回填仅缺失值，不覆盖正确快照。定义number与雪花分配/绑定同事务。非空无DEFAULT增量（如draft_revision_id）同产物一样先可空ADD、回填、校验后收紧；计数器等来源缺初值的增量须补契约后才部署。本文件不声称这些实现测试已运行。

## 6. 状态与枚举全集

| 表/列或结构 | 全部已定义值/缺口 | 来源 |
|---|---|---|
| subject_kind | pipeline、workflow | v3 §11/12.1 |
| pipeline_runs.state | queued、running、waiting、cancelling、succeeded、failed、cancelled；无执行级timed_out | v2 §8.1 |
| pipeline_run_jobs.state | pending、queued、running、waiting、approved、succeeded、failed、timed_out、skipped、reused、cancelled | v2 §8.1/8.7 |
| pipeline_run_steps.state | 完整闭集源未定义，待补；不能套含approved/reused的job闭集 | v2 §8/16；entities.rs为String |
| workflow_executions.state | queued/running/waiting/succeeded/failed/cancelled；图侧cancelling契约源未定义，待补 | v3 §12.1复用v2、§18.7 |
| workflow_node_runs.state | 节点状态引用pending/running/succeeded/failed/skipped/bypassed/cached/pinned/waiting；§18.7复用reused；取消/失败继续完整码源未定义，待补，不猜CHECK | v3 §12.1节点运行引用、§18.7/18.9 |
| runs/jobs.end_reason | 空串、superseded、orchestrator_lost、run_timeout、rejected、approval_timeout、protection_denied、cancelled_by_user；没有trigger_invalid | v2 §8.1 |
| workflow_executions.end_reason | 适用v2原因，加暂停paused；图失败完整原因码源未定义，待补 | v3 §18.7 |
| pipeline_runs.trigger_kind | manual、schedule、push、pull_request、tag、pipeline、rerun；历史manual不猜来源 | v2 §9.1 |
| workflow_executions.trigger_kind | 手动/事件/定时/Webhook/表单/聊天/上游/错误/轮询/重跑语义已要求，完整字符串码源未定义，待补 | v3 §12.1/18.6；PRD FR-TRIG |
| trigger_bindings.kind | push、pull_request、tag；其它触发持久化码源未定义，待补 | v2 §9.2；v3 §11 |
| trigger_deliveries.state | pending、created、skipped、failed；人工重投redelivery_seq+1 | v2 §9.3b |
| execution_intents.state | pending、started、done、unknown | v3 §18.8 |
| execution_intents.kind | 部署/调用流水线或工作流/通知等语义，完整字符串码源未定义，待补 | v2 §8.5；v3 §18.8 |
| human_tasks.kind | 审批、表单、挑选、编辑，正式英文码源未定义，待补 | v3 §11/12.1 |
| human_tasks状态/超时动作 | 状态码源未定义，待补；审批on_timeout=approve/reject；非审批默认7天fail | v2 §13.1；ADR Q08 |
| human_responses.decision | approve、reject、returned；其它回应空值/码源未定义，待补 | v2 §13.1；ADR review defaults |
| pipeline_run_notifications.state/event | state：sending/sent/failed/unknown；event：started/waiting/succeeded/failed/cancelled，started不是run.state | v2 §14 |
| 通知渠道配置type | webhook/dingtalk/feishu/wecom，地址密钥不写定义/日志/主机；不新增表字段 | v2 §14 |
| pipeline_artifacts.state | active、superseded | v2 §8.6/16 |
| assets.kind | image/video/audio/text/file/model；S5新增model3d | v3 §12.1资产引用、§18.4 |
| asset_refs.owner_kind | execution/node_run/cache/pin/user_pin | v3 §12.1 |
| models.kind | checkpoint/lora/vae/controlnet/upscaler/tts/llm | v3 §12.1模型引用 |
| models.commercial_use | allowed/restricted/forbidden，必填；非allowed首次确认具体许可并审计 | v3 §12.1；AC-GPU-01 |
| models.license_id/family | SPDX或custom；family待V6选型的开放集合 | v3 §12.1；ADR Q15 |
| host_models.state | absent/downloading/ready/failed | v3 §12.1模型引用 |
| workflow_executions.mode | manual/test/production/retry/webhook_sync；评测模式码源未定义，待补 | v3 §12.1/18.3 |
| webhook_endpoints.mode/auth/respond_mode/method | test/production；auth配置none/basic/header/jwt；回应三种语义码、HTTP方法闭集源未定义，待补 | v3 §12.1/18.10 |
| cost_records.unit/currency/provider/model | unit开放集合input_tokens/output_tokens/seconds/images/video_seconds/characters等；currency凭据配置不换汇；provider/model待V6/V7不锁白名单 | v3 §12.1；ADR Q11/Q15 |
| 定义生命周期/save_data | 草稿/发布/归档及保存策略语义已要求，物理状态列与值源未定义，待补；归档非soft delete | ADR Q12；v3 §18.7 |


## 7. 字段 → PRD FR/AC追溯

JSON内部键不算独立表字段；未知物理字段按需求名标缺口，不将界面文案擅自变成列。

| 字段/需求 | 校验与约束 | PRD FR/AC | 来源/落库状态 |
|---|---|---|---|
| definition_revisions.definition/number；执行revision_id | 实质不变/只改元数据不增版；快照不可覆盖；被执行引用/最近50版不清 | FR-VER-01；AC-VER-01；FR-PIPE-21、AC-PIPE-20 | v2 §15 |
| 发布版本名（物理列源未定义，待补） | 必填1–64字符，定义内唯一，跨定义可重名；不能用number替代 | FR-VER-02；AC-VER-02/03 | ADR Q07；名称列及唯一键待补 |
| definition_revisions.message | 可选≤200字符；发布说明是否复用保存message待确认 | FR-VER-02；AC-VER-03 | v2 §15/ADR Q07；DDL长度CHECK |
| draft_revision_id/published_revision_id/trigger_run_as | 保存草稿不切生产、不重绑；发布确认才绑定发布人；在途版本不改，执行前鉴权 | FR-VER-03/04、FR-PIPE-19；AC-VER-01/02/04/05、AC-PIPE-05/12 | v3 §11；ADR Q14；workflow身份正式列待补 |
| 归档状态（物理列源未定义，待补） | 自动撤销发布、停生产触发/公开入口，历史保留；恢复草稿，仅归档可删除 | FR-LIST-05、FR-PIPE-14、FR-VER-03；AC-LIST-03/04、AC-VER-05 | ADR Q12 |
| human_responses.decision/comment | reject/returned意见必填1–500字符；returned结束任务，由图分支修改/重做 | FR-HUM-10；AC-HUM-08/09；FR-ENV-02、AC-ENV-01 | v2 §13.1；ADR review defaults；DDL CHECK |
| human_responses唯一键/response | 同人同任务同尝试一次；回应加密持久化后推进唤醒，不覆盖原稿 | FR-HUM-05/07/08/11；FR-RUN-03、AC-RUN-02 | v2 §13.1；通用轮次/外审/关联键待补 |
| human_tasks.external_token_hash/expires_at/expires_by | 外审72h、一次性且可撤销；非审批默认7天fail，不能混用两个期限 | FR-HUM-04/08/09 | ADR Q08；撤销/已用/每30秒草稿载体待补 |
| pipeline_contexts.protection JSON | 人员/角色并集；min_approvals 1–10；timeout_minutes 0–43200、0不超时；allow_initiator=false；allowed_refs创建时校验 | FR-ENV-01/02；AC-ENV-01；FR-PIPE-06 | v2 §13.0/13.1；job_key=__environment__保护门 |
| jobs.skipped_by/reason/state/attempt | all且受空间权限上限；仅failed/timed_out主机任务；不跳审批/部署/环境门；不增attempt、保留失败日志 | FR-PIPE-17；AC-PIPE-09/10 | ADR Q04/review defaults；reason精确长度未定义 |
| jobs.attempt、steps/logs.job_key/attempt | 总尝试1–5、间隔0–3600秒；审批/部署不自动重试；步骤尝试唯一，日志run+seq唯一；旧job_key=''、attempt=1 | FR-PIPE-10；AC-PIPE-04；FR-PIPE-08 | v2 §8/16；重试配置属于definition内部，未额外造表 |
| runs.rerun_of_id/root_run_id/attempt、artifacts.name/hash/state | 重跑新编号与谱系，不改旧记录；复制有效产物，superseded不被下游消费 | FR-PIPE-05/07；AC-PIPE-03 | v2 §8.6/16；Actions名称路径解析回填 |
| hosts.pipeline_slots/gpu_slots | 流水线默认2、范围1–32；GPU默认0，首报GPU数量后可手改 | FR-RUN-06；FR-GPU-02/05、AC-GPU-03 | v2 §8.2；v3 §12.1 |
| workflow_node_runs.output/output_asset；settings.save_data | >64KiB转资产；条目≤1MiB、节点输出≤16MiB；运行中持久化，终态再清；清后只全部重跑 | NFR-CAP-01；FR-RUN-03/10、AC-RUN-03 | v3 §12.1、§18.7/18.14 |
| iteration_path/hash、attempt、cache_key/cache_hit_from | 轮次隔离；逐条缓存；redo_round入签名，不缓存失败/错误条目；reused不是cached | FR-RUN-02/03、AC-RUN-02；FR-HUM-03/06 | v3 §18.7/18.9；redo_round物理载体待补 |
| workflow.settings限额；definition图JSON | 图≤2MiB/500节点；节点执行默认5000/最大50000，树默认20000；调用深度10；嵌套3、while默认10最大1000；执行并发默认8最大64；并行循环默认4 | NFR-CAP-01/02；FR-RUN-08、AC-RUN-07 | v3 §18.15；内部完整schema待补 |
| assets.workspace_id/sha256/pinned；asset_refs | 同空间去重；默认50GiB；固定不清；无引用24h后复查 | FR-ASSET-01/02/03/04；AC-ASSET-01/02 | v3 §12.1；GC标记待补 |
| webhook_endpoints.endpoint_id/path/mode/auth/cors/ip_allowlist/respond_mode | 随机稳定ID；生产需发布/测试仅窗口；认证/来源/IP先校验；同步默认30秒1–300，满组503、等待202、超时504后台继续 | FR-TRIG-02/03/04/07；FR-PIPE-14、AC-PIPE-30 | v3 §12.1/18.10；窗口/超时/未送达字段待补 |
| trigger_deliveries.event；runs.source_refs/trigger_actor/trigger_ref | merged MR冻结精确merge SHA，不跟目标head；fork不自动执行；旧投递不覆盖、不自动重试 | FR-TRIG-01/06、FR-PIPE-09/18；AC-PIPE-02/11 | ADR Q05；v2 §9 |
| execution_intents各级唯一键、state/external_id；execution_parents.intent_id | 逐条副作用先意图后执行，接管按意图找下游；无幂等未知窗口不自动重提 | FR-RUN-09、AC-RUN-08；FR-AI-05、AC-AI-03 | v3 §18.8；通知至多一次 |
| trigger_state（所属表/键源未定义，待补） | 游标加密持久化，接管不从0重复 | FR-TRIG-05 | v3 §18.3，不造trigger_states表 |
| data_table_rows.i1..i5 | 最多5个类型化索引列；每表≤100000行，其它JSON过滤全表扫描需明示 | FR-NODE-02、FR-PLAT-11；NFR-CAP-08、AC-CAP-02/03 | v3 §12.1；具体类型/关联键/JSON列待补 |
| vector_items（向量列源未定义，待补） | SQLite/MySQL每集合≤20000，PG可选pgvector；不承诺可移植JSON索引 | FR-NODE-02；NFR-CAP-09 | v3 §12.1 |
| models.commercial_use/license_id/license_url/restrictions、host_models.state | 商用allowed/restricted/forbidden必填；非allowed首次确认具体许可并审计；哈希不符不ready | FR-GPU-01/03/04；AC-GPU-01/02 | v3 §12.1模型引用 |
| cost_records.amount_micros/currency；settings.max_cost | 分币种不换汇；缺价拒执行；AI调用前预留后结算；月预算和单次上限独立 | FR-AI-02/04/06、AC-AI-01/02/04/05/07；FR-STAT-02 | v3 §18.13；价格/预留/credential关联待补 |
| 工作空间时区、纯等待累计（物理列源未定义，待补） | 分桶用空间时区，未配用服务器时区；耗时扣排队/等待；零分母“—”；恢复时间按同定义失败结束→下次成功结束，reused不重复计 | FR-STAT-01/03/04 | ADR Q11/review defaults；现有started/finished不足以精确累计多段等待 |
| 手动启动等待与受理标记（源未定义，待补） | 等点击、恢复保留等待，重复点击不重复执行；取消后不能启动 | FR-PIPE-11；AC-PIPE-07/13/14/15 | PRD行为已明确，没有确切新增列不臆造 |
| 运行备注、收藏、子工作流调用名单（源未定义，待补） | 备注可重新读取；收藏不改权限；子调用真父执行/同空间/名单校验，禁止伪caller | FR-PIPE-13/21、AC-PIPE-17/23；FR-RUN-13、AC-RUN-10/11/12/13 | PRD明确行为，限定来源未给正式字段/表 |
| 单产物保留期、缓存闲置天数（源未定义，待补） | 到期只清符合条件对象，其他有效产物/缓存保留 | FR-PIPE-12/16；AC-PIPE-22 | PRD明确；不能擅改已有expires_at或新增idle_days |

Q03对象授权/组继承并集/空间权限上限/所有权转移已确认，限定来源未给物理ACL表，源未定义，待补。Q06问题分级、Q09上传20MB/最多5个（可配上界200MB/20个）、每IP每分钟10次、聊天每会话每分钟20条、验证码默认关、类型来自表单字段；Q10全局队列原因；Q15型号/provider待选均遵ADR，不扩造存储字段。Q09配置载体、Q03权限载体、AI凭据限流配置范围与落库载体均源未定义，待补。

## 8. SQL示例与删除保留

绑定命名参数，MySQL由builder转方言；无FK不表示可以跨工作空间引用。

### 8.1 领取与发件箱

~~~sql
SELECT id, workspace_id, lease_epoch FROM pipeline_runs
WHERE state IN ('queued','waiting')
  AND (not_before IS NULL OR not_before<=:now)
ORDER BY priority DESC, created_at ASC LIMIT 32;
-- 先获执行锁；后续runs/jobs/steps/logs均带epoch栅栏。
UPDATE pipeline_runs
SET state='running', lease_owner=:instance_boot, lease_epoch=lease_epoch+1,
    started_at=COALESCE(started_at,:now)
WHERE id=:run_id AND state IN ('queued','waiting') AND lease_epoch=:old_epoch;
-- started_at为存量，不顺便补started_by。
SELECT id FROM trigger_deliveries
WHERE state='pending' ORDER BY created_at LIMIT 32;
-- create_run与下列条件写同一事务，不能分两次提交。
UPDATE trigger_deliveries
SET state='created', run_id=:run_id, updated_at=:now, updated_by=0, version=version+1
WHERE id=:delivery_id AND state='pending';
~~~

候选SQL仅展示索引/条件写；完整调度须加v2 §8.2的空间轮转、饱和组过滤及并发短锁。新workflow_executions时间写入必须同时写started_by/finished_by，不照抄存量runs省略。影响行数0意味着失租约或已推进，不能忽略。

### 8.2 尝试、唯一键与成本

~~~sql
SELECT * FROM pipeline_run_steps
WHERE run_id=:run_id AND job_key=:job_key AND attempt=:attempt ORDER BY step_index;
SELECT * FROM pipeline_run_logs
WHERE run_id=:run_id AND seq>:last_seq ORDER BY seq LIMIT 200;
SELECT * FROM trigger_bindings
WHERE workspace_id=:workspace_id AND connection_id=:connection_id
  AND repository_hash=:repository_hash AND kind=:kind;
SELECT * FROM execution_intents
WHERE subject_kind=:subject_kind AND execution_id=:execution_id
  AND node_key=:node_key AND iteration_hash=:iteration_hash
  AND item_index=:item_index AND attempt=:attempt;
SELECT * FROM execution_parents WHERE intent_id=:intent_id;
SELECT currency, SUM(amount_micros) FROM cost_records
WHERE workspace_id=:workspace_id AND created_at>=:start AND created_at<:end
GROUP BY currency;
~~~

日志尝试过滤仍可按run_id+seq后过滤，来源无专用复合键不悄悄增加。pending意图恢复先按下游唯一意图关联查询，查到补external_id，查不到才尝试创建；通知pending/sending不重发而unknown。quantity不是价格，缺价不是免费。

### 8.3 引用保护与两阶段GC

~~~sql
SELECT * FROM definition_revisions
WHERE subject_kind=:subject_kind AND subject_id=:subject_id
ORDER BY number DESC LIMIT 50;
SELECT id FROM assets WHERE workspace_id=:workspace_id AND sha256=:sha256;
SELECT a.id FROM assets a
WHERE a.workspace_id=:workspace_id AND a.pinned=0
  AND NOT EXISTS (SELECT 1 FROM asset_refs r WHERE r.asset_id=a.id);
-- 仅选候选，不能立即删；先持久化无引用标记，默认24h后复查。
-- 标记载体源未定义，待补，不臆造UPDATE gc_at。
SELECT owner_kind, owner_id FROM asset_refs WHERE asset_id=:asset_id;
~~~

GC须与并发去重插入/引用事务协调，复查至实际对象删除期间也须防新引用，具体锁/标记schema源未定义，待补。不能仅凭NOT EXISTS宣称完整两阶段GC。保护被执行引用版本、最近50版及草稿/发布指针。删除定义先查归档与授权，停生产入口，再按保留规则保护历史；名称/参数快照缺定义时不能级联抹历史回避。

### 8.4 删除保留结论

Ferry当前无soft delete；归档是业务状态，不用deleted_at/is_deleted替代。未来归档/GC/撤销时间若定义也须xx_by。执行/日志/产物/报告/成本/回应按来源保留，未给天数不自定30/90天；定义覆盖全局保留的物理键源未定义，待补。固定资产不随来源执行过期删。等待任务、活动租约、恢复输出/意图不可被终态清理器误删。默认local存储，集群S3兼容共享后端提前S0；对象删除失败的恢复账本源未定义，待补，不声称数据库/对象存储跨系统事务原子。

## 9. 来源缺口汇总

本文件完整整理限定来源用于评审；来源不足以形成可直接上线的完整迁移。以下均为「源未定义，待补」：

1. 版本名列/唯一键、发布说明与message关系；归档状态列/值/恢复删除载体。
2. 触发pipeline_id/run_id泛化及唯一键隔离；人工关联/轮次/外审幂等、状态/指派/截止/撤销已用及30秒草稿。
3. 数据表完整键/JSON列、i1..i5类型与索引；向量完整键/向量列/维度/距离度量/二进制编码。
4. GC标记与并发删除保护、固定资产历史参数追溯、对象删除失败恢复。
5. 工作流发布身份正式列、编号计数器初值/事务、等待累计/时区、标注归属/索引、trigger_state与cron派生标记。
6. 成本credential关联、quantity精度、单价/预算预留结算释放、凭据限流及跨月保留边界。
7. 图日志裁剪、step/图状态/触发/human kind/intent kind完整码、非循环条目键、通用通知归属。
8. Webhook测试窗口/超时/未送达、endpoint_id唯一与组合唯一冲突、认证配置schema。
9. 许可审计载荷、ACL、公开上传/聊天配置、seed/redo_round/组件版本、未给的宽度/可空性/时间单位。
10. 存量生产发布授权、旧审批过渡、产物解析失败、定义级保留及具体清理期限/删除引用规则。
11. 手动任务等待/启动受理、运行备注、收藏、子工作流调用名单、单产物/缓存闲置策略的正式字段。

### 表与字段盘点

- 新表（26）：definition_revisions、workflows、workflow_executions、workflow_node_runs、workflow_execution_logs、node_cache、assets、asset_refs、asset_tags、trigger_bindings、trigger_deliveries、human_tasks、human_responses、execution_intents、execution_parents、pipeline_run_notifications、pipeline_run_reports、cost_records、data_tables、data_table_rows、vector_collections、vector_items、models、host_models、execution_annotations、webhook_endpoints。
- 存量变更目标（11）：pipeline_runs、hosts、pipeline_run_jobs、pipeline_run_steps、pipeline_run_logs、pipeline_contexts、pipeline_artifacts、deployment_revisions、pipelines、workspaces、cron_jobs。cron_jobs仅声明待补增量，无确切新增列。
- 已定型字段：新表272列，存量新增37列，共309列，包含新表version与时间审计；不含旧列、未定型i1..i5五列、未命名缺口及JSON内部键。计数为设计投影，不代表全部可以直接上线。
- 校验：逐表字段与DDL列一一一致、新时间审计配对、新表version与UTF-8/LF检查通过；已定型DDL在内存SQLite按当前实体的最小存量夹具执行通过。夹具为空数据，不代表历史回填、MySQL/PostgreSQL或真实迁移实现已验证。
- 约束正反例：SQLite中资产引用唯一性、说明200/201字符、returned意见NULL/500/501字符、可空意图唯一键多NULL共7项均按预期通过；Markdown表列数及全部审计列BIGINT NOT NULL DEFAULT 0检查通过。
- 仅新增本文档，无暂存、提交或推送。

## 10. v1.5 技术补充目标（评审稿，不执行迁移）

### 10.1 约定、覆盖关系与迁移边界

来源：本轮用户“先补充缺口，做好评审”，PRD §15，已确认ADR。新增技术设计不伪装成旧来源已有结论。BIGINT ID前端字符串；时间统一UTC Unix毫秒（db::now），每个新xx_at配xx_by BIGINT NOT NULL DEFAULT 0。新业务行version默认1；配置CAS更新，历史不可变。可空关联用NULL，不用0表示“某个实体”。所有跨表关系在同事务校验workspace/kind/owner，延续无数据库外键架构。

§10.10的完整CREATE替代§4同名新表的占位/不完整CREATE；不是先建旧表再补，尚未实现的新表直接用完整目标建表。§10.11为其它目标的增量，只有在审查夹具中接到§4原始投影后执行；生产迁移必须通过当前migration builder按真实存量做可重入ADD/回填/索引/收紧，不能直接复制这些ALTER。发布表、GC/预算账本等新增表是已要求功能的技术载体，不新增产品功能。

产品默认依PDR-01～06（2026-11-04已确认01～05，06仍待选型）；DDL中的技术状态默认不等于确认UI上限/授权/保留期。任何待决产品配置必须在实现前落实到版本化Schema，不能把空配置解释成任意权限。

### 10.2 定义、发布、通用归属与编号

- definition_revisions保留每次保存的不可变修订；definition_publications单独存发布名、change_note、revision_id、run_as、幂等键。name_key由确认的发布名规范化规则生成；已确认trim后按精确大小写唯一，不做lowercase。数据库唯一键为workspace/kind/subject/name_key，API先校验1–64字符；按已确认PDR-01在三库使用一致二进制排序规则，大小写不等价。
- pipelines/workflows增加owner_id、lifecycle(active/archived)、archived_at/by。workflow补trigger_run_as；published_revision_id为NULL表示未发布。定义owner迁移从确切创建者追溯，无法追溯时待管理员认领，不猜历史owner/发布人。现存流水线如何恢复自动触发依PDR-04，未授权不自动发布。
- 新共用表trigger_bindings.pipeline_id→subject_id；trigger_deliveries.pipeline_id→subject_id、run_id→execution_id；human_tasks/responses.run_id→execution_id；pipeline_run_notifications→execution_notifications并加subject_kind。这是未实现目标的命名精化，不改当前API。
- execution_parents的父身份从intent取得，不重复存猜测parent_run_id；补child_subject_kind，child_execution_id在该类型下读取。父子关联必须校验同空间、允许调用名单与递归深度。
- workflow_execution_number初值0，创建事务锁workspace计数器并+1，写执行与投递同事务；唯一(workspace_id,number)。不要MAX()+1；事务回滚不留下执行，编号允许因技术失败出现间隙，不以连续无缝编号作幂等保障。

### 10.3 数据表、向量与标注

data_tables保存column_schema与index_schema（最多5个逻辑槽），rows的data为规范JSON。每槽以iN有界二进制前缀+nN DOUBLE PRECISION数字列实现，index_schema绑定类型，另一个列必须NULL；原文本始终保留在data，前缀只能选候选，必须复核原值。v1.9的精确编码/范围/排序与跨库映射以§13为准，覆盖此前VARCHAR(191)文本投影。数字只接受有限IEEE754值，与JS number一致。每槽两个索引含workspace/table/值/id，保持5个逻辑索引额度，不是给用户10个索引。S2仍须三库排序/溢出/NULL/执行计划实测；JSON非索引字段过滤必须标全扫描，≤10万行计数在table锁内检查，不能先count再无锁insert。

vector_collections记录dimension/metric/embedding_model/normalization，向量items以同维有限float32小端字节保存，长度=dimension*4；SQLite BLOB、PG BYTEA、MySQL LONGBLOB。非有限、长度错、零向量cosine请求拒绝。metric支持cosine/dot/euclidean的具体产品集合随节点manifest评审；cosine排序高分优先，euclidean低距离优先。更新collection.version后失效进程缓存；同一集合写入锁内限制SQLite/MySQL≤20000，PG pgvector采用单独适配迁移，不把BLOB伪装成已支持索引向量。embedding模型/维度不能原地换，重建集合避免混向量。

execution_annotations为工作空间/类型/执行/key唯一的标注；value保留JSON，value_hash=规范JSON SHA256用于精确等值过滤。hash命中仍比较原value，不将哈希碰撞当等值；不声称支持TEXT任意全文搜索。执行注释与用户收藏分别在execution_notes/definition_favorites，不混入节点输出。

### 10.4 资产、GC、参数与缓存

assets增加provenance JSON（revision_id、节点实现版本、resolved_params摘要、seed、源资产哈希、provider/model版本；脱敏加密规则同输出）。完整参数仍在node_run.resolved_params及其受保护资产，固定资产保留必要快照引用，不依赖可被清理的运行日志。GC队列记录对象storage_key、eligible_at/by、状态/重试/错误。

引用/固定/标记删除均抢`asset:{workspace}:{id}`唯一owner短锁，并检查资产version和GC状态；宽限24小时内新引用取消candidate。删除者在同锁内重查无引用/无pin、将candidate→deleting，提交后删对象；deleting期间拒绝新引用。删除请求明确完成且HEAD核实不存在后，事务删元数据/账本；失败留failed重试。unknown保留删除状态，不仅凭一次HEAD不存在就认定在途旧请求已结束；无法证明旧删除完成时继续隔离，不允许对象键被新上传复用，防迟到删除误伤新对象。对象存储生命周期规则不得越过固定/引用保护。

node_cache增加source_subject_id用于“按工作流清缓存”，不是凭source_execution_id猜归属。retention/cache_idle覆盖放定义settings（NULL继承，0永久），单产物expires_at沿现字段；有效活动执行引用不因覆盖策略删。手动清缓存释放cache引用，不删除仍被execution/pin使用的资产。

### 10.5 预算、价格与并发

budget_buckets键为workspace/credential/currency/period_key（空间时区月份，时区及UTC边界快照），spent/reserved微币；price_snapshot在reservation与cost_record保留单价/单位/版本/币种；执行max_cost快照独立检查，不能只检查月桶。reservation.operation_key取execution_intent或等价稳定调用身份，唯一键保证重领不会再预留。

事务：锁bucket及execution预算行，比较spent+reserved+estimated≤cap，再插reservation并增加两级reserved；调用外部后，以reservation状态CAS由reserved/unknown→settled一次，冲减reserved并累加真实actual。explicit not_sent才release；结果未知转unknown冻结，不能因为执行终态或过月就释放。价格舍入/月份归属依PDR-05；整数乘除用至少128位中间值，转换BIGINT前检查溢出。实付超估算可overrun，但不能丢真实费用。

cost_records补id主键、credential_id、subject_kind、reservation_id、quantity_micros、unit_price_micros、price_snapshot、event_key；quantity旧草图列由quantity_micros替代，单位在unit内明确，已运行数据库若存在旧成本必须先确认旧尺度再回填，不乘一个猜测常数。唯一(workspace_id,event_key)，允许同reservation多项输入/输出价格明细，累计结算与明细写同事务。

凭据QPS/并发是跨实例共享控制，以workspace+credential范围的持久租约/锁计数；QPS为请求速率、并发为在途请求，不用CPU或主机slots替代。令牌桶/窗口精度与429回退作为S2实现设计验证，未选方案前不能声称限流精确。外部超时但可能仍运行的请求继续占unknown预算，执行容量和预算额度各自释放规则分离。

### 10.6 人工任务、模型与执行快照

human_tasks以execution/job(or node)/iteration/attempt/round唯一；assignment保存允许的user_ids/role_ids和min_approvals，提交时复查当前资格。human_responses按task_id+responder_key唯一；user响应者key=user:{id}，外审key=token:{token_id}，系统超时key=system:timeout。草稿表仅本人能读写，每30秒保存、CAS冲突不覆盖；submit回应与终结/计数/唤醒同事务。min_approvals需1≤N≤去重人数，角色成员变动使不足时显示等待条件，不自动降N。

external_task_tokens每位外审人一条稳定ID的hash记录，含expires/revoked/used及配对by；明文只返回一次，旧token撤销后不得接受。轮换仅CAS替换同一行hash，不为同一受指派人创建新token身份；used后不允许轮换再投票，保证responder_key中的token_id稳定。对同一token重发同一幂等请求可读取原结果，不能再插第二回应。token资格、任务状态与截止在同事务校验；浏览器双击/两实例/超时竞争只有一个终结。

host_models增加workspace_id及唯一(workspace,host,model)，downloaded_bytes/error与下载lease_owner/epoch避免多个实例写同一临时文件；verified_at/by已有不重复新增。模型删除先禁止新推理，等待活动推理/下载释放，删权重后更新元数据，不把删元数据当删文件成功。gpu_slots按已确认PDR-01取1–8，默认1。许可确认记录activity event/model/license_hash/user/time，UI声明本次确认并不替代法律许可文本。

workflow_node_runs增加resolved_params（敏感字段按已有vault AAD封装）、seed、redo_round、error_policy；非循环iteration_path=[]及SHA256规范JSON，整体节点item_index=-1、逐条从0计数（不与第0条混同），缓存/intent键必须使用相同规范。components保存于图版本，内联ports/subgraph或引用definition_revision_id/hash，不能跟随latest改变历史。

### 10.7 触发、公开窗口与同步回应

webhook_endpoints_id保留全局唯一，删除冗余route唯一索引；HTTP方法与mode是入口配置约束。test窗口在webhook_test_windows绑定endpoint/user/draft_revision、expires_at/by，生产绑定已发布修订；测试窗口关闭/过期立即拒绝，但不影响已创建执行快照。

workflow_executions.context保存冻结inputs、source_refs、entry_node、batch_id/index、resolved_trigger、call_allowlist、settings及发布身份引用；敏感输入用执行专属AAD加密。response_state=pending/delivered/undelivered/not_applicable；deadline_at/by、delivered_at/by单独存，响应连接丢失/接管后pending→undelivered，不能把HTTP发送成功与客户端收到混同；执行继续且有原因可查。

trigger_states持久provider_cursor、dedupe窗口与lease；每个provider页面结果先事务写delivery后推进游标，失败不跳页。cron_jobs追加managed_subject_kind/id/node_id/revision_id，派生行由发布管理；手动定时任务这些列全NULL，已有config/run_as复用。旧cron字段不猜新归属，生产授权按PDR-04。

public_sessions存token_hash/workspace/definition/已发布revision/expires/revoked；上传先鉴权/限频/字段类型及流式字节限额，资产归属会话，执行接受后转执行引用；未被接受上传进入GC宽限，不能直接删共享对象。限频必须跨实例，以受信代理解析IP+endpoint+窗口计数；原始IP按部署隐私策略处理，不把限频表当永久用户追踪档案。验证码只存适配器凭据引用，服务商选择待PDR-06。

### 10.8 ACL、分组与同步

definition_acl唯一含workspace/subject_kind/subject_id/user_id/role(all/run/view)；subject_kind允许pipeline/workflow/pipeline_group/workflow_group，后两者是**资源分组**，不是新增人员组。来源为流水线清单A9（分组级授权、组内流水线继承）和ADR Q03；不得误读成引入另一套用户组成员体系。owner单独在定义owner_id，禁止普通ACL修改owner。role按操作能力判断而非取字符串最大值；workspace权限是上限，当前资源分组与直接授权并集，无deny。对象移组事务后立即用新组判定权限，不能缓存旧组授权。对象过滤应是查询的EXISTS/授权集合条件，然后计数/分页，不能取20条再过滤只剩2条却报total20。分组授权与收藏分开；转让管理员/owner资格按产品权限，不能因能读ACL就可授权。

workflow_sync_states保存base_commit/base_definition_hash、last_remote_commit、状态/错误/version；pull冲突不改published指针，push使用远端预期SHA。扩展/评测/外送先复用图settings、node输出及投递账本；S5注册的扩展包只存manifest/hash/签名元数据，不在服务器直接加载第三方JS/Python。不得以一段JSON存在代替逐成员Schema和真实证据。

### 10.9 等待累计、查询与保留

execution_waits每段记录started/finished与对应by、reason、lease_epoch；唯一(workspace,kind,execution,segment_key)，重启同段不重复插。只有从“本执行没有活动可运行任务”进入waiting才开段；任务级等待与执行级等待不能叠加扣两次。关闭区间与executions.waiting_ms递增同事务CAS，已关闭再次请求不递增。暂停、审批、重试纯等待遵PRD §15.9。

统计按workspace/kind/definition与UTC finished_at范围查询，提前完成对象授权；成本按credential/currency聚合；MTTR不把无成功样本当0。恢复查询示例：

```sql
SELECT f.id, MIN(s.finished_at) - f.finished_at AS recovery_ms
FROM workflow_executions f
LEFT JOIN workflow_executions s ON s.workspace_id=f.workspace_id
 AND s.workflow_id=f.workflow_id AND s.state='succeeded'
 AND s.finished_at>f.finished_at AND s.finished_at<=:as_of
WHERE f.workspace_id=:workspace AND f.workflow_id=:authorized_workflow
 AND f.state='failed' AND f.finished_at>=:start AND f.finished_at<:end
GROUP BY f.id, f.finished_at;
-- NULL仍未恢复；均值只含非NULL，另列未恢复数。start/end/as_of全为UTC毫秒。
SELECT credential_id, currency, SUM(amount_micros) AS amount_micros
FROM cost_records WHERE workspace_id=:workspace AND execution_id=:authorized_execution
 AND subject_kind=:kind AND created_at>=:start AND created_at<:end
GROUP BY credential_id,currency;
```

清理顺序：终态运行数据按保存策略释放中间输出引用；保留固定资产provenance及引用；已被execution引用的修订与最近50版受保护；unknown意图/预算预留、waiting任务、GC失败队列不得按终态历史顺带删除。预算成本历史被清理时，月桶已结算聚合仍保留到对应会计周期不再受配置约束；具体会计保留年限无来源，不新增期限，依PDR-05部署政策。不能删除幂等账本后又接收同一个外部请求并承诺绝不重复。

### 10.10 完整目标表DDL

SQLite评审DDL；同名表替换§4目标，非生产迁移。索引中的VARCHAR在MySQL须固定utf8mb4二进制/大小写策略，不能依库默认排序；PG BYTEA/MySQL LONGBLOB替换BLOB，DOUBLE PRECISION存有限数的限制由API和适配器一致验证。字段表见§10.10a，按实际列列出；默认“无”表示调用方须显式提供或NULL，禁止猜业务默认。

```sql
CREATE TABLE definition_publications (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL,
 subject_kind VARCHAR(16) NOT NULL CHECK(subject_kind IN ('pipeline','workflow')),
 subject_id BIGINT NOT NULL, revision_id BIGINT NOT NULL,
 name VARCHAR(64) NOT NULL, name_key VARCHAR(64) NOT NULL,
 change_note VARCHAR(200), run_as BIGINT NOT NULL,
 request_key VARCHAR(64), created_at BIGINT NOT NULL,
 created_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX publication_name ON definition_publications(workspace_id,subject_kind,subject_id,name_key);
CREATE UNIQUE INDEX publication_request ON definition_publications(workspace_id,subject_kind,subject_id,request_key);
CREATE TABLE human_tasks (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, subject_kind VARCHAR(16) NOT NULL,
 execution_id BIGINT NOT NULL, job_key VARCHAR(191) NOT NULL, iteration_hash VARCHAR(64) NOT NULL,
 attempt BIGINT NOT NULL CHECK(attempt>=1), round BIGINT NOT NULL CHECK(round>=1),
 kind VARCHAR(32) NOT NULL, state VARCHAR(16) NOT NULL DEFAULT 'waiting',
 assignment TEXT NOT NULL, min_approvals BIGINT NOT NULL CHECK(min_approvals>=1),
 form_schema TEXT NOT NULL, expires_at BIGINT, expires_by BIGINT NOT NULL DEFAULT 0,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0,
 version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX human_task_identity ON human_tasks(workspace_id,subject_kind,execution_id,job_key,iteration_hash,attempt,round);
CREATE INDEX human_task_inbox ON human_tasks(workspace_id,state,expires_at);
CREATE TABLE human_responses (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, task_id BIGINT NOT NULL,
 responder_key VARCHAR(96) NOT NULL, user_id BIGINT, user_name VARCHAR(191),
 decision VARCHAR(16) NOT NULL, comment VARCHAR(500), response TEXT,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1,
 CHECK(decision<>'returned' OR (comment IS NOT NULL AND length(trim(comment)) BETWEEN 1 AND 500))
);
CREATE UNIQUE INDEX human_response_once ON human_responses(workspace_id,task_id,responder_key);
CREATE TABLE human_task_drafts (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, task_id BIGINT NOT NULL,
 user_id BIGINT NOT NULL, response TEXT NOT NULL,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX human_draft_user ON human_task_drafts(workspace_id,task_id,user_id);
CREATE TABLE external_task_tokens (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, task_id BIGINT NOT NULL,
 token_hash VARCHAR(64) NOT NULL, expires_at BIGINT NOT NULL, expires_by BIGINT NOT NULL DEFAULT 0,
 revoked_at BIGINT, revoked_by BIGINT NOT NULL DEFAULT 0,
 used_at BIGINT, used_by BIGINT NOT NULL DEFAULT 0,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX external_token_hash ON external_task_tokens(token_hash);
CREATE TABLE data_tables (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, name VARCHAR(191) NOT NULL,
 column_schema TEXT NOT NULL, index_schema TEXT NOT NULL, row_count BIGINT NOT NULL DEFAULT 0,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX data_table_workspace ON data_tables(workspace_id,id);
CREATE TABLE data_table_rows (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, table_id BIGINT NOT NULL, data TEXT NOT NULL,
 i1 BLOB, n1 DOUBLE PRECISION, i2 BLOB, n2 DOUBLE PRECISION,
 i3 BLOB, n3 DOUBLE PRECISION, i4 BLOB, n4 DOUBLE PRECISION,
 i5 BLOB, n5 DOUBLE PRECISION,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1,
 CHECK(i1 IS NULL OR n1 IS NULL), CHECK(i2 IS NULL OR n2 IS NULL), CHECK(i3 IS NULL OR n3 IS NULL),
 CHECK(i4 IS NULL OR n4 IS NULL), CHECK(i5 IS NULL OR n5 IS NULL),
 CHECK(i1 IS NULL OR (typeof(i1)='blob' AND length(i1)<=128)),
 CHECK(i2 IS NULL OR (typeof(i2)='blob' AND length(i2)<=128)),
 CHECK(i3 IS NULL OR (typeof(i3)='blob' AND length(i3)<=128)),
 CHECK(i4 IS NULL OR (typeof(i4)='blob' AND length(i4)<=128)),
 CHECK(i5 IS NULL OR (typeof(i5)='blob' AND length(i5)<=128))
);
CREATE INDEX data_rows_table ON data_table_rows(workspace_id,table_id,id);
CREATE INDEX data_rows_i1 ON data_table_rows(workspace_id,table_id,i1,id);
CREATE INDEX data_rows_n1 ON data_table_rows(workspace_id,table_id,n1,id);
CREATE INDEX data_rows_i2 ON data_table_rows(workspace_id,table_id,i2,id);
CREATE INDEX data_rows_n2 ON data_table_rows(workspace_id,table_id,n2,id);
CREATE INDEX data_rows_i3 ON data_table_rows(workspace_id,table_id,i3,id);
CREATE INDEX data_rows_n3 ON data_table_rows(workspace_id,table_id,n3,id);
CREATE INDEX data_rows_i4 ON data_table_rows(workspace_id,table_id,i4,id);
CREATE INDEX data_rows_n4 ON data_table_rows(workspace_id,table_id,n4,id);
CREATE INDEX data_rows_i5 ON data_table_rows(workspace_id,table_id,i5,id);
CREATE INDEX data_rows_n5 ON data_table_rows(workspace_id,table_id,n5,id);
CREATE TABLE vector_collections (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, name VARCHAR(191) NOT NULL,
 dimension BIGINT NOT NULL CHECK(dimension>0), metric VARCHAR(16) NOT NULL,
 embedding_model VARCHAR(191) NOT NULL, normalization VARCHAR(16) NOT NULL,
 item_count BIGINT NOT NULL DEFAULT 0,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX vector_collection_workspace ON vector_collections(workspace_id,id);
CREATE TABLE vector_items (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, collection_id BIGINT NOT NULL,
 item_key TEXT NOT NULL,
 item_key_hash BLOB NOT NULL CHECK(typeof(item_key_hash)='blob' AND length(item_key_hash)=32),
 key_slot BIGINT NOT NULL CHECK(key_slot>=0),
 embedding BLOB NOT NULL, metadata TEXT NOT NULL,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX vector_item_key ON vector_items(workspace_id,collection_id,item_key_hash,key_slot);
CREATE TABLE execution_annotations (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, subject_kind VARCHAR(16) NOT NULL,
 execution_id BIGINT NOT NULL, key VARCHAR(191) NOT NULL, value TEXT NOT NULL, value_hash VARCHAR(64) NOT NULL,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX annotation_key ON execution_annotations(workspace_id,subject_kind,execution_id,key);
CREATE INDEX annotation_search ON execution_annotations(workspace_id,key,value_hash,execution_id);
CREATE TABLE execution_notes (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, subject_kind VARCHAR(16) NOT NULL,
 execution_id BIGINT NOT NULL, body TEXT NOT NULL,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX execution_notes_history ON execution_notes(workspace_id,subject_kind,execution_id,id);
CREATE TABLE definition_favorites (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, user_id BIGINT NOT NULL,
 subject_kind VARCHAR(16) NOT NULL, subject_id BIGINT NOT NULL,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX favorite_user ON definition_favorites(workspace_id,user_id,subject_kind,subject_id);
CREATE TABLE asset_gc_jobs (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, asset_id BIGINT NOT NULL,
 storage_key TEXT NOT NULL, state VARCHAR(16) NOT NULL DEFAULT 'candidate',
 eligible_at BIGINT NOT NULL, eligible_by BIGINT NOT NULL DEFAULT 0,
 attempt BIGINT NOT NULL DEFAULT 0, error TEXT,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX gc_asset ON asset_gc_jobs(workspace_id,asset_id);
CREATE INDEX gc_dispatch ON asset_gc_jobs(state,eligible_at,id);
CREATE TABLE budget_buckets (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, credential_id BIGINT NOT NULL,
 currency VARCHAR(16) NOT NULL, period_key VARCHAR(16) NOT NULL, timezone VARCHAR(64) NOT NULL,
 period_start BIGINT NOT NULL, period_end BIGINT NOT NULL,
 cap_micros BIGINT NOT NULL CHECK(cap_micros>=0), spent_micros BIGINT NOT NULL DEFAULT 0,
 reserved_micros BIGINT NOT NULL DEFAULT 0 CHECK(reserved_micros>=0),
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX budget_bucket_identity ON budget_buckets(workspace_id,credential_id,currency,period_key);
CREATE TABLE budget_reservations (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, bucket_id BIGINT NOT NULL,
 subject_kind VARCHAR(16) NOT NULL, execution_id BIGINT NOT NULL, operation_key VARCHAR(191) NOT NULL,
 estimated_micros BIGINT NOT NULL CHECK(estimated_micros>=0), actual_micros BIGINT,
 price_snapshot TEXT NOT NULL, state VARCHAR(16) NOT NULL DEFAULT 'reserved',
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX budget_reservation_once ON budget_reservations(workspace_id,operation_key);
CREATE INDEX budget_reservation_execution ON budget_reservations(workspace_id,subject_kind,execution_id,state);
CREATE TABLE execution_budget_limits (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, subject_kind VARCHAR(16) NOT NULL,
 execution_id BIGINT NOT NULL, credential_id BIGINT NOT NULL, currency VARCHAR(16) NOT NULL,
 cap_micros BIGINT NOT NULL CHECK(cap_micros>=0), spent_micros BIGINT NOT NULL DEFAULT 0,
 reserved_micros BIGINT NOT NULL DEFAULT 0 CHECK(reserved_micros>=0),
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX execution_budget_identity ON execution_budget_limits(workspace_id,subject_kind,execution_id,credential_id,currency);
CREATE TABLE definition_acl (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, subject_kind VARCHAR(16) NOT NULL,
 subject_id BIGINT NOT NULL, user_id BIGINT NOT NULL,
 role VARCHAR(16) NOT NULL CHECK(role IN ('all','run','view')),
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX acl_identity ON definition_acl(workspace_id,subject_kind,subject_id,user_id,role);
CREATE INDEX acl_principal ON definition_acl(workspace_id,user_id,subject_kind,subject_id);
CREATE TABLE execution_waits (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, subject_kind VARCHAR(16) NOT NULL,
 execution_id BIGINT NOT NULL, segment_key VARCHAR(64) NOT NULL, reason VARCHAR(64) NOT NULL,
 lease_epoch BIGINT NOT NULL, started_at BIGINT NOT NULL, started_by BIGINT NOT NULL DEFAULT 0,
 finished_at BIGINT, finished_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1,
 CHECK(finished_at IS NULL OR finished_at>=started_at)
);
CREATE UNIQUE INDEX execution_wait_segment ON execution_waits(workspace_id,subject_kind,execution_id,segment_key);
CREATE TABLE trigger_states (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, trigger_id BIGINT NOT NULL,
 cursor TEXT NOT NULL, lease_owner VARCHAR(128), lease_epoch BIGINT NOT NULL DEFAULT 0,
 lease_until BIGINT, window_start BIGINT, next_poll_at BIGINT, next_poll_by BIGINT NOT NULL DEFAULT 0,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX trigger_state_binding ON trigger_states(workspace_id,trigger_id);
CREATE TABLE webhook_test_windows (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, endpoint_id VARCHAR(191) NOT NULL,
 user_id BIGINT NOT NULL, revision_id BIGINT NOT NULL,
 expires_at BIGINT NOT NULL, expires_by BIGINT NOT NULL DEFAULT 0,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX webhook_test_lookup ON webhook_test_windows(workspace_id,endpoint_id,expires_at);
CREATE TABLE public_sessions (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, subject_id BIGINT NOT NULL,
 revision_id BIGINT NOT NULL, token_hash VARCHAR(64) NOT NULL,
 expires_at BIGINT NOT NULL, expires_by BIGINT NOT NULL DEFAULT 0,
 revoked_at BIGINT, revoked_by BIGINT NOT NULL DEFAULT 0,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX public_session_token ON public_sessions(token_hash);
CREATE TABLE workflow_sync_states (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL, workflow_id BIGINT NOT NULL,
 base_commit VARCHAR(64), base_definition_hash VARCHAR(64), last_remote_commit VARCHAR(64),
 state VARCHAR(16) NOT NULL, error TEXT,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0, version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX workflow_sync_identity ON workflow_sync_states(workspace_id,workflow_id);
CREATE TABLE definition_tags (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL,
 subject_kind VARCHAR(16) NOT NULL CHECK(subject_kind IN ('pipeline','workflow')),
 subject_id BIGINT NOT NULL, tag_id BIGINT NOT NULL,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX definition_tag_identity ON definition_tags(workspace_id,subject_kind,subject_id,tag_id);
CREATE INDEX definition_tag_filter ON definition_tags(workspace_id,tag_id,subject_kind,subject_id);
CREATE TABLE public_session_executions (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL,
 session_id BIGINT NOT NULL, execution_id BIGINT NOT NULL,
 request_key VARCHAR(64) NOT NULL, input_hash VARCHAR(64) NOT NULL,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX public_session_request ON public_session_executions(workspace_id,session_id,request_key);
CREATE UNIQUE INDEX public_execution_owner ON public_session_executions(workspace_id,execution_id);
CREATE TABLE public_rate_subjects (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL,
 policy_key VARCHAR(64) NOT NULL, subject_hash VARCHAR(64) NOT NULL,
 updated_at BIGINT NOT NULL, updated_by BIGINT NOT NULL DEFAULT 0,
 version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX public_rate_subject_identity ON public_rate_subjects(workspace_id,policy_key,subject_hash);
CREATE TABLE public_rate_events (
 id BIGINT NOT NULL PRIMARY KEY, workspace_id BIGINT NOT NULL,
 rate_subject_id BIGINT NOT NULL,
 created_at BIGINT NOT NULL, created_by BIGINT NOT NULL DEFAULT 0,
 version BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX public_rate_rolling_count ON public_rate_events(workspace_id,rate_subject_id,created_at,id);
```

### 10.10a 完整目标字段表

以下与§10.10实际CREATE列逐一对应；CHECK/联合唯一键见上方DDL。只列技术映射，不以VARCHAR宽度代替产品上限。

**definition_publications**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `subject_id` | BIGINT | 是 | 无 | 定义或资源分组ID，依kind解释 |
| `revision_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `name` | VARCHAR(64) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `name_key` | VARCHAR(64) | 是 | 无 | 发布名规范化检索键；大小写政策依PDR-01 |
| `change_note` | VARCHAR(200) | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `run_as` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `request_key` | VARCHAR(64) | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**human_tasks**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `execution_id` | BIGINT | 是 | 无 | 执行ID，依subject_kind解释 |
| `job_key` | VARCHAR(191) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `iteration_hash` | VARCHAR(64) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `attempt` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `round` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `kind` | VARCHAR(32) | 是 | 无 | 具体种类，受kind对应Schema限制 |
| `state` | VARCHAR(16) | 是 | 'waiting' | 状态按PRD §15.6及本节处理规则 |
| `assignment` | TEXT | 是 | 无 | 去重后的指派配置；提交时复核资格 |
| `min_approvals` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `form_schema` | TEXT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `expires_at` | BIGINT | 否 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `expires_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**human_responses**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `task_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `responder_key` | VARCHAR(96) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `user_id` | BIGINT | 否 | 无 | 工作空间成员；授权时实时核对 |
| `user_name` | VARCHAR(191) | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `decision` | VARCHAR(16) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `comment` | VARCHAR(500) | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `response` | TEXT | 否 | 无 | 敏感JSON按快照规则加密 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**human_task_drafts**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `task_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `user_id` | BIGINT | 是 | 无 | 工作空间成员；授权时实时核对 |
| `response` | TEXT | 是 | 无 | 敏感JSON按快照规则加密 |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**external_task_tokens**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `task_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `token_hash` | VARCHAR(64) | 是 | 无 | 只存SHA256，不存token明文 |
| `expires_at` | BIGINT | 是 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `expires_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `revoked_at` | BIGINT | 否 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `revoked_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `used_at` | BIGINT | 否 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `used_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**data_tables**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `name` | VARCHAR(191) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `column_schema` | TEXT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `index_schema` | TEXT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `row_count` | BIGINT | 是 | 0 | 按§10对应模块契约；关联值需同空间校验 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**data_table_rows**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `table_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `data` | TEXT | 是 | 无 | 规范JSON；字段类型依column_schema |
| `i1` | BLOB | 否 | 无 | 原文本UTF-8前128字节；精确值在data中，与n1互斥；§13 |
| `n1` | DOUBLE PRECISION | 否 | 无 | 逻辑槽有限数字值；与同号i列互斥 |
| `i2` | BLOB | 否 | 无 | 原文本UTF-8前128字节；精确值在data中，与n2互斥；§13 |
| `n2` | DOUBLE PRECISION | 否 | 无 | 逻辑槽有限数字值；与同号i列互斥 |
| `i3` | BLOB | 否 | 无 | 原文本UTF-8前128字节；精确值在data中，与n3互斥；§13 |
| `n3` | DOUBLE PRECISION | 否 | 无 | 逻辑槽有限数字值；与同号i列互斥 |
| `i4` | BLOB | 否 | 无 | 原文本UTF-8前128字节；精确值在data中，与n4互斥；§13 |
| `n4` | DOUBLE PRECISION | 否 | 无 | 逻辑槽有限数字值；与同号i列互斥 |
| `i5` | BLOB | 否 | 无 | 原文本UTF-8前128字节；精确值在data中，与n5互斥；§13 |
| `n5` | DOUBLE PRECISION | 否 | 无 | 逻辑槽有限数字值；与同号i列互斥 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**vector_collections**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `name` | VARCHAR(191) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `dimension` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `metric` | VARCHAR(16) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `embedding_model` | VARCHAR(191) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `normalization` | VARCHAR(16) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `item_count` | BIGINT | 是 | 0 | 按§10对应模块契约；关联值需同空间校验 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**vector_items**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `collection_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `item_key` | TEXT | 是 | 无 | 完整原键的规范JSON字符串封装，读后解码；保留NUL，不trim/折叠/归一化；§13 |
| `item_key_hash` | BLOB | 是 | 无 | 原键严格UTF-8的32字节SHA256，仅桶定位；须核对原键 |
| `key_slot` | BIGINT | 是 | 无 | 同hash桶碰撞槽，非负；由集合锁内分配，非客户端字段 |
| `embedding` | BLOB | 是 | 无 | 有限float32小端字节；长度=dimension×4 |
| `metadata` | TEXT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**execution_annotations**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `execution_id` | BIGINT | 是 | 无 | 执行ID，依subject_kind解释 |
| `key` | VARCHAR(191) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `value` | TEXT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `value_hash` | VARCHAR(64) | 是 | 无 | 规范JSON的SHA256；命中后复核原value |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**execution_notes**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `execution_id` | BIGINT | 是 | 无 | 执行ID，依subject_kind解释 |
| `body` | TEXT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**definition_favorites**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `user_id` | BIGINT | 是 | 无 | 工作空间成员；授权时实时核对 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `subject_id` | BIGINT | 是 | 无 | 定义或资源分组ID，依kind解释 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**asset_gc_jobs**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `asset_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `storage_key` | TEXT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `state` | VARCHAR(16) | 是 | 'candidate' | 状态按PRD §15.6及本节处理规则 |
| `eligible_at` | BIGINT | 是 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `eligible_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `attempt` | BIGINT | 是 | 0 | 按§10对应模块契约；关联值需同空间校验 |
| `error` | TEXT | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**budget_buckets**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `credential_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `currency` | VARCHAR(16) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `period_key` | VARCHAR(16) | 是 | 无 | 原预留月份键，时区与UTC边界同时快照 |
| `timezone` | VARCHAR(64) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `period_start` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `period_end` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `cap_micros` | BIGINT | 是 | 无 | 额度，币种微单位；不跨币比较 |
| `spent_micros` | BIGINT | 是 | 0 | 已结算实际金额，超估算仍如实记录 |
| `reserved_micros` | BIGINT | 是 | 0 | 在途及unknown预留；不因过月清零 |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**budget_reservations**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `bucket_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `execution_id` | BIGINT | 是 | 无 | 执行ID，依subject_kind解释 |
| `operation_key` | VARCHAR(191) | 是 | 无 | 稳定调用身份，如intent:{id}；不得每次重试随机生成 |
| `estimated_micros` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `actual_micros` | BIGINT | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `price_snapshot` | TEXT | 是 | 无 | 本次单价/单位/币种/版本的不可变JSON |
| `state` | VARCHAR(16) | 是 | 'reserved' | 状态按PRD §15.6及本节处理规则 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**execution_budget_limits**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `execution_id` | BIGINT | 是 | 无 | 执行ID，依subject_kind解释 |
| `credential_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `currency` | VARCHAR(16) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `cap_micros` | BIGINT | 是 | 无 | 额度，币种微单位；不跨币比较 |
| `spent_micros` | BIGINT | 是 | 0 | 已结算实际金额，超估算仍如实记录 |
| `reserved_micros` | BIGINT | 是 | 0 | 在途及unknown预留；不因过月清零 |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**definition_acl**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `subject_id` | BIGINT | 是 | 无 | 定义或资源分组ID，依kind解释 |
| `user_id` | BIGINT | 是 | 无 | 工作空间成员；授权时实时核对 |
| `role` | VARCHAR(16) | 是 | 无 | all/run/view；owner在定义上管理 |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**execution_waits**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | 对象类型；取值及授权按本节契约 |
| `execution_id` | BIGINT | 是 | 无 | 执行ID，依subject_kind解释 |
| `segment_key` | VARCHAR(64) | 是 | 无 | 持久化等待段幂等键 |
| `reason` | VARCHAR(64) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `lease_epoch` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `started_at` | BIGINT | 是 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `started_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `finished_at` | BIGINT | 否 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `finished_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**trigger_states**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `trigger_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `cursor` | TEXT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `lease_owner` | VARCHAR(128) | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `lease_epoch` | BIGINT | 是 | 0 | 按§10对应模块契约；关联值需同空间校验 |
| `lease_until` | BIGINT | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `window_start` | BIGINT | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `next_poll_at` | BIGINT | 否 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `next_poll_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**webhook_test_windows**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `endpoint_id` | VARCHAR(191) | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `user_id` | BIGINT | 是 | 无 | 工作空间成员；授权时实时核对 |
| `revision_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `expires_at` | BIGINT | 是 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `expires_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**public_sessions**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `subject_id` | BIGINT | 是 | 无 | 定义或资源分组ID，依kind解释 |
| `revision_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `token_hash` | VARCHAR(64) | 是 | 无 | 只存SHA256，不存token明文 |
| `expires_at` | BIGINT | 是 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `expires_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `revoked_at` | BIGINT | 否 | 无 | UTC Unix毫秒；NULL仅在允许未发生/无截止时使用 |
| `revoked_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `created_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `created_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**workflow_sync_states**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键，不自增；PRIMARY KEY |
| `workspace_id` | BIGINT | 是 | 无 | Scope验证后的空间；关联查询必须同空间 |
| `workflow_id` | BIGINT | 是 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `base_commit` | VARCHAR(64) | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `base_definition_hash` | VARCHAR(64) | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `last_remote_commit` | VARCHAR(64) | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `state` | VARCHAR(16) | 是 | 无 | 状态按PRD §15.6及本节处理规则 |
| `error` | TEXT | 否 | 无 | 按§10对应模块契约；关联值需同空间校验 |
| `updated_at` | BIGINT | 是 | 无 | UTC Unix毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 对应同前缀时间的设置/操作人；0=system |
| `version` | BIGINT | 是 | 1 | 条件写版本；不可变行禁止覆盖 |

**definition_tags**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键 |
| `workspace_id` | BIGINT | 是 | 无 | 已验证空间 |
| `subject_kind` | VARCHAR(16) | 是 | 无 | pipeline或workflow |
| `subject_id` | BIGINT | 是 | 无 | 同空间定义ID |
| `tag_id` | BIGINT | 是 | 无 | 复用同空间tags.id |
| `created_at` | BIGINT | 是 | 无 | UTC毫秒 |
| `created_by` | BIGINT | 是 | 0 | 操作者；0仅系统 |
| `version` | BIGINT | 是 | 1 | 乐观锁版本 |

**public_session_executions**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键 |
| `workspace_id` | BIGINT | 是 | 无 | 已验证空间 |
| `session_id` | BIGINT | 是 | 无 | public_sessions.id，同空间 |
| `execution_id` | BIGINT | 是 | 无 | workflow_executions.id；一次执行仅归一个公共会话 |
| `request_key` | VARCHAR(64) | 是 | 无 | 客户端请求幂等键的SHA256；不是访问令牌 |
| `input_hash` | VARCHAR(64) | 是 | 无 | 规范化输入/附件引用摘要，用于重复键冲突检测 |
| `created_at` | BIGINT | 是 | 无 | 接受请求UTC毫秒 |
| `created_by` | BIGINT | 是 | 0 | 公共匿名提交为0，不当成run_as |
| `version` | BIGINT | 是 | 1 | 乐观锁版本 |

**public_rate_subjects**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 雪花主键；限流串行化对象 |
| `workspace_id` | BIGINT | 是 | 无 | 服务端入口确定的空间 |
| `policy_key` | VARCHAR(64) | 是 | 无 | endpoint+操作类别的规范化摘要；不含密钥 |
| `subject_hash` | VARCHAR(64) | 是 | 无 | 带服务端密钥的IP或session作用域摘要；不是裸IP散列 |
| `updated_at` | BIGINT | 是 | 无 | 最近限流检查UTC毫秒 |
| `updated_by` | BIGINT | 是 | 0 | 系统限流器 |
| `version` | BIGINT | 是 | 1 | 事务CAS/互斥版本 |

**public_rate_events**

| 字段 | SQL类型 | 必填 | 默认 | 含义/约束 |
|---|---|---|---|---|
| `id` | BIGINT | 是 | 无 | 接受一次限流额度的事件ID |
| `workspace_id` | BIGINT | 是 | 无 | 同rate_subject空间 |
| `rate_subject_id` | BIGINT | 是 | 无 | 持锁检查过的public_rate_subjects.id |
| `created_at` | BIGINT | 是 | 无 | 额度接受UTC毫秒；滚动窗口计数依据 |
| `created_by` | BIGINT | 是 | 0 | 系统限流器 |
| `version` | BIGINT | 是 | 1 | 版本字段 |

### 10.11 其它目标的字段增量与索引

新增列“无DEFAULT”意味着显式写入/先可空回填，不意味着让生产存量直接NOT NULL失败；owner无法回填时不收紧，阻断认领前授权。下例针对§4已知目标和最小旧表夹具，不是全库migration。pipeline_run_steps的end_reason、workflow_executions的context等均属本轮新增技术列，不能漏进字段盘点。

```sql
ALTER TABLE trigger_bindings RENAME COLUMN pipeline_id TO subject_id;
ALTER TABLE trigger_deliveries RENAME COLUMN pipeline_id TO subject_id;
ALTER TABLE trigger_deliveries RENAME COLUMN run_id TO execution_id;
DROP INDEX trigger_deliveries_dedupe;
DROP INDEX trigger_deliveries_history;
CREATE UNIQUE INDEX trigger_delivery_identity ON trigger_deliveries(workspace_id,subject_kind,subject_id,dedupe_key,redelivery_seq);
CREATE INDEX trigger_delivery_history ON trigger_deliveries(workspace_id,subject_kind,subject_id,created_at);
ALTER TABLE trigger_deliveries ADD COLUMN lease_owner VARCHAR(128);
ALTER TABLE trigger_deliveries ADD COLUMN lease_epoch BIGINT NOT NULL DEFAULT 0;
ALTER TABLE trigger_deliveries ADD COLUMN lease_until BIGINT;
ALTER TABLE pipeline_run_notifications RENAME TO execution_notifications;
ALTER TABLE execution_notifications RENAME COLUMN run_id TO execution_id;
ALTER TABLE execution_notifications ADD COLUMN subject_kind VARCHAR(16) NOT NULL DEFAULT 'pipeline';
DROP INDEX pipeline_run_notifications_once;
CREATE UNIQUE INDEX execution_notification_once ON execution_notifications(workspace_id,subject_kind,execution_id,job_key,event,channel_index);
ALTER TABLE workflows ADD COLUMN owner_id BIGINT;
ALTER TABLE workflows ADD COLUMN trigger_run_as BIGINT;
ALTER TABLE workflows ADD COLUMN lifecycle VARCHAR(16) NOT NULL DEFAULT 'active';
ALTER TABLE workflows ADD COLUMN archived_at BIGINT;
ALTER TABLE workflows ADD COLUMN archived_by BIGINT NOT NULL DEFAULT 0;
ALTER TABLE pipelines ADD COLUMN owner_id BIGINT;
ALTER TABLE pipelines ADD COLUMN lifecycle VARCHAR(16) NOT NULL DEFAULT 'active';
ALTER TABLE pipelines ADD COLUMN archived_at BIGINT;
ALTER TABLE pipelines ADD COLUMN archived_by BIGINT NOT NULL DEFAULT 0;
ALTER TABLE workflow_executions ADD COLUMN context TEXT;
ALTER TABLE workflow_executions ADD COLUMN waiting_ms BIGINT NOT NULL DEFAULT 0;
ALTER TABLE workflow_executions ADD COLUMN response_state VARCHAR(32) NOT NULL DEFAULT 'not_applicable';
ALTER TABLE workflow_executions ADD COLUMN response_deadline_at BIGINT;
ALTER TABLE workflow_executions ADD COLUMN response_deadline_by BIGINT NOT NULL DEFAULT 0;
ALTER TABLE workflow_executions ADD COLUMN response_delivered_at BIGINT;
ALTER TABLE workflow_executions ADD COLUMN response_delivered_by BIGINT NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX workflow_execution_number ON workflow_executions(workspace_id,number);
CREATE INDEX workflow_execution_statistics ON workflow_executions(workspace_id,workflow_id,state,finished_at);
ALTER TABLE pipeline_runs ADD COLUMN waiting_ms BIGINT NOT NULL DEFAULT 0;
ALTER TABLE pipeline_run_steps ADD COLUMN end_reason VARCHAR(64) NOT NULL DEFAULT '';
ALTER TABLE workflow_node_runs ADD COLUMN resolved_params TEXT;
ALTER TABLE workflow_node_runs ADD COLUMN seed BIGINT;
ALTER TABLE workflow_node_runs ADD COLUMN redo_round BIGINT NOT NULL DEFAULT 0;
ALTER TABLE workflow_node_runs ADD COLUMN error_policy VARCHAR(16) NOT NULL DEFAULT 'stop';
ALTER TABLE assets ADD COLUMN provenance TEXT;
ALTER TABLE node_cache ADD COLUMN source_subject_id BIGINT;
CREATE INDEX node_cache_subject ON node_cache(workspace_id,source_subject_id,last_used_at);
ALTER TABLE cost_records ADD COLUMN id BIGINT;
ALTER TABLE cost_records ADD COLUMN credential_id BIGINT;
ALTER TABLE cost_records ADD COLUMN subject_kind VARCHAR(16) NOT NULL DEFAULT 'workflow';
ALTER TABLE cost_records ADD COLUMN reservation_id BIGINT;
ALTER TABLE cost_records RENAME COLUMN quantity TO quantity_micros;
ALTER TABLE cost_records ADD COLUMN unit_price_micros BIGINT;
ALTER TABLE cost_records ADD COLUMN price_snapshot TEXT;
ALTER TABLE cost_records ADD COLUMN event_key VARCHAR(191);
CREATE UNIQUE INDEX cost_record_identity ON cost_records(id);
CREATE UNIQUE INDEX cost_record_event ON cost_records(workspace_id,event_key);
CREATE INDEX cost_credential_period ON cost_records(workspace_id,credential_id,currency,created_at);
ALTER TABLE host_models ADD COLUMN workspace_id BIGINT;
ALTER TABLE host_models ADD COLUMN downloaded_bytes BIGINT NOT NULL DEFAULT 0;
ALTER TABLE host_models ADD COLUMN error TEXT;
ALTER TABLE host_models ADD COLUMN lease_owner VARCHAR(128);
ALTER TABLE host_models ADD COLUMN lease_epoch BIGINT NOT NULL DEFAULT 0;
ALTER TABLE host_models ADD COLUMN lease_until BIGINT;
CREATE UNIQUE INDEX host_model_identity ON host_models(workspace_id,host_id,model_id);
ALTER TABLE cron_jobs ADD COLUMN managed_subject_kind VARCHAR(16);
ALTER TABLE cron_jobs ADD COLUMN managed_subject_id BIGINT;
ALTER TABLE cron_jobs ADD COLUMN managed_node_id VARCHAR(191);
ALTER TABLE cron_jobs ADD COLUMN managed_revision_id BIGINT;
CREATE INDEX cron_managed_subject ON cron_jobs(workspace_id,managed_subject_kind,managed_subject_id);
ALTER TABLE workspaces ADD COLUMN timezone VARCHAR(64);
DROP INDEX webhook_endpoints_route;
ALTER TABLE execution_parents RENAME COLUMN parent_run_id TO parent_execution_id;
ALTER TABLE execution_parents ADD COLUMN parent_subject_kind VARCHAR(16) NOT NULL DEFAULT 'pipeline';
ALTER TABLE execution_parents ADD COLUMN child_subject_kind VARCHAR(16) NOT NULL DEFAULT 'pipeline';
```

字段语义：`owner_id/trigger_run_as`为空时禁止据此授权；`lifecycle`只取active/archived；`context/resolved_params/provenance/price_snapshot`按JSON Schema与敏感快照规则封装；`waiting_ms/downloaded_bytes/redo_round`非负；lease_epoch只递增；`response_*`依§10.7；所有managed_*必须全NULL或完整同空间合法关联。parent_execution_id/parent_subject_kind保留作检索快照，必须与intent真实父身份一致，不能让请求体重绑。

cost_records.id在正式建表必须为主键；本增量夹具用可空ADD+唯一索引展示回填阶段，生产应校验全部回填后NOT NULL/主键重建。host_models.workspace_id/source_subject_id同理；不得以夹具允许NULL表示业务允许无租户行。trigger_bindings/deliveries及通知因尚未实现，正式目标直接建通用名称，不执行RENAME绕行。

### 10.12 字段盘点、三库验证与未覆盖边界

本轮字段清单以§10.10完整DDL及§10.11每个ADD/RENAME为精确技术定义；§4的309列只保留历史来源证据。新表默认/可空性、CHECK、唯一键均可由DDL读取，不再给只有version的占位表。SQL中的VARCHAR(191)是跨库索引映射，产品文本限制依PRD PDR-01，超出索引映射的合法输入须使用hash检索键或调整映射，不能截断数据。

实施前必做：三库migration builder与SQLite重建表保全部旧列/索引、MySQL DDL重入/PostgreSQL锁时长、历史行backfill/坏数据隔离、发布授权确认、真实旧二进制拒混跑；向量字节序/有限数/数字排序及JSON规范化跨语言对照。审查SQLite约束通过只说明本稿SQL和所测约束，不证明这些实际迁移/业务并发/外部幂等通过。

仍未定稿：PDR产品默认、V6/V7真实服务商/模型及其manifest、逐扩展OAuth/外送协议版本、三库执行计划/规模性能、跨平台沙箱/真实SSH与GPU、全部长尾专属屏/六语言译文。这些在对应阶段前阻断，不计为“完整平台已可直接上线”。

## 11. v1.6-review 组织关联、公共结果与限流查询

本节及§10.10新增4张完整目标表是现有需求的技术载体，不是已执行迁移。其字段表见§10.10a。旧§4仍只作来源投影，现行补充以本节为准。

### 11.1 资源分组与标签

复用`pipeline_contexts`，现有`kind='group'`继续表示流水线资源分组；工作流分组使用新增技术判别值`kind='workflow_group'`。不把environment或人员角色当资源分组。`workflows.group_id`引用同空间workflow_group，`pipelines`保留既有group引用方式。分组目标配置字段不成为组授权的执行主机绑定；移动对象时检查新组类型/空间并原子更新定义version，提交后重算继承ACL。实施须补共享context枚举/列表过滤回归，当前产品尚未支持workflow_group。

既有`tags`是标签资源，`host_tags`只关联主机；不能复用其host_id承载工作流ID。新增`definition_tags`关联pipeline/workflow与同空间tags。授权来自定义，标签不授予对象权限；标签计数/筛选只统计已授权定义。删除标签与关联同事务，删除定义清理关联；不删除被其他定义引用的标签资源。数据库未声明多态外键，服务层须在同事务检查对象、标签和分组确属同空间。

列表查询先验证当前工作空间`view`上限，再按下列授权集合查询；`:workspace_view`及`:object_override`只能由服务端权限解析器提供，不能从请求体读取。override仅代表既有权限策略明确允许绕过对象ACL的身份，普通workspace view不能充当override。owner不绕过workspace权限上限。

```sql
WITH visible AS (
 SELECT w.* FROM workflows w
 WHERE w.workspace_id=:workspace AND :workspace_view=1
  AND w.lifecycle=:lifecycle
  AND (:object_override=1 OR w.owner_id=:user_id OR EXISTS (
   SELECT 1 FROM definition_acl a
   WHERE a.workspace_id=w.workspace_id AND a.user_id=:user_id
    AND a.role IN ('view','run','all')
    AND ((a.subject_kind='workflow' AND a.subject_id=w.id)
      OR (a.subject_kind='workflow_group' AND a.subject_id=w.group_id))
  ))
  AND (:tag_id IS NULL OR EXISTS (
   SELECT 1 FROM definition_tags dt
   WHERE dt.workspace_id=w.workspace_id AND dt.subject_kind='workflow'
    AND dt.subject_id=w.id AND dt.tag_id=:tag_id
  ))
)
SELECT id,name,group_id,updated_at,version FROM visible
ORDER BY updated_at DESC,id DESC LIMIT :limit OFFSET :offset;
```

total查询复用同一visible集合并`SELECT COUNT(*) FROM visible`，同一只读快照返回total/items；不能在分页后做ACL或标签过滤。使用keyset时增加`updated_at < :cursor_time OR (updated_at=:cursor_time AND id<:cursor_id)`并编码查询条件，total仍为未加游标的授权集合；游标不是授权凭据。

目标索引补充如下；真实迁移先检查是否已有等价索引，不重复建同列索引。名称搜索的包含查询不能宣称由这些B-tree加速，全文方案待规模实测。

```sql
CREATE INDEX workflow_authorized_list ON workflows(workspace_id,lifecycle,updated_at,id);
CREATE INDEX workflow_group_lookup ON workflows(workspace_id,group_id,id);
CREATE INDEX workflow_context_groups ON pipeline_contexts(workspace_id,kind,position,id);
```

### 11.2 公共会话的结果归属与幂等

`public_sessions.subject_id`只表示workflow，不混用pipeline；若以后支持其它类型，须先版本化契约。token随机生成，只保存hash；服务端按hash定位会话并验证expires/revoked、空间公共入口开关、目标可用性及该操作资格。ID不能代替token，公共会话不能获取workspace成员权限。

接受提交时锁定有效会话与当前可执行发布资格；`request_key`为请求幂等键摘要，`input_hash`为规范化JSON及有序附件引用摘要（不含令牌）。同键同输入返回已有执行；同键不同输入409，不执行第二次。创建execution、输入资产引用与public_session_executions必须同事务，执行身份取已授权发布身份，不取匿名created_by。唯一键冲突应回滚后重读匹配项，不吞数据库错误。会话绑定的workflow/revision必须与execution一致，由事务服务层检查，不能仅依赖关联表的唯一键。

结果读取必须经映射关系，不能只凭“同workflow”或调用者提供的execution_id查询整个历史：

```sql
SELECT e.id,e.state,e.end_reason,e.created_at,e.finished_at
FROM public_sessions s
JOIN public_session_executions p ON p.workspace_id=s.workspace_id AND p.session_id=s.id
JOIN workflow_executions e ON e.workspace_id=p.workspace_id AND e.id=p.execution_id
 AND e.workflow_id=s.subject_id AND e.revision_id=s.revision_id
WHERE s.token_hash=:token_hash AND s.workspace_id=:workspace
 AND s.revoked_at IS NULL AND s.expires_at>:now
 AND e.id=:execution_id;
```

此查询还需前置入口开关/目标可用性检查；只返回公共契约允许的结果投影，不返回context、内部日志、凭据或任意中间资产。下载另查该执行授权输出引用并签发短期下载资格；资产ID不可直接当下载权。PDR-03会话时限按已确认规则执行，未定义的其它共享范围不能从本表推导默认值。

### 11.3 跨实例滚动限流

采用持久化滚动事件技术方案，避免固定整分钟边界允许瞬间双倍请求。现有ADR Q09的表单10次/IP/分钟、聊天20条/会话/分钟不改；配置上限从已验证入口配置读入，不写死在DDL。不同操作使用独立policy_key；IP从可信代理规则解析，无可信代理时只用连接对端，不能直接信任任意X-Forwarded-For。

policy_key绑定空间/入口/操作；subject_hash对作用域及IP或会话ID作服务端HMAC，不存裸IP或低熵IP的普通hash。摘要密钥轮换须保留活动窗口的连续计数或在切换时同步转移，不能通过换key重置额度；密钥管理与隐私保留期限依部署策略，不新增产品默认。

同一subject的检查在单个事务串行化：先创建或取得public_rate_subjects唯一行，再锁行（PG/MySQL SELECT FOR UPDATE；SQLite写事务）并CAS version。`:now`使用统一数据库时钟，至少不小于该行updated_at；`:cutoff=now-60000`。删除该subject中created_at<=cutoff的过期事件，再计数`cutoff < created_at <= now`，小于配置上限才插入事件并提交；达到上限返回429，Retry-After由最早有效事件到期计算，至少1秒。更新锁行与计数/插入不能分事务。

```sql
UPDATE public_rate_subjects SET version=version+1,updated_at=:now
WHERE workspace_id=:workspace AND id=:subject_id AND version=:expected_version;
DELETE FROM public_rate_events
WHERE workspace_id=:workspace AND rate_subject_id=:subject_id AND created_at<=:cutoff;
SELECT COUNT(*) AS used,MIN(created_at) AS oldest
FROM public_rate_events WHERE workspace_id=:workspace AND rate_subject_id=:subject_id
 AND created_at>:cutoff AND created_at<=:now;
```

UPDATE影响行数不为1时回滚并重读重试整个事务；DDL唯一键竞争也必须走各库事务适配器，不将SQLite语法当通用SQL。一次额度接受对应一个event ID；429不插event，故障在事务提交前回滚不占额。提交后网络断开已占额，不能凭客户端声称“未收到”退还；请求是否已创建执行由11.2幂等键恢复。重试请求仍先限流，再回传既有执行，避免幂等查询成为不限频旁路。

垃圾回收锁定同一subject再删除过期events，不得与并发额度接受竞态；subject删除需确认无活跃events并满足既有保留策略。不同策略若要求一次请求同时扣IP与session额度，按稳定subject ID顺序取得所有锁，一次事务共同通过或回滚；不得半扣。此处只用于公共入口，不能替代凭据QPS、外部在途并发租约或预算预留。

### 11.4 验证门槛

文档检查须覆盖4张新表字段/唯一键、重复标签、同键重复提交/输入冲突服务守卫、一个执行不得挂到另一会话、撤销/过期/跨会话结果隔离、授权过滤先于分页、滚动分钟边界及超额拒绝。SQLite顺序夹具只证明结构和查询；多实例争抢最后一额度、时钟回拨、代理伪造、上传流超限、真实三库执行计划仍需对应阶段运行验收，不计为本轮已通过。

## 12. v1.8-review 数据表与向量节点查询契约

### 12.1 绑定、类型与检索

新增[3个S2内置节点契约](../../src/shared/workflow-contracts/workflow-data-nodes.json)：data.table、vector.upsert、vector.search。数据表对应清单B008；向量节点来自v3 §7.4/§12.1，不虚构B节源成员。执行前从已验证workspace范围检查表/集合读写资格，不接受请求体提供的权限布尔值。按实际column_schema校验列名、类型、必填、null和默认值；未知列拒绝，不静默删字段。列/索引Schema摘要在节点尝试上下文冻结并复查，不能把会因行数变化而递增的table.version当独立schema版本。

filters只接受声明列及固定操作枚举；值作绑定参数，列名通过index_schema映射到i1..i5/n1..n5白名单。索引槽编号及方向同样不能来自未经映射的SQL文本。日期/布尔/JSON等不支持的物理比较不得偷偷按字符串比较；非索引字段明确全扫描，不能宣称被B-tree覆盖。SQL示例表示column_schema中某数字列已映射n1，且服务端已经完成资格验证：

```sql
SELECT id,version,data FROM data_table_rows
WHERE workspace_id=:workspace AND table_id=:table_id
 AND n1>=:minimum AND (:after_id IS NULL OR id>:after_id)
ORDER BY id ASC LIMIT :limit;
```

data.table select的limit显式必填，不新增产品默认上限；after_row_id只支持默认id升序，不与order_by混用。显式order_by按字段值排序并用id打破同值平局，后续多列游标须另行版本化，不复用一个row_id假装可定位任意排序。输出每个Item含`{row_id,version,data}`，保留用户data中的id/version原值，不能与存储元数据混淆。空结果输出空items。

索引当前顺序：data_rows_table(workspace,table,id)、每槽data_rows_iN/nN(workspace,table,typed_prefix_or_number,id)、vector_item_key(workspace,collection,item_key_hash,key_slot)、集合workspace/id。数值比较用nN而非iN，index_schema最多5逻辑槽，不能按10物理索引声称支持10列。字符串映射与精确排序见§13；三库实际执行计划仍须S2验证；具体metric集合及适配按§10.3与节点评审，不因本节而确认外部服务商。

**v1.8发现、v1.9补充：** iN与item_key的VARCHAR(191)投影不能作为任意用户JSON字符串的产品上限。§13已补有界二进制前缀、完整原值残余过滤与向量hash桶碰撞槽，§10.10/10.10a同步为当前目标；S2真实三库执行计划/锁与大规模性能仍待实测。不得将§13的技术设计验收误算生产迁移通过。

### 12.2 写入事务与重放

本地数据写入与execution_intents完成标记、结果引用、当前执行租约栅栏同事务；不先提交数据再单独提交intent。intent唯一键限定一次节点尝试，系统接管重放同一intent时返回原结果；用户显式新尝试的再执行沿既有策略，不能无条件承诺跨所有attempt“只执行一次”。insert生成的row_id在意图中冻结，不能在每次重放重新生成。

表容量锁：对data_tables对应行取得写锁并读取当前version；SQLite写事务，PG/MySQL行锁。下面计数CAS、行插入及intent结果必须共同提交；任一失败全部回滚。row_count与真实行数不符时停止该表写入并进入校验/修复，不能继续用错误计数放行。

```sql
UPDATE data_tables SET row_count=row_count+1,version=version+1,
 updated_at=:now,updated_by=:actor
WHERE workspace_id=:workspace AND id=:table_id
 AND version=:table_version AND row_count<100000;
INSERT INTO data_table_rows
 (id,workspace_id,table_id,data,n1,created_at,created_by,updated_at,updated_by,version)
VALUES (:row_id,:workspace,:table_id,:data,:n1,:now,:actor,:now,:actor,1);
```

UPDATE影响行数不是1时不得执行INSERT。`:data`为已经校验的规范JSON，n1从同一data字段计算，其它索引槽由同一规则写入，不允许二者脱节。以下是已校验单行更新/删除的CAS，不提供无条件批量删除：

```sql
UPDATE data_table_rows SET data=:data,n1=:n1,version=version+1,
 updated_at=:now,updated_by=:actor
WHERE workspace_id=:workspace AND table_id=:table_id
 AND id=:row_id AND version=:expected_version;
DELETE FROM data_table_rows
WHERE workspace_id=:workspace AND table_id=:table_id
 AND id=:row_id AND version=:expected_version;
```

两条分别是不同operation，不连续执行；零行表示冲突/对象已不可用，不回退无条件更新。更新后同事务递增父表version并写审计；删除成功后同事务在row_count>0条件下减1并递增version，否则回滚重查计数。values与filters/order/分页参数不能在不适用的operation中静默忽略，Schema已拒绝相应组合。

### 12.3 向量存储与缓存快照

向量写入/查询只引用已配置集合，不在节点覆盖模型、维度、metric或normalization。输入数字先转有限float32，再检查字节长度和cosine非零；例如1e-50转float32为0，不能只在转码前判非零。持久化为小端float32，字节数dimension×4；非有限/溢出拒绝，不截断、补零或混入另一维度。

读取一致快照，先取集合元数据和version，再取该集合items。下面两条在同一只读事务执行；所有结果标记这一collection_version。进程缓存键含workspace、collection、version及模型/维度/metric/normalization；每次新检索先读取数据库version，不只靠进程TTL或本机失效通知：

```sql
SELECT id,dimension,metric,embedding_model,normalization,version
FROM vector_collections WHERE workspace_id=:workspace AND id=:collection_id;
SELECT item_key,embedding,metadata FROM vector_items
WHERE workspace_id=:workspace AND collection_id=:collection_id
ORDER BY id ASC;
```

命中相同版本的缓存可复用；写后新版本不得读旧缓存。写入锁定集合行，按(workspace,collection,item_key)判断新增或更新；SQLite/MySQL新增时原子检查item_count<20000，已有键更新不占新额度。item_count/version、向量行、intent结果同事务；不能先无锁count再insert，也不能无条件每次upsert加1。PG pgvector仍是独立适配，不把BLOB或这段SQL冒充向量索引。

持有集合锁并核对item_key是否存在后执行下列容量CAS；`:is_new`只能为服务端查得的0/1，`:enforce_bound`由数据库适配能力决定（SQLite/MySQL为1），不能由节点输入关闭。CAS失败回滚，不修改向量行：

```sql
UPDATE vector_collections SET item_count=item_count+:is_new,version=version+1,
 updated_at=:now,updated_by=:actor
WHERE workspace_id=:workspace AND id=:collection_id AND version=:collection_version
 AND (:is_new=0 OR :enforce_bound=0 OR item_count<20000);
```

同一事务对已有键执行UPDATE，对新增键执行INSERT，二者互斥；SQL均为参数化普通语句，不依赖某库UPSERT语法。集合锁、唯一键及intent重放共同保障，不拿客户端重复提交当新插入：

```sql
UPDATE vector_items SET embedding=:embedding,metadata=:metadata,
 version=version+1,updated_at=:now,updated_by=:actor
WHERE workspace_id=:workspace AND collection_id=:collection_id
 AND id=:item_id AND version=:expected_version;
INSERT INTO vector_items
 (id,workspace_id,collection_id,item_key,item_key_hash,key_slot,embedding,metadata,created_at,created_by,updated_at,updated_by,version)
VALUES (:item_id,:workspace,:collection_id,:item_key,:item_key_hash,:key_slot,:embedding,:metadata,:now,:actor,:now,:actor,1);
```

已有键按§13查hash桶并复核完整原键后，以item_id/version更新，影响行数须为1；新增唯一键冲突须整体回滚重读，不在已递增计数的事务里吞错再UPDATE。所有值/元数据来自冻结参数和已校验float32字节；item_id在意图中冻结，item_key_hash/key_slot只由服务端派生，不能来自节点输入。

内置检索的score保留metric，不统一冒充概率：cosine=点积/(两模长乘积)，dot=点积，euclidean为欧式距离。cosine/dot降序，euclidean升序，同分按item_key稳定排序；返回至多top_k个，零项返回空items。距离集合仍按既有设计建议评审，节点不新增选择器；此处只给已有三种技术候选的明确口径。未按L2归一化存储时，不用点积直接代替cosine。

默认节点缓存关闭。显式opt-in时仍需权限与资源存续检查、冻结的表/集合快照身份及缓存失效规则，不能因为输入向量相同就复用不同版本集合的搜索结果。实际服务商嵌入、真实集合性能、跨实例同键写入/容量争抢和三库事务隔离继续作为S2验收门槛。

## 13. v1.9-review 长文本索引与向量键映射

### 13.1 原值与索引派生值

此节覆盖§10/§12此前未冻结的长字符串物理映射。用户文本在data JSON或完整item_key中保留，不trim、不大小写折叠、不自动做Unicode归一化。精确相等比较原值，不能交给部署数据库的默认collation。已有定义/请求大小限制继续有效，128字节索引前缀不是产品文本长度上限。

数据表文本槽iN保存`strict_utf8(value)[:128]`原始字节，不解码前缀；切在中文/emoji编码中间仍是合法索引字节。完整字符串只从data读取。空字符串是长度0的非NULL BLOB；缺失和JSON null的槽为SQL NULL，二者仍须通过data的字段存在性区分。非法Unicode代理码点拒绝，不以替换字符改变原值。数字槽nN仍按有限数规则，文本/数字不能同时占同一逻辑槽。

| 载体 | SQLite评审目标 | PostgreSQL适配 | MySQL适配 |
|---|---|---|---|
| i1..i5文本前缀 | BLOB，typeof为blob且length≤128 | BYTEA，octet_length≤128 | VARBINARY(128)，禁止BINARY补齐及静默截断 |
| 完整vector item_key | TEXT，规范JSON字符串封装，不建全文唯一键 | TEXT，相同封装转义NUL，不直接建原键B-tree | LONGTEXT utf8mb4，相同封装，解码原值后由应用精确比较 |
| item_key_hash | BLOB，必须32字节 | BYTEA，octet_length=32 | VARBINARY(32)，必须32字节 |
| key_slot | BIGINT非负，集合锁内分配 | BIGINT及非负约束 | BIGINT及非负约束 |

SQLite的typeof/length是评审DDL，不能原样复制到PG/MySQL。索引均有workspace/table或collection前缀；iN索引只含至多128字节，向量唯一索引只含32字节hash和整数槽，不随用户文本长度增长。若目标MySQL版本不强制CHECK，适配器仍须执行相同字节长度/非负验证，并实测禁止截断；不假定数据库默认行为已安全。

### 13.2 候选筛选、精确结果与分页

字符串精确比较使用合法Unicode原值的UTF-8字节序；不按数据库locale比较。前缀映射保持非严格单调性，但不是一一映射。候选规则如下，所有候选均须在同一读取快照中，按原data完成类型/存在性及全文比较：

| 用户谓词 | 可用的候选条件 | 必须执行的全文复核 |
|---|---|---|
| eq | iN=:prefix | 原字符串完整相等；共享前缀不算相等 |
| gt/gte | iN>=:prefix | 原字符串按请求的严格/非严格比较 |
| lt/lte | iN<=:prefix | 原字符串按请求的严格/非严格比较 |
| ne | 所有文本候选，不排除相等前缀桶 | 原字符串不等；否则会漏掉同前缀不同全文 |
| is_null | iN IS NULL | data中该字段显式为JSON null；缺失不能伪装null |

下列是eq的候选流，不是最终用户分页：

```sql
SELECT id,version,data FROM data_table_rows
WHERE workspace_id=:workspace AND table_id=:table_id
 AND i1=:prefix AND id>:scan_after_id
ORDER BY id ASC LIMIT :scan_batch;
```

扫描游标推进至实际消耗的候选id；每条先复核原值，再加入最终结果，达到请求limit才停止。预取但未消耗的候选不得被游标跳过。total必须对授权后的完整候选集复核并计数，不能COUNT前缀桶，也不能只拿前limit条过滤后返回短页。多条件AND可取安全候选交集，OR须取并集后去重；没有可证明安全的推导时退回全扫描，不为提速遗漏匹配行。

显式按文本字段排序时，先按prefix分桶，再在每个完整桶中按全文字节及id排序；相同prefix的桶必须完整读取后才能截取最终结果。direction只决定全文升降序，相同全文均按id升序打破平局；不得把prefix/id排序直接当全文排序。null/missing的排序位置必须由节点显式nulls指定，二者可处于同一空值排序桶并按id升序排列，但等值/存在性判断仍区分。data.table的order_by因此复用核心Sort的nulls=first/last必填项，无默认值；其row_id游标仍只适用于默认id升序。

遇到超大同前缀桶，按已配置执行资源/超时预算做完整排序或明确失败，不截断桶后宣称精确结果。非二进制locale/大小写折叠查询不能复用此字节前缀作有损筛选，需独立版本化比较键或全扫描精确比较；本批不新增此产品模式。字符串结果及total均先授权/复核再分页，与§11.1的ACL顺序一致。

### 13.3 向量键哈希桶与碰撞

完整item_key以规范JSON字符串封装保存TEXT，例如逻辑键中的NUL保存为JSON转义文本，解决PG TEXT不能含原始NUL的问题；同一规则适用于三库，读取后必须解码再比较/输出，不能把带引号或转义的存储文本返回给用户。item_key_hash=SHA256(strict_utf8(解码后原键))的32字节摘要，不哈希JSON封装文本。key_slot只作同hash桶内的物理区分，唯一键为(workspace,collection,item_key_hash,key_slot)。数据库不再直接对任意长item_key建唯一索引，hash相同也不代表同一个逻辑键。

每次写入沿§12.3取得集合行锁，查询hash桶；在应用层按完整原键精确匹配：已有键使用原item_id/key_slot和当前version更新；不存在时分配桶内max(key_slot)+1（空桶从0开始），插入新item_id。不同完整键即使SHA256相同也分配不同槽，不能覆盖或拒绝另一个合法键。槽溢出必须显式失败，不回绕；key_slot不是用户输入，也不是逻辑键身份。

```sql
SELECT id,item_key,key_slot,version FROM vector_items
WHERE workspace_id=:workspace AND collection_id=:collection_id
 AND item_key_hash=:item_key_hash
ORDER BY key_slot ASC;
```

所有写入路径（节点、导入、维护）都必须取得同一集合锁，不能仅依赖hash/slot唯一键保证完整原键唯一；直接绕过锁插入第二个相同原键属于实现缺陷。锁、已有/新增判断、item_count、向量行和intent结果同事务提交。查桶及原键比对也必须在集合锁内，不能先无锁查不存在再加锁插入。

hash算法/编码规则属于存储契约版本，不能在已有行上无版本切换。若真实历史表已采用旧VARCHAR键/索引，须全停写入，按完整原值重算摘要和槽，验证每个逻辑原键唯一、计数/向量字节未变后才切换索引。JSON字符串封装须由迁移版本判定，不能根据内容“像JSON字符串”猜测是否已封装；重入不得二次封装，旧原键本身可能包含引号或反斜杠。有损截断过的原值必须从可信源恢复，无法恢复则隔离并报告，禁止凭前缀猜回。工作流新表尚未迁移，本轮只修改目标DDL，不运行生产升级。

### 13.4 文档验证与阶段门槛

参考夹具验证二进制前缀128字节边界、中文/emoji/空字符串、同前缀不同全文、范围条件、全文排序、先复核后分页/计数、原Unicode区别及NUL；强制相同hash验证两个完整键可并存、同键重放不新增、空间/集合隔离、已有键更新不占新增额度。SQLite目标DDL同时拒绝超长/非BLOB前缀与非32字节hash。

这些是可重复的设计参考算法与SQLite结构证据，不能证明当前产品已实现，也不能替代PG/MySQL类型/执行计划、真实并发集合锁、数据迁移及规模资源测试。G-11索引技术设计可按精确映射与参考边界验收，真实三库阶段门槛继续保留；不以静态通过批准生产迁移或新增产品默认。

## 14. v1.11-review Merge输入冻结与写栅栏

本节细化[PRD §21](ferry-工作流平台PRD.md)，复用§10的workflow_executions.context、lease_owner/epoch/version，workflow_node_runs的唯一尝试键、resolved_params/redo_round及资产引用协议；没有新增表/列/DDL，完整目标仍26表300字段、43时间配对、70增量语句。下述context封装是技术目标，不代表当前服务已能存取。

### 14.1 context中的冻结封装

context新增受版本标识的merge_inputs映射，每个activation_key对应一个不可变封装。key为规范JSON身份元组摘要；读取时核对完整identity，不能仅靠hash声称相同。identity包含workspace_id、execution_id、revision_id、node_id、iteration_hash、attempt、redo_round；iteration_hash须对应完整规范iteration_path，非循环仍按既有[]规则。lease_epoch为写资格，不进入语义输入hash，否则接管会无意改变输入或缓存身份。

| 封装字段 | 类型/必要约束 |
|---|---|
| schema | 整数1，遇未知版本明确拒绝恢复 |
| identity | 完整身份对象，当前空间/执行/修订与目标尝试一致；保留iteration_path以复核hash |
| params_hash | 已冻结resolved_params的规范摘要；mode/wait/input_count改变不能复用原封装 |
| decision | run或skipped；只是就绪决策，不是计算成功状态 |
| inputs / inputs_asset | 二选一；有序投影数组，或同空间受保护资产引用+内容hash；不能两者同时出现或均缺失 |
| snapshot_hash | 对schema、identity、params_hash、decision及完整有序投影的规范内容摘要，间接资产展开后校验，不哈希临时下载URL |

每个输入投影字段：input_index为0～input_count-1且连续唯一；source_node_run_id/source_attempt/source_iteration_hash指向确切来源，source_port/source_port_index指向冻结修订中的确切连接；symbol为D/E/S/F，item_count为非负整数，output_ref/output_sha256指向所选端口的有序输出。D/F要求正数及有效引用/hash；E为零项，引用可空但摘要为规范空数组的hash；S为零项且引用/hash为空。S须有已提交的分支选择/跳过依据，不能仅因某端口读取到零项而推断。

正常来源属于本执行/轮次；reused来源可指明确绑定的父执行node_run，须通过既有重跑闭包、轮次和当前权限验证，不允许请求体任意指向其他执行。cached/pinned同样记录确切来源，命中或固定数据的标记不替代来源授权。所有资产/输出引用绑定执行保留范围；snapshot不能复制密钥或保留短期签名URL，敏感元数据沿context执行AAD加密。

来源元数据与输出引用按input_index排列，条目仍按原索引；引用不能指向运行中分片或失败后作废的尝试。每个输入当前只投影已能确定的单一来源；多来源同端口排序未冻结时不生成合格封装。本封装不新增画布连线唯一性约束。输出及快照资产超过内联边界时走既有资产外置机制，元数据和资产字节均计执行预算；不能靠移动到context绕过定义/运行数据与资产配额。context中只保留有界索引和引用，不能把所有循环的完整输出堆在单行JSON。

### 14.2 原子边界与接管

冻结事务：核对当前execution lease_owner/epoch、可运行state及version，验证修订、精确来源终态/最终尝试与输出引用，提交context封装及目标node_run尝试绑定/状态变更。所有检查处于同一可用一致视图，来源已提交结果不可变；事务失败则封装、尝试绑定与新增引用全部回滚。跨实例同时激活时，以执行行栅栏和node_run唯一(execution_id,node_id,iteration_hash,attempt)拒绝重复；冲突后重读同一封装，不靠重试循环额外增加attempt。

计算在事务外使用已冻结输入。完成事务重新核对执行的可运行state、owner/epoch及version，按目标尝试的state/version条件写终态，并同事务提交输出引用、items_in/items_out及错误/来源信息。大型对象可先临时写，但未提交引用不能公开，失败临时对象走既有GC。取消或接管先赢得执行栅栏后，旧计算不能再成功提交；完成先提交则取消按既有终态处理，不反向改写历史。

伪SQL约束：对workflow_executions的条件UPDATE必须带id、workspace_id、lease_owner、lease_epoch、version及允许state；受影响行数不为1即回滚。node_run更新必须带execution/node/iteration/attempt和预期state/version。不能先独立提交执行version再提交节点结果，也不能只锁node_run却忽略执行已取消。实际三库行锁/事务隔离、原子唤醒与扫描去重仍需实现验证；本节不发明第二份租约系统。

恢复重放先核对身份、参数及snapshot_hash。已完成同一尝试直接返回已提交结果，不重复执行终态提交；未完成尝试由新租约持有者依同一快照重算。来源尝试后来重试、用户重跑或不同轮次不得偷偷替换冻结引用。完整输入不能读取时明确报不可恢复，不能用[]或当前latest补齐。执行仍活动时引用保护生效；终态清理按原save_data策略，清后只全部重跑，不留下“快照存在所以可恢复”的假象。

### 14.3 参考证据与限制

check_contracts.py含9例SQLite内存最小列事务投影：提交前异常回滚、上下文保持、旧owner与旧version拒写、接管沿用同一尝试、重复完成不增加版本、取消拦截提交。fixture直接给定冻结context，未验证快照构建、完整封装Schema/hash、真实资产事务、进程崩溃、多实例调度及PG/MySQL隔离；不得把它记为真实恢复验收。26表完整DDL和原约束/查询夹具另由文档校验器验证，二者证明范围不同。

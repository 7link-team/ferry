# Agent 知识、学习与协作：数据与接口契约

版本v1.0-review，2026-11-11；关联[主PRD](ferry-Agent知识学习与协作PRD.md)。这是新增拟定设计，非现有schema。**模型源未定义，待补产品确认**；字段/表名/限额均为本PRD提案，未经批准不进入迁移。SQL为设计示例，本次仅在内存SQLite校验，未连接项目或生产数据库。

## 1. 共用约束

ID由服务端生成，API全部十进制字符串。所有对象引用同workspace；时间UTC毫秒，xx_at均有xx_by BIGINT NOT NULL DEFAULT 0，0=system；可空时间为空时by=0。version正整数，新增1，CAS成功后递增。审计字段拒绝客户端赋值。

每表完整展开公共字段，无隐含列。数据库约束只有字段表所声明PK、NOT NULL、DEFAULT及索引；枚举、引用、长度、范围、JSON结构在服务端事务中强校验。禁止依赖SQLite的VARCHAR限制长度。逻辑外键同事务校验workspace/类型/权限并做删除保护，不新增未声明的物理FK。幂等/hash/技能名规范化为ASCII；技能name小写。索引不用TEXT及条件唯一；nullable唯一键只约束非空值，三数据库允许多个NULL。MySQL索引字节需实施实测，禁止擅自扩宽model。

正文/计划/回执/向量复用Store与asset_refs；资产存在不授予下载权。每个owner_kind都须补授权/引用/GC适配。定义修订复用definition_revisions，正文仅存资产locator/hash，不另造技能修订表。存量assets/definitions不批量补历史审计列。

## 2. 完整字段与索引

### ai_memories

对应FR-A1/A3，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| owner_id | BIGINT | 是 | 无，服务端赋值 | 所有者，不可转移 |
| visibility | VARCHAR(16) | 是 | 'private' | private/workspace，共享另建对象 |
| agent_id | BIGINT | 是 | 0 | 0=不绑定；否则同工作区授权Agent |
| kind | VARCHAR(16) | 是 | 无，服务端赋值 | fact/preference/context |
| title | VARCHAR(128) | 是 | 无，服务端赋值 | 1–128字 |
| body_asset_id | BIGINT | 是 | 无，服务端赋值 | 同工作区正文资产 |
| state | VARCHAR(16) | 是 | 'draft' | draft/active/invalid/archived |
| user_locked | BIGINT | 是 | 0 | 0/1；锁定自动覆盖，不绕过来源失效 |
| expires_at | BIGINT | 否 | 无，服务端赋值 | 到期UTC毫秒；可空 |
| expires_by | BIGINT | 是 | 0 | 设定到期的人，空时间时0 |

索引：

- `ix_ai_memories_scope`：(workspace_id,owner_id,state,id)。
- `ix_ai_memories_agent`：(workspace_id,agent_id,state,id)。

### ai_knowledge_evidence

对应FR-A2，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| target_kind | VARCHAR(24) | 是 | 无，服务端赋值 | memory/proposal/skill_revision |
| target_id | BIGINT | 是 | 无，服务端赋值 | 领域ID；skill_revision为definition_revisions ID |
| source_kind | VARCHAR(16) | 是 | 无，服务端赋值 | manual/message/turn |
| conversation_id | BIGINT | 否 | 无，服务端赋值 | manual为空 |
| message_id | BIGINT | 否 | 无，服务端赋值 | message来源必填 |
| turn_id | BIGINT | 否 | 无，服务端赋值 | turn来源必填 |
| source_version | BIGINT | 是 | 0 | manual=0，其余为实际读取版本 |
| source_hash | VARCHAR(64) | 是 | 无，服务端赋值 | SHA256小写hex；manual为输入正文hash |

索引：

- `ix_ai_knowledge_evidence_target`：(workspace_id,target_kind,target_id)。
- `ix_ai_knowledge_evidence_source`：(workspace_id,conversation_id,message_id)。
- `ix_ai_knowledge_evidence_turn`：(workspace_id,turn_id)。

### ai_skills

对应FR-C1/C2，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| owner_id | BIGINT | 是 | 无，服务端赋值 | 所有者 |
| visibility | VARCHAR(16) | 是 | 'private' | private/workspace，共享另建 |
| name | VARCHAR(64) | 是 | 无，服务端赋值 | 小写ASCII技能名，所有者内唯一 |
| title | VARCHAR(128) | 是 | 无，服务端赋值 | 1–128字 |
| state | VARCHAR(16) | 是 | 'draft' | draft/enabled/disabled/archived |
| candidate_revision_id | BIGINT | 否 | 无，服务端赋值 | 候选definition_revisions ID |
| published_revision_id | BIGINT | 否 | 无，服务端赋值 | 发布definition_revisions ID |

索引：

- `ix_ai_skills_name`：UNIQUE (workspace_id,owner_id,name)。
- `ix_ai_skills_scope`：(workspace_id,visibility,state,id)。

### ai_learning_proposals

对应FR-D3，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| owner_id | BIGINT | 是 | 无，服务端赋值 | 接收人 |
| job_id | BIGINT | 否 | 无，服务端赋值 | 生成任务；手动可空 |
| kind | VARCHAR(24) | 是 | 无，服务端赋值 | memory_create/memory_update/memory_merge/skill_create/skill_refine |
| target_id | BIGINT | 否 | 无，服务端赋值 | 创建为空；修订填目标 |
| base_version | BIGINT | 是 | 0 | 创建=0；修订为目标版本 |
| candidate_asset_id | BIGINT | 是 | 无，服务端赋值 | 完整候选JSON资产 |
| state | VARCHAR(16) | 是 | 'pending' | pending/applied/rejected/stale/failed/cancelled |
| client_id | VARCHAR(64) | 是 | 无，服务端赋值 | ASCII幂等键 |
| payload_hash | VARCHAR(64) | 是 | 无，服务端赋值 | 规范请求hash |
| receipt_asset_id | BIGINT | 否 | 无，服务端赋值 | 回执JSON资产 |
| error_code | VARCHAR(64) | 是 | '' | 错误代码，非上游原始正文 |

索引：

- `ix_ai_learning_proposals_client`：UNIQUE (workspace_id,owner_id,client_id)。
- `ix_ai_learning_proposals_inbox`：(workspace_id,owner_id,state,id)。
- `ix_ai_learning_proposals_target`：(workspace_id,kind,target_id)。

### ai_knowledge_settings

对应FR-D2/灰度，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| owner_id | BIGINT | 是 | 无，服务端赋值 | 0为工作区配置，其余本人 |
| agent_id | BIGINT | 是 | 无，服务端赋值 | 0仅用于工作区配置，否则Agent |
| capabilities_asset_id | BIGINT | 是 | 无，服务端赋值 | 设置JSON资产，白名单schema |
| profile_id | BIGINT | 否 | 无，服务端赋值 | 已授权API档案，未配可空 |
| embedding_provider_id | BIGINT | 否 | 无，服务端赋值 | 已有provider，未配关闭语义 |
| embedding_model | VARCHAR(256) | 是 | '' | 模型精确ID |
| cron_job_id | BIGINT | 否 | 无，服务端赋值 | 已有cron_jobs ID |

索引：

- `ix_ai_knowledge_settings_owner`：UNIQUE (workspace_id,owner_id,agent_id)。

### ai_knowledge_jobs

对应FR-B2/D1/D2，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| owner_id | BIGINT | 是 | 无，服务端赋值 | 实际发起人 |
| agent_id | BIGINT | 是 | 无，服务端赋值 | 执行Agent |
| settings_id | BIGINT | 是 | 无，服务端赋值 | 配置ID |
| policy_version | BIGINT | 是 | 无，服务端赋值 | 冻结设置version |
| conversation_id | BIGINT | 否 | 无，服务端赋值 | 汇总任务可空 |
| turn_id | BIGINT | 否 | 无，服务端赋值 | 完成轨迹来源可空 |
| kind | VARCHAR(24) | 是 | 无，服务端赋值 | extract/skill_synthesis/daily_review/index |
| origin | VARCHAR(16) | 是 | 无，服务端赋值 | human/completion/cron/learning |
| state | VARCHAR(16) | 是 | 'queued' | queued/running/completed/failed/cancelling/cancelled/unknown |
| dedupe_key | VARCHAR(64) | 是 | 无，服务端赋值 | 设置+来源或本地日期+policy_version的hash |
| lease_owner | VARCHAR(128) | 是 | '' | 现有进程启动身份 |
| epoch | BIGINT | 是 | 0 | 单调递增租约fence |
| lease_expires_at | BIGINT | 否 | 无，服务端赋值 | 租约截止UTC毫秒 |
| lease_expires_by | BIGINT | 是 | 0 | 设定人，0=system |
| snapshot_asset_id | BIGINT | 是 | 无，服务端赋值 | 冻结输入元数据，不授予永久权限 |
| error_code | VARCHAR(64) | 是 | '' | 失败原因代码 |

索引：

- `ix_ai_knowledge_jobs_dedupe`：UNIQUE (workspace_id,dedupe_key)。
- `ix_ai_knowledge_jobs_queue`：(workspace_id,state,id)。

### ai_memory_embeddings

对应FR-B1，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| memory_id | BIGINT | 是 | 无，服务端赋值 | 同工作区记忆 |
| memory_version | BIGINT | 是 | 无，服务端赋值 | 正文源版本 |
| provider_id | BIGINT | 是 | 无，服务端赋值 | 嵌入provider |
| model | VARCHAR(256) | 是 | 无，服务端赋值 | 精确模型ID |
| dimensions | BIGINT | 是 | 无，服务端赋值 | 1–65536；实际模型能力验证 |
| vector_asset_id | BIGINT | 否 | 无，服务端赋值 | 有限数向量JSON资产 |
| state | VARCHAR(16) | 是 | 'pending' | pending/ready/stale/failed |
| error_code | VARCHAR(64) | 是 | '' | 失败代码 |

索引：

- `ix_ai_memory_embeddings_version`：UNIQUE (workspace_id,memory_id,memory_version,provider_id,model)。
- `ix_ai_memory_embeddings_ready`：(workspace_id,state,memory_id)。

### ai_collaboration_runs

对应FR-E，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| conversation_id | BIGINT | 是 | 无，服务端赋值 | 当前有权会话 |
| owner_id | BIGINT | 是 | 无，服务端赋值 | 发起人/审批人 |
| coordinator_agent_id | BIGINT | 是 | 无，服务端赋值 | 负责人 |
| plan_revision_id | BIGINT | 是 | 无，服务端赋值 | definition_revisions冻结计划 |
| state | VARCHAR(24) | 是 | 'draft' | draft/queued/running/waiting_human/summarizing/completed/cancelling/failed/cancelled/stopping_unknown |
| client_id | VARCHAR(64) | 是 | 无，服务端赋值 | 启动幂等键 |
| payload_hash | VARCHAR(64) | 是 | 无，服务端赋值 | 启动请求hash |
| round_count | BIGINT | 是 | 0 | 负责人续调度次数 |
| lease_owner | VARCHAR(128) | 是 | '' | 父运行租约身份 |
| epoch | BIGINT | 是 | 0 | 父运行fence |
| lease_expires_at | BIGINT | 否 | 无，服务端赋值 | 租约截止 |
| lease_expires_by | BIGINT | 是 | 0 | 设定者 |
| deadline_at | BIGINT | 是 | 无，服务端赋值 | 整体UTC毫秒截止 |
| deadline_by | BIGINT | 是 | 0 | 设定者 |
| failure_code | VARCHAR(64) | 是 | '' | 失败或停止原因 |

索引：

- `ix_ai_collaboration_runs_client`：UNIQUE (workspace_id,owner_id,client_id)。
- `ix_ai_collaboration_runs_conversation`：(workspace_id,conversation_id,id)。
- `ix_ai_collaboration_runs_active`：(workspace_id,state,id)。

### ai_collaboration_tasks

对应FR-E2，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| run_id | BIGINT | 是 | 无，服务端赋值 | 父run |
| step_key | VARCHAR(64) | 是 | 无，服务端赋值 | 稳定ASCII计划键 |
| agent_id | BIGINT | 是 | 无，服务端赋值 | 执行成员 |
| turn_id | BIGINT | 否 | 无，服务端赋值 | 派发前空，派发后现有ai_turns ID |
| state | VARCHAR(24) | 是 | 'pending' | pending/dispatched/running/waiting_human/completed/failed/cancelling/cancelled/unknown |
| spec_asset_id | BIGINT | 是 | 无，服务端赋值 | 冻结任务说明、依赖与授权上限 |
| result_asset_id | BIGINT | 否 | 无，服务端赋值 | 结果引用资产 |
| error_code | VARCHAR(64) | 是 | '' | 错误代码 |

索引：

- `ix_ai_collaboration_tasks_step`：UNIQUE (workspace_id,run_id,step_key)。
- `ix_ai_collaboration_tasks_turn`：UNIQUE (workspace_id,turn_id)。
- `ix_ai_collaboration_tasks_barrier`：(workspace_id,run_id,state)。

### ai_context_snapshots

对应FR-B3/F，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| turn_id | BIGINT | 是 | 无，服务端赋值 | 现有turn唯一关联 |
| branch_id | BIGINT | 否 | 无，服务端赋值 | 旧主链可空 |
| snapshot_asset_id | BIGINT | 是 | 无，服务端赋值 | 上下文组成JSON资产 |
| snapshot_hash | VARCHAR(64) | 是 | 无，服务端赋值 | 完整快照hash |

索引：

- `ix_ai_context_snapshots_turn`：UNIQUE (workspace_id,turn_id)。
- `ix_ai_context_snapshots_branch`：(workspace_id,branch_id,id)。

### ai_conversation_branches

对应FR-F1，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| conversation_id | BIGINT | 是 | 无，服务端赋值 | 所属会话 |
| owner_id | BIGINT | 是 | 无，服务端赋值 | 创建人，不替代会话成员权限 |
| title | VARCHAR(128) | 是 | 无，服务端赋值 | 1–128字 |
| parent_branch_id | BIGINT | 否 | 无，服务端赋值 | 主链空 |
| fork_message_id | BIGINT | 否 | 无，服务端赋值 | 分叉点，主链空 |
| head_message_id | BIGINT | 否 | 无，服务端赋值 | 当前head，空分支为空 |

索引：

- `ix_ai_conversation_branches_conversation`：(workspace_id,conversation_id,id)。

### ai_branch_messages

对应FR-F1，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| branch_id | BIGINT | 是 | 无，服务端赋值 | 所属分支 |
| message_id | BIGINT | 是 | 无，服务端赋值 | 已有消息 |
| sequence | BIGINT | 是 | 无，服务端赋值 | 分支内正整数序号 |

索引：

- `ix_ai_branch_messages_message`：UNIQUE (workspace_id,branch_id,message_id)。
- `ix_ai_branch_messages_sequence`：UNIQUE (workspace_id,branch_id,sequence)。

### ai_dispatch_intents

对应FR-F2，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| turn_id | BIGINT | 是 | 无，服务端赋值 | 现有turn |
| attempt | BIGINT | 是 | 1 | 从1开始，unknown不能递增 |
| request_id | VARCHAR(64) | 是 | 无，服务端赋值 | 随机请求ID，回传须匹配 |
| host_id | BIGINT | 是 | 无，服务端赋值 | 实际执行节点 |
| binding_hash | VARCHAR(64) | 是 | 无，服务端赋值 | 实际执行绑定hash |
| epoch | BIGINT | 是 | 无，服务端赋值 | 当前执行fence |
| state | VARCHAR(16) | 是 | 'prepared' | prepared/sent/accepted/running/unknown/terminal/aborted |
| outcome | VARCHAR(16) | 是 | '' | 空或completed/cancelled/timed_out/failed |
| confirm_deadline_at | BIGINT | 否 | 无，服务端赋值 | 确认截止UTC毫秒 |
| confirm_deadline_by | BIGINT | 是 | 0 | 设定者 |

索引：

- `ix_ai_dispatch_intents_request`：UNIQUE (workspace_id,request_id)。
- `ix_ai_dispatch_intents_attempt`：UNIQUE (workspace_id,turn_id,attempt)。
- `ix_ai_dispatch_intents_state`：(workspace_id,state,id)。

### ai_knowledge_requests

对应CON-01；所有新增领域写操作的幂等回执，不构造通用平台框架，来源为主PRD拟定设计P，非已实现模型。

| 字段 | 类型 | 必填 | 默认 | 约束/用途 |
|---|---|---|---|---|
| id | BIGINT | 是 | 无，服务端赋值 | 服务端雪花ID，PK |
| workspace_id | BIGINT | 是 | 无，服务端赋值 | 路径Scope工作区 |
| version | BIGINT | 是 | 1 | CAS版本正整数 |
| created_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒创建时间 |
| created_by | BIGINT | 是 | 0 | 创建者，0=system |
| updated_at | BIGINT | 是 | 无，服务端赋值 | UTC毫秒修改时间 |
| updated_by | BIGINT | 是 | 0 | 修改者，0=system |
| owner_id | BIGINT | 是 | 无，服务端赋值 | 请求人 |
| client_id | VARCHAR(64) | 是 | 无，服务端赋值 | ASCII幂等键 |
| action | VARCHAR(64) | 是 | 无，服务端赋值 | 动作名，参与payload hash |
| target_kind | VARCHAR(32) | 是 | 无，服务端赋值 | 领域对象类型 |
| target_id | BIGINT | 否 | 无，服务端赋值 | 创建前空，成功后目标ID；删除不保留正文引用 |
| payload_hash | VARCHAR(64) | 是 | 无，服务端赋值 | 规范化action及请求hash |
| result_asset_id | BIGINT | 否 | 无，服务端赋值 | 结果元数据资产，无正文/密钥 |
| state | VARCHAR(16) | 是 | 'pending' | pending/completed |

索引：

- `ix_ai_knowledge_requests_client`：UNIQUE (workspace_id,owner_id,client_id)。
- `ix_ai_knowledge_requests_target`：(workspace_id,target_kind,target_id)。

## 3. JSON资产与复用定义

所有JSON含schema_version=1，未知键拒绝；深度≤8，普通JSON资产≤128KiB，vector资产≤4MiB；正文/ID遵主PRD约束，模型不能自动创建未知资源。向量必须有限且长度匹配模型，不能只因满足字节上限就接受。以下为字段白名单，不存在未声明业务字段。

| 载体 | 字段及规则 |
|---|---|
| memory candidate | schema_version,kind,title,body_asset_id,visibility(private),agent_id(null或授权ID),expires_at(null或毫秒),expires_by, sources(≤40证据描述),targets(merge用ID+base_version数组≤8)；owner由服务端确定 |
| skill candidate | schema_version,skill_id(null或目标),base_version,name,title,body_asset_id,sources；完整正文，不接受仅patch |
| skill definition | schema_version,name,title,body_asset_id,body_hash；subject_kind=ai_skill，subject_id为技能ID，number沿既有序号 |
| collaboration plan definition | schema_version,coordinator_agent_id,mode(sequence/parallel/handoff),tasks(1–16),max_parallel(1–4),round_limit(1–10),failure_policy(fail_fast),timeout_seconds；tasks元素为step_key/agent_id/depends_on/input_asset_id/permission(read_only或on_request)/required(true)；图无环 |
| task spec | schema_version,step_key,dependencies,input_asset_id,permission_cap,native_binding_snapshot；不可超出计划 |
| context snapshot | schema_version,branch_id,head_message_id,memories(ID/version/hash),skills(ID/revision/hash),source_ids,omitted(ID/reason),byte_count,origin；不复制永久正文 |
| workspace settings | schema_version,enabled_packages(A–F布尔),max_memory_candidates(1–100),max_injected_memories(1–8),max_scan_items(1–10000),memory_bytes(1–8192),skill_bytes(1–16384),max_tasks(1–16),max_parallel(1–4),round_limit(1–10),default_timeout_seconds(60–86400)；owner=agent=0；能力默认关闭，其余取上限/1800秒 |
| owner+agent settings | schema_version,extraction_enabled(false),review_enabled(false),scope_conversation_ids(≤100已授权ID),timezone(必选IANA),schedule(合法cron或null)；不能改workspace键；review启用必须cron引用 |
| job snapshot | schema_version,source_ids及version/hash,local_date(复盘YYYY-MM-DD),model_profile_revision_id,evidence_cutoff_ms,truncated,budget_reference(已有账本标识)；cutoff是采样界限，不是无操作者的事件时间 |
| result | schema_version,task_id,outcome,message_ids,artifact_ids,verification_status(verified/unverified/failed)；引用读取再鉴权 |
| receipt | schema_version,proposal_id,before_versions,after_ids_versions,result,applied_at,applied_by；不可变 |
| vector | schema_version,provider_id,model,dimensions,memory_version,values(有限浮点数组，长度等于dimensions) |

## 4. 三数据库建表与索引DDL

DDL逐字派生于上方字段/索引，没有新增未声明列。全新schema示例不使用IF NOT EXISTS；正式迁移由既有SchemaManager按版本与存在性保证可重入。MySQL DDL非全事务，失败仅重试缺失对象，不DROP业务表。BIGINT ID手动赋值，无自增。此处没有TEXT列，三方言仅MySQL尾缀差异。

### SQLite

```sql
CREATE TABLE ai_memories (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  visibility VARCHAR(16) NOT NULL DEFAULT 'private',
  agent_id BIGINT NOT NULL DEFAULT 0,
  kind VARCHAR(16) NOT NULL,
  title VARCHAR(128) NOT NULL,
  body_asset_id BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'draft',
  user_locked BIGINT NOT NULL DEFAULT 0,
  expires_at BIGINT,
  expires_by BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX ix_ai_memories_scope ON ai_memories (workspace_id,owner_id,state,id);
CREATE INDEX ix_ai_memories_agent ON ai_memories (workspace_id,agent_id,state,id);

CREATE TABLE ai_knowledge_evidence (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  target_kind VARCHAR(24) NOT NULL,
  target_id BIGINT NOT NULL,
  source_kind VARCHAR(16) NOT NULL,
  conversation_id BIGINT,
  message_id BIGINT,
  turn_id BIGINT,
  source_version BIGINT NOT NULL DEFAULT 0,
  source_hash VARCHAR(64) NOT NULL
);
CREATE INDEX ix_ai_knowledge_evidence_target ON ai_knowledge_evidence (workspace_id,target_kind,target_id);
CREATE INDEX ix_ai_knowledge_evidence_source ON ai_knowledge_evidence (workspace_id,conversation_id,message_id);
CREATE INDEX ix_ai_knowledge_evidence_turn ON ai_knowledge_evidence (workspace_id,turn_id);

CREATE TABLE ai_skills (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  visibility VARCHAR(16) NOT NULL DEFAULT 'private',
  name VARCHAR(64) NOT NULL,
  title VARCHAR(128) NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'draft',
  candidate_revision_id BIGINT,
  published_revision_id BIGINT
);
CREATE UNIQUE INDEX ix_ai_skills_name ON ai_skills (workspace_id,owner_id,name);
CREATE INDEX ix_ai_skills_scope ON ai_skills (workspace_id,visibility,state,id);

CREATE TABLE ai_learning_proposals (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  job_id BIGINT,
  kind VARCHAR(24) NOT NULL,
  target_id BIGINT,
  base_version BIGINT NOT NULL DEFAULT 0,
  candidate_asset_id BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'pending',
  client_id VARCHAR(64) NOT NULL,
  payload_hash VARCHAR(64) NOT NULL,
  receipt_asset_id BIGINT,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_learning_proposals_client ON ai_learning_proposals (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_learning_proposals_inbox ON ai_learning_proposals (workspace_id,owner_id,state,id);
CREATE INDEX ix_ai_learning_proposals_target ON ai_learning_proposals (workspace_id,kind,target_id);

CREATE TABLE ai_knowledge_settings (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  agent_id BIGINT NOT NULL,
  capabilities_asset_id BIGINT NOT NULL,
  profile_id BIGINT,
  embedding_provider_id BIGINT,
  embedding_model VARCHAR(256) NOT NULL DEFAULT '',
  cron_job_id BIGINT
);
CREATE UNIQUE INDEX ix_ai_knowledge_settings_owner ON ai_knowledge_settings (workspace_id,owner_id,agent_id);

CREATE TABLE ai_knowledge_jobs (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  agent_id BIGINT NOT NULL,
  settings_id BIGINT NOT NULL,
  policy_version BIGINT NOT NULL,
  conversation_id BIGINT,
  turn_id BIGINT,
  kind VARCHAR(24) NOT NULL,
  origin VARCHAR(16) NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'queued',
  dedupe_key VARCHAR(64) NOT NULL,
  lease_owner VARCHAR(128) NOT NULL DEFAULT '',
  epoch BIGINT NOT NULL DEFAULT 0,
  lease_expires_at BIGINT,
  lease_expires_by BIGINT NOT NULL DEFAULT 0,
  snapshot_asset_id BIGINT NOT NULL,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_knowledge_jobs_dedupe ON ai_knowledge_jobs (workspace_id,dedupe_key);
CREATE INDEX ix_ai_knowledge_jobs_queue ON ai_knowledge_jobs (workspace_id,state,id);

CREATE TABLE ai_memory_embeddings (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  memory_id BIGINT NOT NULL,
  memory_version BIGINT NOT NULL,
  provider_id BIGINT NOT NULL,
  model VARCHAR(256) NOT NULL,
  dimensions BIGINT NOT NULL,
  vector_asset_id BIGINT,
  state VARCHAR(16) NOT NULL DEFAULT 'pending',
  error_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_memory_embeddings_version ON ai_memory_embeddings (workspace_id,memory_id,memory_version,provider_id,model);
CREATE INDEX ix_ai_memory_embeddings_ready ON ai_memory_embeddings (workspace_id,state,memory_id);

CREATE TABLE ai_collaboration_runs (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  conversation_id BIGINT NOT NULL,
  owner_id BIGINT NOT NULL,
  coordinator_agent_id BIGINT NOT NULL,
  plan_revision_id BIGINT NOT NULL,
  state VARCHAR(24) NOT NULL DEFAULT 'draft',
  client_id VARCHAR(64) NOT NULL,
  payload_hash VARCHAR(64) NOT NULL,
  round_count BIGINT NOT NULL DEFAULT 0,
  lease_owner VARCHAR(128) NOT NULL DEFAULT '',
  epoch BIGINT NOT NULL DEFAULT 0,
  lease_expires_at BIGINT,
  lease_expires_by BIGINT NOT NULL DEFAULT 0,
  deadline_at BIGINT NOT NULL,
  deadline_by BIGINT NOT NULL DEFAULT 0,
  failure_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_collaboration_runs_client ON ai_collaboration_runs (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_collaboration_runs_conversation ON ai_collaboration_runs (workspace_id,conversation_id,id);
CREATE INDEX ix_ai_collaboration_runs_active ON ai_collaboration_runs (workspace_id,state,id);

CREATE TABLE ai_collaboration_tasks (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  run_id BIGINT NOT NULL,
  step_key VARCHAR(64) NOT NULL,
  agent_id BIGINT NOT NULL,
  turn_id BIGINT,
  state VARCHAR(24) NOT NULL DEFAULT 'pending',
  spec_asset_id BIGINT NOT NULL,
  result_asset_id BIGINT,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_collaboration_tasks_step ON ai_collaboration_tasks (workspace_id,run_id,step_key);
CREATE UNIQUE INDEX ix_ai_collaboration_tasks_turn ON ai_collaboration_tasks (workspace_id,turn_id);
CREATE INDEX ix_ai_collaboration_tasks_barrier ON ai_collaboration_tasks (workspace_id,run_id,state);

CREATE TABLE ai_context_snapshots (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  turn_id BIGINT NOT NULL,
  branch_id BIGINT,
  snapshot_asset_id BIGINT NOT NULL,
  snapshot_hash VARCHAR(64) NOT NULL
);
CREATE UNIQUE INDEX ix_ai_context_snapshots_turn ON ai_context_snapshots (workspace_id,turn_id);
CREATE INDEX ix_ai_context_snapshots_branch ON ai_context_snapshots (workspace_id,branch_id,id);

CREATE TABLE ai_conversation_branches (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  conversation_id BIGINT NOT NULL,
  owner_id BIGINT NOT NULL,
  title VARCHAR(128) NOT NULL,
  parent_branch_id BIGINT,
  fork_message_id BIGINT,
  head_message_id BIGINT
);
CREATE INDEX ix_ai_conversation_branches_conversation ON ai_conversation_branches (workspace_id,conversation_id,id);

CREATE TABLE ai_branch_messages (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  branch_id BIGINT NOT NULL,
  message_id BIGINT NOT NULL,
  sequence BIGINT NOT NULL
);
CREATE UNIQUE INDEX ix_ai_branch_messages_message ON ai_branch_messages (workspace_id,branch_id,message_id);
CREATE UNIQUE INDEX ix_ai_branch_messages_sequence ON ai_branch_messages (workspace_id,branch_id,sequence);

CREATE TABLE ai_dispatch_intents (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  turn_id BIGINT NOT NULL,
  attempt BIGINT NOT NULL DEFAULT 1,
  request_id VARCHAR(64) NOT NULL,
  host_id BIGINT NOT NULL,
  binding_hash VARCHAR(64) NOT NULL,
  epoch BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'prepared',
  outcome VARCHAR(16) NOT NULL DEFAULT '',
  confirm_deadline_at BIGINT,
  confirm_deadline_by BIGINT NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX ix_ai_dispatch_intents_request ON ai_dispatch_intents (workspace_id,request_id);
CREATE UNIQUE INDEX ix_ai_dispatch_intents_attempt ON ai_dispatch_intents (workspace_id,turn_id,attempt);
CREATE INDEX ix_ai_dispatch_intents_state ON ai_dispatch_intents (workspace_id,state,id);

CREATE TABLE ai_knowledge_requests (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  client_id VARCHAR(64) NOT NULL,
  action VARCHAR(64) NOT NULL,
  target_kind VARCHAR(32) NOT NULL,
  target_id BIGINT,
  payload_hash VARCHAR(64) NOT NULL,
  result_asset_id BIGINT,
  state VARCHAR(16) NOT NULL DEFAULT 'pending'
);
CREATE UNIQUE INDEX ix_ai_knowledge_requests_client ON ai_knowledge_requests (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_knowledge_requests_target ON ai_knowledge_requests (workspace_id,target_kind,target_id);

```

### PostgreSQL

```sql
CREATE TABLE ai_memories (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  visibility VARCHAR(16) NOT NULL DEFAULT 'private',
  agent_id BIGINT NOT NULL DEFAULT 0,
  kind VARCHAR(16) NOT NULL,
  title VARCHAR(128) NOT NULL,
  body_asset_id BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'draft',
  user_locked BIGINT NOT NULL DEFAULT 0,
  expires_at BIGINT,
  expires_by BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX ix_ai_memories_scope ON ai_memories (workspace_id,owner_id,state,id);
CREATE INDEX ix_ai_memories_agent ON ai_memories (workspace_id,agent_id,state,id);

CREATE TABLE ai_knowledge_evidence (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  target_kind VARCHAR(24) NOT NULL,
  target_id BIGINT NOT NULL,
  source_kind VARCHAR(16) NOT NULL,
  conversation_id BIGINT,
  message_id BIGINT,
  turn_id BIGINT,
  source_version BIGINT NOT NULL DEFAULT 0,
  source_hash VARCHAR(64) NOT NULL
);
CREATE INDEX ix_ai_knowledge_evidence_target ON ai_knowledge_evidence (workspace_id,target_kind,target_id);
CREATE INDEX ix_ai_knowledge_evidence_source ON ai_knowledge_evidence (workspace_id,conversation_id,message_id);
CREATE INDEX ix_ai_knowledge_evidence_turn ON ai_knowledge_evidence (workspace_id,turn_id);

CREATE TABLE ai_skills (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  visibility VARCHAR(16) NOT NULL DEFAULT 'private',
  name VARCHAR(64) NOT NULL,
  title VARCHAR(128) NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'draft',
  candidate_revision_id BIGINT,
  published_revision_id BIGINT
);
CREATE UNIQUE INDEX ix_ai_skills_name ON ai_skills (workspace_id,owner_id,name);
CREATE INDEX ix_ai_skills_scope ON ai_skills (workspace_id,visibility,state,id);

CREATE TABLE ai_learning_proposals (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  job_id BIGINT,
  kind VARCHAR(24) NOT NULL,
  target_id BIGINT,
  base_version BIGINT NOT NULL DEFAULT 0,
  candidate_asset_id BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'pending',
  client_id VARCHAR(64) NOT NULL,
  payload_hash VARCHAR(64) NOT NULL,
  receipt_asset_id BIGINT,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_learning_proposals_client ON ai_learning_proposals (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_learning_proposals_inbox ON ai_learning_proposals (workspace_id,owner_id,state,id);
CREATE INDEX ix_ai_learning_proposals_target ON ai_learning_proposals (workspace_id,kind,target_id);

CREATE TABLE ai_knowledge_settings (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  agent_id BIGINT NOT NULL,
  capabilities_asset_id BIGINT NOT NULL,
  profile_id BIGINT,
  embedding_provider_id BIGINT,
  embedding_model VARCHAR(256) NOT NULL DEFAULT '',
  cron_job_id BIGINT
);
CREATE UNIQUE INDEX ix_ai_knowledge_settings_owner ON ai_knowledge_settings (workspace_id,owner_id,agent_id);

CREATE TABLE ai_knowledge_jobs (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  agent_id BIGINT NOT NULL,
  settings_id BIGINT NOT NULL,
  policy_version BIGINT NOT NULL,
  conversation_id BIGINT,
  turn_id BIGINT,
  kind VARCHAR(24) NOT NULL,
  origin VARCHAR(16) NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'queued',
  dedupe_key VARCHAR(64) NOT NULL,
  lease_owner VARCHAR(128) NOT NULL DEFAULT '',
  epoch BIGINT NOT NULL DEFAULT 0,
  lease_expires_at BIGINT,
  lease_expires_by BIGINT NOT NULL DEFAULT 0,
  snapshot_asset_id BIGINT NOT NULL,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_knowledge_jobs_dedupe ON ai_knowledge_jobs (workspace_id,dedupe_key);
CREATE INDEX ix_ai_knowledge_jobs_queue ON ai_knowledge_jobs (workspace_id,state,id);

CREATE TABLE ai_memory_embeddings (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  memory_id BIGINT NOT NULL,
  memory_version BIGINT NOT NULL,
  provider_id BIGINT NOT NULL,
  model VARCHAR(256) NOT NULL,
  dimensions BIGINT NOT NULL,
  vector_asset_id BIGINT,
  state VARCHAR(16) NOT NULL DEFAULT 'pending',
  error_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_memory_embeddings_version ON ai_memory_embeddings (workspace_id,memory_id,memory_version,provider_id,model);
CREATE INDEX ix_ai_memory_embeddings_ready ON ai_memory_embeddings (workspace_id,state,memory_id);

CREATE TABLE ai_collaboration_runs (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  conversation_id BIGINT NOT NULL,
  owner_id BIGINT NOT NULL,
  coordinator_agent_id BIGINT NOT NULL,
  plan_revision_id BIGINT NOT NULL,
  state VARCHAR(24) NOT NULL DEFAULT 'draft',
  client_id VARCHAR(64) NOT NULL,
  payload_hash VARCHAR(64) NOT NULL,
  round_count BIGINT NOT NULL DEFAULT 0,
  lease_owner VARCHAR(128) NOT NULL DEFAULT '',
  epoch BIGINT NOT NULL DEFAULT 0,
  lease_expires_at BIGINT,
  lease_expires_by BIGINT NOT NULL DEFAULT 0,
  deadline_at BIGINT NOT NULL,
  deadline_by BIGINT NOT NULL DEFAULT 0,
  failure_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_collaboration_runs_client ON ai_collaboration_runs (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_collaboration_runs_conversation ON ai_collaboration_runs (workspace_id,conversation_id,id);
CREATE INDEX ix_ai_collaboration_runs_active ON ai_collaboration_runs (workspace_id,state,id);

CREATE TABLE ai_collaboration_tasks (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  run_id BIGINT NOT NULL,
  step_key VARCHAR(64) NOT NULL,
  agent_id BIGINT NOT NULL,
  turn_id BIGINT,
  state VARCHAR(24) NOT NULL DEFAULT 'pending',
  spec_asset_id BIGINT NOT NULL,
  result_asset_id BIGINT,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX ix_ai_collaboration_tasks_step ON ai_collaboration_tasks (workspace_id,run_id,step_key);
CREATE UNIQUE INDEX ix_ai_collaboration_tasks_turn ON ai_collaboration_tasks (workspace_id,turn_id);
CREATE INDEX ix_ai_collaboration_tasks_barrier ON ai_collaboration_tasks (workspace_id,run_id,state);

CREATE TABLE ai_context_snapshots (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  turn_id BIGINT NOT NULL,
  branch_id BIGINT,
  snapshot_asset_id BIGINT NOT NULL,
  snapshot_hash VARCHAR(64) NOT NULL
);
CREATE UNIQUE INDEX ix_ai_context_snapshots_turn ON ai_context_snapshots (workspace_id,turn_id);
CREATE INDEX ix_ai_context_snapshots_branch ON ai_context_snapshots (workspace_id,branch_id,id);

CREATE TABLE ai_conversation_branches (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  conversation_id BIGINT NOT NULL,
  owner_id BIGINT NOT NULL,
  title VARCHAR(128) NOT NULL,
  parent_branch_id BIGINT,
  fork_message_id BIGINT,
  head_message_id BIGINT
);
CREATE INDEX ix_ai_conversation_branches_conversation ON ai_conversation_branches (workspace_id,conversation_id,id);

CREATE TABLE ai_branch_messages (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  branch_id BIGINT NOT NULL,
  message_id BIGINT NOT NULL,
  sequence BIGINT NOT NULL
);
CREATE UNIQUE INDEX ix_ai_branch_messages_message ON ai_branch_messages (workspace_id,branch_id,message_id);
CREATE UNIQUE INDEX ix_ai_branch_messages_sequence ON ai_branch_messages (workspace_id,branch_id,sequence);

CREATE TABLE ai_dispatch_intents (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  turn_id BIGINT NOT NULL,
  attempt BIGINT NOT NULL DEFAULT 1,
  request_id VARCHAR(64) NOT NULL,
  host_id BIGINT NOT NULL,
  binding_hash VARCHAR(64) NOT NULL,
  epoch BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'prepared',
  outcome VARCHAR(16) NOT NULL DEFAULT '',
  confirm_deadline_at BIGINT,
  confirm_deadline_by BIGINT NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX ix_ai_dispatch_intents_request ON ai_dispatch_intents (workspace_id,request_id);
CREATE UNIQUE INDEX ix_ai_dispatch_intents_attempt ON ai_dispatch_intents (workspace_id,turn_id,attempt);
CREATE INDEX ix_ai_dispatch_intents_state ON ai_dispatch_intents (workspace_id,state,id);

CREATE TABLE ai_knowledge_requests (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  client_id VARCHAR(64) NOT NULL,
  action VARCHAR(64) NOT NULL,
  target_kind VARCHAR(32) NOT NULL,
  target_id BIGINT,
  payload_hash VARCHAR(64) NOT NULL,
  result_asset_id BIGINT,
  state VARCHAR(16) NOT NULL DEFAULT 'pending'
);
CREATE UNIQUE INDEX ix_ai_knowledge_requests_client ON ai_knowledge_requests (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_knowledge_requests_target ON ai_knowledge_requests (workspace_id,target_kind,target_id);

```

### MySQL

```sql
CREATE TABLE ai_memories (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  visibility VARCHAR(16) NOT NULL DEFAULT 'private',
  agent_id BIGINT NOT NULL DEFAULT 0,
  kind VARCHAR(16) NOT NULL,
  title VARCHAR(128) NOT NULL,
  body_asset_id BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'draft',
  user_locked BIGINT NOT NULL DEFAULT 0,
  expires_at BIGINT,
  expires_by BIGINT NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE INDEX ix_ai_memories_scope ON ai_memories (workspace_id,owner_id,state,id);
CREATE INDEX ix_ai_memories_agent ON ai_memories (workspace_id,agent_id,state,id);

CREATE TABLE ai_knowledge_evidence (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  target_kind VARCHAR(24) NOT NULL,
  target_id BIGINT NOT NULL,
  source_kind VARCHAR(16) NOT NULL,
  conversation_id BIGINT,
  message_id BIGINT,
  turn_id BIGINT,
  source_version BIGINT NOT NULL DEFAULT 0,
  source_hash VARCHAR(64) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE INDEX ix_ai_knowledge_evidence_target ON ai_knowledge_evidence (workspace_id,target_kind,target_id);
CREATE INDEX ix_ai_knowledge_evidence_source ON ai_knowledge_evidence (workspace_id,conversation_id,message_id);
CREATE INDEX ix_ai_knowledge_evidence_turn ON ai_knowledge_evidence (workspace_id,turn_id);

CREATE TABLE ai_skills (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  visibility VARCHAR(16) NOT NULL DEFAULT 'private',
  name VARCHAR(64) NOT NULL,
  title VARCHAR(128) NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'draft',
  candidate_revision_id BIGINT,
  published_revision_id BIGINT
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_skills_name ON ai_skills (workspace_id,owner_id,name);
CREATE INDEX ix_ai_skills_scope ON ai_skills (workspace_id,visibility,state,id);

CREATE TABLE ai_learning_proposals (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  job_id BIGINT,
  kind VARCHAR(24) NOT NULL,
  target_id BIGINT,
  base_version BIGINT NOT NULL DEFAULT 0,
  candidate_asset_id BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'pending',
  client_id VARCHAR(64) NOT NULL,
  payload_hash VARCHAR(64) NOT NULL,
  receipt_asset_id BIGINT,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_learning_proposals_client ON ai_learning_proposals (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_learning_proposals_inbox ON ai_learning_proposals (workspace_id,owner_id,state,id);
CREATE INDEX ix_ai_learning_proposals_target ON ai_learning_proposals (workspace_id,kind,target_id);

CREATE TABLE ai_knowledge_settings (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  agent_id BIGINT NOT NULL,
  capabilities_asset_id BIGINT NOT NULL,
  profile_id BIGINT,
  embedding_provider_id BIGINT,
  embedding_model VARCHAR(256) NOT NULL DEFAULT '',
  cron_job_id BIGINT
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_knowledge_settings_owner ON ai_knowledge_settings (workspace_id,owner_id,agent_id);

CREATE TABLE ai_knowledge_jobs (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  agent_id BIGINT NOT NULL,
  settings_id BIGINT NOT NULL,
  policy_version BIGINT NOT NULL,
  conversation_id BIGINT,
  turn_id BIGINT,
  kind VARCHAR(24) NOT NULL,
  origin VARCHAR(16) NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'queued',
  dedupe_key VARCHAR(64) NOT NULL,
  lease_owner VARCHAR(128) NOT NULL DEFAULT '',
  epoch BIGINT NOT NULL DEFAULT 0,
  lease_expires_at BIGINT,
  lease_expires_by BIGINT NOT NULL DEFAULT 0,
  snapshot_asset_id BIGINT NOT NULL,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_knowledge_jobs_dedupe ON ai_knowledge_jobs (workspace_id,dedupe_key);
CREATE INDEX ix_ai_knowledge_jobs_queue ON ai_knowledge_jobs (workspace_id,state,id);

CREATE TABLE ai_memory_embeddings (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  memory_id BIGINT NOT NULL,
  memory_version BIGINT NOT NULL,
  provider_id BIGINT NOT NULL,
  model VARCHAR(256) NOT NULL,
  dimensions BIGINT NOT NULL,
  vector_asset_id BIGINT,
  state VARCHAR(16) NOT NULL DEFAULT 'pending',
  error_code VARCHAR(64) NOT NULL DEFAULT ''
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_memory_embeddings_version ON ai_memory_embeddings (workspace_id,memory_id,memory_version,provider_id,model);
CREATE INDEX ix_ai_memory_embeddings_ready ON ai_memory_embeddings (workspace_id,state,memory_id);

CREATE TABLE ai_collaboration_runs (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  conversation_id BIGINT NOT NULL,
  owner_id BIGINT NOT NULL,
  coordinator_agent_id BIGINT NOT NULL,
  plan_revision_id BIGINT NOT NULL,
  state VARCHAR(24) NOT NULL DEFAULT 'draft',
  client_id VARCHAR(64) NOT NULL,
  payload_hash VARCHAR(64) NOT NULL,
  round_count BIGINT NOT NULL DEFAULT 0,
  lease_owner VARCHAR(128) NOT NULL DEFAULT '',
  epoch BIGINT NOT NULL DEFAULT 0,
  lease_expires_at BIGINT,
  lease_expires_by BIGINT NOT NULL DEFAULT 0,
  deadline_at BIGINT NOT NULL,
  deadline_by BIGINT NOT NULL DEFAULT 0,
  failure_code VARCHAR(64) NOT NULL DEFAULT ''
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_collaboration_runs_client ON ai_collaboration_runs (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_collaboration_runs_conversation ON ai_collaboration_runs (workspace_id,conversation_id,id);
CREATE INDEX ix_ai_collaboration_runs_active ON ai_collaboration_runs (workspace_id,state,id);

CREATE TABLE ai_collaboration_tasks (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  run_id BIGINT NOT NULL,
  step_key VARCHAR(64) NOT NULL,
  agent_id BIGINT NOT NULL,
  turn_id BIGINT,
  state VARCHAR(24) NOT NULL DEFAULT 'pending',
  spec_asset_id BIGINT NOT NULL,
  result_asset_id BIGINT,
  error_code VARCHAR(64) NOT NULL DEFAULT ''
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_collaboration_tasks_step ON ai_collaboration_tasks (workspace_id,run_id,step_key);
CREATE UNIQUE INDEX ix_ai_collaboration_tasks_turn ON ai_collaboration_tasks (workspace_id,turn_id);
CREATE INDEX ix_ai_collaboration_tasks_barrier ON ai_collaboration_tasks (workspace_id,run_id,state);

CREATE TABLE ai_context_snapshots (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  turn_id BIGINT NOT NULL,
  branch_id BIGINT,
  snapshot_asset_id BIGINT NOT NULL,
  snapshot_hash VARCHAR(64) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_context_snapshots_turn ON ai_context_snapshots (workspace_id,turn_id);
CREATE INDEX ix_ai_context_snapshots_branch ON ai_context_snapshots (workspace_id,branch_id,id);

CREATE TABLE ai_conversation_branches (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  conversation_id BIGINT NOT NULL,
  owner_id BIGINT NOT NULL,
  title VARCHAR(128) NOT NULL,
  parent_branch_id BIGINT,
  fork_message_id BIGINT,
  head_message_id BIGINT
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE INDEX ix_ai_conversation_branches_conversation ON ai_conversation_branches (workspace_id,conversation_id,id);

CREATE TABLE ai_branch_messages (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  branch_id BIGINT NOT NULL,
  message_id BIGINT NOT NULL,
  sequence BIGINT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_branch_messages_message ON ai_branch_messages (workspace_id,branch_id,message_id);
CREATE UNIQUE INDEX ix_ai_branch_messages_sequence ON ai_branch_messages (workspace_id,branch_id,sequence);

CREATE TABLE ai_dispatch_intents (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  turn_id BIGINT NOT NULL,
  attempt BIGINT NOT NULL DEFAULT 1,
  request_id VARCHAR(64) NOT NULL,
  host_id BIGINT NOT NULL,
  binding_hash VARCHAR(64) NOT NULL,
  epoch BIGINT NOT NULL,
  state VARCHAR(16) NOT NULL DEFAULT 'prepared',
  outcome VARCHAR(16) NOT NULL DEFAULT '',
  confirm_deadline_at BIGINT,
  confirm_deadline_by BIGINT NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_dispatch_intents_request ON ai_dispatch_intents (workspace_id,request_id);
CREATE UNIQUE INDEX ix_ai_dispatch_intents_attempt ON ai_dispatch_intents (workspace_id,turn_id,attempt);
CREATE INDEX ix_ai_dispatch_intents_state ON ai_dispatch_intents (workspace_id,state,id);

CREATE TABLE ai_knowledge_requests (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  version BIGINT NOT NULL DEFAULT 1,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  owner_id BIGINT NOT NULL,
  client_id VARCHAR(64) NOT NULL,
  action VARCHAR(64) NOT NULL,
  target_kind VARCHAR(32) NOT NULL,
  target_id BIGINT,
  payload_hash VARCHAR(64) NOT NULL,
  result_asset_id BIGINT,
  state VARCHAR(16) NOT NULL DEFAULT 'pending'
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
CREATE UNIQUE INDEX ix_ai_knowledge_requests_client ON ai_knowledge_requests (workspace_id,owner_id,client_id);
CREATE INDEX ix_ai_knowledge_requests_target ON ai_knowledge_requests (workspace_id,target_kind,target_id);

```

## 5. 历史数据与迁移up/down

拟追加独立迁移，注册序号在实施时确定。新增14张表，不自动提取历史、不改旧消息/turn、不改已有session绑定，不改存量角色权限。旧主分支兼容读取，不批量改写。definition_revisions扩展subject_kind、asset_refs扩展owner_kind为业务适配，必须验证安全边界。

up：备份→维护窗口验证版本→检查表/索引形状→创建缺失对象→登记迁移版本→默认关闭新功能。形状不一致停止，不DROP重建；MySQL中途失败保留已建对象，只补缺失项。三种真实数据库验证在实施阶段，不能用SQLite替代。

down：不自动DROP领域业务表；迁移down明确拒绝破坏性回退。停止所有实例、备份、关闭新能力、等待活动任务停止后保schema退二进制；旧版不能处理新数据时隔离入口或恢复备份并说明升级后损失。不允许旧新调度器混跑新协作。真实迁移、生产配置及恢复须另行授权。

## 6. 检索与原子性

第一版无需独立向量库：按updated_at/id游标分批扫描当前已授权active记忆，最多10000条；授权后读取正文/向量资产，服务端内存执行NFC+Unicode小写后连续子串关键词匹配及同模型余弦；分别取前100再按主PRD RRF融合。超扫描上限返回partial=true并提示缩小范围，不能假装完整结果。容量/延迟基线在真实样本补测。

缓存按workspace和权限上下文隔离，注入前再验来源；向量不可解码/模型不匹配时降级并标degraded，不把未知填0。资产先stage；目标/证据CAS→attach→领域更新和审计同事务；失败暂存交既有GC。合并锁定全部目标，不部分生效。模型调用放事务外，返回后重验授权、配置、version。提案幂等重放查询原回执。

## 7. REST接口契约

全部路径前缀 `/api/workspaces/{workspace}`；仅Scope决定workspace；下表接口是拟定设计，不冒充当前存在。

| 接口 | 请求关键字段 | 授权/结果 |
|---|---|---|
| GET/POST /ai-memories；GET/PATCH/DELETE /ai-memories/{id} | q/kind/state/agent_id/page/page_size/sort；title/body/kind/agent_id/expires_at；写带version/client_id | 对应能力及owner；列表信封，正文详情才读取；仅归档且无活动引用可删 |
| POST /ai-memories/{id}/actions | activate/invalidate/archive/restore/lock/unlock/create_shared_draft；version/client_id；共享须提交独立title/body及确认声明，不复制原私聊正文 | 状态、来源及权限；共享返回新draft ID |
| GET /ai-memories/{id}/evidence | page/page_size | 当前可见来源；无权不返回源标题 |
| POST /ai-memories/{id}/evidence | source_kind/conversation_id?/message_id?/turn_id?/version/client_id | owner+update；服务端读取源version/hash，不信客户端hash；active修改来源先回draft |
| POST /ai-memory-searches | q/agent_id?/limit(1–8)/client_id | ai-memories.use；items含id/version/title/match_sources/evidence_status；partial/degraded |
| GET/POST /ai-skills；GET/PATCH/DELETE /ai-skills/{id} | name/title/body/version/client_id | 正文只写候选；对象范围授权 |
| GET /ai-skills/{id}/revisions | page/page_size | 授权后复用definition revisions |
| POST /ai-skills/{id}/actions | publish/disable/enable/archive/restore_revision；version/revision_id?/client_id；enable与publish共享版本/来源检查 | 发布、恢复需ai-skills.publish；返回指针与回执；archived恢复只为draft |
| GET/PATCH /ai-knowledge-settings/{id} | version/配置白名单/profile/embedding/cron | 本人+configure；owner=0需工作区配置管理；不编辑他人设置 |
| GET/POST /ai-knowledge-jobs；POST /ai-knowledge-jobs/{id}/cancellations | kind/settings_id/conversation_id?/turn_id?/client_id | ai-learning.run/cancel；后台仍校验原owner当前权限 |
| GET /ai-learning-proposals；GET /ai-learning-proposals/{id} | state/kind/page/page_size | ai-learning.read+owner，证据再鉴权 |
| POST /ai-learning-proposals/{id}/decisions | accept/reject/cancel；version/client_id | ai-learning.review+目标权限；技能接受只产候选，不发布 |
| GET/POST /ai-collaborations；GET /ai-collaborations/{id} | conversation_id/plan/version/client_id | 会话权限+协作权限；POST只建draft |
| POST /ai-collaborations/{id}/actions | start/cancel；version/client_id | 原发起人当前权限、capability、计划不变 |
| GET /ai-collaborations/{id}/tasks | page/page_size | 当前会话权限；结果引用分别鉴权 |
| POST /ai-collaborations/{id}/tasks/{task}/decisions | request_id/allow/version/client_id | 原发起人单次审批，内部复用turn审批 |
| GET/POST /conversations/{id}/branches | title/parent_branch_id?/fork_message_id?/version/client_id | 当前会话权限；fork完成可见消息 |
| GET /conversations/{id}/branches/{branch}/messages | page/page_size | 继承路径+本分支，撤回实时反映 |
| POST /conversations/{id}/turns | 既有字段+branch_id?/expected_head_id? | capability门控；旧请求仍主链 |
| GET /conversations/{id}/turns/{turn}/context | 无 | 上下文元数据，不返回无权正文 |
| GET /conversations/{id}/turns/{turn}/dispatch | 无 | 意图/attempt/状态证据；无密钥与原生敏感诊断 |

新资源settings通过GET列表(本人/工作区行)、POST创建设置补齐生命周期，create带agent_id/client_id/配置；唯一owner+agent防重复；关闭设置不删除审计。workspace行由系统创建全关闭，更新需ai-learning.configure和现有工作区管理权限，私有内容不因管理权开放。

列表统一 `{items,total,page,page_size}`；page默认1，page_size默认20上限200；sort仅id/title/created_at/updated_at适用列，默认-created_at。搜索动作 `{items,partial,degraded}` 不是管理列表。详情直接对象；错误 `{error,fields?}` 本地化。创建201，读取/幂等200，删除204，异步启动202；400字段、401认证、403可见对象无权操作、404不可见、409版本/状态/能力、429现有配额、502供应商明确失败、503依赖不可用。未知供应商原始响应不能透出。500给追踪ID并保留输入。未实现工具必须失败，不能success:true。

## 8. 协议冻结保留项

可信确认要求request_id/attempt/epoch、binding和只读capability，当前Request并不证明已具备完整Runner状态查询。正式新增帧源未定义，待补协议设计；不得伪造存量帧。去重需跨Runner重启持久化与有界清理策略，清理不能早于服务端确认终态。未证实不重复启动前，unknown禁止重派。缺新capability阻止新功能，旧会话维持原兼容行为。

该项R04阻断F/E开发就绪；模型表、DDL和领域API完整不代表协议、角色映射、价格适配和真实数据库验收已经完成。

## 9. 协议与幂等的拟定精确规则

以下为新增协议提案，**源未定义，待补实施前确认与适配器验证**，不声称当前已存在对应帧。当前Request有deny_unknown_fields，因此不能把新字段直接发送给旧Runner。

| 拟定能力 / 消息 | 字段 | 处理规则 |
|---|---|---|
| capability `agent_execution_v2` | protocol_version=2；durable_dedup、status_query、read_only_enforced、isolated_session布尔 | 只有认证连接实际报告并通过适配器验证的能力才可用；缺一项阻断对应新模式 |
| start_v2 | request_id(ASCII≤64), workspace_id, turn_id, attempt, epoch, binding_hash, request(现有Request结构) | 外层新消息类型承载元数据，旧Request结构不追加未知键；服务端与Runner都校验绑定 |
| ack_v2 | request_id,attempt,epoch,status(accepted/rejected),error_code? | accepted仅说明受理/进程实际启动确认，不说明任务成功；重复请求返回原状态，不再次spawn |
| query_v2 | request_id,attempt,epoch | 只查询本认证节点/执行用户的操作，不允许跨身份查进程 |
| status_v2 | request_id,attempt,epoch,state(not_received/spawn_intent/spawned/terminal/unknown),outcome? | not_received只有在持久化记录及实际进程均可证明未接收/未启动时可信；存储损坏返回unknown |
| cancel_v2 | request_id,attempt,epoch | 幂等停止请求；不能以已发cancel表示已停止；回传终态需进程清理完成 |
| terminal_commit_v2 | request_id,attempt,epoch | 服务端已持久化终态的确认，仅在Runner验证无活动进程后允许进入既有状态清理策略；不提前删除去重记录 |

本地去重记录放Runner已有受管状态目录，文件与目录权限沿用既有安全策略；按服务端实例身份+workspace+request_id隔离。记录schema_version、turn_id、attempt、epoch、binding_hash、state、process_identity(进程ID及创建身份，不能只查PID)、outcome、updated_at/updated_by。原子替换与文件锁先写spawn_intent后spawn。崩溃在两者之间属于unknown，不能因为进程暂未查到就重复启动；这里保证“不盲目重复启动”，不宣称跨进程启动与数据库事务实现严格exactly-once。

本地状态查询/清理必须有真实重启与PID复用测试；确切目录复用点、锁实现及既有清理策略适配仍在R04评审范围，不能在PRD阶段编造为已验证代码。无需为这个提案修改操作系统全局CLI配置。

新增领域HTTP写操作的ai_knowledge_requests与业务变更同事务；请求幂等键在owner/workspace内唯一，action计入payload_hash。result资产仅存对象ID、版本、结果/状态；重放先重新鉴权，目标已删返回missing，不能恢复已删除正文。pending没有提交业务变更时可安全回滚；已提交queued任务只返回同task ID。领域外部执行仍使用dispatch intent，不能把HTTP回执当作执行成功证据。

分支与session关联不新增另一份原生session表：ai_context_snapshots关联turn/branch；恢复查同branch最近成功turn的候选已提交session及其原有执行绑定。首次分叉不复用父session；virtual主链branch=null，子分支parent=null+fork_message_id表示从旧主链分叉。分支消息创建、head/version CAS、turn及快照入库同事务；每分支一个活动turn，跨分支仍服从同目录执行锁。

创建/编辑记忆可提交sources描述，但source_version/hash必须服务端读取；active更改正文/证据先变draft。manual evidence由服务端生成作者/hash；不可由客户端自选其他owner。共享action只接受独立输入及明确声明，新对象不复制私聊内容。技能草稿与证据采用相同血缘校验；删除资产引用只能经过领域对象的授权操作。

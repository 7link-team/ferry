# Task State
## Task / Objective
T3b：执行 docs/tasks/T3b-bigint-ids.md；已读设计 §18 与 CLAUDE.md；不 commit/push。
## Current Phase
ACCEPTANCE — 实现及任务验收完成。
## Completed Work
- State::next_id -> Result<Id>，发号失败返回 503；删除 db::id，无兼容壳。
- 14 张业务表共 41 个 TEXT ID 列重写 BIGINT（实数超过任务预估约 39）。保留原生 SQL、Spec 白名单。
- SQL 绑定 i64，读取 db::int + Id，JSON ID 为字符串；字符串请求体显式解析；空引用存 0；可选引用 "0" 可回传；非数字资源路径 404。
- 贯通会话、工作空间、资源、审计、runner 注册/重连/心跳、SSH 与凭据 AAD。
- 新增三库 8 项回归：>2^53 真实雪花完整 CRUD、JSON 字符串、字符串 group_id 关联、非法路径 404。原矩阵脚本除新增段外逐字相同，未改原断言。
- 新增 SQLite 实际迁移 schema 逐列测试；MySQL/Postgres information_schema 人工逐列查询均为 bigint。
## ID Inventory（41 列，全部 BIGINT）
| 表 | 列 | 数量 |
|---|---|---:|
| users | id | 1 |
| sessions | id, user_id, workspace_id | 3 |
| workspaces | id, owner_id | 2 |
| workspace_roles | id, workspace_id | 2 |
| workspace_members | id, workspace_id, user_id, role_id | 4 |
| activity | id, workspace_id, actor_id, resource_id | 4 |
| host_groups | id, workspace_id, parent_id | 3 |
| hosts | id, workspace_id, group_id, credential_id（ALTER） | 4 |
| tags | id, workspace_id | 2 |
| host_tags | host_id, tag_id | 2 |
| runner_tokens | id, workspace_id, host_id, used_by, created_by | 5 |
| credentials | id, workspace_id | 2 |
| repositories | id, workspace_id, credential_id | 3 |
| pipelines | id, workspace_id, repository_id, target_ref | 4 |
worker_ids.worker_id 原为 BIGINT 不变；worker_ids.instance_id、locks.owner/name 和业务字符串不变。
## Verification（实际退出码）
- cargo fmt --all -- --check：0。
- cargo check --workspace --all-targets：0；无警告。/tmp/t3b-check-final.log。
- cargo test --workspace：0；protocol 3、runner 7、server 80，0 failed。/tmp/t3b-tests-final.log。
- npm run typecheck && npm test：0；2 files / 11 tests。/tmp/t3b-npm-final.log。
- npm run build：0；既有 chunk >500kB 提示，未扩展打包优化。/tmp/t3b-npm-build.log。
- MySQL DROP/CREATE ferry：0；Postgres DROP ferry：0、CREATE ferry：0；SQLite 每轮临时库自动新建。
- FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' ./scripts/test-backends.sh：首轮及恢复后最终均 0；三库各 98/98（原90+新增8）。/tmp/t3b-matrix1.log、/tmp/t3b-matrix-final.log。
- 变异：/tmp/t3b-mutation.sh 临时去掉 host_json 的 Id 包装后运行矩阵 SQLite；退出 1，95 passed / 3 failed（新增列表、详情、更新后详情精确字符串检查）。EXIT trap 已恢复源码，最终全绿。/tmp/t3b-mutation.log。
- git diff --check：0；改动文件 LF；原41个TEXT声明无遗漏、db::id零残留、原矩阵断言逐字保留审计脚本：0。
- 实现过程中初次 cargo check 有类型贯通错误（101），修正边界后通过；正式矩阵未出现非变异失败，未修改断言迁就。
## Self-Review / Final Acceptance
范围、复用（T3a Id/Generator、db::int、现有 SQL/列表）、架构、复杂度、回归、逐列清单、变异恢复、状态一致性均检查通过。Direct 执行，无子代理。
## Known Issues / Scope
现有会话直接以 sessions.id 作为 bearer cookie；雪花 ID 可预测，不能提供 UUID 的随机性。任务要求所有主键改雪花，未擅自扩展认证方案；建议后续将随机会话令牌与内部主键分离，部署前处理。
## Remaining Work / Next Action
本任务无剩余实现或验收动作；未 git commit/push，修改保留在工作区。原用户未跟踪任务书未改动。会话令牌问题需另行授权认证设计变更。
## Last Checkpoint
最终三库各 98/98，EXIT 0；变异源码恢复并重新编译验证。

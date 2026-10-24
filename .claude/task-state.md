# Task State
## Task / Objective
T3c：执行 docs/tasks/T3c-seaorm.md；已读设计 §18.5 与 CLAUDE.md。保留列名白名单、三库99/99、全部验收及两次变异；不commit/push。
## Current Phase
ACCEPTANCE — PASS，全部实现、验证、自审与原始需求验收完成。
## Completed
- entities.rs：16张表的普通SeaORM镜像，复用原locks/worker_ids实体；无继承抽象。host_tags尚无业务调用，按每表实体要求保留，expect(dead_code)明确说明原因，不伪造调用。
- 全部业务db::execute/one/all调用改Entity/ActiveModel/Select；列名使用实体Column。所有JSON ID显式Id字符串，更新保留workspace/id/version条件及影响行数。
- resource::list唯一实现：泛型Select/FromQueryResult、Condition与绑定值；Spec过滤/排序白名单完整保留。4项原安全测试等价迁移，未删除。
- credentials列表/详情使用Summary最小投影，仅计算has_secret，不读出密文；新增SQLite实际查询结果测试保证secret列不存在。
- 删除db::statement/execute/one/all/int/text；dialect::query/placeholder_count/numbered_placeholders及4个废弃测试；删除无调用者State::db。
- 保留dialect::schema/index_target。迁移DDL与执行逻辑逐字未变；migration.rs仅schema测试直接使用PRAGMA并注明ORM无SQLite元数据接口。
## Delegation / Review
Hybrid：resources负责四资源；identity负责auth/workspace/main；connections负责runner/remote/ssh/lock/worker。主线程负责共享层、集成及全部最终验收。
独立复审认证/连接无问题；四资源复审发现凭据完整实体读取密文，主线程恢复最小投影并加测试。未改业务文案、未放宽任何矩阵断言。
## Verification（实际退出码）
| 命令 | 退出码 / 结果 | 日志 |
|---|---|---|
| cargo build -p ferry-server（改动前） | 0 | /tmp/t3c-baseline-build.log |
| 原始二进制三库矩阵（FERRY_BINARY=/tmp/t3c-baseline-server） | 0，每库99/99 | /tmp/t3c-baseline-matrix.log |
| cargo fmt --all -- --check | 0，变异恢复后确认 | 无输出 |
| cargo check --workspace --all-targets | 0，无任何警告/never used | /tmp/t3c-check-final.log |
| cargo test --workspace | 0，protocol3、runner7、server77，共87通过 | /tmp/t3c-tests-final.log |
| npm run typecheck && npm test | 0，11/11 | 工具执行输出 |
| npm run build | 0，既有chunk>500kB提示 | /tmp/t3c-npm-build.log |
| 三库矩阵（初次迁移后） | 0，每库99/99 | /tmp/t3c-matrix.log |
| 三库矩阵（全部变异恢复、凭据投影修复后） | 0，每库99/99 | /tmp/t3c-matrix-final.log |
| git diff --check | 0 | 无输出 |
| 16表逐列类型/迁移实现不变/矩阵断言不变/LF/两处变异恢复审计 | 0 | 工具执行输出 |

矩阵实际命令（使用新建隔离库ferry_t3c，避免清除已有ferry；其余参数同任务书）：
```sh
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry_t3c' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry_t3c' \
./scripts/test-backends.sh
```
每轮重建专用测试库：docker exec ferry-my mysql -uroot -pdevpass -e 'DROP DATABASE ferry_t3c; CREATE DATABASE ferry_t3c'；docker exec ferry-pg dropdb -U postgres ferry_t3c；docker exec ferry-pg createdb -U postgres ferry_t3c。每条退出0。
## Mutation Results
1. /tmp/t3c-mutate-sort.py把排序白名单判断临时改成false，运行cargo test -p ferry-server resource::tests：退出101，未知sort拒绝断言失败（3通过/1失败）；finally恢复后同命令退出0，4/4。日志/tmp/t3c-mutation-sort.log、/tmp/t3c-sort-restored.log。凭据投影修复后再次执行同变异，仍101；最终完整测试再次覆盖恢复后的4项，全绿。
2. /tmp/t3c-mutate-version.py去掉update_host的Version.eq(body.version)，运行完整三库矩阵：退出1，每库97通过/2失败，stale update错误接受和read back被覆盖，总6失败；finally恢复。日志/tmp/t3c-mutation-version.log。凭据投影修复后重跑仍得到同结果；最终每库99/99，退出0。
两个变异均是预期断言失败，不是编译失败；测试脚本和原矩阵断言未改。
## Failures / Resolution
初期集成check退出101：实体生成脚本误匹配迁移测试中的CREATE文本，修复后逐列对照16表；补全泛型Model Sync约束、ExprTrait方法歧义及类型导入。没有修改断言迁就。正式矩阵无非变异失败。
## Residual SQL / Scope
业务手写SQL零残留。迁移DDL明确留待T3d；schema检查测试的SQLite PRAGMA无法用实体查询表达，已有就地注释。
现有host_tags无业务调用者，未扩展标签关联功能；既有前端大chunk提示未扩展打包优化。SSH实机用例未配置目标时按原逻辑返回，不宣称已做实机验证（非本任务门槛）。
## Self-Review / Final Acceptance
范围、复用、架构、复杂度、回归、白名单、Id、version、迁移边界、状态一致性、命令退出码与变异验证：PASS。
没有git commit/push、没有暂存；用户原未跟踪任务书未改。源文件和状态记录留工作区。
## Remaining Work / Next Action
本任务无剩余实现或验收动作，无需用户补充输入。

# Task State
## Task / Objective
T3d：执行 docs/tasks/T3d-schema-builder.md；已读 CLAUDE.md、设计§3.1/§18.5。不 commit/push。
## Current Phase
ACCEPTANCE — PASS
## Completed / Design
- migration.rs：16张表、全部索引和加列使用SeaQuery builder，保留7个迁移名/顺序。Table IF NOT EXISTS；Index/Column通过SchemaManager.has_index/has_column查询再执行，覆盖部分失败后的重跑。
- 显式big_integer/string_len/text。SeaQuery 1.0.2的Text在MySQL实际生成TEXT，故长列用.text()并仅在MySQL通过.custom("longtext")保留旧容量；没有SQL改写。所有MySQL建表用builder设置utf8mb4/utf8mb4_bin。
- schema_tests构建生产迁移的真实MySQL SQL（全部7组含ALTER），逐列断言TEXT/LONGTEXT无DEFAULT、无裸TEXT；保留definition容量检查，新增charset/collation检查。
- SQLite BigInteger生成INTEGER，元数据断言适配为INTEGER，并对同41业务ID列核验MySQL生成SQL仍是BIGINT，避免接受32位列。
- 删除dialect.rs、main.rs的mod dialect、SQL加列解析及过时解析器测试；脚本仅更新已删除dialect的注释，可执行部分逐字未变。
## Files Changed
crates/server/src/migration.rs、crates/server/src/main.rs、scripts/test-backends.sh（仅注释）、.claude/task-state.md、.claude/decisions.md；删除crates/server/src/dialect.rs。
用户原未跟踪docs/tasks/T3d-schema-builder.md未改。
## Execution / Delegation
Direct；同文件紧耦合实现与验收，无子代理。
## Verification — 实际命令与退出码
| 命令 | 退出码 / 结果 | 日志 |
|---|---|---|
| cargo check --workspace --all-targets（删dialect前） | 0；LONG_COLUMNS/schema/mysql_schema/index_target全部never used | /tmp/t3d-check-before-delete.log |
| cargo fmt --all -- --check | 0 | /tmp/t3d-fmt.log |
| cargo check --workspace --all-targets（删后最终） | 0；零警告，额外断言日志无warning: | /tmp/t3d-check-final.log |
| cargo test --workspace | 0；protocol3/runner7/server73，共83通过 | /tmp/t3d-tests-final.log |
| npm run typecheck && npm test | 0；11/11 | 工具输出 |
| npm run build | 0；已有chunk>500kB提示 | /tmp/t3d-npm-build.log |
| 三库命令（见下） | 0；三库各99/99 | /tmp/t3d-matrix.log |
| python3 /tmp/t3d-rerun.py | 0；两库最后迁移及全部迁移重放均通过 | /tmp/t3d-rerun.log |
| python3 /tmp/t3d-audit.py | 0；原DDL与新MySQL的14业务表124列类型/NULL/default、30主键/索引完全相同 | 工具输出 |
| git diff --check | 0 | 无输出 |
| 范围/变异恢复/矩阵主体/零警告审计 | 0；无业务修改、LF一致 | 工具输出 |

全新建库（以下3条分别退出0，SQLite由矩阵创建全新临时目录）：
```sh
docker exec ferry-my mysql -uroot -pdevpass -e 'DROP DATABASE IF EXISTS ferry; CREATE DATABASE ferry'
docker exec ferry-pg psql -U postgres -c 'DROP DATABASE IF EXISTS ferry'
docker exec ferry-pg psql -U postgres -c 'CREATE DATABASE ferry'
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
./scripts/test-backends.sh
```
实际输出：
```
SQLite:    99 passed, 0 failed
MySQL:     99 passed, 0 failed
Postgres:  99 passed, 0 failed
All backends passed.
```
## 迁移重跑：实际命令和输出
执行python3 /tmp/t3d-rerun.py，退出0。脚本通过subprocess逐条检查数据库命令退出码：
- MySQL前缀：docker exec ferry-my mysql -uroot -pdevpass -BN ferry -e
- PostgreSQL前缀：docker exec ferry-pg psql -U postgres -d ferry -At -v ON_ERROR_STOP=1 -c
- 各执行SQL：DELETE FROM seaql_migrations WHERE version='id_infrastructure_v1'（最后迁移）；确认记录数7→6。
- 使用target/debug/ferry-server启动，环境FERRY_DATABASE_URL为上述对应URL、FERRY_BIND为127.0.0.1:4321/4322、FERRY_DATA_DIR为隔离临时目录、随机FERRY_MASTER_KEY、RUST_LOG=info,sea_orm_migration=info。
- HTTP /api/status=200；记录6→7，id_infrastructure_v1仅1条；比较information_schema.columns、MySQL statistics/PG pg_indexes以及14业务表SELECT * ORDER BY 1，完全相同。
- SIGTERM后wait退出0。
- 额外执行DELETE FROM seaql_migrations，记录7→0，重启验证全部7迁移重放，结果同样通过，实际覆盖has_index和has_column。
实际输出（两库各一次last及一次all）：
```
MySQL delete exit=0; migration records=6
MySQL restart /api/status HTTP=200
MySQL migration records=7; last record=1; schema/indexes/14 business tables unchanged
MySQL service shutdown exit=0
Applying migration 'id_infrastructure_v1'
Migration 'id_infrastructure_v1' has been applied
PostgreSQL delete exit=0; migration records=6
PostgreSQL restart /api/status HTTP=200
PostgreSQL migration records=7; last record=1; schema/indexes/14 business tables unchanged
PostgreSQL service shutdown exit=0
Applying migration 'id_infrastructure_v1'
Migration 'id_infrastructure_v1' has been applied
PASS: both backends replay last and all migrations without changing schema or business rows
```
详尽输出/tmp/t3d-rerun.log；4次启动日志/tmp/t3d-rerun-{MySQL,PostgreSQL}-{last,all}.log。
## Mutation
执行python3 /tmp/t3d-mutation.py（包装器退出0）：将secret临时替换成ColumnDef::new(Alias::new("secret")).text().not_null().default("")。
- 变异前cargo test -p ferry-server migration::schema_tests：0，5/5。
- 同命令变异后：101，3通过/2失败；明确是断言失败而非编译失败：no_long_column_carries_a_default、no_text_column_survives_into_mysql。
- 断言输出：MySQL rejects a DEFAULT on a TEXT/LONGTEXT column: `SECRET` TEXT NOT NULL DEFAULT ''。
- finally恢复后同命令：0，5/5；最终工作区测试及矩阵均在恢复后执行。
- 日志/tmp/t3d-mutation.log、/tmp/t3d-schema-restored.log；源码恢复审计通过。
## Failures / Resolution
首次schema测试101：SQLite BigInteger生成INTEGER，旧测试预期BIGINT；新增MySQL宽度校验同时修正SQLite元数据期望。新增collation测试最初使用CHARACTER SET/空格拼法，但builder实际输出DEFAULT CHARSET=/COLLATE=；按实际SQL语法修正。没有放宽列宽、MySQL TEXT或矩阵保护。
## Self-Review / Final Acceptance
范围、复用、架构、复杂度、业务/JSON/文案、整数宽度、长文本容量、大小写、迁移幂等、两项保护及变异、全新三库99/99、全部验收退出码、状态一致性：PASS。
没有git commit/push/暂存；代码和记录留工作区。原设计§3.1的历史描述未做范围外文档重写；已有前端大chunk提示未扩展优化。
## Remaining Work / Next Action
无剩余实现或验收；无需用户输入。交付中文报告，保留未提交改动。

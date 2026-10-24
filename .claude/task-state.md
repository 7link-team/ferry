# Task State
## Task
T3a 雪花 ID、数据库锁、worker 分配；用户要求禁止 commit/push。
## Current Phase
ACCEPTANCE — PASS
## Completed Work
已先读 §18、任务书、db/migration/state/error/CLAUDE。
直接执行，无子代理。新增 snowflake.rs、lock.rs、worker.rs（12项测试）。
生成器使用 Mutex、可注入时钟、41/10/12 位、2026 epoch、JSON字符串；10万单调、10线程×1万唯一；4096序列耗尽等待；小回拨等待、大回拨拒绝。
锁 UPDATE 再 INSERT，无 upsert；release/renew 都限定 owner；真实 SQLite 文件库并发与过期测试。
worker 事务内分配锁，错误/取消回滚；优先复用，最小空闲/过期槽位，CAS避免覆盖并发恢复的心跳；10秒心跳/60秒TTL，发号和等待期间检查 Instant 租约；被回收不能续期。关闭置心跳0保留重启亲和性。
新增 ORM schema builder 迁移；main/state 接入，真实启动拒绝/关闭已验证。
## Changed Files
crates/server/src/{snowflake,lock,worker,main,state,migration}.rs
crates/server/Cargo.toml、Cargo.lock（测试用 tempfile 与其传递依赖）
.claude/task-state.md、.claude/decisions.md
用户原有设计文档变更和未追踪任务书保留未改；现有业务模块/主键/db::id调用未改。
## Verification — Commands / Exit Codes
- cargo fmt --all：0
- cargo fmt --all -- --check：0，变异恢复后最终复核0
- cargo check --workspace --all-targets：0，变异恢复后最终复核0，无警告
- cargo test -p ferry-server --bin ferry-server：0，首轮78项通过
- cargo test --workspace：0，最终protocol 3 + runner 7 + server 78 = 88项通过
- npm run typecheck && npm test：0，11项通过
- npm run build：0，既有chunk超过500kB提示（非本任务改动）
- FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' ./scripts/test-backends.sh：0，SQLite/MySQL/Postgres各90项通过；脚本内cargo build -p ferry-server成功
- python3 /tmp/ferry-t3a-lifecycle.py：0；独立SQLite文件库实进程验证心跳更新、重启复用、两次优雅退出均0且heartbeat_at=0；填满1024槽位后启动退出1（预期拒绝）、分配锁回滚无残留。临时脚本验证后删除。
- git diff --check：0
- git status --short / git diff --stat / git diff --numstat / git diff --cached --stat / git ls-files --eol：0；无暂存内容，相关源码LF
## Mutation Verification
1. cargo test -p ferry-server expired_heartbeat_stops_ids_and_recovers：基线0（1通过）；删除心跳过期检查后101（1失败，unwrap_err收到Ok ID）；逐字节恢复后0（1通过）。
2. cargo test -p ferry-server non_owner_cannot_release_or_renew：基线0（1通过）；删除release owner条件后101（1失败，实际影响1行、期望0）；逐字节恢复后0（1通过）。
两个Python变异驱动均退出0，finally恢复并校验原字节；最终全量测试通过，无变异残留。
## Design Differences / Constraints
按T3a明确范围，不执行§18.5旧表/旧SQL全量重写及§18.6第7项的全仓库清理；新增机制全用ORM，无新增raw SQL。未改旧主键和db::id调用。
Instant本地租约、事务/CAS、关闭保留映射置0是落实安全与重启要求的细化，详见decisions.md。
跨实例租约仍依赖设计中的墙钟同步；instance_id必须代表唯一运行实例，不能复制数据目录并同时运行。
## Reviews
Self-review PASS：范围/复用/架构/复杂度/回归/状态/无垃圾暂存。
Final acceptance PASS：所有T3a要求、验收命令、两次变异、中文报告内容已核对；未commit/push。
## Remaining Work
无待实现或待验证项。三库矩阵测试数据仍保留于本地测试数据库；工作树包含本次未提交变更及用户原有文档变更。
## Next Action
交付中文报告；不提交、不推送，无需用户补充信息。

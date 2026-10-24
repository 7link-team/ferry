# Task State
## Task
T2 pipelines 服务端定义与校验；不 commit/push。
## Current Phase
ACCEPTANCE — PASS
## Completed Work
新增 pipelines CRUD、纯函数定义校验、pipelines_v1 迁移、definition LONGTEXT、路由、仓库删除引用保护。
复用 resource::list、Scope、hosts::stale/activity、channel::check_path；四条 CRUD 权限已存在。
新增12组定义单测、1个 LONGTEXT 单测，三库脚本新增32项检查。
## Decisions
见 decisions.md。直接执行，无子代理；不做执行引擎/界面、不改旧迁移及禁止修改模块。
## Verification
- cargo fmt --all：0
- cargo fmt --all -- --check：0（变异恢复后再次0）
- cargo check --workspace --all-targets：0，无警告
- cargo test --workspace：0，protocol 3 + runner 7 + server 66，共76项
- npm run typecheck && npm test：0，11项
- bash -n scripts/test-backends.sh：0
- FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' ./scripts/test-backends.sh：0，各87项通过，脚本内 cargo build -p ferry-server 成功
- git diff --check：0（恢复后再次0）
- git status --short / git diff --stat / git diff --numstat / git diff --指定源码：均0
- git ls-files --eol（5个已追踪源码/脚本）：0，全部LF
- docker ps --format '{{.Names}} {{.Ports}}'：0，测试容器和端口可用
- docker exec ferry-pg psql -U postgres -d ferry -Atc 'SELECT count(*) FROM users'：1，当时尚无users表；后续脚本迁移与三库验收成功
- 任务/必读文件/相关实现的 cat、sed、rg、ls 查询：全部0
- 文件生成、更新、备份、变异、恢复的 Python/heredoc 命令：全部0
## Mutation
cargo test -p ferry-server pipelines::tests：
变异前0，12通过；白名单条件替换为 if false（全部放行）后101，11通过1失败；
失败项 template_whitelist_rejects_adversarial_tokens_everywhere：{{unknown}} 意外返回Ok；
恢复原文件（逐字节确认）后0，12通过。临时备份已删除，工作树无变异残留。
## Reviews
自审及原任务验收PASS；没有commit/push。用户原有未追踪任务书保留未改。
## Known Issues
现有 hosts::bulk_delete_hosts 不检查流水线引用，删除主机可能留下失效目标；按本任务只增加仓库删除保护的边界未改。
引用检查沿用现有先查询再写入模式，未引入并发引用完整性锁或外键。
三库端到端测试数据保留在测试数据库；不宣称工作树干净。
## Remaining Work
本任务无待执行项。
## Next Action
交付中文报告，提交由用户自行决定。

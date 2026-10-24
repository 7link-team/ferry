# Task State
## Task
T1 repositories 服务端资源；不 commit/push。
## Current Phase
ACCEPTANCE — PASS
## Design
直接执行，复用 credentials 结构、resource::list、Scope、hosts::stale/activity。
追加 repositories_v1，不修改旧迁移。权限已存在。URL 上限 255 字符以兼容 MySQL。
## Confirmed Constraints
不改 channel/remote/ssh/runner/vault，不做界面或流水线。保留用户已有 docs 修改。
## Remaining Work
全部实现及验收完成；无待执行工作。未 commit/push。
## Verification
PASS: fmt check、cargo check 全目标（无警告）、cargo test（62 项）、npm typecheck/test（11 项）、bash -n、git diff --check。
辅助检查曾误查不存在的 activity.rs（退出 2），已定位 hosts.rs；Postgres 表数查询引号错误（退出 1），已修正并退出 0。
## Next Action
交付中文报告，用户自行提交。

## Final Verification
- cargo fmt --all：0
- cargo fmt --all -- --check：0
- cargo check --workspace --all-targets：0，无警告
- cargo test --workspace：0，62 项
- npm run typecheck && npm test：0，11 项前端测试
- cargo build -p ferry-server：0
- FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' ./scripts/test-backends.sh：0，三库各 55 项通过
- bash -n scripts/test-backends.sh：0
- git diff --check：0
## Reviews
自审 PASS；原任务验收 PASS。旧迁移与禁止修改的模块未改，用户 docs 修改保留。
## Known Issues
现有 credentials::remove 只检查 hosts 引用，未检查新增 repositories 引用；依 T1 范围未改。
Docker 使用已运行的 ferry-my/ferry-pg；端到端测试数据保留在其测试库。

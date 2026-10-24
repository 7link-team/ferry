# T1 — `repositories` 资源

阶段 4 的第一块。目标是把 Git 仓库登记做成一个标准资源，为后续的流水线提供仓库与凭据的来源。

## 范围

只做 `repositories` 这一个资源的服务端部分。**不做**流水线、不做执行引擎、不做界面。

## 必读（先读，再动手）

- `crates/server/src/credentials.rs` — **照着它写**。本任务与它结构几乎一致：Spec、COLUMNS、to_json、check、list/get/create/update/remove。
- `crates/server/src/resource.rs` — 列表过滤/排序/分页的唯一实现。不要自己拼 WHERE。
- `crates/server/src/migration.rs` — 迁移是纯 SQL，**追加**新的一条，绝不修改已有的 `Initial`/`HostResources`/`RunnerEnrolment`/`Credentials`。
- `crates/server/src/workspace.rs` — `Scope` 提取器与 `PERMISSIONS` 清单。
- `docs/ferry-运维平台产品与技术方案.md` 第 6 节（REST 契约）与第 17 节（阶段 4 设计）。
- `CLAUDE.md` — 本仓库的工程约定，必须遵守。

## 要做的事

### 1. 迁移（追加一条，名字 `repositories_v1`）

```sql
CREATE TABLE IF NOT EXISTS repositories (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    default_branch TEXT NOT NULL DEFAULT 'main',
    credential_id TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    version BIGINT NOT NULL DEFAULT 1)
CREATE INDEX IF NOT EXISTS repositories_workspace ON repositories (workspace_id, name)
```

注意两条已经踩过的坑，`migration::schema_tests` 会检查：
- MySQL 不允许 TEXT 列带 `DEFAULT`。只有 `dialect::LONG_COLUMNS` 里的列会变成 LONGTEXT；上面的列都不在其中，所以带 DEFAULT 是安全的。
- 所有整数列用 `BIGINT`，不要用 `INTEGER`（PostgreSQL 的 INTEGER 是 i32，绑定 i64 会类型不符）。

### 2. `crates/server/src/repositories.rs`

标准 CRUD，照 `credentials.rs` 的结构：

- 过滤：`q` 匹配 `name`/`url`；排序白名单 `name`/`created_at`/`updated_at`，默认 `name`。
- 校验：
  - `name` 必填，60 字以内。
  - `url` 必填。只接受 `https://`、`http://`、`ssh://` 开头，或 `git@host:path` 形式。**明确拒绝** `file://` 和本地路径——那会让一次构建读到服务器自己的磁盘。
  - `default_branch` 留空时默认 `main`。
  - `credential_id` 如果非空，必须是本工作空间里真实存在的一条凭据，且 `kind` 是 `password`、`ssh-key` 或 `token` 之一；不存在就报错，不要静默接受。
- 乐观并发：`PATCH` 带 `version`，不一致走 `crate::hosts::stale(...)`（已经是 `pub`）。
- 删除：第一版没有引用它的表，直接删即可；留一句注释说明流水线落地后这里要加引用检查。
- 每个写操作调 `crate::hosts::activity(...)` 落操作记录，详情里**不要**放凭据相关的任何内容。

### 3. 权限

在 `workspace.rs` 的 `PERMISSIONS` 里加四条（`repositories.read/create/update/delete`），分组名「仓库」。检查 `builtin_permissions` 里 `developer` 角色已经包含 `repositories.*`（它是按模块名批量生成的，确认一下即可）。`workspace::tests` 里已有的三个测试会验证依赖闭包，必须继续通过。

### 4. 路由

直接改 `crates/server/src/main.rs`（本任务允许，你是唯一在动这个仓库的人）：

```rust
mod repositories;
.route("/{workspace}/repositories", get(repositories::list).post(repositories::create))
.route("/{workspace}/repositories/{id}",
       get(repositories::get).patch(repositories::update).delete(repositories::remove))
```

放在 credentials 的路由旁边。**改完务必确认 `cargo check --workspace --all-targets` 没有 `never used` 警告**——模块忘了声明或路由忘了接，代码就等于不存在，这个坑在本仓库出现过。

### 5. 测试

**单元测试**（放在 `repositories.rs` 的 `#[cfg(test)]` 里）：URL 校验是纯函数，必须有测试。至少覆盖：接受 `https://`、`git@github.com:a/b.git`、`ssh://`；拒绝 `file:///etc`、`/etc/passwd`、空串、超长串。

**端到端**：在 `scripts/test-backends.sh` 的 `run_suite` 里追加检查（照着里面已有的 `ck` / `ckn` 写）：
- 建一条仓库成功
- `file://` 的 URL 被拒绝
- 引用不存在的 `credential_id` 被拒绝
- 引用真实凭据成功
- 列表能按 `q` 搜到
- 同一个 `version` 连续 PATCH 两次，第二次被拒

## 验收门槛（我会逐条复验，请你自己先全部跑过）

全部按**退出码**判断，不要 grep 输出里的成功字样：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets      # 且无 never-used 警告
cargo test --workspace
npm run typecheck && npm test              # 本任务不该影响前端，确认没弄坏
```

三种数据库的端到端（Docker 里起 MySQL 与 PostgreSQL）：

```sh
cargo build -p ferry-server
FERRY_REQUIRE_ALL=1 \
FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
  ./scripts/test-backends.sh
```

必须三种数据库全部通过。`FERRY_REQUIRE_ALL=1` 的作用是：缺少某个数据库时直接失败，而不是悄悄只测 SQLite。

## 不要做的事

- 不要修改已有的迁移。
- 不要改 `channel.rs`、`remote.rs`、`ssh.rs`、`runner.rs`、`vault.rs`。
- 不要 `git commit` 或 `git push`——我来提交。
- 不要为了让测试通过而放宽断言。

## 交回

写完后报告：改了哪些文件、新增了哪些测试、你实际跑过的命令与**退出码**、以及任何你认为有问题但没改的地方。

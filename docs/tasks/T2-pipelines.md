# T2 — `pipelines` 资源与定义校验

阶段 4 的第二块。流水线的**定义**与**校验**，不含执行。

## 范围

只做 `pipelines` 这一个资源的服务端部分。**不做**执行引擎、不做 runs/steps/logs 表、不做界面。触发执行的 `/actions/run` 留给 T3。

## 必读（先读，再动手）

- `docs/ferry-运维平台产品与技术方案.md` **第 17 节**——这是本任务的设计依据，尤其 17.2 流水线定义。
- `crates/server/src/repositories.rs` — 上一个任务的产物，结构照它写。
- `crates/server/src/resource.rs`、`crates/server/src/channel.rs`（`check_path` 是 `pub`，直接用，不要另写一份路径校验）。
- `crates/server/src/migration.rs`、`workspace.rs`、`CLAUDE.md`。

## 要做的事

### 1. 迁移（追加 `pipelines_v1`）

```sql
CREATE TABLE IF NOT EXISTS pipelines (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    name TEXT NOT NULL,
    repository_id TEXT NOT NULL DEFAULT '',
    definition TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    version BIGINT NOT NULL DEFAULT 1)
CREATE INDEX IF NOT EXISTS pipelines_workspace ON pipelines (workspace_id, name)
```

`definition` 存 JSON 字符串，会超过 255 字符，所以**必须把 `definition` 加进 `dialect::LONG_COLUMNS`**，否则 MySQL 上会被截成 VARCHAR(255)。加进去之后它就不能带 `DEFAULT`——MySQL 不允许 TEXT 列有默认值。`migration::schema_tests` 的两个测试会检查这两点，它们必须继续通过。

### 2. 定义校验 —— 本任务的核心

定义形状见设计文档 17.2。把校验写成**纯函数**（输入 `&str` JSON，输出 `Result<Definition>`），这样它可以被充分测试而不需要数据库。

必须拒绝的（不是全部，自己再想）：

- `steps` 缺失、不是数组、为空数组；步骤数超过 50
- 步骤缺 `name` 或 `run`；`name` 为空或超过 60 字；`run` 为空或超过 8KB
- `timeout_seconds` 不是正整数，或超过 86400（一天）
- `workdir` 不是绝对路径——**用 `channel::check_path`**，别自己写
- `env` 项的 `name` 不符合 `[A-Za-z_][A-Za-z0-9_]*`；重复的 `name`
- 整个 definition 超过 64KB
- **模板变量白名单**：只允许 `{{run}}`、`{{branch}}`、`{{repo_url}}`、`{{commit}}`。出现 `{{其它}}` 一律拒绝，并在错误里指出是哪一个。理由：拼错的变量名如果放过去，会在执行时原样出现在命令里，等到那时才发现就晚了。不要实现通用模板引擎。

错误信息是**给运维人员看的完整中文句子**，并且要**指明是第几步**，例如
`第 2 步「构建」的超时时间必须在 1 到 86400 秒之间。`

### 3. 引用校验

- `repository_id` 非空时，必须是本工作空间真实存在的仓库。
- `target_kind` 只能是 `host` 或 `host-group`；`target_ref` 必须是本工作空间里对应表真实存在的一行。
- 三者都不存在就报错，**不要静默接受**。跨工作空间引用必须被拒绝。

### 4. 资源与路由

标准 CRUD（`list`/`get`/`create`/`update`/`remove`），过滤 `q` 匹配 `name`，另支持按 `repository_id`、`target_kind` 过滤；排序白名单 `name`/`created_at`/`updated_at`。

在 `workspace.rs` 的 `PERMISSIONS` 里确认 `pipelines.*` 四条已存在（应该已经有）。

路由直接改 `main.rs`（你是唯一在动这个仓库的人）：

```rust
mod pipelines;
.route("/{workspace}/pipelines", get(pipelines::list).post(pipelines::create))
.route("/{workspace}/pipelines/{id}",
       get(pipelines::get).patch(pipelines::update).delete(pipelines::remove))
```

改完确认 `cargo check --workspace --all-targets` **零 `never used` 警告**。模块忘了声明等于代码不存在，这个坑在本仓库真实发生过。

同时：`repositories::remove` 现在要拒绝删除仍被流水线引用的仓库，照
`credentials::remove` 里遍历引用表的写法加上。

## 测试要求 —— 这一条比 T1 严格

T1 的校验函数我在验收时补了 14 项注入用例，**这次由你自己写**。

定义校验是纯函数，必须有**对抗性**测试，不只是「正常输入能通过」。至少覆盖：

- 上面列出的每一条拒绝理由各一例
- 模板变量：`{{unknown}}`、`{{ run }}`（带空格）、`{{RUN}}`（大小写）、嵌套 `{{a{{b}}}}`
- `run` 字段里的换行、`;`、反引号、`$()`——**说明你的判断**：这些是该拒绝，还是因为命令本来就要执行 shell 而必须允许？把结论写进注释。想清楚再写，这是设计判断不是填空。
- 超长：步骤数、单步 `run` 长度、整个 definition 大小各一例边界

端到端（`scripts/test-backends.sh` 的 `run_suite` 里追加）：
- 建一条合法流水线成功
- 定义不是合法 JSON → 被拒
- 引用别的工作空间的仓库 → 被拒
- 引用不存在的主机 → 被拒
- `{{typo}}` → 被拒，且错误里出现变量名
- 同一个 `version` 连续 PATCH 两次，第二次被拒
- 仍被流水线引用的仓库不能删除

## 验收门槛

全部按**退出码**判断：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets      # 零 never-used 警告
cargo test --workspace
npm run typecheck && npm test
```

三库端到端（脚本现在会自己构建，构建失败即拒绝运行）：

```sh
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
  ./scripts/test-backends.sh
```

**另外要你自己做一次变异测试**：把定义校验里的模板变量白名单改成「全部放行」，确认你的测试会因此失败。如果不失败，说明那些测试是摆设，要重写。把变异前后的结果写进报告。

## 不要做的事

- 不改已有迁移；不改 `channel.rs`/`remote.rs`/`ssh.rs`/`runner.rs`/`vault.rs`/`credentials.rs` 的业务逻辑（给 `repositories::remove` 加引用检查除外）。
- 不 `git commit` / `git push`。
- 不为了让测试通过而放宽断言。

## 交回

报告：改动文件、新增测试清单、每条命令及其**退出码**、变异测试的前后结果、以及任何你认为有问题但按边界没改的地方。

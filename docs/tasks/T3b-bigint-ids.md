# T3b — 主键与全部 ID 换成 BIGINT 雪花

把 T3a 做出来的生成器真正用上：所有表的主键和所有引用字段从 `TEXT`(UUID) 换成 `BIGINT`(雪花)。

**本任务不改数据访问方式**——仍然是 `db.rs` 的原生 SQL。改成 SeaORM 强类型是 T3c。这样拆是为了出问题时能分清是「换了类型」还是「换了访问方式」引入的。

## 必读

- `docs/ferry-运维平台产品与技术方案.md` **§18**（尤其 18.2 与 18.5 的迁移策略）。
- `crates/server/src/snowflake.rs`（T3a 的产物，`Id` 类型与生成器）、`state.rs`、`db.rs`。
- `CLAUDE.md`。

## 1. 发号入口

`db::id()` 现在返回 UUID 字符串，必须换掉。生成器在 `State` 上（T3a 已接好），所以发号要经过它：

- 提供一个明确的入口（例如 `State::next_id()` 或 `s.ids.next()`），返回 `Result<Id>`——发号**可能失败**（心跳过期时必须拒绝，见 §18.4），调用方要处理，不能 `unwrap`。
- 删掉 `db::id()`，让编译器把每一个调用点都指出来。**不要**保留一个兼容壳，否则漏改的地方会静默继续用 UUID。

## 2. 全部表

迁移策略按 §18.5：项目未发版、没有任何部署，**直接重写已有迁移**，不做兼容迁移。改完之后 `seaql_migrations` 的内容会变，本地测试库直接重建即可。

需要改的不只是主键。**每一个引用字段都是 ID**，漏一个就会出现类型不匹配或静默失配。自己通读迁移文件逐列确认，下面是容易漏的几处：

- `sessions.user_id`、`sessions.workspace_id`
- `workspace_members.workspace_id` / `user_id` / `role_id`
- `workspace_roles.workspace_id`
- `workspaces.owner_id`
- `hosts.workspace_id` / `group_id` / `credential_id`
- `host_groups.workspace_id` / `parent_id`
- `tags.workspace_id`、`host_tags.host_id` / `tag_id`
- `credentials.workspace_id`、`repositories.workspace_id` / `credential_id`
- `pipelines.workspace_id` / `repository_id`
- **`pipelines.target_ref`** —— 多态引用，指向 host 或 host-group，同样是 ID
- `runner_tokens.workspace_id` / `host_id` / `created_by` / `used_by`
- `activity.workspace_id` / `actor_id`、**`activity.resource_id`** —— 多态引用

**不要换的**：
- `worker_ids.instance_id` 和 `locks.owner` 标识的是一个服务进程，不是一行数据，保持原样。
- `workspaces.slug`、`join_code`、`credentials.kind` 等业务字符串。

多态引用字段（`activity.resource_id`、`pipelines.target_ref`）如果存在「空值表示没有引用」的用法，用 `0` 表示而不是空字符串，并在注释里写明。

## 3. 读写两侧

- 读：`db::text(row,"id")` 全部换成 `db::int(row,"id")` 之类。`db::int` 已经能处理三种后端的整数宽度差异。
- 写：绑定 `i64`，不要绑字符串。
- **JSON 输出必须仍然是字符串**：所有 `"id"`、`"*_id"`、`"target_ref"`、`"resource_id"` 字段用 `Id` 类型序列化，不要直接 `json!(i64)`。理由见 §18.2——雪花 ID 大约在 epoch 后 25 天越过 2^53，生产环境发出的每个 ID 都在 JavaScript `Number` 的精确范围之外。
- **JSON 输入必须接受字符串**：前端传回来的 id 是字符串，请求体里的 `*_id` 字段要能从字符串解析（`Id` 的 `Deserialize` 已经同时接受字符串和数字）。
- 路径参数（`/hosts/{id}`）是字符串形式的 ID，解析失败要返回「记录不存在」而不是 500。

## 4. 不变量

- `resource::list` 的列名白名单照旧，不动。
- 三库矩阵必须**从 90/90 继续全绿**。这是「行为未变」的唯一证据；任何一项变红都要查清原因，不要改测试去迁就。
- 前端 `npm run typecheck && npm test` 必须全过。前端的 TypeScript 把 id 声明为 `string`，如果序列化错了会在这里暴露。

## 5. 新增测试

- 建一条记录，读回来，断言返回的 `id` 是**带引号的字符串**而不是数字。放进 `scripts/test-backends.sh`，三库都要验。
- 用字符串形式的 id 作为请求体字段（例如给主机设置 `group_id`）能正确关联。
- 用一个**大于 2^53** 的 id 走完整的「创建 → 列表 → 详情 → 更新 → 删除」链路。可以直接把生成器的时钟拨到 epoch 之后一年来构造这样的 id。
- 路径里给一个非数字的 id（例如 `/hosts/not-a-number`），返回 404 而不是 500。

## 验收门槛

按**退出码**判断：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets      # 零 never-used 警告
cargo test --workspace
npm run typecheck && npm test
```

三库（本地测试库先 drop 重建，因为迁移被重写了）：

```sh
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
  ./scripts/test-backends.sh
```

**自己做一次变异测试**：把某个资源的 `id` 改成以数字形式序列化（去掉 `Id` 包装），确认你新增的「id 是字符串」检查会失败。不失败说明那条检查是摆设。

## 不要做的事

- 不引入 SeaORM 实体、不改数据访问方式（那是 T3c）。
- 不保留 `db::id()` 的兼容壳。
- 不 `git commit` / `git push`。
- 任何测试变红，先查原因，不要改断言迁就。

## 交回

报告：改动文件、逐表确认过的 ID 字段清单（证明没有漏）、每条命令及**退出码**、变异测试前后结果、以及任何你认为有问题但按边界没改的地方。

# T3c — 数据访问改 SeaORM 强类型

把业务代码里的手写 SQL 换成 SeaORM 实体与查询构建器。**迁移暂不动**——迁移改用 schema builder 是 T3d，那样两步各自可以独立验证。

## 必读

- `docs/ferry-运维平台产品与技术方案.md` **§18.5**，这是设计依据。
- `crates/server/src/resource.rs`（列表的唯一实现，含列名白名单）、`db.rs`、`dialect.rs`。
- 任一个现有资源，例如 `credentials.rs` 或 `repositories.rs`，看清现在的写法。
- `CLAUDE.md`。

## 要做的事

### 1. 实体

为每张业务表写 SeaORM 实体。`workspace_id`、时间戳、`version` 这些共有列在每个实体里都有，不要为了"复用"造一个继承式的抽象——SeaORM 的实体就是表的镜像，重复是它的正常形态。

### 2. 数据访问

所有 `db::execute` / `db::one` / `db::all` 的调用点换成类型化查询。

**保留手写 SQL 的唯一理由**是 SeaORM 确实表达不了，并且必须在注释里写清楚是哪一点表达不了。不要因为"改起来麻烦"保留。如果你发现某处 SeaORM 写出来反而更难读，先改写查询本身，再考虑保留。

### 3. `resource::list` —— 这里要小心

它现在用 `Spec` 的白名单拼 `WHERE`。换成 SeaORM 的 `Condition` 之后：

- **白名单必须保留。** 过滤字段名、排序字段名仍然只能来自 `Spec`，不能来自请求。换了 ORM 不等于取消这层边界——SeaORM 的动态列名一样可以是字符串。
- `resource::tests` 里那四个测试（默认值、页大小上限、未知列拒绝、值全部绑定、过滤不逃逸工作空间）必须继续通过。如果某条因为实现方式变了而不再适用，**改写成等价的新测试，不要删**。

### 4. 删掉不再需要的代码

换完之后这些就没有用了，删掉而不是留着：

- `dialect::query`（`?` → `$1` 的占位符改写）——SeaORM 自己处理参数绑定。
- `db::statement` 里的占位符/绑定值计数 `debug_assert`——强类型下那类错误不可能发生。
- `dialect::placeholder_count` 及其测试，如果没有别的使用者。

**不要删** `dialect::schema` 和 `dialect::index_target`：迁移还在用，T3d 才处理。删之前先确认没有调用者（`cargo check --all-targets` 的 `never used` 警告会告诉你）。

删掉之后 `cargo check --workspace --all-targets` 必须**零 `never used` 警告**——有残留说明删漏了或删多了。

### 5. 不变量

- 三库矩阵必须**从 99/99 继续全绿**。这是「行为未变」的唯一证据。
- JSON 里的 ID 仍然是字符串（`Id` 类型）。SeaORM 的实体字段是 `i64`，序列化时不要直接吐数字。
- 乐观并发仍然按 `version` 条件更新并检查影响行数。
- 前端 `npm run typecheck && npm test` 全过。

## 验收门槛

按**退出码**判断：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets      # 零 never-used 警告
cargo test --workspace
npm run typecheck && npm test
```

三库：

```sh
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
  ./scripts/test-backends.sh
```

**自己做两次变异测试**，并在报告里写明前后结果：

1. 把 `resource::list` 的排序字段白名单改成全部放行 → 对应测试必须失败。
2. 去掉某处 `version` 的乐观并发条件 → 三库矩阵里「同一 version 连续 PATCH 两次」的检查必须失败。

任一变异之后测试仍全绿，说明那些测试是摆设，要重写而不是算通过。

## 不要做的事

- 不改迁移（T3d）。
- 不改业务行为：同样的请求必须得到同样的响应，包括错误文案。
- 不 `git commit` / `git push`。
- 任何测试变红，先查原因，不要改断言迁就。

## 交回

报告：改动文件、删掉了哪些代码（以及确认没有调用者的方式）、保留手写 SQL 的地方及其理由、每条命令及**退出码**、两次变异测试前后结果、以及任何你认为有问题但按边界没改的地方。

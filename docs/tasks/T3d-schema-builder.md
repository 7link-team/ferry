# T3d — 迁移改用 SeaORM schema builder，删掉 dialect

数据层重做的最后一块。迁移从手写 DDL 字符串换成 SeaORM 的 `Table::create()` / `Index::create()`，随后删掉 `dialect.rs`。

## 必读

- `crates/server/src/migration.rs`（现状）、`dialect.rs`（即将删除的那层做了什么）。
- `docs/ferry-运维平台产品与技术方案.md` §3.1 与 §18.5。
- `CLAUDE.md`。

## 背景：dialect 到底在替我们挡什么

现在的迁移是 SQLite 风味的 DDL 字符串，`dialect::schema` 把它改写给 MySQL：

- `TEXT` 列在 MySQL 上不能直接建索引 → 按列名分配 `VARCHAR(64)`（id/`*_id`）或 `VARCHAR(255)`，JSON 类列给 `LONGTEXT`。
- MySQL **不允许 TEXT/LONGTEXT 列带 `DEFAULT`**（错误 1101）。
- MySQL 没有 `CREATE INDEX IF NOT EXISTS`，所以 `dialect::index_target` 把索引名抠出来先查存在性。
- 建表要追加 `utf8mb4_bin`，否则大小写不敏感，`Admin` 和 `admin` 会在唯一索引里撞车。

换成 schema builder 之后，**这些规则不会消失，只是换了表达方式**：由列类型的选择来承担。所以不要以为换了 API 就不用管了——选错类型，MySQL 上照样起不来。

## 要做的事

### 1. 迁移改写

每张表用 `Table::create()`，每个索引用 `Index::create()`。类型对应关系：

| 现在 | schema builder |
|---|---|
| `BIGINT`（id、`*_id`、时间戳、version） | `ColumnType::BigInteger` |
| 被 dialect 改成 `VARCHAR(64)/(255)` 的 TEXT | `ColumnType::String(StringLen::N(...))` |
| 被 dialect 改成 `LONGTEXT` 的（`permissions`、`details`、`secret`、`definition`） | `ColumnType::Text`，并且**不能带 default** |

建表时为 MySQL 设置 `utf8mb4_bin`（schema builder 支持 per-backend 选项；若实在设不了，在注释里说明并保留等价手段）。

`CREATE INDEX IF NOT EXISTS` 的问题由 `SchemaManager::has_index` 承担——现有的 `execute_schema` 已经在做，保留这个思路，只是输入从字符串换成构建器对象。迁移必须可重跑：中途失败后再跑一次应当是无操作，MySQL 的 DDL 各自提交，这一点不能丢。

### 2. 删掉 dialect

改完后 `dialect::schema`、`dialect::index_target` 应该都没有调用者了，连同 `dialect.rs` 整个删除。删之前用 `cargo check --workspace --all-targets` 的 `never used` 警告确认，删之后必须**零警告**。

### 3. 保住那两条保护

`migration::schema_tests` 现在对**真实的迁移语句**断言两件事：没有 LONGTEXT 带 DEFAULT、没有裸 TEXT 漏进 MySQL。这两条都是真实踩过的坑（`metrics` 和 `credentials.secret` 各踩过一次）。

换成 schema builder 之后，**不要删掉这层保护**。改写成等价的：用 SeaORM 为 MySQL 后端把迁移里的建表语句构建出来，对生成的 SQL 断言同样两件事。这样不需要数据库也能在 `cargo test` 里挡住。

如果确实无法从迁移定义构建出 SQL 来检查，说明理由，并解释三库矩阵为什么足以覆盖。

### 4. 不变量

- 三库矩阵必须**从 99/99 继续全绿**，并且是在**全新建库**上跑的——迁移被重写了，旧库的 `seaql_migrations` 不作数。先 DROP 再 CREATE。
- 业务行为、JSON 形状、错误文案一律不变。

## 验收门槛

按**退出码**判断：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets      # 零 never-used 警告
cargo test --workspace
npm run typecheck && npm test
```

三库（务必先 drop 重建）：

```sh
docker exec ferry-my mysql -uroot -pdevpass -e "DROP DATABASE IF EXISTS ferry; CREATE DATABASE ferry"
docker exec ferry-pg psql -U postgres -c "DROP DATABASE IF EXISTS ferry"
docker exec ferry-pg psql -U postgres -c "CREATE DATABASE ferry"
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
  ./scripts/test-backends.sh
```

**额外必做：迁移可重跑验证。** 在 MySQL 和 PostgreSQL 上各做一次：删掉 `seaql_migrations` 里最后一条记录，重启服务，确认迁移重新应用时是无操作而不是报错。把命令和结果写进报告。

**变异测试一次**：把某个 JSON 类列（`permissions` / `details` / `secret` / `definition` 之一）改成带 `default` 的 `Text`，确认你改写后的 schema 测试会失败。不失败说明那层保护没保住。

## 不要做的事

- 不改业务逻辑、不改 JSON 形状。
- 不 `git commit` / `git push`。
- 不为了让测试通过而放宽断言；任何变红先查原因。

## 交回

报告：改动文件、删掉了什么、那两条保护改写成了什么形式、每条命令及**退出码**、迁移可重跑验证的实际输出、变异测试前后结果、以及任何按边界没改的问题。

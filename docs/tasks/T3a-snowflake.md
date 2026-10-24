# T3a — 雪花 ID、分布式锁、worker id 分配

数据层重做的第一块：把机制做出来并测透。**本任务不改任何现有表的主键**——那是 T3b。做完之后现有功能必须完全不受影响，三库矩阵照常全绿。

## 必读

- `docs/ferry-运维平台产品与技术方案.md` **第 18 节**——这是设计依据，尤其 18.2、18.3、18.4。实现与它不一致的地方，先说明理由。
- `crates/server/src/db.rs`、`migration.rs`、`state.rs`、`error.rs`、`CLAUDE.md`。

## 1. `crates/server/src/snowflake.rs`

### `Id` 类型

```rust
pub struct Id(i64);
```

- **`Serialize` 输出字符串**，`Deserialize` 同时接受字符串和数字。
  这是本任务最关键的一条：雪花 ID 是 64 位，JavaScript 的 `Number` 只能精确表示到 2^53，直接序列化成 JSON 数字会被**静默**舍入——前端拿到的 id 和数据库里的不是同一个，而且不报错。必须有测试：取一个大于 2^53 的真实雪花值，序列化后是带引号的字符串，且往返回来数值不变。
- `Display`、`FromStr`、`From<i64>`、`Into<i64>`，方便和数据库交互。

### 生成器

按 18.2：1 位符号 + 41 位毫秒（epoch `2026-01-01T00:00:00Z`）+ 10 位 worker + 12 位序列。

- 同一毫秒内序列号用满 4096 个，自旋等到下一毫秒，**不向后借用**。
- **时钟回拨**：当前毫秒小于上次发号毫秒时等待追上；回拨超过 5 秒则返回错误，不再发号。继续发号会产生重复 ID，比停机严重得多。
- 用 `Mutex<State>`，不要无锁结构。每秒几千次调用锁根本不是瓶颈，而无锁实现的正确性难验证得多。

### 测试（必须覆盖）

- 单调递增；连续生成 10 万个无重复。
- **多线程**并发生成（至少 8 线程 × 1 万个）无重复——这是生成器唯一真正的风险点。
- 解析回时间戳/worker/序列，与输入一致。
- 时钟回拨：注入一个可控时钟，回拨 1 秒 → 不产生重复；回拨 10 秒 → 返回错误。**时钟必须可注入**，否则这条测不了。
- `Id` 的 JSON 往返：大于 2^53 的值序列化成字符串、解析回来不变。

## 2. `crates/server/src/lock.rs` —— 基于数据库的分布式锁

迁移追加 `locks` 表（见 18.3）。

获取锁**不要用 upsert 语法**：SQLite/PostgreSQL 是 `ON CONFLICT`，MySQL 是 `ON DUPLICATE KEY`，三库不通用。按 18.3 的两步法：先 `UPDATE ... WHERE name=? AND (owner=? OR expires_at<?)` 看影响行数，为 0 再 `INSERT`，主键冲突即未获得。

- 释放：`DELETE WHERE name=? AND owner=?`
- 续期：`UPDATE ... WHERE name=? AND owner=?`
- **两者都必须带 owner 条件**，否则会删掉或续期别人的锁。
- 锁必须有 TTL 且自动过期，否则崩溃的实例会把锁永久锁死。
- 提供一个 RAII 守卫或显式 `release`，并保证出错路径也会释放。

### 测试

需要真实数据库（SQLite 文件库即可，别用 `sqlite::memory:` 共享连接——本仓库已经因为它出过假通过）：

- 两个并发获取者只有一个成功。
- 过期后可被另一个 owner 抢占。
- 非持有者 `release` / `renew` 都不生效（影响行数 0），且不影响真正的持有者。
- 持有者释放后，别人能立刻获得。

## 3. worker id 动态分配

迁移追加 `worker_ids` 表（见 18.4）。

启动时：取 `locks` 里名为 `worker-id` 的锁 → 优先复用本 `instance_id` 上次的槽位（重启后 ID 不跳变）→ 否则取最小空闲槽位（无行，或心跳超过 TTL）→ 写入并释放锁。之后按固定间隔心跳。

### 必须守住的安全性质

> 一旦本实例距上次**成功**心跳超过 TTL，就必须立即停止发号。

因为此时别的实例可能已经回收了这个槽位，继续发号会产生重复 ID。实现上：发号前检查距上次成功心跳的时长，超限返回错误，让请求失败而不是发出可能重复的 ID；心跳恢复后自动恢复发号。

**这条必须有测试**，而且是本任务最重要的一个测试——正常路径永远测不到它。做法：把心跳时间设成过去很久，断言发号返回错误；再更新心跳，断言恢复正常。

TTL 取心跳间隔的若干倍（例如心跳 10 秒、TTL 60 秒），留足网络抖动和 GC 停顿余量。1024 个槽位用满时要有明确的错误信息，不要 panic。

### 测试

- 两个不同 `instance_id` 拿到不同槽位。
- 同一 `instance_id` 重启后复用原槽位。
- 心跳过期的槽位可被回收。
- **心跳过期后发号失败**（见上）。
- 槽位耗尽时报错而不是 panic。

## 4. 接进服务

- `State` 上挂生成器；启动时分配 worker id，失败则**拒绝启动**并说明原因（没有 worker id 就无法安全发号）。
- 心跳作为后台任务，与现有的优雅关闭配合：关闭时释放槽位。
- **本任务不改任何现有表的主键，不改任何现有的 `db::id()` 调用点。** 现有功能必须完全不受影响。

## 验收门槛

按**退出码**判断：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets      # 零 never-used 警告
cargo test --workspace
npm run typecheck && npm test
```

三库矩阵必须**照常全绿**（本任务不该改变任何现有行为）：

```sh
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
  ./scripts/test-backends.sh
```

**自己做两次变异测试**并在报告里写明前后结果：

1. 去掉「心跳过期停止发号」的检查 → 对应测试必须失败。
2. 把锁的 `release` 去掉 owner 条件 → 对应测试必须失败。

如果某个变异之后测试依然全绿，说明那些测试是摆设，要重写而不是算通过。

## 不要做的事

- 不改现有表的主键，不改 `db::id()` 的调用点。
- 不改 `channel.rs`/`remote.rs`/`ssh.rs`/`runner.rs`/`vault.rs`/`pipelines.rs`/`repositories.rs` 的业务逻辑。
- 不 `git commit` / `git push`。
- 不用 `sqlite::memory:` 做多连接测试。

## 交回

报告：改动文件、测试清单、每条命令及**退出码**、两次变异测试的前后结果、以及任何与设计文档 §18 不一致之处及其理由。

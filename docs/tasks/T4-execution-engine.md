# T4 — 流水线执行引擎

阶段 4 的最后一块服务端工作：让流水线真正跑起来。

（编号说明：阶段 4 原计划的 T3 与数据层重做的 T3a–T3d 冲突，这里改称 T4；界面是 T5。）

## 必读

- `docs/ferry-运维平台产品与技术方案.md` **§17.1、17.3、17.4、17.5**——17.5 是验收标准，先读它再动手。
- `crates/server/src/channel.rs`（`HostChannel`，执行的唯一入口）、`remote.rs`（它的现有用法）。
- `crates/server/src/pipelines.rs`（定义与校验，T2 的产物）、`credentials.rs`（`unseal`）。
- `crates/server/src/runner.rs` 里的调度思路（单进程 + 数据库乐观锁）。
- `CLAUDE.md`。

## 1. 三张表

按 §17.1 追加 `pipeline_runs`、`pipeline_run_steps`、`pipeline_run_logs`。

- `pipeline_runs.number` 是**工作空间内**的自增序号（给人看的「#12」），不是全局的。并发触发两次不能拿到同一个号——用一条带条件的更新或等价手段保证，不要先查后写。
- 日志按 `seq` 分片存储。`seq` 在一次 run 内单调递增，前端按 `after=seq` 增量拉取。不要把日志塞进 run 的一个大字段：一次构建几十 MB，整列更新会让每次追加都重写整列。

## 2. 执行

**只经过 `HostChannel`**，不要直接 ssh、不要直接发 runner 协议。这样 runner 主机和 SSH 主机跑同一套逻辑。

- 逐步执行，**失败即停**：某步非零退出，后续步骤标记 `skipped`，run 标记 `failed`。不自动重试。
- 每步有超时（定义里的 `timeout_seconds`，无则给一个合理默认）。**超时要和普通失败区分开**，`pipeline_run_steps` 要能看出是哪一种——两者的排查方向不同。
- 模板变量替换：只认 `{{run}}`、`{{branch}}`、`{{repo_url}}`、`{{commit}}`。T2 已经保证定义里不会出现别的，这里直接替换即可。
- `workdir` 和 `env` 按定义应用。

### 凭据注入 —— 这里不能图省事

仓库凭据在执行时由服务端解密，但**绝不能拼进命令行**：命令行会出现在目标主机的 `ps` 输出里，任何能登录那台机器的人都看得到。

用 `GIT_ASKPASS` 指向一个临时脚本，或写一个临时的 credential 文件（权限 0600，用完即删）。无论哪种，都要保证：

- 明文不出现在任何一条命令的 argv 里
- 明文不出现在日志里（包括失败时的 stderr）
- 明文不出现在 `activity` 的详情里
- 任务结束（含失败和取消）后临时文件被删除

## 3. 接口

按 §17.4：

```
POST /api/workspaces/{ws}/pipelines/{id}/actions/run      触发
GET  /api/workspaces/{ws}/pipeline-runs                   历史（标准列表）
GET  /api/workspaces/{ws}/pipeline-runs/{id}              详情 + 各步状态
GET  /api/workspaces/{ws}/pipeline-runs/{id}/logs?after=  增量日志
POST /api/workspaces/{ws}/pipeline-runs/{id}/actions/cancel
```

`logs` 返回 `{items:[{seq,step_index,stream,chunk}], next_seq}`。

权限：触发要 `pipelines.run`；查看要 `pipelines.read`；取消至少要 `pipelines.run`。

## 4. 调度与取消

单进程调度器 + 数据库乐观锁取任务，和 `runner.rs` 里定时任务的思路一致。一个服务实例够用；多实例留到以后换成带租约的抢占。

取消：标记 run 为 `cancelled` 并中止正在等待的步骤。**诚实地说明局限**——`HostChannel::exec` 是捕获完整输出后返回的，取消能让服务端停止等待并标记状态，但远端命令可能仍在跑完。把这一点写进代码注释和 §17 的文档里，不要让界面暗示「取消 = 远端立刻停止」。

## 5. 验收标准

§17.5 列了 8 条，**必须用真实的 Git 仓库和真实的主机跑通**，不接受只有单元测试。

测试用的 Git 仓库不要依赖外网：在本地建一个 bare 仓库并用 HTTP 暴露（例如 `git daemon`，或 `git http-backend`，或任何能让 `git clone http://127.0.0.1:PORT/x.git` 成功的方式）。`file://` 和本地路径会被 T1 的 URL 校验拒绝，这是有意的。

主机两种都要测：runner 接入和 SSH 接入，同一条流水线结果必须一致。`scripts/test-runner.sh` 和 `scripts/test-ssh.sh` 里已经有可复用的起法。

逐条对应 §17.5：

1. 登记仓库 + 用凭据拉取成功
2. 两步流水线在 **runner 主机**跑通，日志完整、顺序正确
3. 同一条在 **SSH 主机**跑通，结果一致
4. 第一步失败 → 第二步不执行，run 为 failed，失败步的退出码正确
5. 超时的步骤标记为超时，**不是**普通失败
6. 取消运行中的 run → 标记 cancelled
7. **凭据明文不出现在**日志、目标主机 `ps` 输出、`activity` 详情里
8. 跨工作空间：A 的流水线不能引用 B 的仓库或主机

把这些做成可重跑的脚本放进 `scripts/`，照 `test-runner.sh` 的写法（带最少检查数下限，0 个检查跑过不算通过）。

## 验收门槛

按**退出码**判断：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets      # 零 never-used 警告
cargo test --workspace
npm run typecheck && npm test
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
  ./scripts/test-backends.sh      # 必须从 99/99 继续全绿
```

**变异测试两次**：

1. 去掉「失败即停」——让失败后继续执行下一步 → 对应检查必须失败。
2. 把凭据改成拼进命令行 → 第 7 条检查必须失败。如果不失败，说明那条检查没有真的去看 `ps` 或日志。

## 不要做的事

- 不做界面（T5）。
- 不加 `HostChannel::exec` 的 HTTP 路由——那是另一个待定的产品决定。
- 不 `git commit` / `git push`。
- 任何测试变红先查原因，不要改断言迁就。

## 交回

报告：改动文件、三张表的结构、§17.5 八条逐条的验证方式与实际输出、每条命令及**退出码**、两次变异前后结果、以及任何按边界没改的问题。

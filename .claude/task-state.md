# Task State
## Task / Objective
T4：docs/tasks/T4-execution-engine.md；已先读任务书、设计 §17 与 CLAUDE.md。项目仓库未 commit/push/暂存。用户原未跟踪任务书未改。真实 Git 夹具只在临时仓库创建提交。
## Current Phase / Result
ACCEPTANCE — PASS
## Execution / Delegation
Direct，无子代理。范围/复用/架构/复杂度/回归/凭据安全/真实验收/状态一致性两轮复核通过。
## Implemented / Files
- 新增 crates/server/src/pipeline_runs.rs：5 个 API、标准历史列表、单进程 CAS 调度、线性执行、失败即停、超时 timed_out、取消、日志脱敏/分片、模板/工作目录/env。
- 修改 entities.rs、migration.rs：3 张表及索引；workspaces.pipeline_run_number 事务内 UPDATE + 读自己的值，唯一索引兜底；迁移保护扩展至新增 ID。
- 修改 main.rs：路由、调度生命周期；pipelines.rs：复用校验后定义类型；workspace.rs：计数器初始化。
- 新增 scripts/test-pipelines.sh、test-pipelines.py、pipeline-git-http.py、test-pipeline-mutations.py：真实 Basic Auth HTTP bare Git、runner、Docker SSH、最低40检查、两次源码变异及 finally 恢复。
- 更新设计 §17、.claude/decisions.md 与本状态文件。
## Schema
- pipeline_runs：id/workspace_id/pipeline_id/host_id/credential_id/triggered_by（BIGINT），number，state，trigger_kind，definition/repo_url/branch/commit 快照，created_at/started_at/finished_at。UNIQUE(workspace_id,number)，INDEX(state,created_at)。
- pipeline_run_steps：id/run_id，step_index，name，state，nullable exit_code，started_at/finished_at；UNIQUE(run_id,step_index)。
- pipeline_run_logs：id/run_id，seq，step_index，stream，chunk；UNIQUE(run_id,seq)。脱敏后 UTF-8 边界分片，每片≤16KiB，增量接口每页≤200片。
## Decisions / Boundaries
所有执行和文件传输经过 HostChannel。0700 随机临时目录，0600 密码/用户名/私钥/步骤文件，0700 askpass；凭据不进入 exec argv。各正常结束路径含失败/超时/取消清理目录。activity 仅记录 ID/序号。
取消停止服务端等待，远端可能继续；cancelled 状态可能先于清理完成。服务重启不重试遗留 running。复用现有 HostChannel 单步完整捕获/输出上限，未扩展为流式或修改通道上限。
主机组偏好未获补充后已告知采用最小单目标假设：选直接在线成员中 ID 最小者，固定 host_id；无在线成员拒绝。可选请求 branch/commit 是模板上下文；缺省仓库分支/空 commit，不自动 checkout。
未做 UI、未加 exec HTTP、未扩展多实例租约或多机编排。前端原有 chunk>500kB 提示未改。
## Final Gates — executed exit codes
| Command | Exit / actual result | Log |
|---|---|---|
| cargo fmt --all -- --check | 0 | /tmp/t4-fmt-final.log |
| cargo check --workspace --all-targets | 0，零 warning/never-used（额外断言日志无 warning:） | /tmp/t4-check-final.log |
| cargo test --workspace | 0；protocol3 + runner7 + server76 = 86通过 | /tmp/t4-tests-final.log |
| npm run typecheck && npm test | 0；11/11 | /tmp/t4-npm-tests.log（typecheck输出在工具结果） |
| npm run build | 0；已有大chunk提示 | /tmp/t4-npm-build.log |
| 三库原矩阵，完整命令见下 | 0；SQLite99/99、MySQL99/99、Postgres99/99 | /tmp/t4-matrix.log |
| ./scripts/test-pipelines.sh | 0；44 passed, 0 failed, 0 skipped | /tmp/t4-live-final.log |
| FERRY_MASTER_KEY=$(openssl rand -hex 32) FERRY_TEST_DATABASE_URL='mysql://root:devpass@127.0.0.1:13306/ferry_t4_live' ./scripts/test-pipelines.sh | 0；44/44 | /tmp/t4-live-mysql.log |
| FERRY_MASTER_KEY=$(openssl rand -hex 32) FERRY_TEST_DATABASE_URL='postgres://postgres:devpass@127.0.0.1:15432/ferry_t4_live' ./scripts/test-pipelines.sh | 0；44/44 | /tmp/t4-live-postgres.log |
| python3 -I scripts/test-pipeline-mutations.py | 0；内部退出码与指定失败证据均断言，见下 | /tmp/t4-mutations.log |
| git diff --check；脚本语法检查；变异恢复/零警告检查 | 0 | 工具结果 |

三库命令原样执行，未修改 scripts/test-backends.sh，原 ferry 测试库无 users 表所以无需清库；新引擎额外验证使用独立新建 ferry_t4_live 数据库：
```sh
FERRY_REQUIRE_ALL=1 FERRY_MASTER_KEY=$(openssl rand -hex 32) \
FERRY_TEST_MYSQL='mysql://root:devpass@127.0.0.1:13306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:devpass@127.0.0.1:15432/ferry' \
./scripts/test-backends.sh
```
实际输出：
```
SQLite:    99 passed, 0 failed
MySQL:     99 passed, 0 failed
Postgres:  99 passed, 0 failed
All backends passed.
```
## §17.5 — Actual live evidence (same checks on all 3 databases)
1. 本地生成 bare Git，经容器 HTTP git-http-backend 提供 Basic Auth：`ok 1 unauthenticated private HTTP Git denied: HTTP 401`；两类主机均 `authenticated clone contains actual Git commit`。真实 SHA 通过 {{commit}} 与 git rev-parse HEAD 比对。
2. runner：`state=succeeded, steps=['succeeded', 'succeeded']`；stdout/stderr 标记完整，`seq=[1, 2, 3, 4]`；workdir/env 实测正确。
3. SSH：同样 state/steps/seq；`ok 3 runner and SSH Git/build results match`。
4. 两主机：`state=failed, steps=['failed', 'skipped'], exit=7`；检查后续日志无 SHOULD_NOT_RUN、文件副作用不存在。
5. 两主机：`state=failed, step=timed_out, exit=None`；后续 skipped。
6. 等待真实步骤 running 后取消：两主机均 `state=cancelled, steps=['cancelled', 'skipped']`；后续 barrier run 同步清理完成。
7. 从凭据文件主动输出密码至 stdout/stderr 再 exit7，检查脱敏和0600；各结束路径检查临时目录删除；目标 ps 连续采样并确认捕获了 /step argv。SQLite最终两主机各32份、第二次变异各69份。日志/activity/server.log 均不含测试用户名或密码。
8. A引用B仓库/主机均 `HTTP 400`；B读取A run/日志/取消均 `HTTP 404`。
额外：并发序号 `[12,13,14,15]`，另一工作空间 `number=1`；日志 `chunks=220` 无UTF-8丢失，实际跨增量分页；只读用户查看200、触发/取消403；主机组选定runner、空组400。
## Mutations — executed before final group/context additions
Wrapper logs in /var/folders/36/x9500b6s6834kny2k4j8t03c0000gn/T/pipeline-mutations-37d_su7z/.
- baseline：exit0，42 passed / 0 failed。
- continue-after-failure：将 `if state != "succeeded"` 改为仅取消才停止；exit1，36 passed / 6 failed。两主机：`FAIL 4 ... state=succeeded, steps=['failed','succeeded'], exit=7`；检查到 forbidden 文件实际出现。超时后继续也被抓住。
- restored-after-failure：exit0，42/42。
- credential-in-argv：临时将密码放入远端 shell argv，日志脱敏不变；exit1，40 passed / 2 failed。仅 `FAIL 7 runner target ps contains no credential plaintext` 与同名 SSH 检查失败（各69份真实ps采样）。
- final-restored：exit0，42/42；finally逐字恢复源码并断言。后续主机组/模板上下文增加后，最终3库真实验收各44/44。最终静态核对 fail-stop guard 和仅路径 command 都已恢复，产品源码无 LEAKED_PASSWORD。
## Failures resolved honestly
启动夹具 http.py 遮蔽标准库 → 重命名；用户/空间创建实际200 → 修正夹具期望；Alpine git-http-backend 需要git-daemon包 → 补齐夹具依赖；首轮36项绿但最低40失败 → 添加真实权限检查而非降低下限。
初次 cargo test exit101：新增 triggered_by 未纳入 ID 测试识别；LONGTEXT 最后一列后的表级 DEFAULT CHARSET 被误判为列默认值。修正清单和检查范围，禁止长文本列默认值的断言保留，最终86测试/三库通过。
## Cleanup / Review
失败夹具含测试密码的临时目录已删除，容器/runner/server由脚本回收，保留无明文凭据的验收日志。生成的 scripts/__pycache__ 已删；未改baseline矩阵、未暂存；工作区保留实现与文档。
Self-review与原始用户请求Final Acceptance：PASS。没有剩余实现或验收。
## Next Action
向用户交付中文报告（8条、表结构、文件、命令退出码、变异和边界），保持未提交改动。无需用户操作。

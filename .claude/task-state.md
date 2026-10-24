# Task State
## Task / Objective
T6 docs/tasks/T6-ci-release.md：四个真实 harness 接 CI，连续三轮真实结果、前端分包、发布链路可验证。
## Current Phase / Result
BLOCKED — 本地实现与验证完成；最终验收 NOT READY，等待用户提交推送后执行新 workflow。
## Execution / Delegation
Direct，无子代理。修改集中在 workflow/harness，直接集成。
## Completed / Files
- .github/workflows/build.yml：ubuntu-24.04 四个 e2e matrix job（ssh/runner/pipelines/build-ui），各连续3轮，首个失败退出、不是重试。Summary记录轮次/run id/SHA，artifact保存日志。rust-cache按harness key；Playwright浏览器缓存，显式安装系统依赖。checks增加--all-targets。
- release needs新增e2e；dispatch复用下载dist-*、校验和及dry-run artifact，只有push v* tag调用发布action（dispatch即使选择tag也不发布）。desktop/runner资产生成及server-image workflow保持不变。
- scripts/test-runner.sh：强制依赖，所有检查必跑，最少35项；复用pipeline API验证stdout/stderr/exit7，替代不存在exec HTTP route的跳过。
- scripts/test-build-ui.mjs：临时裸仓库fixture branch兼容detached checkout，至少4组验收；文件屏障修复实际复现的轮询竞态；追加xterm首屏不加载/终端按需加载并连接真runner的回归检查。
- scripts/test-ssh.sh：修复零匹配失败提示反引号执行mod命令；保留11+1检查下限及FERRY_REQUIRE_LIVE=1。
- package.json/package-lock.json：锁定playwright1.64.1。
- vite.config.ts、src/App.tsx：manualChunks分离xterm/react/radix，lazy加载终端；未提高500kB阈值。
- .claude/decisions.md：执行策略、边界、实际flaky根因记录。
## Verification — actual results
- actionlint .github/workflows/*.yml：PASS。
- bash -n三个shell harness / node --check UI / git diff --check：PASS。变更源码LF。
- npm run build：PASS，无chunk警告；最终重复build字节大小一致。
- npm run typecheck && npm test：PASS，4文件23项。
- cargo fmt --all -- --check && cargo check --workspace --all-targets && cargo test --workspace：PASS，3+7+76=86项。
- cargo build --locked -p ferry-server -p ferry-runner：PASS。
- SSH零匹配负例：复用原live_tests函数、cargo替身输出running 0 tests，实际exit1，期望11/实际0。
- detached checkout→裸仓库fixture branch→clone：HEAD一致，PASS，没有创建commit。

### 本地真实harness连续三轮（macOS，不能当作ubuntu CI结果）
日志目录：/tmp/ferry-t6-local/；每轮均exit0。
| harness | 第1轮 | 第2轮 | 第3轮 | 日志 |
| --- | --- | --- | --- | --- |
| ssh | 11+1 PASS | 11+1 PASS | 11+1 PASS | ssh-{1,2,3}.log |
| runner | 35 PASS | 35 PASS | 35 PASS | runner-{1,2,3}.log |
| pipelines | 44 PASS | 44 PASS | 44 PASS | pipelines-{1,2,3}.log |
| build-ui修复后 | 4组 PASS | 4组 PASS | 4组 PASS | build-ui-fixed-{1,2,3}.log |
runner包含120MB往返/重连/exec三项，无skip；pipelines无skip。UI日志每轮2→3→4，终态请求分别10/8/8，后续3.5秒不增长；真实终端加载连接通过。

### Actual flaky / resolution
初次UI序列第1轮PASS、第2轮FAIL（build-ui-2.log）：CLONE_OK已显示，步骤1成功、2/3排队，立即断言列表含运行中失败。详情/日志独立轮询使固定sleep不能保证观察时刻。
修复为fixture文件屏障：build/finish等待浏览器确认前一步日志并放行；保留原运行态/日志递增/三步成功/停止轮询断言，无重试；修复后重启连续三轮，全部PASS。

### Bundle sizes (kB, Vite minified / gzip)
Before：主JS804.42/235.21，CSS66.73/11.76，有500kB警告。
After：xterm330.51/83.42，react222.28/69.21，业务136.25/43.22，radix112.22/37.35，HostTerminal2.30/1.28；CSS63.11+3.62，无超限。dist HTML只preload react/radix。
构建日志：/tmp/ferry-t6-build-before.log、/tmp/ferry-t6-build-after.log。

## Confirmed Constraints / Boundaries
没有commit、暂存、push、tag、release、可见性变更；未改原未跟踪docs/tasks/T6-ci-release.md。测试沿用pipeline harness在临时fixture仓库的提交，不在项目仓库创建提交。
不放宽断言/不降检查数/不重试掩盖失败。未改产品执行引擎、轮询或server-image发布策略。
## Remote verification / blocker
远端build.yml blob=8d1649578c8231efb09f49c9605e0eccf2da372a，与原本地HEAD一致，尚无新job。没有触发旧workflow来冒充本次验证。
新job CI三轮：NOT_RUN，run id不存在。dispatch实际产物下载/校验和：NOT_RUN。
本地未安装act，且不能替代四平台实际安装包验证。已完成actionlint/needs/触发条件/产物链路静态检查。
server-image保持独立工作流；未来推tag前应确认对应SHA双架构验证镜像已构建，避免版本镜像等待超时。
## Self-Review / Final Acceptance
范围、复用、架构、复杂度、本地回归、持久状态、提交卫生：PASS。
最终验收：NOT READY（CI三轮和实际发布产物收集尚未执行），不能宣称任务完成或已经证明CI稳定。
## Remaining / Next Action
等待用户自行提交推送上述改动，并提供分支/SHA。随后由助手执行gh workflow run build.yml --ref <分支>，获取并核对run id/head SHA，验证四个job的3轮Summary与日志、全部前置job及release dry-run artifact/校验和。
若失败调查根因并本地修复，再等待用户推送；不自行commit/push/tag/release。最终中文报告必须补实际CI run id与每轮结果。

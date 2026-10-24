# Task State
## Task / Objective
T5 docs/tasks/T5-build-ui.md：仓库/流水线配置、历史、执行详情与增量日志。已先读任务书及全部必读文件。
## Current Phase / Result
ACCEPTANCE — PASS
## Execution / Delegation
Direct，无子代理。使用 ui-ux-pro-max 检查交互/生命周期/稳定日志渲染，沿用既有样式。
## Completed / Files
- src/resources.tsx：repositories/pipelines ListConfig+FormConfig、pipeline-runs ListConfig、pipelines.run触发权限、菜单派生详情导航；当前工作空间下拉分页加载。
- src/components/dsl.ts、ResourceForm.tsx：最小扩展code等宽多行和声明式resetOnChange，保持既有字段行为。ResourceTable未改。
- src/components/PipelineRunDetail.tsx：概要、步骤、区别超时、诚实取消及pipelines.run权限、增量日志排空后停止、隐藏暂停/恢复、卸载abort、稳定seq+memo、底部跟随。
- src/pipeline.ts：合同类型、日志合并/耗时/徽标/运行态纯函数。
- src/App.tsx、src/nav.ts：view路由和执行历史导航。
- src/pipeline.test.ts、src/components/PipelineRunDetail.test.tsx：新增12项，原11项全部保留。
- scripts/test-build-ui.mjs：真实浏览器共享表单+真实服务/runner/HTTP Git验收；用现有历史clone裸仓库，不创建任何commit；子进程与临时数据finally清理。
- scripts/test-build-ui-mutation.py：baseline→终态继续轮询变异→finally逐字恢复→restored。
## Verification — actual exit codes
- npm run typecheck：0（单独一次，之后与build串行一次也通过）。
- npm test：0；4文件23项（原11+新增12）。
- npm run build：0；Vite 804.42KB主chunk大小警告，不影响退出码。
- cargo fmt --all -- --check && cargo check --workspace --all-targets && cargo test --workspace：0；86测试（3 protocol+7 runner+76 server）。
- cargo build -p ferry-server -p ferry-runner：0，用于真实UI嵌入最新dist。
- PLAYWRIGHT_MODULE=/Users/apple/.npm/_npx/e41f203b7505f1fb/node_modules/playwright/index.mjs UI_SCREENSHOT=/tmp/t5-build-ui.png node scripts/test-build-ui.mjs：0。
- python3 scripts/test-build-ui-mutation.py：0；baseline0（9/9），mutant1（3 failed/6 passed），restored0（9/9）；断言请求数期望5实际21，三种终态均捕获；源码原字节恢复。
- git diff --check：0；服务端/runner/protocol/依赖/ResourceTable diff为空。已跟踪修改文件均LF。
## Real observations
Chromium 1280×900：共享表单创建仓库/流水线；host↔host-group联动清空；timeout=0完整服务端错误句子；有效定义创建成功。
在流水线行操作触发并进入/ops/build/runs/{id}；真实HTTP Git clone到真实runner主机；克隆/构建/完成三步依次推进，最终全部成功、exit0。
日志分片2→3→4，含CLONE_OK/BUILD_OK/FINISH_OK；after游标增长；最终26个详情/日志请求，后续3.5秒保持26；历史显示#1且无触发按钮；无浏览器异常。已读取截图确认布局。
## Failed attempts / Resolved
首次浏览器脚本exit1：选择了缓存中的alpha Playwright，缺匹配浏览器；改用已有1.64.1与匹配Chromium。
第二次exit1：测试夹具git:// URL被现有API拒绝；改用本地HTTP裸仓库（git update-server-info+python http.server），第三次完整通过。不改后端校验。
## Confirmed Constraints / Boundaries
未commit/push/暂存；未改服务端。用户原未跟踪docs/tasks/T5-build-ui.md未修改。
接口只返回关联ID，明确标为ID；不新增名称解析/后端字段。日志按步落库，取消不保证远端命令停止，文案明确说明。
## Self-Review / Final Acceptance
范围、复用、架构、复杂度、回归、真实测试、用户三项重点、变异、持久状态：PASS。
## Remaining / Next Action
没有剩余实现或验收；交付中文报告，保留未提交工作区改动。无需用户确认。

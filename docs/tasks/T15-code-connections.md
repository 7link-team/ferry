# T15 — 代码连接与流水线源码选择修正

## Goal / Requirements
用户明确要求：连接阶段选择平台和授权（支持复用已有凭证），不选仓库/分支；流水线选择连接、可见仓库和分支，实际拉取对应源码。普通 Git 在流水线填写 URL/ref。单一代码连接菜单，无独立平台授权 tab。

## Existing Implementation / Reuse
复用 repository_connections、repository_auth provider adapters、credentials vault、ResourceForm/ResourceTable、pipeline repository_selection 原子绑定、现有 runner。repositories 保留为旧流水线和应用市场的具体源码绑定，保持历史兼容。

## Design / Task Decomposition
- [x] 后端连接资源：命名连接，平台授权及普通 Git；已有凭证只引用，不复制/改写。连接不绑定分支/仓库。兼容旧连接和 OAuth；新连接不能覆盖其他连接。
- [x] 用真实连接 ID 浏览可见仓库；流水线源码选择独立于认证表单，选择或输入分支。
- [x] 共享表单/列表单一代码连接入口；已有凭证按类型选择，网页授权保留 popup/COOP/state 保护。
- [x] 回归和实际端到端验收，独立审查。

## Execution Mode
Direct：认证生命周期、数据库和表单契约紧密耦合，直接实施。最终独立审查。使用当前目录和缓存，不另开 worktree。

## Files to Change / Add
repository_auth.rs / entities.rs / migration.rs / app router；RepositorySource.tsx / PipelineRepository.tsx / resources.tsx / nav.ts；现有 repository auth/API/UI 验收脚本。必要时独立连接资源模块。

## Testing / Acceptance Criteria
先 RED 再 GREEN：已有 PAT 无仓库/分支创建连接；错误类型/跨空间/无效令牌拒绝；凭据不复制改写；两个连接互不覆盖；旧流水线和 OAuth 仍正常。浏览器完整链路选择非默认分支，执行实际 Git checkout 并断言内容。相关 Rust/前端/三数据库回归。

## Risks / Out of Scope
授权归属沿用当前 workspace/user 边界；不得扩大共享权限。OAuth 未保存授权与正式连接要区分；更新明确目标。保持 HostChannel、vault、权限、事务和历史快照。K8s/Helm、发布和无关功能不在本次修正内。

## 本轮明确修正（用户反馈 alpha.12）
- 创建流水线使用可见区域全宽；基本信息只保留名称、分组、应用、标签、环境和备注。
- 执行主机/主机组和容器运行方式放在任务节点。环境只为未显式绑定的任务提供执行器与变量。
- 仓库连接、具体仓库、分支/ref 放到拉取代码步骤，使用稳定 step.id 绑定，不在基本信息中自动向所有任务添加 checkout。
- 显式添加串行后续任务、并行任务和修改依赖；运行器真正并发调度就绪任务，保留取消、清理、日志序号及升级排他。
- 标签与环境可选择、搜索并按权限就地创建；空列表解释原因。复用共享表单的字段和弹窗，不新建业务菜单。
- 仓库菜单改为命名连接资源；平台连接无 URL/默认分支，普通 Git 可保留地址但无分支。PAT/账号密码/SSH 使用已有授权认证；OAuth 保留平台动态配置。

## 分工与接口
Hybrid：连接后端、Actions 执行后端、画布交互为独立单元；主代理负责共享表单、连接/checkout UI、流水线 API 全局字段兼容、集成及验收。
Document.checkout_bindings: [{job_id,step_id,connection_id,repository_url,ref,path}]；不含密钥。新建 Actions 仅用节点绑定，旧全局字段保持历史/API 兼容。
验证：布局及表单浏览器、命名连接复用凭据且不创建副本、真实 Git 非默认分支按节点顺序拉取、并行 barrier 与串行依赖、跨空间拒绝、取消及日志清理。Rust/Go 构建测试只在 Actions，不在本地执行。

## 验收进度（2026-10-29）
- 本地前端 typecheck、build、98 项单测通过（1 项 Windows 专属跳过）。画布真实浏览器验证串并行依赖、删除/循环保护、保存重载、1280/900/390 布局；凭证选择与 OAuth 弹窗桥接通过。
- 云端 40733975250 已通过服务器/Runner 编译及服务器单测、完整页面、SQLite/MySQL/PostgreSQL 连接、真实 SSH/Agent 并行与依赖、取消、产物隔离及兼容验收。原生服务 40732239047 在相同产品代码上通过 Linux/Windows/macOS。
- 审查补回归：并发产物名称原子归属、每次任务独立容器身份、混合旧全局/新节点来源、CSV/XLSX 保留节点绑定、应用市场沿用新连接、YAML 删除清理绑定及凭证分页。

## 最终验收
本轮用户列出的全宽表单、节点源码与执行器、串并行关系、标签/环境选择及创建、无分支的连接创建、已有凭证引用均已通过。独立审查发现的问题已修复并补回归；范围、共享组件复用、历史兼容和源码交付检查通过。实际第三方账户不可用，平台接口/OAuth 使用隔离测试服务，Git 拉取及 Runner/SSH 执行是真实运行。本轮没有发布新安装包，alpha.12 不含这些修正。

# Agent CLI 安装、升级与配置隔离

## Implementation Plan

Goal：Runner 管理 CLI 安装和升级，运维可分别开关；托管会话支持独立配置目录。

Requirements：用户确认自动安装、自动升级、可禁用和隔离。Existing Implementation：`agent/probe.rs` 查 PATH；`agent/mod.rs` 执行原生协议、锁目录和回收进程；server 的工作空间权限、会话绑定保持不变。Reuse：现有 native_command、进程组/Windows Job、探测、托管目录和绑定校验。Execution Mode：Direct；本工作集中于同一执行链，不委派。Task Decomposition：策略读取→版本安装发布→探测执行接线→真实安装与隔离回归。

Design：Runner state 内 `agent-runtime.json` 为运维配置；缺省全部关闭。软件只安装在 state 内，不覆盖全局 CLI；固定官方 npm 包，npm 校验包完整性，关闭生命周期脚本。版本安装到新目录，验证 `--version` 成功才发布指针。旧会话绑定原绝对路径，升级不替换正在运行的软件。首次安装失败阻止新会话；升级失败保留旧版本。升级检查间隔为24小时，只在创建托管会话时执行。

Files to Change：Runner agent/probe/session、server 托管会话等待时间。Files to Add：Runner agent/provision.rs。Files Not to Change：数据库、模型凭据、API Agent、SSH执行、权限语义和会话页面。Testing：策略和路径负例、版本选择、隔离环境、失败保留、真实官方npm安装/CLI探测、相关Runner回归、workspace检查。Acceptance Criteria：关闭不安装；安装不影响全局；更新不替换旧版本；隔离不继承用户配置。Risks：Node/npm前置依赖；供应商CLI兼容性；磁盘保留历史版本。Out of Scope：操作系统沙箱、自动安装Node、CLI账号授权、平台API配置下发、API Agent推理。

## 配置

在 Runner state 目录内创建 `agent-runtime.json`（UTF-8）；修改后新探测读取，无需重启：

```json
{
  "auto_install": true,
  "auto_upgrade": false,
  "isolate_config": true,
  "versions": {}
}
```

`versions` 为可选的运行时→精确SemVer映射，示例版本仅演示语法，不保证仓库中存在；未指定时选择官方npm包latest。固定版本不随latest更新。支持 codex、claude、gemini、qwen、opencode、pi、dsh。安装/升级开关彼此独立；关闭安装仍可升级已托管版本，关闭升级保留已安装版本。没有托管版本且关闭安装时继续使用PATH中的CLI。DSH 安装要求 Node.js ≥24.9.0，安装器启用 engine-strict；不自动安装或替换系统 Node。

隔离只用于默认托管工作目录：每个 Agent 独立 HOME、原生配置、缓存和临时目录；工作目录按会话/Agent分开，同一Agent的新会话复用自己的登录配置。原有主机账号登录态不会复制。需要在 `state/agent-configs/<Agent ID>` 对应的隔离环境完成厂商授权，不能把安装成功视为登录成功。原生授权不依赖成功创建会话；Agent ID来自已保存的Agent记录。手工共享工作目录与隔离模式不兼容，拒绝执行。关闭隔离保留现有原生用户配置行为。更改隔离开关后，已有会话的执行身份校验会拦截配置切换，需新建会话；不会悄悄混用两套登录态。

这不是OS安全边界：同一操作系统用户仍可访问其权限内的文件/进程。需要强安全隔离时使用独立低权限系统用户或独立主机/容器。现有平台MCP每次调用仍执行当前空间和角色授权。

旧版本为历史会话保留，不自动删除；运维在确认没有历史会话使用后清理。安装过程不包含模型密钥；不记录原生CLI输出或npm诊断。安装只在新建托管会话时发生，普通目录/模型查询无安装副作用。

## 设计评审

| 维度 | 判定 |
|---|---|
| 事实来源 | 用户请求；现有Runner CLI协议/托管目录/绑定和工作空间权限 |
| 本期范围 / 不做范围 | 以上Plan与Out of Scope |
| 灰度策略 | 无；默认全关闭，逐Runner启用 |
| 流程闭环 | 未安装→暂存安装→版本验证→发布；失败清理暂存，旧版本不变 |
| 异常覆盖 | 无Node/npm、网络/包/版本失败、配置非法、锁竞争、路径逃逸均拒绝；不暴露原生诊断 |
| 操作记录 | 现有会话创建审计；Runner记录运行时安装/升级成功与失败，不记录密钥和子进程输出 |
| 删除保留 | 无业务删除；失败暂存清理，旧版本保留，不删除用户配置 |
| 权限规则 | 本地运维策略；服务端沿用会话/Agent/host空间校验，安装不开放新HTTP接口 |
| 计算逻辑 | 无业务计算；24小时检查间隔 |
| 模式差异 | 查询无安装；创建托管会话允许安装/升级；执行固定已绑定路径 |
| 字段联动 | 三个独立bool，缺省false；versions可选精确SemVer；未知键拒绝 |
| 状态操作 | 安装锁排他，升级发布原子化；失败回到旧版本，禁用不卸载 |
| 校验规则 | 配置≤16KiB；已知运行时、精确SemVer；包bin必须位于安装根目录；工作目录不得符号链接逃逸 |
| 数据模型 / 时间审计 / 索引设计 / SQL示例 | 无数据库变更，DDL无；Runner本地版本清单不作为业务模型 |
| 文案完整 | 无新UI文案；运维文档中文，原生错误不显示诊断；现有服务端错误沿用i18n |
| 配置来源 | Runner本地运维JSON；官方包标识是协议常量，不含部署品牌 |
| 状态覆盖 | 全关闭默认；已安装、首次安装、升级成功/失败、隔离/兼容模式 |
| 影响核心流程 | 新会话可等待安装（安装最多300秒，托管探测最多360秒）；历史执行固定原路径 |
| 影响历史数据 | 否，无迁移，up/down无 |
| 是否依赖三方 | 已有Node/npm、官方CLI npm包；不新增Cargo/npm项目依赖 |
| 版本兼容 | AgentProbe 增加可选 binding，旧帧缺省为空；未启用隔离时保留旧身份哈希。配置隔离需要配套的新 Server/Runner；新运行时需要升级 Runner |
| 逻辑冲突 | 隔离模式与手工共享目录冲突时拒绝，不悄悄取消隔离 |
| 可测试性 | 单元负例+真实npm安装/探测；无厂商账号时不声称推理通过 |
| 验收追踪 | provision测试与本地维护证据；无新增屏，Figma无 |
| 响应式交付 | 无UI改动；Desktop/Pad/Phone均沿用现有会话页面、操作、滚动与安全区 |
| 模糊描述 | 无待猜测规则 |

本功能没有新增页面，三类设备布局重排/导航操作/数据呈现/滚动/安全区键盘均无变更，无新增设计屏或截图要求。

## DSH 接入计划与上游契约（2026-11-11，实现与验证中）

Goal/Requirements：用户确认 DeepSeek Harness，接入同一模型/Agent/会话链路，沿用安装开关、配置隔离与空间授权。Reuse：现有 ACP stdio、进程回收、目录绑定、权限决策、MCP 桥和共享模型表单。Execution Mode：Direct；协议、启动权限与会话恢复相互依赖，不委派。

Design：共享 Runtime 增加 dsh；命令 dsh acp；DSH_HOME 跟随独立 Agent 配置目录。复用官方权限服务，在每次 Agent 创建/恢复时将沙箱重置为 read-only；涉及写入的提权经现有逐次授权，平台 MCP 仍由服务器检查用户与 Agent 权限交集。通过临时插件调用官方服务，不修改原生日志。仅接受当前会话公布的模型选项，歧义或不可用时拒绝，禁止悄悄回退。

执行前探测携带已保存的会话 binding；手工指定路径的探测不带 binding，开启隔离时拒绝复用托管目录。DSH 的 ACP 会将原生 error/blocked 映射为 end_turn，因此同一临时插件还转发原生 turn/end 原因；Runner 只接受当前根会话的结束回执，缺失回执或未知原因不能判定成功，子 Agent 结束不改变根会话结果。

Files：protocol/agent.rs；runner 的 agent/mod、probe、provision、protocol/acp 与测试；server 会话允许的未知登录态；模型共享表单选项。无数据库/审计字段/索引/DDL/up/down/历史数据迁移；无新页面或响应式布局。沿用本文设计评审各维度、六语错误提示和共享表单的 Desktop/Pad/Phone 契约，供应商产品名不翻译。测试：协议新建/恢复/模型拒绝/权限拒绝/跨会话/MCP；真实官方安装、真实进程、隔离与取消。无账号时不声称厂商推理成功。Out of Scope：全局 Node 安装、DSH 自身 Web UI、Grok/Kimi 适配、登录续期策略、生产发布。

用户补充 dsh 指 DeepSeek Harness。已从官方仓库核实：

- 项目：https://github.com/deepseek-ai/deepseek-harness ，默认分支 master。
- 官方包：@deepseek-ai/dsh；CLI 为 dsh；当前 package.json 要求 Node.js >=24.9.0。
- 官方 CLI 文档：https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/cli-help.md 。提供 dsh acp、dsh headless --json 和 headless 的 --session-id。
- ACP 契约：https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/acp/acp/README.md 。创建使用 session/new，恢复使用 session/resume，模型使用 session/set_config_option；支持逐次权限确认和 stdio/HTTP MCP。
- 与现有 ACP 适配的差异：当前 Runner 使用 session/load、session/set_mode、session/set_model；DSH 官方明确不支持 load/modes，不能仅增加枚举并照搬现有 ACP 路径。
- 独立配置覆盖 DSH_HOME；Runtime 枚举与模型选择已接入 dsh。真实官方包 0.2.2-rc.1 已通过隔离安装、禁用开关与升级失败保留验证；协议回归 32 项通过。真实 ACP/MCP 会话验收使用确定性测试模型，不调用厂商推理服务，证据保存在 data/maintenance/dsh-*。

本节仅记录用户已明确的 CLI 身份及已检查的上游协议，不把外部功能当作 Ferry 产品需求。后续适配复用现有进程生命周期、平台 MCP 权限和工作空间数据边界；安装成功不代表厂商登录或推理成功。

## Installed product identity review (2026-11-11)

New defaults: service `ferry-runner-agent`, supervisor `ferry-runner-service(.exe)`, signed worker `ferry-runner(.exe)`, MCP command `ferry-mcp` (legacy `platform-mcp` alias retained). MCP server `ferry` and tools `ferry_discover` / `ferry_request` already identify the product. No product Skill is generated today; do not add a placeholder or rename third-party tools.

Existing default installations reuse the original registration, supervisor path and state directory. A worker upgrade does not rename the existing supervisor or service registration. Old signed version filenames remain readable and require the same signature/hash verification.

| Review dimension | Decision |
|---|---|
| Source | Explicit user requirement; service.rs/update.rs/install scripts/MCP registration |
| Scope / exclusions | New installation/upgrade identity and compatibility; no automatic live-service registration migration, protocol-enum or third-party renaming |
| Rollout | Through normal installation/upgrade; no separate feature flag |
| Flow | Verify before staging; retain original safety checks and reinstall collision rejection |
| Errors / extremes | Missing or corrupt files fail; an existing invalid new filename never authorizes fallback to the old filename |
| Audit | Existing installation output and upgrade records |
| Deletion | CLI preserves state; UI uninstall supports both defaults and stops removal after deregistration failure |
| Permissions | Existing system/user privileges, enrollment tokens, MCP grants and workspace boundaries |
| Calculations | None |
| Modes | New installation uses new names; existing installation preserves identity; custom names remain supported |
| Field linkage | Default name determines directory; old default configuration is reused only if new default configuration is absent |
| State operations | Existing install/start/stop/upgrade/rollback state machines |
| Validation | Existing 80-character service-name limit, charset, directory safety, signatures and hashes |
| Data model / time audit / indexes / SQL | No new model, field, index or DDL |
| Translations | No new UI translation keys; software identifiers are untranslated; native service descriptions identify Ferry |
| Configuration | Software defaults and existing custom service-name installation parameter |
| State coverage | Existing default/missing/error/success states |
| Core impact | Install/update/uninstall commands and MCP launch command alias |
| Historical data | No database changes or up/down migration; preserve old disk layout |
| Third parties | Existing OS service managers; no new dependency |
| Compatibility | Compatible upgrade; retain old command, signed versions and registered executable paths |
| Conflicts | Reinstall must not silently create a second default host identity |
| Testability | Runner/CLI tests, MCP canonical+legacy commands, native installation/restart/uninstall, generated command execution |
| Acceptance trace | scripts/test-platform-mcp.py, scripts/test-runner-service.py, RunnerConnection.test.tsx; no new Figma screen |
| Responsive delivery | No new page/layout; existing command fields retain Desktop/Pad/Phone layout; no new screenshots. Native services: not applicable |
| Ambiguity | None |

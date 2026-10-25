# T7 — Compose 部署最小闭环

## Goal / Requirements

承接 v0.1.0-alpha.1 与产品方案 §10、§15，进入阶段 5。首个可验收结果：创建应用 → 查看确定的 Compose 产物 → 经 runner 或 SSH 部署 → 查看结果 → 更新 → 回滚到已保存的 revision。

本文件记录已确认的首批实施范围；不表示整个阶段 5 已完成。

## Existing Implementation / Reuse

- `channel.rs` 已有 HostChannel：复用文件写入及 exec，两种接入不增加第三条执行路径。
- `resource.rs`、Scope、雪花 Id、SeaORM entities/schema builder：沿用资源接口、租户隔离和并发版本约定。
- `workspace.rs` 已定义 deployments.read/create/update/delete/apply/rollback 权限。
- `nav.ts` 已有应用入口；资源列表和表单仍使用 ResourceTable/ResourceForm 配置。
- `pipeline_runs.rs` 有快照、状态和审计的实现可参考，但部署 revision 不冒充 pipeline run，不把两类业务硬塞进一个抽象。
- 现有 variables、app-templates、deployments 都只有导航/权限占位；没有可复用的密钥变量解析器或部署渲染器。
- 已发布版本的迁移保持不动，只追加新迁移。方案 §12 的旧 TEXT ID / raw SQL 描述已被 §18 及实际代码替代。

## Scope decision — 用户已确认（2026-10-25）

首批实现 Compose + 单主机目标 + 镜像来源，可包含多个服务。首批明确拒绝 `valueFrom`、`fromTemplate`、artifact、主机组及其他部署模式；不能忽略配置或替换成空值。密钥变量与配置模板待对应模块存在后接入。

这会缩小首个交付批次，不改变产品最终支持六种部署模式的目标。若首批必须部署依赖密钥的应用，需要先补 variables 资源与解析，不能在 manifest 放明文密钥作为替代。

## Task Decomposition / Execution Mode

设计阶段 Direct；实现按下面依赖顺序进行。渲染契约明确前不并行写多个模块。后续只有在独立文件和接口边界确定后才评估委派，主代理负责集成与验收。

### T7a — manifest 校验与纯渲染

- 明确首批字段契约、默认值和不支持字段的错误，解析后存 JSON；UI 支持 YAML 输入需使用维护中的解析库，先检查已有依赖，不手写 YAML 解析器。
- Compose renderer 是普通纯函数；相同 manifest 和目标配置输出相同字节，不读取环境变量、时间、网络或主机。
- 支持镜像、多服务、command、普通 env、ports、依赖和命名卷；其他字段逐项按能力表决定支持或明确拒绝。Compose 不支持滚动更新，不降级成 recreate。
- 规定 overrides 的对象递归合并、标量/数组替换规则，合并后重新校验；不得借 overrides 绕过未实现的变量/模板处理。
- 验证重复服务/端口、缺失依赖、依赖循环、镜像与 artifact 互斥、多副本与固定宿主端口冲突。
- 字面 `$` 和 Compose 插值要区分，防止预览值在目标主机悄悄改变。
- 单测之外，把渲染产物交给真实 `docker compose config` 验证；重复渲染字节一致。

### T7b — 部署资源与 revision

- 新增 deployments、deployment_revisions 的 SeaORM 实体和追加迁移；BIGINT ID、三数据库一致。
- 复用资源列表、Scope 和乐观并发控制；目标主机必须属于当前工作空间。
- `render` 仅需 deployments.read，不连接目标。保存即校验模式能力。
- revision 保存实际渲染产物和执行结果；执行中使用快照，后续修改不能改变在途部署。
- apply/rollback 并发规则、失败状态及重启恢复须在实现前补齐到本文件；不得仅照搬流水线取消语义。

### T7c — 部署、预览与回滚

- 沿 HostChannel 写入产物并调用 Docker Compose v2+，runner/SSH 均可运行。检查 Docker 和 Compose 依赖，缺失时明确失败。
- 项目名及远程目录按工作空间和部署 ID 稳定隔离，用户名称不直接拼接 shell。
- render 是纯渲染；preview 检查实际目标产物并展示差异，不能把数据库里的上一版冒充线上状态。
- apply 持久化快照后执行并记录结果；失败保留诊断信息，不自动回滚或清理业务卷。
- rollback 使用保存的产物，不能重新渲染旧 manifest。可变镜像 tag 的确定性需要设计并验证 digest 固定策略；仅保存镜像 tag 不能声称精确回滚。
- 删除资源是否同时卸载应用、历史保留、健康检查成功条件等产品行为需先明确；不默认执行 `down -v`。

### T7d — 界面与真实验收

- 应用列表/表单只增加 DSL 配置；新增部署详情视图承载产物、差异、revision 和执行状态。
- 首批界面清楚呈现 Compose 与单主机支持范围，禁用未实现动作。
- 成功场景：真实应用 v1 → v2 → rollback v1，HTTP 响应确认版本变化，runner/SSH 都跑通。
- 失败场景：跨空间访问、权限不足、过期 version、并发 apply、目标离线、坏镜像、Compose 缺失、命令失败；审计与状态一致。
- 发布后 SQLite/MySQL/PostgreSQL 库升级保留既有账号、主机、凭据、流水线和运行历史。
- UI 实际验证预览和执行产物一致、终态停止刷新；所有新增 harness 接入 CI，不能零检查或静默跳过。

## Files to Change / Add

预计修改：`crates/server/src/{main,entities,migration}.rs`、`src/{resources,App}.tsx`；根据解析库选择修改 Cargo 配置/锁文件。现有权限若已满足则不改。

预计新增：manifest/Compose 纯函数模块、部署资源与执行模块、部署详情组件、真实部署 harness。文件数量按实际责任控制，不预建插件框架。

不改：SSH/runner 协议、既有 pipeline 行为、已发布迁移、发布版本号；用户后续已明确要求将本任务与Windows安装范围、应用内更新一并发布为v0.1.0-alpha.2。

## Testing / Acceptance Criteria

每批执行相关单测和真实验证，最后跑 npm build/typecheck/test、cargo fmt/check/test、三数据库升级及回归、两通道部署与回滚和浏览器验收。编译成功不能替代真实部署。

最终 PASS 要有 render 字节一致、实际部署与更新成功、保存产物回滚成功、租户/权限隔离、三库升级保留数据的证据。未完成模块保持明确未完成状态。

## Risks / Out of Scope

风险：Compose 插值、可变镜像 tag、远端在服务中断后继续运行、overrides 与能力校验冲突、卷数据不随镜像回滚。逐项设计与验收，不能用假成功或自动重试遮盖。

不在首批：其余五种部署模式、应用市场、报警、定时任务、流水线模板。应用市场来源尚未确认，不在这里替用户决定。

## Next Action

完成真实部署、三库升级和桌面更新验收后发布预发布版本；验证结果见任务状态。

## 2026-10-25 后续确认
用户“继续”“都要”确认：单应用串行、host/workdir不可变、失败不自动回滚、中断需人工核实、删除仅平台记录；同时做Windows双安装范围、应用内检查/安装更新，并发布alpha.2。
预览对比远端保存产物，明确不等于实时容器状态；preview解析镜像digest并返回绑定产物、版本、来源revision和目标hash的token；apply验证token后保存并执行原样产物；rollback直接使用已保存产物。更新使用官方Tauri签名及版本绑定；预发布发现走GitHub releases列表，配置仓库来源，不用latest稳定版接口。

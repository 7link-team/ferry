# T8 — 共享变量与 Compose 密钥引用

## Goal / Requirements
用户已确认继续密钥变量模块。实现工作空间普通变量和密钥变量，以及 Compose manifest `valueFrom: variable/name` 引用。配置模板后续实施；不扩展部署模式或流水线变量插值。

## Existing Implementation / Reuse
复用 Scope、variables.* 权限、SeaORM/资源列表、雪花 ID、vault（现有主密钥+A​​AD）、HostChannel、共享 ResourceTable/ResourceForm。复用 Compose 固定摘要、预览 token、版本检查、远端锁、人工核实及保存 revision 的机制。已发布迁移保持不变，追加迁移。此阶段 Direct；模块接口与安全契约相互依赖，不委派。

## Design — 已有架构约束
- 唯一资源 `/api/workspaces/{ws}/variables`，入口通过 scope=global/deploy/build 预置参数复用同一列表表单。
- 唯一键 workspace/scope/scope_ref/name；scope_ref=0 表示全局或模块，deploy 对象指向部署，build 对象指向流水线；对象必须属于同一空间。全局不得指定对象。
- 名称用于 variable/name 引用；名称、scope、scope_ref 和类型创建后固定，避免隐式移动引用或将密钥降级为公开值。支持普通/密钥类型，值不 trim，保留空串、空白、换行和美元符号；拒绝环境变量无法承载的 NUL，限制64KB。
- 值使用已有 vault 加密保存。列表/详情对密钥只投影 has_value，不读取或返回密文。普通变量可以显示其值；无任何值写入审计详情。
- PATCH 省略 value 表示不改变；明确提供空字符串表示设置为空。表单明确区分“保留原值”和“替换值”，不把空串悄悄解释成不更新。
- `POST /variables/{id}/actions/reveal` 要求 variables.reveal，成功审计后返回明文；响应 no-store，前端显式显示/隐藏，不自动加载，不写持久存储。
- 解析按全局→模块→对象覆盖；返回引用名称、来源层、变量 ID/版本和密钥标记，不向部署预览返回密钥值。未定义引用、跨空间引用、无效对象均拒绝。活动配置引用中的变量不可删除，先修改引用；历史 revision 的处理见待确认项。
- 变量变更不自动重启容器。用户重新预览并执行部署才更新；变更变量或作用域覆盖后旧预览必须失效。已入队任务使用固定快照，后续编辑不得改变在途任务。
- 渲染仍为纯函数，不读取密钥。Compose config 解析镜像时不接触真实密钥；公开产物包含受控 env_file 路径/引用元数据，禁止 overrides 绕过环境引用规则。预览及执行的公开 Compose 产物一致。
- 执行阶段通过 HostChannel 写0600环境文件，密钥不进入命令参数或原始 Compose JSON。包含密钥的执行不保存可能携带容器错误输出的原始文本，只保存安全状态/退出码；缺失配置及连接故障保留不含值的诊断。

## Confirmed decision — 回滚与旧密钥保留
推荐：每个 revision 的变量值使用 vault 加密保存，AAD 绑定工作空间/revision；回滚还原当时变量值，与既有“保存产物精确回滚”一致。更新/删除当前变量不会抹除历史 revision 内的旧值；随部署历史删除而删除。风险：旧密钥轮换后可能已经失效，回滚界面须明确提示。

备选：回滚旧镜像/配置时重新解析当前变量，历史不保留旧密钥。风险：回滚不再恢复完整旧运行配置，变量删除会阻止回滚；需改变既有精确回滚语义，并重新预览当前变量。

用户于本轮明确选择“恢复历史变量值（推荐，加密随部署历史保留）”。实施历史加密快照，不再等待此项确认。

## Files to Change / Add
新增 variables.rs 与变量测试；修改 entities.rs/migration.rs/main.rs、manifest.rs/deployments.rs、资源 DSL 配置及共享表单、部署详情引用来源展示、现有部署/数据库/UI harness。是否增加独立快照表由确认后的回滚语义决定。暂不修改版本号、桌面生命周期、SSH/runner协议、已发布迁移。

## Testing / Acceptance
1. 三库迁移、空间隔离、CRUD/版本冲突、所有权限、reveal审计/no-store、列表与详情无密钥/密文投影。
2. 空串/空白/引号/反斜杠/多行/美元符号真实传入容器；全局/模块/对象覆盖与来源；缺失引用和 overrides 绕过被拒绝。
3. 变量编辑后旧预览拒绝、并发变更/执行不混用、已入队快照不变、按确认语义回滚。
4. runner和SSH真实部署；远端0600；预览/历史/审计/参数/错误输出不含密钥。
5. 共享UI三入口与显式明文查看、隐藏/换页清理；现有凭据表单与部署闭环不回归。
6. fmt/check/test、前端build/typecheck/test、三数据库升级与现有部署harness全部通过后才交付。后续仍为alpha，不宣称正式版。

## Risks / Out of Scope
env_file插值/多行规则必须以真实Compose验证；锁与预览失效必须覆盖变量覆盖层新增/删除竞争。无法完全阻止有权配置任意容器或登录主机的人在主机内读取环境变量，此模块保证平台接口/日志不泄露。配置模板、构建变量消费、自动密钥轮换、定时任务不在本批。

## 独立资源部分实施检查点（2026-10-25）
已实现加密CRUD、投影隔离、审计reveal/no-store、作用域解析、共享UI与引用删除检查。对象仍有专属变量时，必须显式删除变量后才能删除对象，避免孤立记录。变量与部署配置修改通过现有portable lock在事务内协调。

pure render使用`x-variable-references`标记尚未绑定的引用；执行前拒绝未绑定标记，防止接入未完成时静默丢失环境项。实际env_file绑定、预览失效、执行快照/回滚仍未完成，不能视为Compose变量支持已交付。

验证：6项变量服务测试、manifest引用/override校验、5项共享表单测试通过；前端全39项与build通过；真实SQLite HTTP回归119项通过；真实Chromium验证三入口默认scope、创建/查看/隐藏/切页清除及空串替换通过。MySQL/PostgreSQL实际回归、runner/SSH变量部署尚未运行。回滚保留选择仍待答复；没有发布或提交本批改动。

## 本轮绑定实现细化
Direct执行；复用现有variables事务锁、vault和HostChannel。追加revision.variable_snapshot密文列，AAD绑定workspace/revision；不增加快照资源接口。预览记录变量ID/版本/作用域/类型元数据，镜像摘要解析不携带值；返回前重检绑定元数据。入队在同一变量锁下重新解析并校验预览元数据、固定加密快照。回滚解密历史快照后按新revision重新加密。每服务受控env_file仅执行时上传0600，命令退出时清理；产物、预览、历史输出不含引用变量明文。现有无变量revision保持兼容。真实Compose验证特殊字符、轮换预览失效、历史回滚及runner/SSH路径。

## 完成验收（2026-10-26，源码未发布）

用户已确认历史值回滚；资源、绑定、快照、失效检查、执行与回滚现已实现。变量引用预览显示来源层、ID/版本及密钥标记；不返回值。入队事务同时检查绑定版本并保存vault密文；AAD绑定workspace/revision，回滚重新加密到新revision。无变量的旧revision兼容空快照。

每服务环境文件通过HostChannel以0600上传；正常结束或命令失败时清理，引用变量的原始输出不保存，仅保留安全状态/退出码。服务或远端连接异常仍进入现有“需核实”流程；强制中断时需检查远端进程和可能残留的环境文件。主机管理员及容器本身仍能读取运行时环境。删除部署记录会删除其历史及快照，但不会停止/删除既有容器。

实际验证：
- SQLite/MySQL8.4/PostgreSQL17接口回归各119项通过；alpha.6数据升级三库通过。
- 真实Docker Compose2.40.3＋runner/SSH共102项通过：特殊字符/空串、全局模块对象覆盖、新覆盖/轮换使预览失效、在途快照不变、历史值回滚、删除当前变量后回滚、0600、正常及失败清理、故意将密钥输出到stderr仍不进入平台输出。
- Chromium变量资源三入口、明文显式查看/清除、部署引用来源与历史回滚提示均通过。
- Rust102项通过，1项既有默认忽略（真实Compose另由上述harness覆盖）；前端44项、build/typecheck/fmt/check通过。

测试过程修正了SQLite单连接在入队事务中再次访问连接池导致的等待；所有相关读取改用事务连接。一次旧runner重连后的503未在后续两次执行复现，最终完整harness通过；未用失败重试机制隐藏它。未发版、未推送CI。

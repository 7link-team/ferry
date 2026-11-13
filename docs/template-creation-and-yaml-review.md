# 模板选择与 YAML 展示修正

2026-11-12。用户指出部署创建时模板与参数混排，应用模板详情仍显示 JSON。本次只修正这两项交互，不新增模板分类、收藏、市场入口、蓝图表单、数据库字段或迁移。执行方式 Direct：共享选择器与模板编辑器相互关联，无需委派。

## 交互与验收

1. 目录阶段只显示搜索、空白创建（原入口允许时）、模板卡片、服务端分页和下一步。每页 12 项，沿用已有接口；选择模板不生成、不保存、不执行。
2. 参数阶段只显示所选模板名称/版本、参数、返回目录和生成操作。返回保留搜索/页码与当前参数；改选其他模板采用新模板默认参数，不混入旧值。
3. 带 `template` 的深链接直接进入参数阶段。详情加载失败时可重试或返回目录；生成失败保留值、显示原错误并可重试。生成中禁止切换与重复提交。空白创建沿用原行为。
4. 应用模板加载的有效 JSON 转成 YAML 作为编辑展示；已有 YAML（包括注释）、未完成或无效源码保持原文。输入后的源码不在打字过程中被强制格式化。仅展示/编辑草稿转换，不自动写回历史记录。
5. `ferry/blueprint-v1` 模板使用现有 YAML 源码编辑器，不进入只支持 `services` 的普通应用表单。沿用后端保存校验，不构建新的蓝图表单或前端业务规则。

## 功能评审

| 维度 | 选择/参数阶段 | 应用模板 YAML/蓝图源码 |
|---|---|---|
| 事实来源 | TemplateChooserDialog、ResourceForm、templateGenerator 与用户链接实测 | templates/resource-config、ManifestEditor、blueprint 模型与用户详情实测 |
| 本期范围 / 不做范围 | 两阶段和现有目录分页；不加分类/收藏/新目录接口 | 默认 YAML、蓝图正确源码入口；不做蓝图表单/迁移 |
| 灰度策略 | 无，沿用共享组件全部调用方 | 无，仅应用模板内容编辑器 |
| 流程闭环 | 目录→参数→生成→现有表单；加载/校验未通过禁止生成；生成中锁定 | 有效 JSON 载入转 YAML；用户输入保留；后端既有校验决定保存 |
| 异常覆盖 | 空目录、搜索无结果、列表/详情失败、超长名称、翻页、生成失败、加载竞争 | 无效/不完整源码保留；YAML 注释不丢；蓝图不误报 services |
| 操作记录 | 沿用生成与业务保存记录；浏览/切阶段不新增审计 | 沿用模板保存审计；浏览不写回 |
| 删除保留 | 无删除及保留策略变化 | 无删除及保留策略变化 |
| 权限规则 | 沿用 generator 与工作空间权限；客户端不扩大范围 | 沿用模板查看/编辑权限，readonly 不可修改 |
| 计算逻辑 | 沿用总数/12向上取整及现有末页校正 | 无业务计算 |
| 模式差异 | 创建允许空白；导入不允许；深链接进入参数 | 普通应用继续表单/源码；蓝图仅源码；查看只读 |
| 字段联动 | 选新模板载入该版本默认值；同模板返回保留值 | 仅 content 展示变化，不更改参数模型 |
| 状态操作 | 下一步需已选模板；生成需详情成功且参数有效；忙时不可切换 | readonly 禁止写；既有保存/错误行为不变 |
| 校验规则 | 复用参数字段约束与 incompleteFields | 复用 YAML 解析及后端模板校验，不增加业务约束 |
| 数据模型 | 无新增；使用既有 id/name/version/fields/values | 无新增；使用既有 content/parameters |
| 时间审计 | 无模型变更 | 无模型变更 |
| 索引设计 | 无变更 | 无变更 |
| SQL示例 | 无新表/DDL | 无新表/DDL |
| 文案完整 | 新阶段文案覆盖 zh-CN/en-US/ms-MY；其余沿用 | 复用现有 YAML/模板文案 |
| 配置来源 | 页大小沿用12；标题来自模板；新文案i18n | blueprint-v1 为已有协议标识；语言yaml沿用现有编辑器 |
| 状态覆盖 | 目录、详情加载、参数、生成、失败、取消 | 普通应用/蓝图、JSON/YAML/无效源码、只读/编辑 |
| 影响核心流程 | 共享模板创建/导入的阶段顺序 | 应用模板详情/编辑展示 |
| 影响历史数据 | 无，不迁移 | 无，不写回浏览数据，不改变存储格式 |
| 是否依赖三方 | 已有 Radix/React；无新依赖 | 已有 yaml/Monaco；无新依赖 |
| 版本兼容 | API及template深链接兼容 | 后端JSON存储及YAML/JSON输入兼容 |
| 逻辑冲突 | 需调整原“混排可见”测试，保留搜索/分页/参数保持语义 | 蓝图不能用普通应用结构校验；分支仅识别已有apiVersion |
| 可测试性 | 单元测试阶段切换、参数保持/重置、深链接/重试；浏览器分页及参数跳转 | 单元测试语义不变/注释保留/用户输入不重排/蓝图只读；真实详情验证 |
| 验收追踪 | 测试对应1–3；Figma暂缺，不声称有设计稿 | 测试对应4–5；Figma暂缺 |
| 响应式交付 | 下表与三类浏览器截图；固定操作区，目录内滚动 | 下表与三类详情截图；源码在命名容器内滚动 |
| 模糊描述 | 无 | 无 |

## 响应式契约

| 设备形态 | 布局重排 | 导航/操作 | 数据呈现 | 滚动 | 安全区/键盘 |
|---|---|---|---|---|---|
| Desktop ≥1200 | 目录3列；参数2列 | 阶段标题、返回/下一步/生成常驻 | 所选模板摘要与参数分阶段 | 目录/参数体内滚动 | 保留既有对话框高度约束与键盘Focus |
| Pad 768–1199 | 目录2列；参数1–2列 | 控件触控≥44px，可换行 | 长名称换行，搜索/分页独立 | 体内滚动，不产生整页横滚 | 横竖切换保留状态；底部安全区 |
| Phone 320–767 | 目录1列；参数1列 | 返回/生成始终可达，不依赖hover | 模板说明截断、名称可换行 | 对话框体内滚动；源码容器滚动 | 使用dvh与safe-area，输入区可滚动到可见 |

## 修改与验证边界

修改现有 TemplateChooserDialog、模板内容配置及三语文案；新增应用模板内容组件仅承载普通应用/蓝图分流与载入格式，不新建资源表格。修改相关回归测试。后端、协议、数据库、市场来源、部署执行器和非应用模板格式不改。构建与测试通过后在隔离前端和4291既有只读数据上检查交互，不创建或部署应用。
## Final verification and local delivery (2026-11-12)

- Related regression suites: 79/79 PASS; `npm run build` and `cargo build -p ferry-server`: PASS. Existing bundle-size/dead-code warnings remain.
- Browser checks on final 4291: zh-CN/en-US/ms-MY x Desktop 1440 / Pad 820 / Phone 375, 9/9 PASS. Both user deep links verified; Blueprint shows YAML without the services error. No application was saved or deployed by acceptance testing.
- Catalog stress check uses an intercepted, read-only 49-template GET fixture with real template detail responses; it does not insert production records.
- Evidence: `data/maintenance/template-ux-live4291-review/result.json` and screenshots; `data/maintenance/template-creation-yaml-local-update.json`; `data/maintenance/product-poc-20261112/live4291-review.json`.
- Live process 54532; backup `data/backups/pre-template-creation-yaml-20261112-204334`. Served HTML/JS match final dist; SQLite integrity, existing business rows, schema, master key and instance identity preserved; both original Runners online.
- Self-review and original-request acceptance: PASS. No migration, dependency addition, Git commit or push. Figma remains unavailable; browser screenshots provide actual implementation evidence.

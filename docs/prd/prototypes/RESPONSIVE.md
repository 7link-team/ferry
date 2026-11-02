# 响应式原型交付

本交付没有 Figma 文件。**HTML prototypes（HTML 原型）+ PNG screenshots（PNG 截图）替代 Figma screens/variants**；应按此命名，不得声称是 Figma 文件或 Figma 画板。16 页共用同一套 HTML，不创建设备专用副本。

Desktop = 1440×900；Pad = 768×1024；Phone = 390×844。均为 deviceScaleFactor 1、full-page capture，因此 PNG 高度可大于视口高度。下表的 ✓ 表示已生成并验证，D/P/M 截图链接分别对应三种布局。

| 页面 | Desktop | Pad | Phone | 收缩策略 | 核心信息/操作保留清单 | 截图路径 |
|---|---|---|---|---|---|---|
| 01 工作流列表 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 分组改为折行筛选；表格区域横向滚动，首列及操作列固定 | 名称、分组、发布/草稿、未发布变更、触发方式、最近执行/时间/成本；搜索、状态/触发/时间筛选、导入、新建、打开、运行、复制、归档、分页 | [D](../images/01-workflow-list.png) / [P](../images/pad/01-workflow-list.png) / [M](../images/phone/01-workflow-list.png) |
| 02 工作流编辑器 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | Pad/Phone 节点库与画布页签；Pad 画布区域滚动；Phone 原节点纵向只读列表 | 发布版/草稿、节点类型/状态、循环并发、缓存、主机、耗时/成本、日志；版本选择、设置、撤销、重做、测试运行、发布；节点库/端口筛选仍可切换查看 | [D](../images/02-workflow-editor.png) / [P](../images/pad/02-workflow-editor.png) / [M](../images/phone/02-workflow-editor.png) |
| 03 节点详情 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 输入、参数/设置、输出改为可切换页签；Phone 字段单列 | 输入 Schema/来源/表达式、模型/凭据、补全/预览、种子/数量、重试/错误/缓存/运行目标、输出引用/缓存来源；执行到此节点、关闭、固定为测试数据，均可从对应页签到达 | [D](../images/03-node-detail.png) / [P](../images/pad/03-node-detail.png) / [M](../images/phone/03-node-detail.png) |
| 04 问题面板 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 画布与问题列表页签；Phone 原错误节点纵向列表 | 问题数量、阻断原因、错误节点及修复指引；重新校验、发布入口及其原禁用态、逐项定位、配置凭据、下载到主机、配置输入映射、配置单价 | [D](../images/04-problems-panel.png) / [P](../images/pad/04-problems-panel.png) / [M](../images/phone/04-problems-panel.png) |
| 05 执行详情 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 画布与节点详情页签；Phone 状态节点纵向列表；指标折行 | 执行状态/版本/身份/成本、轮次、复用/失败/取消、节点输出；取消、重试、以本次定义打开、上一轮/选择轮次/下一轮、从此节点重跑、处理人工任务 | [D](../images/05-execution-detail.png) / [P](../images/pad/05-execution-detail.png) / [M](../images/phone/05-execution-detail.png) |
| 06 执行与队列 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 三栏队列转单列，筛选与指标折行 | 运行/排队/等待、槽位、触发/版本、挂起原因与任务；刷新、搜索、状态/触发/版本筛选、待我处理、取消、去处理 | [D](../images/06-executions-queue.png) / [P](../images/pad/06-executions-queue.png) / [M](../images/phone/06-executions-queue.png) |
| 07 人工挑选 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 媒体网格 4→2→1 列；任务侧栏下移 | 候选预览、已选标记、选择数量、说明、任务/剩余时间、重做轮次；选择候选、填写说明、全部不满意/重做、确认选择 | [D](../images/07-human-pick.png) / [P](../images/pad/07-human-pick.png) / [M](../images/phone/07-human-pick.png) |
| 08 人工编辑 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 编辑/对比区及任务侧栏纵向排列 | 编辑文本、原文/改后对比、校验、任务与输出范围；编辑内容、退回修改、提交编辑结果 | [D](../images/08-human-edit.png) / [P](../images/pad/08-human-edit.png) / [M](../images/phone/08-human-edit.png) |
| 09 资产库 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 网格 4→2→1 列；上下文菜单下移为常显操作区 | 四类媒体、名称/规格/来源/时间、固定标记、配额；上传、搜索、类型/标签/来源筛选、插入到工作流、下载、查看来源执行、以来源参数打开、固定资产、分页 | [D](../images/09-assets-library.png) / [P](../images/pad/09-assets-library.png) / [M](../images/phone/09-assets-library.png) |
| 10 模型库 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 表格区域横向滚动、首列/操作列固定；许可弹层转下方表单 | 类型/版本/大小/许可/商用限制、主机下载状态、许可确认与目标；登记、搜索、类型/商用/主机筛选、下载到主机、详情、条款、取消、确认许可并下载 | [D](../images/10-model-library.png) / [P](../images/pad/10-model-library.png) / [M](../images/phone/10-model-library.png) |
| 11 公开表单 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 双栏转单列；Phone 字段单列；没有工作空间菜单 | 步骤、主题/风格/时长/补充说明、上传限制与已上传文件、验证占位、公开页面隔离边界；填写需求、上传/移除素材、验证入口、下一步确认需求 | [D](../images/11-public-form.png) / [P](../images/pad/11-public-form.png) / [M](../images/phone/11-public-form.png) |
| 12 流水线编辑器 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 画布与部署参数页签；Pad 图区域滚动；Phone 复用任务节点为只读纵向列表 | 校验、阶段、推送/MR/手动触发、主机/服务端任务、部署环境映射、产物/来源/回滚；撤销、重做、YAML、保存、添加主机任务、添加服务端任务、按阶段排版、参数页签；Phone 图编辑提示桌面端 | [D](../images/12-pipeline-editor.png) / [P](../images/pad/12-pipeline-editor.png) / [M](../images/phone/12-pipeline-editor.png) |
| 13 流水线执行 DAG | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | Pad 图区域滚动且编号置顶；Phone 复用现有任务项纵向列表，不创建第二份任务数据 | 父执行/尝试链、环境门、复用来源、审批/部署状态、日志与详情入口；取消、重新运行、重跑失败任务、审批、从此任务重跑、查看原执行、复制任务 ID、日志/产物/报告/变量/触发详情 | [D](../images/13-pipeline-run-graph.png) / [P](../images/pad/13-pipeline-run-graph.png) / [M](../images/phone/13-pipeline-run-graph.png) |
| 14 环境保护 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 规则表单与执行顺序侧栏纵向排列；Phone 字段单列 | 必需审批成员/角色、会签/超时、ref、自审/绕过、超时动作、变量下发顺序、阻断执行；取消、保存保护规则、查看执行、维护规则、前往审批 | [D](../images/14-environment-protection.png) / [P](../images/pad/14-environment-protection.png) / [M](../images/phone/14-environment-protection.png) |
| 15 发布与版本对比 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 版本并排对比转上下；发布弹层转下方表单，避免遮挡 | 旧版/草稿、新增/删除、发布者、版本名/说明、受影响触发器与执行身份确认；返回编辑器、填写发布信息、确认身份提示、取消、确认发布 | [D](../images/15-publish-diff.png) / [P](../images/pad/15-publish-diff.png) / [M](../images/phone/15-publish-diff.png) |
| 16 模型服务商凭据 | ✓ 1440×900 | ✓ 768×1024 | ✓ 390×844 | 连接/预算双栏转单列；Phone 字段单列；单价表区域滚动且首列固定 | 凭据类型/URL/脱敏密钥/模型、逐模型单价、QPS/并发、已用/预留/余额、单次成本、幂等能力；测试连接、保存、连接配置、单价及额度配置 | [D](../images/16-provider-credential.png) / [P](../images/pad/16-provider-credential.png) / [M](../images/phone/16-provider-credential.png) |

## 共享规则与边界

- 复用 `src/style.css` 的 640px 字段断点：Phone <640px；Pad 640–1023px；Desktop ≥1024px。视觉沿用现有原型，不更换设计系统。
- Pad/Phone 的工作空间左菜单和顶栏分区收进“菜单”按钮的原生 dialog 抽屉，复用原导航 DOM。Enter 打开、Escape 关闭、关闭后焦点回到按钮；公开表单本就没有该导航。
- 遵循总方案 §7.3：表格有独立 `.table-scroll` 滚动所有者、固定表头和不透明首列，操作列亦固定；筛选与分页保持在滚动区外。表格未删除任何列，700px 最小宽度（部署映射表 500px），最大滚动高度 560px，键盘可聚焦滚动区。**表格采用区域横向滚动，不转卡片**；队列和媒体网格原本就是卡片。
- 遵循 v2 §19.5：Phone 图复用原 `.node` 任务项为纵向列表，状态/来源/元数据与上下文菜单保留，显示“只读节点列表 · 在桌面端编辑”；不复制 DAG 数据。Pad 保留区域内滚动的 1000px 图面。节点可 Tab 聚焦，方向键切换节点，Enter 聚焦已有详情/日志，存在参数页签时打开该页签。所有主操作仍在标题区、页签或任务项中可到达；Phone 提示不新增业务能力，原型中的静态操作仍是静态展示，不宣称已实现后端行为。
- 多面板页使用真正的 `tablist/tab/tabpanel`，支持左右/Home/End 键。编号保持唯一：窄屏隐藏面板内的原编号移到页签上；表格内编号移到滚动提示栏；Pad 独立 DAG 编号移到图上方。Phone 图中的编号保留在纵向项目内。图例留在页面下方。
- 窄屏表单单列，保留字段、帮助、校验文案、原禁用态、脱敏与确认语义；不新增需求。静态非按钮操作项补充键盘焦点。

## 复现与验收

运行 `node docs/prd/prototypes/render.mjs`。复用仓库 Playwright，使用已安装 Chromium 的 headless 模式；禁止网络请求，不安装依赖、不下载浏览器、不打开可见窗口。

渲染器生成 48 张 PNG，并校验文件存在且 >15×1024 bytes、UTF-8 中文和 LF、编号/图例数量及唯一性、编号可见性、节点边界、页面 `document.scrollingElement.scrollWidth <= viewport width`，同时检查抽屉 Escape/焦点恢复、所有面板和面板中的原按钮、页签方向键。额外覆盖 ADR “Unified list and form migration” 的 375/768/1280/1920 宽度：16 页及全部页签无页面横向溢出，并实际横向滚动验证固定首列。

完整机器可读结果见 [render-report.json](render-report.json)。下方尺寸由该次实际渲染报告生成，单位 KB = 1024 bytes。

## 溢出与遮挡处理

- 页面级横向溢出：最终三布局 48/48 均为 0；附加四宽度回归 64/64 通过。表格与 Pad 图面仅在明确标识的区域内滚动。
- 原固定侧栏、公开表单 660+300px 网格、并排表单/对比、操作栏固定尺寸：改为抽屉、单列、换行及 `min-width:0`，不通过隐藏页面横向溢出来掩盖内容。
- Pad 执行 DAG 的编号④、Phone 表格的编号③曾在区域内被裁出初始视野：将原编号移到可见的图标注栏/表格提示栏，修复后重新生成所有截图并检查全部编号。
- 原许可/发布弹层可能遮住底层信息：窄屏转为下面的常显表单，保留取消/确认动作及底层表格/对比信息；资产菜单同样下移。

<!-- PNG-SIZES -->

| 页面 | Desktop PNG | Pad PNG | Phone PNG |
|---|---:|---:|---:|
| 01-workflow-list | 109.4 KB | 92.4 KB | 78.0 KB |
| 02-workflow-editor | 147.5 KB | 100.9 KB | 119.8 KB |
| 03-node-detail | 153.2 KB | 58.7 KB | 58.2 KB |
| 04-problems-panel | 116.6 KB | 66.5 KB | 71.7 KB |
| 05-execution-detail | 148.9 KB | 89.8 KB | 102.3 KB |
| 06-executions-queue | 121.6 KB | 106.4 KB | 96.5 KB |
| 07-human-pick | 202.3 KB | 206.9 KB | 236.1 KB |
| 08-human-edit | 110.0 KB | 92.9 KB | 89.8 KB |
| 09-assets-library | 204.5 KB | 211.2 KB | 241.4 KB |
| 10-model-library | 123.1 KB | 103.8 KB | 91.1 KB |
| 11-public-form | 82.4 KB | 78.4 KB | 74.3 KB |
| 12-pipeline-editor | 142.8 KB | 93.0 KB | 96.2 KB |
| 13-pipeline-run-graph | 131.9 KB | 106.4 KB | 108.2 KB |
| 14-environment-protection | 125.2 KB | 106.8 KB | 104.1 KB |
| 15-publish-diff | 105.3 KB | 85.5 KB | 82.9 KB |
| 16-provider-credential | 131.0 KB | 116.5 KB | 110.5 KB |

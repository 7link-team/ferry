# T11 — 应用与流水线模板

## Goal / Requirements
用户在建议下一步补应用/流水线模板后明确“继续”。交付两个模板资源的列表/详情/创建/编辑/复制/删除，以及选用模板、填写参数、生成配置、确认创建真实部署/流水线。保留当前Compose与线性流水线能力边界。

## Existing Implementation / Reuse
复用Scope与既有app-templates/pipeline-templates权限、resource::list、manifest::parse与pipelines::validate_definition、事务审计与版本CAS、共享ResourceTable/ResourceForm、已有复制与导航query机制。参数填充仅发生在创建阶段；运行时{{run}}/{{branch}}/{{repo_url}}/{{commit}}仍由现有引擎处理。

## Design / Execution Mode
Direct执行，不委派。追加app_templates/pipeline_templates两张表，公共字段id/workspace_id/name/description/content/parameters/version/created_at/updated_at。参数定义JSON数组，支持string/integer/boolean、默认值、字符串枚举/长度、整数范围；没有默认值即必填。参数名允许点分段，对应{{ .Values.image.tag }}。
内容先解析JSON/YAML对象，再遍历字符串值填充；整值占位符保留参数类型，字符串中的占位符转换成文本，键名不可含模板。不是通用Go模板引擎，没有循环/表达式/文件访问。声明、引用、参数类型和限制由服务器校验；生成结果必须经过现有业务定义校验。保存模板验证语法和参数声明，生成时验证完整业务配置。
应用secret参数接受现有变量名，仅允许整值用于valueFrom，生成variable/name引用，禁止参数值写入模板审计；不读取变量明文。流水线尚不支持密钥变量消费，明确拒绝secret参数，不伪装成已支持。
模板列表“使用”进入目标共享创建表单，加载模板版本与参数字段，生成按钮将经过服务器验证的manifest/definition填入现有编辑器；仍需用户确认创建、选择主机/仓库等。原有创建表单也提供模板选择。模板编辑后旧参数页生成返回409要求重新载入。创建对象保存完整配置，不与模板建立动态关联；后续模板变更/删除不影响对象。

## Files / Boundaries
新增templates.rs、实体与追加迁移；main注册REST路由；共享DSL/Form增加可选生成器，resources配置模板页面与生成器；tests/CI/README/完成度文档。复用现有表单字段渲染，不手写第二套资源表格或表单。不改运行执行器、桌面、版本号；市场同步、其他部署模式、流水线变量消费不在本轮。

## Testing / Acceptance
真实HTTP三库CRUD/租户与权限/版本冲突/参数校验/秘密引用位置；生成配置创建实际部署与流水线；浏览器完整创建模板、填参生成、确认创建、复制/编辑/删除；真实执行复用现有部署和流水线harness加入模板来源路径。旧数据升级、相关共享表单回归、build/typecheck/Rust/tests；只对通过验收路径更新完成度。不触发发版。

## 验收结果（2026-10-26，本地源码）
- `scripts/test-templates.py`：SQLite / MySQL 8.4 / PostgreSQL 17 各17组检查通过，覆盖CRUD、搜索/分页、复制等价创建、权限/租户、版本冲突、参数与密钥引用校验、生成后创建真实对象、修改/删除模板后的对象独立性、审计与无自动执行。
- `scripts/test-templates-ui.mjs`：Chromium实测两种模板创建/复制/编辑/删除、列表“使用模板”和普通创建表单选择、缺参、整数输入、false布尔默认、生成后参数变更阻止创建、409冲突/重新载入、最终真实对象创建；无浏览器异常。
- `scripts/test-deployments.py`：102项通过，无失败/跳过。初始应用和密钥引用均经模板生成，再通过真实runner/SSH执行Compose、HTTP服务检查、失败清理、变量轮换、加密历史回滚与浏览器验收。
- `scripts/test-pipelines.py`：52项真实runner/SSH/Git检查通过，无失败/跳过；流水线由模板生成，运行时占位符仍由原执行器处理。
- `scripts/test-backends.sh`：三库各119项通过；`scripts/test-upgrade.py`：alpha.6旧账户/会话/资源三库升级保留，新增模板表可创建并生成配置。
- Rust workspace 106项通过，1项既有Compose测试忽略（本轮真实Compose harness已执行）；前端44项通过；既有变量/工作空间管理Chromium回归通过。前端build/typecheck、Rust build/check/fmt通过。
- 初次HTTP测试错误地按字符串比较manifest，既有后端会规范化manifest；改为校验保存的有效内容并验证后续模板变更不影响保存结果。初次浏览器测试把创建按钮识别为链接，改正测试选择器后通过。未更改产品行为以迎合测试。

## Self-Review / Final Acceptance
PASS：范围与已确认需求一致；复用共享资源表格/表单、Scope权限与业务校验；未增加执行引擎、依赖、桌面行为或版本变化。事务内审计、CAS与租户隔离均实际验证。无模板密钥明文存储/读取。功能完成度按实际路径更新为18/25。全部本地验收通过；CI工作流已接入，未声称远端CI已运行。

## Delivery boundary
本轮不推送、不打tag、不发布；最新公开安装包仍是alpha.6。应用/流水线参数模板不等同于服务配置文件模板；市场、监控、定时任务、工作空间配置项、字典和导入导出仍未交付。

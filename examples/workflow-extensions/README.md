# 声明式工作流扩展包 v1

服务端支持安装独立 JSON 声明包，启动时加载，无需重新构建 Ferry。包只描述普通参数表单和固定 HTTPS API 请求，复用工作流权限、表达式、加密凭据、请求账本和执行结果。此规范不加载 JS、WASM、Lua、n8n npm 包或 ComfyUI 插件。

## 安装与启停

1. 部署管理员将审核过的包放入 `<FERRY_DATA_DIR>/workflow-extensions/*.json`。此目录由部署管理员管理，工作空间用户不能通过 API 上传扩展。不要在声明中放入密钥。
2. 重启独立服务，刷新浏览器。打开“工作流 → 节点工作流”，添加节点时搜索包名称；普通参数与创建凭据沿用已有界面。集成凭据页面也会提供扩展选项。
3. 将文件改为 `.json.disabled` 后重启即可禁用，再改回 `.json` 后重启恢复。禁用会使依赖这些类型的工作流报未知节点；不会替换成其他节点或改写保存的定义。历史执行结果保留，正在运行的任务应先结束。

同一二进制可先启动无包实例，再放入包后重启；不需要运行 npm/cargo。`public-repository.json` 是可运行示例：读取公开 GitHub 仓库，默认无需令牌。它是证明独立包加载链路的例子，不计作新增一种平台集成。

错误 JSON、错误命名、无效参数/HTTP 声明、重复节点和同版本内容变化都会阻止启动，并在服务日志中给出原因。修正或禁用错误包后重启。包目录最多100个启用包，每包最大1 MiB、1～100个节点。当前只支持单实例部署；集群配置下存在启用包时拒绝启动，避免不同实例使用不同节点目录。

## 身份与版本

- `format: 1`，包 `id` 使用小写字母开头，后接小写字母、数字或下划线，最长48字符；`version` 为正整数。
- 节点类型为 `integration.ext.<id>.v<version>.<operation>`，操作名规则同包ID；`type_version` 固定为1。包升级使用新的 `version` 和新的节点命名空间，旧版包可并存。
- 凭据 `profile` 固定为 `ext.<id>.v<version>`。扩展不能借用内置 GitHub/Slack 或其他扩展版本的凭据；升级须显式配置新版凭据。
- `<FERRY_DATA_DIR>/workflow-extensions.lock.json` 保存每个包版本的内容 SHA256。JSON缩进/键排序变化不改变摘要；修改参数、目标、请求、标签等内容须发布新版本。禁用不删除锁，不要删除此文件来覆盖已用版本。锁文件与工作流数据一起备份、迁移。

## 节点契约

参照示例文件。包顶层仅接受 `format/id/version/name/nodes`，每个节点接受 `type/type_version/params_schema/example_params/expression_fields/ui/http`；其它执行字段拒绝加载。

- 运行位置固定server，单条目输入/输出main端口、禁止缓存，副作用/恢复行为由已有HTTP执行器统一处理。
- 参数为关闭额外字段的扁平对象：string/integer/number、字符串枚举、长度/数值界限、字符串pattern及textarea。暂不接受嵌套JSON表单、布尔参数、动态选项、远程schema引用。
- `credential_id` 为必填字符串；表达式仅能用于声明的普通字段，不能用于凭据ID。`example_params` 必须通过参数校验。
- `ui` 包含资源、操作、凭据说明和搜索别名；标签使用 `literal:可读文字`，显示时去除前缀。包作者提供文本，本版未提供扩展多语言字典加载。
- `http` 支持固定HTTPS origin、path占位符、GET/POST/PUT/PATCH/DELETE、固定headers、query/body字段映射、token认证及可选success谓词；详见已有声明式HTTP执行器。query/body不能映射凭据ID。只允许GET使用可选认证。
- 网络访问是管理员批准的包能力；目录不是不可信代码沙箱。工作流参数无法修改声明的目标、方法和认证配置。请求超时30秒，响应限1 MiB，禁止重定向，未知结果不自动重发，分页每次一页。

浏览器通过工作空间的 `workflow-node-catalog` 接口读取启用节点目录；该接口只包含声明及包版本/摘要，不包含令牌或工作流数据。新增包无需修改前端节点库、参数表单或Rust业务分发。

验证：`cargo test -p ferry-server workflows::extensions`、工作流前端测试、`node scripts/test-workflow-integrations.mjs`。后者在隔离临时数据目录安装示例包，验证真实GitHub GET、凭据隔离、节点搜索与表单保存重开。

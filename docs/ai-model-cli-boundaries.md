# 模型、凭据与原生 CLI 接入边界

本期由用户要求直接填写 AI 密钥、移除跨模块 AI 凭据入口、逐行编辑模型清单、发现原生模型、接入常用 CLI，并修正超管群聊创建和首屏标题。外部产品用于交互参考，字段和权限以本仓库现有接口为准。

## 依赖与历史兼容

- 模型和工作流 AI 节点复用 `shared/forms/ai-provider-fields`，直接输入 API Key，不跳转通用凭据页面。
- 复用底层加密存储、现有 provider ID、credential ID 和费用账本。新建密钥与模型在同一事务保存；编辑留空保留，非空轮换；失败回滚。密钥不进入响应、版本定义和审计详情。
- 通用凭据不再提供 AI 范围；未指定范围的列表排除 AI 密钥。旧 API 的显式 AI 范围查询保留兼容，不删除历史数据。历史 AI 凭据 UI 书签进入模型入口，不把凭据 ID 当成模型 ID。
- 集成凭据从共享声明契约读取应用类型。凭据 UI 不加载工作流画布；后端凭据不调用工作流执行模块。扩展节点初始化时注册其声明的凭据类型，保留插件能力。
- 超管创建、维护自己群聊时不要求其本人加入空间；其他受邀用户仍须是该空间有效成员。此操作不创建空间成员记录，不放宽私聊可见范围。

## CLI 协议与能力

| CLI | 执行协议 | 模型清单 | 授权边界 |
|---|---|---|---|
| Codex | app-server JSON-RPC | 所选 Runner 原生缓存；无缓存时 model/list | 既有只读 / 单次请求授权 |
| Claude Code | stream-json 双向控制 | 无稳定独立模型目录接口，按行输入 | 既有只读 / 单次请求授权 |
| Gemini CLI | ACP v1，--experimental-acp | 不伪造账号可用模型；按行输入 | 显式 plan 或 default/ask；不继承 yolo |
| Qwen Code | ACP v1，--acp | 同上 | 同上 |
| OpenCode | ACP v1，acp | 同上 | 同上 |
| Pi | RPC，--mode rpc；agent_end 结束；--session 恢复 | get_available_models | 本期仅只读；关闭工具、扩展（包含MCP）、技能和提示模板 |

“已实现协议适配”与“已通过真实厂商推理验收”分别记录。新增 CLI 的登录状态无稳定探测命令时返回 unknown；不凭安装成功声称已登录，实际会话启动返回认证失败。CLI 版本必须支持表中命令和协议；缺少恢复、安全模式或显式模型选择能力时失败，不静默降级。工具策略属于原生 CLI 权限控制，不声称是操作系统沙箱。

Pi 0.74.2 的实测终态为 `agent_end`，不能等待未提供的 `agent_settled` 事件。Ferry 在该终态按最后一条 assistant 消息判定成功、失败或取消，随后关闭本次原生进程；本期不保证终态后的原生自动重试和上下文压缩。以隔离 HOME 和本地模拟 OpenAI SSE 验证了原生目录、返回内容及 UUID 恢复；不等同于商业模型推理验收。

已发现的历史边界：CLI 模型上的自定义 API 连接尚未接入原生执行配置，不能把连接保存成功等同于 CLI 已使用该服务商。当前原生验收使用 Runner 执行用户自己的 CLI 配置；该执行配置覆盖能力仍待处理，不列为已交付能力。

原生清单属于指定主机的执行用户，不是 Ferry 服务器全局清单，不表示其他主机同账号可用。API 模型清单通过已配置 OpenAI/Anthropic 兼容接口发现。逐行输入允许自定义模型；默认模型仍属于清单。不得内置一份猜测的账号模型列表。

用户提及的 `dsh` 尚无唯一对应的 AI CLI 仓库，未添加同名空壳。此次不接入无稳定结构化执行协议的交互终端产品，不安装系统级 CLI，不替用户登录商业服务，不复制 cc-switch 源码。

事实参考：

- [ACP 协议](https://agentclientprotocol.com/protocol/overview)
- [Gemini CLI 配置](https://github.com/google-gemini/gemini-cli/blob/main/packages/cli/src/config/config.ts)
- [Qwen Code 配置](https://github.com/QwenLM/qwen-code/blob/main/packages/cli/src/config/config.ts)
- [OpenCode ACP](https://opencode.ai/docs/acp/)
- [Pi RPC](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/rpc.md)
- [cc-switch](https://github.com/farion1231/cc-switch)：只参考逐行模型管理与连接编辑表达。

## 设计评审与验收契约

| 维度 | 本期判定 |
|---|---|
| 范围 / 不做范围 / 灰度 | 上文明确；无灰度，不修改现有品牌名，不发布远程生产 |
| 事实 / 配置来源 / 三方依赖 | 项目现有模型、凭据、空间授权与会话表；原生 CLI 协议为三方接口；用户输入模型与密钥，Runner 安装和登录属运维配置 |
| 模式 / 联动 / 流程 | 查看为文本；新建直接密钥，编辑留空保留；切 API/CLI 清除不适用草稿字段；目录发现不保存草稿，失败保留用户输入 |
| 状态 / 操作 / 异常 | 继承 enabled/disabled、会话现有状态；加载时禁重复请求，错误显示 alert；空清单允许逐行输入；未选 Runner 不发送发现请求；跨请求变化不覆盖新草稿 |
| 校验 / 计算 | 复用现有名称、URL、模型、价格、额度、币种和时区约束；Key 上限16384字节，不含控制字符；目录≤2000，单模型≤256字节；费用公式不变 |
| 权限 / 数据隔离 | 沿用模型 create/update、credentials read/use/create/update/price/budget；原生目录需 hosts.read 和 ai-agents.probe；所有主机/provider/模型按空间过滤；私聊不因超管身份公开 |
| 操作记录 / 删除保留 | 模型、密钥轮换、发现及群成员变更沿用审计；不记录密钥；不新增删除动作，不删除旧凭据或账本 |
| 模型 / 时间审计 / 索引 / SQL | 无数据库模型、字段、索引或DDL变更；不补写历史时间字段；现有记录审计沿用现有模型与活动记录 |
| 历史 / 兼容 / 冲突 | 无迁移，up/down无；旧provider和账本身份保留；Probe.models为可缺省字段，旧Runner返回空清单；新增Runtime要求升级Runner，未安装CLI明确失败 |
| i18n / 术语 / 设计系统 | 六语共享组件、主题样式、44px行操作；新错误说明覆盖zh-CN/en-US/ms-MY及其余现有语言；CLI产品名保留原名 |
| 核心影响 / 测试追踪 | 模型保存、目录发现、原生执行、群聊创建与编辑、首屏HTML标题；对应 models/tests、conversations/tests、agent/tests、assets/tests 和 test-models.mjs |
| 响应式 / Figma | 下表为布局契约；Figma暂缺，不声称设计稿已验收；浏览器截图在本地验收证据目录；虚拟键盘和原生App实机未覆盖 |
| 可测试性 / 模糊描述 | 上述规则可通过正负用例判定；无待猜测的业务公式；未核验厂商账号推理单独记录 |

| 设备 | 布局重排 | 导航/操作 | 数据呈现 | 滚动 | 安全区/键盘 |
|---|---|---|---|---|---|
| Desktop ≥1200 | 两列短字段，URL/目录整行 | 逐行增加删除，目录来源主机明确 | 只读定义列表 | 内容容器纵向 | 键盘焦点可见，操作可达 |
| Pad 768–1199 | 两列短字段，目录行自适应 | ≥44px操作，保留输入 | 超长模型在行内限制 | 不产生整页横滚 | 横竖切换不丢输入 |
| Phone 320–767 | 单列，模型输入flex可收缩 | 每行删除按钮常驻，核心操作不隐藏 | 文字换行，输入自身滚动 | 页面纵向，标题保留 | 复用CRUD底栏安全区；虚拟键盘待实机验收 |

首屏标题由 URL 语言优先、浏览器语言次之、英文兜底；使用与运行时相同的翻译目录，不添加内联脚本或放宽CSP。站点更名仅提出建议“Ferry · 自动化与协作平台 / Automation & Collaboration”，尚未改名。

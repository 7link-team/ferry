# Decisions
## T2 流水线定义
- Decision：definition 在资源 API 和数据库中使用 JSON 字符串，纯函数校验后返回结构化 Definition。
- Reason：任务明确以 &str JSON 为输入，沿用已有资源字段风格；不引入执行机制。
- Alternatives Considered：直接接收 JSON 对象；本任务不额外增加双格式接口。
- Tradeoffs：客户端需序列化定义；workdir 必填，env 和步骤 timeout_seconds 可省略，未知字段拒绝，避免误拼静默失效。
- Scope：仅 pipelines 服务端。
- Status：已实施。
## T2 shell 与模板边界
- Decision：允许 run 中换行、分号、反引号和 $()；拒绝 NUL；模板只接受四个精确名称，不折叠空格或大小写。
- Reason：run 是运维编写的 shell 程序；定义校验不充当执行沙箱。
- Alternatives Considered：禁止 shell 元字符会阻止正常构建脚本。
- Tradeoffs：执行权限及模板替换安全属于后续执行引擎设计。
- Scope：T2 纯函数校验和对抗测试。
- Status：已实施并单测通过。

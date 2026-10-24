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

## T3a 数据层基础机制
- Decision：新 locks / worker_ids 用 SeaORM 实体与 schema builder，追加迁移；保留旧主键、旧迁移及 db::id 调用。
- Reason：用户明确执行 T3a，其禁止事项限定本次范围；§18.5 全量改造属于后续工作。
- Alternatives Considered：同步重写全部数据层；超出任务范围，未做。
- Tradeoffs：仓库暂时并存既有 raw SQL 与新 ORM 查询；新增机制没有 raw SQL。
- Scope：T3a；Status：实施。

## T3a 租约与分配原子性
- Decision：10 秒心跳 / 60 秒 TTL；本地使用 Instant，从数据库写入开始时刻计时；等待发号时也检查租约。分配在事务内取得锁，并通过旧 owner/heartbeat 条件更新槽位，避免回收期间覆盖刚恢复的心跳。
- Reason：墙钟回拨不能延长本地安全期限，慢响应不能额外续命；分配失败或取消必须回滚释放锁。
- Alternatives Considered：事务外持有 TTL 锁；操作超过 TTL 时无法确保串行分配。
- Tradeoffs：锁竞争/候选槽位并发续期时明确拒绝启动，不增加隐式重试。跨实例租约仍按设计依赖主机时钟同步；复制数据目录不能同时启动同一个 instance_id。
- Scope：worker 生命周期；Status：实施。

## T3a 优雅关闭与重启复用
- Decision：优雅关闭先停心跳/发号，再将本 owner 的 heartbeat_at 置 0，保留槽位映射。
- Reason：同时满足立即释放槽位和同 instance_id 重启优先复用；若槽位已被其他实例回收，则走最小空闲槽位分配。
- Alternatives Considered：删除行会丢失重启亲和性；不释放则占用到 TTL。
- Tradeoffs：保留最多 1024 行；Status：实施。

## T3b ID 存储与边界
- Decision：重写旧迁移的 41 个业务 ID 列为 BIGINT；保留原生 SQL。State::next_id 返回 Result<Id>，失败转 503；Id 绑定为 i64，JSON 序列化为字符串。移除 db::id。
- Reason：任务书与 §18.2/18.5；逐表核对实际为 41 列（14 表），不是预估约 39 列。
- Alternatives Considered：兼容迁移、SeaORM 实体改造；均不属于 T3b。
- Tradeoffs：本地旧测试库重建；可选引用空字符串输入解析为 0，输出为 "0"，校验允许 0 回传；必需引用仍由现有校验拒绝不存在的记录。请求体沿用字符串接收并显式解析，保持既有 400 JSON 错误响应。
- Scope：T3b；Status：实施，三库首轮各 98/98。

## T3c 强类型数据访问
- Decision：16 张表的实体集中在 entities.rs；业务读写使用 Entity/ActiveModel/Select，resource::list 泛型查询保留 Spec 白名单后解析实体 Column，Condition 绑定所有值。
- Reason：任务书和设计 §18.5；不增加继承式公共模型或运行时 SQL 转译层。
- Alternatives Considered：保留字符串 SQL 辅助层、取消列名白名单；均违背任务要求。
- Tradeoffs：host_tags 尚无业务调用者，但任务要求每张表均有镜像，针对该模块以有原因的 expect(dead_code) 标注，不伪造调用。实体不派生 Serialize，HTTP 显式映射 Id。凭据列表/详情通过 Summary 投影只读取公开字段及 has_secret 计算结果，保留原查询层不读取密文的防护；共享list支持类型化投影。
- Scope：只改数据访问；迁移 DDL/执行逻辑不动，migration.rs 仅 schema 测试移除已删除的 db helper 依赖；SQLite PRAGMA 元数据检查保留注释说明。
- Status：完成；三库基线及最终各 99/99，全部门槛退出0，两次变异正确失败并恢复验证通过。

## T3d Schema builder
- Decision：已有迁移全部改为 Table/Index/Alter builder，保留名称与顺序；索引和加列通过 SchemaManager 存在性检查重跑。MySQL 设置 utf8mb4_bin。
- Reason：任务书要求删除 DDL 字符串转换，同时保留 MySQL 可启动、大小写与幂等保护。
- Alternatives Considered：直接 .text() 在 MySQL 上会生成 TEXT 而非 LONGTEXT（SeaQuery 1.0.2 源码已确认）；只使用它会缩小原容量且违反无裸 TEXT 保护。
- Tradeoffs：长列以 .text() 定义，MySQL 专用 builder .custom("longtext") 保留容量；没有 SQL 字符串改写。SQLite 的 BigInteger 正常生成 INTEGER，元数据检查适配，并以实际 MySQL 生成 SQL 验证同一41列仍为 BIGINT，不能退化为32位整数。
- Scope：T3d 迁移与其保护测试；业务、JSON、错误文案和矩阵断言不变。
- Status：完成；全部门槛退出0，新建三库各99/99，两库last/all重放无业务数据或结构变化；变异退出101且恢复后通过。

## T4 执行引擎（进行中）
- Decision：三张独立表；工作空间行的计数器在事务内 UPDATE + 读取分配 number，唯一索引兜底。触发保存定义、仓库配置与目标主机快照；队列 CAS 领取，线性执行。
- Reason：工作空间内并发发号、运行历史可复现、单实例调度约束。
- Alternatives Considered：MAX(number)+1 先查后写有竞态；凭据放 exec/env argv 会泄露，因此拒绝。
- Tradeoffs：HostChannel 捕获输出后返回（沿用既有单步输出上限），本期按步分片。取消只停止等待，远端可能继续；清理临时目录后再结束执行器。取消状态可早于清理完成。
- Scope：T4 后端、真实验收脚本与 §17 文档；不改 UI、不加 exec HTTP、不 commit/push。
- Status：主机组按已告知的最小单目标假设，触发时选直接成员中 ID 最小的在线主机并固定 host_id；无在线成员拒绝，不做多机编排。触发可提供 branch/commit 模板上下文，缺省为仓库默认分支/空字符串。完成：三库原矩阵各99/99，真实执行各44/44，两次变异正确失败并恢复，全部门槛退出0。

## T5 构建界面
- Decision：仓库、流水线、历史仅配置现有 ResourceTable/ResourceForm；DSL 新增 code 与 resetOnChange，旧字段行为不变。唯一新增页面为 PipelineRunDetail，日志分片为其内部 memo 渲染单元。
- Reason：任务明确禁止另写资源表格/表单；JSON 需等宽多行，目标切换需清空旧引用。
- Alternatives Considered：手写资源组件、修改服务端以返回关联名称，均不在范围内。
- Tradeoffs：当前 API 只返回关联 ID，界面明确标注流水线/主机/触发人 ID。日志先读取状态、再按 next_seq 排空每页200片，终态排空后停止；隐藏abort并暂停，恢复可见立即读取未完成任务；稳定对象+seq+memo保留旧日志DOM，只有在底部才跟随。
- Scope：T5 前端及验收脚本；没有服务端、依赖、ResourceTable行为改动，无commit/push。
- Status：完成。23项前端测试通过；真实Chromium→服务端→runner→HTTP Git链路通过；终态继续轮询变异被三项测试捕获，恢复后通过。

## T6 CI / release preparation
- Decision：使用四个 e2e matrix job，每个连续三轮且任一轮失败立即退出；日志与 Summary 标注轮次/run id/SHA。release needs 包含整个 e2e matrix。
- Reason：失败按 harness 定位，连续运行覆盖清理竞态；不是失败重试，不能用后续成功掩盖失败。
- Alternatives Considered：合并单 job 会延长反馈并让前项失败遮住后项；触发三次完整打包成本更高。
- Tradeoffs：三轮共享同一 runner；只能提供已实际运行的证据，不能数学证明永无 flaky。
- Scope：T6；runner exec 复用 pipeline API，UI 用临时裸仓库 fixture branch 兼容 detached HEAD；无产品 API 变更。
- Status：本地四个harness均连续三轮通过；CI 三轮和 dispatch 发布产物收集必须用户提交推送后验证。不 commit/push/tag/release。手动触发即使选择 tag 也不发布，只有 push v tag 调用发布 action。
- T6 stability finding：本地UI第2轮实际复现状态/日志轮询竞态。用fixture文件屏障同步观察与后续执行，替换固定sleep；保留原断言并重新跑连续三轮。不是重试，不改产品轮询行为。

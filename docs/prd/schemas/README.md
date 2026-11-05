# 工作流可机读设计契约

四份 JSON 契约的唯一源码位于 `src/shared/workflow-contracts`，由前后端共用。本目录只维护设计说明和参考校验脚本，不作为产品运行时依赖。下文设计范围与实施进度应分开理解：Schema 存在不代表对应执行器已经交付。

本目录当前批次为 `1.14-review / design_only` 文档交付：保留46个S1核心契约和3个S2内置数据契约，并增加4个S2文件/归档契约，保留长文本/向量键参考夹具；本批补Merge就绪/恢复参考规则。不是已安装的节点注册表、运行引擎或生产迁移。产品待确认项仍以 PRD §15.10 PDR-01～06 为准。

| 文件 | 责任与边界 |
|---|---|
| [workflow-graph.schema.json](../../../src/shared/workflow-contracts/workflow-graph.schema.json) | Draft 2020-12 图结构：节点、连接、设置、布局和固定数据；不单独证明图可运行 |
| [workflow-core-nodes.json](../../../src/shared/workflow-contracts/workflow-core-nodes.json) | 46 个 S1 规范节点的参数 Schema、端口、执行位置、缓存资格、凭据槽用途、表达式字段白名单、语义规则及参数样例；逐项映射 B001～B075 |
| [workflow-data-nodes.json](../../../src/shared/workflow-contracts/workflow-data-nodes.json) | 3个S2内置节点：data.table、vector.upsert、vector.search；B008只映射数据表，向量来源为v3 §7.4/§12.1；不指定嵌入服务商 |
| [workflow-file-nodes.json](../../../src/shared/workflow-contracts/workflow-file-nodes.json) | 4个S2文件/归档节点；格式、显式限额和安全边界见PRD §20；没有安装解析库 |
| [check_contracts.py](check_contracts.py) | 文档校验器；检查元 Schema、参数正反例、源清单映射及选定的静态图夹具，输出本地证据 |
| [check_index_contracts.py](check_index_contracts.py) | 独立SQLite内存夹具：文本候选/全文复核/分页排序、二进制约束及强制hash碰撞；不是生产存储适配器 |

源清单 75 项分期为 S1 53、S2 11、S3 1、S5 6、不对标 4。多个旧节点可映射同一规范节点，便签等映射布局能力；当前53个规范节点不表示75项全部已开发。B008及B005/B020/B021/B041/B047/B057已有parameter_contract，仍属S2，不改清单“未开始”验收状态；其余S2～S5与外部服务商/模型仍需逐项契约。`required_evidence` 是待实施的验收类别，不能当已有成功记录。运行权限、资源配额和版本适配继续由 PRD、v2/v3 及数据模型共同约束，本目录不是完整服务端插件接口。

## 参数与图的校验顺序

1. 保存草稿时检查图结构、类型版本、端口、同层无环与循环容器；节点 ID 在所在图层唯一，跨层引用必须经过容器接口。发布检查同空间引用、权限、凭据类型与必填槽，以及已冻结的节点实现是否存在。
2. `params_schema` 校验表达式解析后的 JSON 值；原始图中的表达式只能出现在 `expression_fields` 声明的 JSON Pointer 路径，`*` 表示数组项。它不是允许任意字段执行表达式的授权。编辑器须先解析表达式语法，不能直接拿解析后 Schema 拒绝合法表达式。
3. 开始一次节点尝试前，在冻结的执行上下文解析表达式，再校验参数 Schema 和 `semantic_rule` 的条件规则。权限撤销、主机 OS/版本/排空、预算和模型就绪等需重新检查；结构合格不意味着这些守卫通过。
4. 图只保存凭据 ID 引用；凭据内容由 vault 在授权执行时解析。未知字段拒绝，`default_cache` 只是契约的预期默认，不表示 JSON Schema 自动修改输入。

本批结构及参数 Schema 使用 Draft 2020-12 的 `const/type/properties/required/additionalProperties/propertyNames/items/prefixItems/enum/pattern/minLength/maxLength/minItems/maxItems/minimum/maximum/allOf/anyOf/if/then/not`。参数自由 JSON 字段仍受整个定义大小约束；`{}` 表示该字段允许任意 JSON 值，并非遗漏字段定义。禁用远程引用、动态代码和网络 Schema 获取。这里的校验器只处理已解析的样例，不执行表达式或所有 `semantic_rule`。

## v1.7 静态语义补充

Schema 中的条件规则要求 Merge fields/join 提供非空 keys，join 提供连接种类；while 提供 condition 且并行度为 1，携带 carry 时也必须串行；Split selected、Aggregate summary、Wait 三种模式、日期运算、加密/TOTP、文本分割、HTTP 分页和文件写入提供各自必填字段。比较条件的 exists 不需要 right，其余比较必须显式提供 right，null 仍是合法的已提供值。

校验器另外检查 Switch case key 唯一、分割 overlap 小于 length、人工选择 min 不大于 max、普通端口 index=0、Switch case 范围/关闭的 fallback、重复连接，以及非哈希密码学操作的 key 凭据引用。Merge 新增显式必填的 input_count（正整数，无默认值），input index 必须小于该值；样例中的 2 不是产品默认。端口数量检查不证明每个输入已连接，也不替代全前驱终态的运行真值表。

JSON 入口拒绝重复键（含嵌套对象）、NaN/Infinity 和 `1e400` 等浮点溢出；同名键出现在不同对象合法。内嵌 input_schema/response_schema 检查元 Schema，并遍历 Schema 子结构拒绝外部 `$ref`/`$dynamicRef`；普通 JSON 数据或 schema default/examples 中的 `$ref` 字符串不作为网络引用执行。局部引用目标解析、字符串时间/时区有效性、正则中断、表达式求值、候选人数、凭据真实类型和服务端权限仍需运行阶段验证。

v1.7阶段215个文档用例通过，较v1.6新增57例；这个数量不是产品验收用例执行数。

## v1.8 内置数据节点

data.table操作显式选择select/insert/update/delete：select必填limit，insert/update必填values，update/delete必须给确切row_id及expected_version。Schema拒绝不适用的操作字段；filters值使用绑定参数，列名按column_schema/index_schema白名单映射。默认id升序的after_row_id与自定义order_by不可混用；它不是任意排序的通用游标。空values对象的业务有效性、列类型/必填/null由实际表Schema判断，不能靠节点外层Schema证明。

向量节点接收已生成的数值数组，不调用服务商，不在节点覆盖集合维度/模型/metric。上下文夹具校验维度、float32溢出/非有限、cosine在转换后的零向量，包括极小值下溢为零；此夹具使用固定集合元数据，未连接真实集合。默认节点缓存关闭，显式opt-in还须绑定实际集合/表快照、授权和版本失效规则。

v1.8阶段243个文档用例通过，保留原215例，新增28例（3节点参数正反例9例、操作条件11例、向量上下文8例）。数据模型§12补SQL与事务/索引规则。容量边界用合成计数夹具检查SQL条件，不冒称10万行/2万向量的负载与并发验证。

## v1.9 长文本与向量键

数据表iN现在是严格UTF-8前128字节的二进制候选键，完整文本始终保留在data。128是内部索引派生长度，不是用户输入上限。eq/范围候选必须复核全文；ne不能排除整个相同前缀桶。精确total、分页和同前缀桶排序均在完整原值比较后产生；不能按SQL候选LIMIT直接返回最终页面。order_by显式必填nulls=first/last，复用核心Sort语义，不新增默认值。原始值的大小写、Unicode组合形式与NUL均保留。

向量item_key改为完整原键的规范JSON字符串封装（TEXT），读取后解码比较/输出，转义NUL以兼容PG TEXT；hash仍根据解码后原键计算。新增32字节item_key_hash及非负key_slot；查询hash桶后在集合锁内比对原键，碰撞分配不同槽。唯一索引约束的是物理桶槽，不能代替服务层完整键唯一与并发集合锁。SQLite BLOB/PG BYTEA/MySQL VARBINARY的具体映射及迁移边界见模型§13；没有执行真实PG/MySQL适配或迁移。

当前参数/图契约245例通过（原243+显式null排序2例），索引参考夹具32例通过。复现索引夹具：

```powershell
python -X utf8 docs/prd/schemas/check_index_contracts.py
```

结果写到本地 `data/maintenance/workflow-index-validation.json`，并按内容SHA256保留 `workflow-review-evidence/indexes-<hash>.json` 快照。该夹具只使用SQLite内存库和参考算法，不连接产品实例；不证明三库实际排序/锁隔离、历史迁移、生产查询性能及当前产品功能已实现。

缓存按 v3 §18.9 精化 §6.1：人工、触发、等待、发送和响应节点禁止缓存；其他副作用仅能按明确 opt-in 规则参与，不能统一当作纯节点。JS 的 deterministic 声明必须由受限沙箱兑现；循环纯度取决于整个 body。文档声明不证明沙箱确定性已实现。

## 复现校验

在仓库根目录执行，依赖仅安装到被忽略的本地维护目录：

```powershell
python -m pip install --target data/maintenance/workflow-schema-python jsonschema==4.26.0
python -X utf8 docs/prd/schemas/check_contracts.py
```

最新结果写入 `data/maintenance/workflow-contract-validation.json`；当前及被替换的结果按内容SHA256保存在本地 `data/maintenance/workflow-review-evidence/contracts-<hash>.json`，不覆盖已有快照。计数及逐例结果以实际输出为准。通过仅证明文档 Schema 和夹具符合所检查规则；不证明真实表达式、完整条件语义、权限/数据库并发、OS 沙箱、主机/第三方执行、三端页面或六语言验收通过。主机 Git 的 depth/submodules 等字段是目标契约，不能据此声称当前 checkout 已支持。

## v1.10 文件与归档契约

新增file.archive_pack、file.archive_unpack、file.encode、file.decode共4个S2节点，逐源映射B005/B020/B021/B041/B047/B057。输入输出基数、逐格式有损边界、CSV转义、XLSX日期、归档安全、资源计数与失败恢复见[PRD §20](../ferry-工作流平台PRD.md)。显式参数限额不提供未经批准默认值；部署硬限制和解析器版本尚未冻结。

v1.10阶段check_contracts.py共303例（原245+新增58）检查53套manifest、75项源映射与PRD逐源名称、格式条件、虚拟路径/重复/父子冲突、CSV分隔/引号字符及sheet名称。样例与负例使用已解析参数；它没有读取ZIP/PDF/Office文件，也没有执行文件字节转换、流式资源守卫、权限查询或OS隔离。required_evidence表示未来实测要求，格式往返不能由Schema通过替代。索引参考夹具保留32例。

## v1.11 Merge就绪与恢复参考

core目录新增merge_readiness：16行两输入真值表、符号定义、恢复身份及输入投影字段，规范语义见[PRD §21](../ferry-工作流平台PRD.md)及[模型 §14](../ferry-工作流平台数据模型.md)。manifest参数与53节点/75源成员不变。现有参数正反例全部保留，v1.11阶段check_contracts.py共376例（原303+73）；两输入决策32例、N输入/到达顺序1组、门槛10例、归一16例、非法投影5例、SQLite最小事务9例。

merge_projection消费的是已提交的所连端口投影状态；分支的S需要从选择/跳过记录构建，不能拿If整体succeeded+空数组猜测。merge_readiness输出wait/hold/stop等只是参考决策标签，不新增产品状态。SQL夹具只验证手工给定快照下的局部事务条件，未执行调度器、真实崩溃恢复或完整Merge数据运算。多来源同端口、error_output未选main的E/S、position/fields/join值冲突及重复键规则仍未冻结。

## v1.12 内嵌Schema编译参考

v1.12阶段check_contracts.py共430例（原376+54），53个manifest/75源映射保持。核心目录embedded_schema_contract给出单资源2020-12设计配置、完整关键字及子位置分类、显式夹具预算；规则见[PRD §22](../ferry-工作流平台PRD.md)。局部$ref解析到真实Schema位置，支持Pointer与唯一anchor，校验前向/转义/百分号/false目标；引用环只检测不展开，不能当运行终止保证。

本设计配置拒绝$id资源切换、动态词汇、其它方言及未知关键字；普通default/const/examples数据不作为引用指令。format/content*为注解，默认值不执行。预算数字只是文档例值，运行部署配置和整图聚合预算仍待冻结。校验器不执行不可信实例的断言、正则中断、OS资源隔离或提交事务，具体边界以PRD为准；历史§v1.7“局部引用未验”现在由本阶段静态范围部分覆盖。

## v1.13 凭据逻辑槽与授权投影

核心目录credential_contract及data.crypto/key、net.http/auth、net.smtp/smtp补逻辑profile和操作条件，规范见[PRD §23](../ferry-工作流平台PRD.md)。存储kind与profile分开，现有host/build/deploy scope保持，不新增workflow枚举；适配器配置存储和工作流module绑定尚未冻结。缓存投影复用凭据行version避免同ID轮换旧结果，不读/不hash秘密；所有写路径仍需实际审计。

v1.13阶段check_contracts.py共473例（原430+43），53manifest/75映射保持。credential_projection只用合成当前资格元数据检查同空间/module/身份/profile与版本，输出非敏感分区；不是实际鉴权、解密、缓存查找或外部调用实现。4个阶段撤权例是反复调用参考资格函数，不是实际发布/队列/缓存的端到端证据；type/profile相容不证明实际payload、远端认证或OAuth刷新可用。

## 表达式预览 HTTP 接口

`POST /api/workspaces/{workspace}/workflows/{id}/expression-previews` 使用当前空间的 `workflows.update` 权限和工作流对象编辑授权。请求中的 `version` 必须匹配草稿；`node_path` 只接受 `/nodes/0` 或 Loop 内 `/nodes/0/params/body/nodes/0` 这样的节点路径。可选 `params` 是未保存参数；省略时使用保存的节点参数。

```json
{
  "version": 1,
  "node_path": "/nodes/1",
  "params": {"mode": "duration", "duration_ms": "={{ $json.duration }}"},
  "data": {"items": [{"json": {"duration": 1000}}], "upstream": []},
  "selections": [0]
}
```

`data` 是调用方提供的样例快照，不会按其中的节点或资产标识读取持久化执行数据或资产字节。`selections` 指定每份解析参数对应的输入条目，`null` 表示批次模式，可使用 `$input.all()`，但不存在隐式当前条目。上下游条目和资产引用沿用图数据契约。`upstream` 每项为 `{node,name,iteration_hash,items}`，来源身份不可重复。

服务端只执行 manifest 的 `expression_fields`，只求值一次，随后用原始 `params_schema` 和已实现的参数语义检查结果；不把字符串数字自动转为数字。一批中任一表达式或参数失败，整批失败。上下文提供当前空间 `global` 范围的普通变量、节点/工作流信息、`$execution.mode="preview"` 和冻结的 `$now`/`$today`；禁止客户端覆盖上下文，密钥、构建/部署变量和主机环境不进入表达式。

每次预览由服务端固定一个毫秒时间戳，同批条目共用。`settings.timezone` 接受 IANA 时区，省略时使用服务端本地时区（`zoneName="local"`）；无效值以400定位 `/settings/timezone`，不回退。`$today` 是该本地日期首个有效时刻：重复午夜取较早瞬间，午夜被跳过则取当天首个有效秒。只读日期对象提供 `toISO()`、`toISODate()`、`toMillis()`、`toString()`、`toJSON()`、`valueOf()`，以及 `zoneName`、`offset`（分钟）、`year/month/day/hour/minute/second/millisecond`。直接结果与 JSON 序列化为 ISO 字符串，模板插值直接插入 ISO 文本；数值运算使用毫秒。对象、原型及方法冻结，确定性沙箱仍无 `Date` 或主机时钟；此接口不承诺完整 Luxon API。未来图执行须持久化尝试的冻结时间，恢复时复用；当前预览不创建执行记录。

成功返回 `{items,total,page,page_size,version}`，`items` 按 `selections` 顺序包含解析后的参数。版本冲突为409，权限拒绝为403，跨空间或非法节点路径为404，参数/语法错误为400，计算预算耗尽为429，隔离服务不可用为503；字段错误使用六语言 JSON Pointer 定位。预览不运行节点执行器，不写草稿、历史、执行或审计数据；它不替代保存、发布或执行前的其它守卫。真实 HTTP 验收脚本为 `scripts/test-workflow-drafts.py`。

## 节点尝试持久化（内部）

内部节点尝试持久化由 `workflows::journal` 承接：使用正式运行表，写事务先按执行租约和 version 锁定父行，最后一次 SQL 再验证租约未过期；参数、冻结时刻和确切来源保存在执行/节点身份 AAD 加密的快照中，输出按端口保存“跳过”和“执行但为空”的区别。接管返回原尝试，不重新选择参数或来源；重复完成保持既有版本和终态。计时等待保存加密检查点，所有运行节点排空后才可挂起，唤醒仍使用既有 `not_before` 租约门槛。

参数快照与节点输出的规范JSON超过64KiB时，通过`content_assets`外置；执行列保存绑定原执行/节点/字段AAD的加密定位信息，`output_asset`与输出定位信息一致。资产对象使用workspace/id/hash AAD独立加密，明文hash只在同空间去重；引用随节点结果同事务提交，重放不重复累计`data_bytes`。内联阈值与原1MiB条目、16MiB输出限制不变；执行context尚保持内联。

存储后端复用已确认D2的`object_store`，默认local；S3构造使用`FERRY_ARTIFACT_STORE=s3`及`FERRY_ARTIFACT_S3_ENDPOINT/BUCKET/REGION`，凭据读取标准AWS环境配置。该构造已用于内部资产接入及下述手动执行首批入口/调度；手动运行的实际S3端到端与流水线产物读写仍待验证/接线，不能据此宣称现有流水线已经使用S3。资产写入前提交配额预留与24h候选；短资产锁串行引用、固定和开始删除，物理删除成功后才清元数据，错误保留deleting重试。对象键包含资产代次，后续同hash新上传不会复用旧删除请求的物理键。生产后台GC派发、平台配额配置与资产API/界面仍待交付。

该层的参数快照与节点输出已接入上述资产外置；原1MiB/条目、16MiB/节点输出和64KiB外置边界不变。手动执行入口、顶层计划/调度与权限复查的首批实现见下节；超时、费用预算、历史清理及外部副作用/多实例集群恢复仍须交付，内部数据库测试不能替代这些端到端验收。

## v1.14 图阶段与拓扑诊断

### 发布版本账本 HTTP 接口

`GET/POST /api/workspaces/{workspace}/workflows/{id}/publications` 分别读取不可变
发布记录和发布当前草稿。POST 接收 `version`、`revision_id`、`name`，可选
`change_note`、`request_key`、`previous_run_as`，拒绝其它字段。名称 trim 后1～64字符，
按精确大小写在定义内唯一；说明最多200字符，请求键为1～64字符。发布同时要求实时
空间与对象的编辑、运行权限，run_as 只能为调用者；与上次发布者不同时，必须用
previous_run_as 明确确认旧身份。生产身份及修订指针、账本与审计在空间锁内同事务更新。

新记录返回201；同一请求键及相同修订/名称/说明/发布者返回原记录与200，不再次切换
生产指针，版本或确认值不是重复变更的依据。变更载荷/发布者复用请求键返回409。
重复请求仍重新鉴权。列表沿用分页封装与对象 View，包含发布者及不可变修订信息，
不返回请求键、内部名称键或定义内容。唯一索引在 MySQL 显式使用 utf8mb4_bin。

`POST /api/workspaces/{workspace}/workflows/{id}/unpublication` 仅接收当前 version，
要求空间及对象编辑权，清除生产指针并保留历史及上次身份。归档后不能新增发布；恢复
仍为未发布草稿。普通保存不改变生产指针或身份，正在执行的快照不随发布而改写。

当前发布门槛复用图校验、计划与实际执行器支持检查；只支持已交付的平面手动图，
不执行租户参数表达式，生产校验忽略手动 pin_data。尚未实现的节点、主机/凭据适配、
自动触发与流水线发布仍待接入，不以创建账本代替这些能力。实际验证结果见 MEMORY。

### 归属转让 HTTP 接口

`POST /api/workspaces/{workspace}/workflows/{id}/owner-transfers` 接收
`{"owner_id":"十进制成员ID","version":1}`，拒绝其他字段。仅当前 owner 在空间
`workflows.update` 与对象编辑权限内可调用；平台管理员也不能代替其他 owner 转让。
目标必须为同空间活跃成员、活跃账号，并已具备空间及对象编辑权限；分组与直接授权
沿用现有并集判定。不自动增加授权，旧 owner 也不自动获得保留授权。

工作空间锁内重新读取权限、检查版本、CAS 更新归属并同事务记录 transfer 审计，包含
旧/新 owner ID 和版本。成功返回 200 及更新后的 Workflow；版本冲突 409、缺少权限
403、非成员目标 400、跨空间对象 404、未知请求字段 422。普通草稿更新不能修改
owner；转让不更改修订、发布指针、生产身份或在途执行。接口验收以 MEMORY 及原始
HTTP 证据为准，不表示 UI 或流水线转让已经完成。

核心目录graph_publication_contract及[PRD §24](../ferry-工作流平台PRD.md)区分结构参考、完整参数、发布评审及显式触发入口。structure只放过缺必填凭据/host绑定，仍检查解析后参数；不能冒充真实草稿API。拓扑诊断只分析结构合格的顶层图并返回warning，缺Merge端口/同端口多来源/未选入口依赖不伪造E/S或自动升级Q06发布错误；循环body接口、model/main冲突与typed运行支持保留待冻结。

当前check_contracts.py共505例（原473+32），53manifest/75映射不变。新增16端口关系、5阶段区别、9拓扑/入口及2个Unicode固定数据容量边界；当前数据类型仍为main，typed/GPU例只测参考关系。pin_data按严格UTF-8封装计1MiB，不以ASCII转义后的长度误拒中文。该参考检查不包含运行发布事务、触发器或生产计划构建，问题面板/六语言仍未验。

## 手动图执行首批接口（实现范围）

节点尝试执行预算沿用v3既定默认5000次：新增尝试前，在父执行的版本/租约写锁内查询并检查总尝试数，覆盖重试和并发尝试；第5001次拒绝，不提交节点或执行计数。恢复同一已登记尝试不重复消耗次数，预算已满仍可恢复；超过预算终结执行的短码为`budget_exceeded`，不把消息文本写入状态码。本限制与单条目1MiB、单节点输出16MiB、64KiB资产外置及工作空间资产配额分别生效，不据此宣称AI费用预算或新的单执行总字节配额已经实现。`timeout_seconds`是否计入等待/停机时间仍须明确，首批继续拒绝非零超时设置。

`POST /api/workspaces/{workspace}/workflows/{id}/executions` 创建执行并返回201摘要：

```json
{"mode":"manual","revision":"draft","version":1,"entry_node_id":"start","inputs":{"answer":41}}
```

`revision` 可为 `draft` 或 `published`，后者必须已有发布指针；本接口不负责发布。`version` 是当前工作流CAS版本，入口必须显式选择可达的 `core.trigger.manual`；输入按其原JSON Schema验证，不强转类型、不加载外部引用。接口复查当前工作空间 `workflows.run` 与对象运行权限；资产准备后在写事务内再次复查权限、归档状态和修订指针。执行编号由工作空间锁递增，不用MAX。冻结输入、普通变量和计划保存在绑定执行身份的加密资产中，引用与执行同事务提交。

`GET .../workflow-executions/{id}` 返回状态、修订、编号、计时和数据字节摘要；`GET .../workflow-executions/{id}/node-runs` 返回 `{items,total,page,page_size}` 的节点元数据，不返回参数、输入或输出明文。两者均复查 `workflows.read` 与工作流对象查看权限，跨工作空间404。

首批仅运行手动入口、`data.set` 和 `core.wait` 的duration/until计时模式。计划只消费所选入口的可达图，以节点ID稳定拓扑排序；缺少输入、同端口多个来源或依赖另一入口明确拒绝，不用空数组补洞。上述节点支持`active`、`mute`和`bypass`：静音不求值参数、不执行，提交`skipped`端口并向下游传播；透传不求值参数、不执行节点逻辑，原样传递输入条目，记录`bypassed`，下游继续。上游已跳过时，透传节点也保持跳过。模式保存在加密尝试快照中；旧快照缺省仍为active，重启恢复不重放已完成的透传。always_output仅在active节点实际执行成功且输出零项时生成一个空Item（空json/binary/paired）；已有条目保持原样，不补额外条目。跳过、透传、等待或失败不会因此产出正常条目。execute_once只处理首条输入，参数仅对此条求值，input.all仍读取完整输入，bypass不截断透传数据。手动/测试执行的active节点可使用不可变修订中的pin_data，保留原条目与空数组并记录pinned；不求值原参数，也不因always_output伪造条目。mute与上游跳过仍跳过、bypass仍透传，生产模式忽略固定数据。固定数据的二进制引用在草稿修订事务中复用资产的空间/hash/mime/size与对象存在性检查，并以pin引用保留到原修订；后续草稿删除固定数据不移除历史引用。运行输出再登记node_run引用，恢复不重放已完成pinned节点。Loop、其它节点执行器、非stop错误策略、重试多次、超时及数据保留策略尚未接入，含这些设置的执行明确拒绝，草稿仍可保存。图并发组和错误工作流也不在首批运行范围内。

调度复用SQL租约和监督器；启动身份、执行ID、epoch的SHA256作为64字符锁owner，兼容三库锁表约束。节点提交复查触发者当前运行权限，恢复复用原修订、参数、来源和冻结时钟。计时等待先持久化、排空后释放租约，恢复原尝试并累计等待时间。真实双库检查在 `scripts/test-workflow-drafts.py`，覆盖等待状态重启，以及51节点资产数据链处于运行状态时强杀：停机后直接查询持久化running/epoch，等待真实30秒租约过期，再核对新epoch、首次开始时间、原尝试身份和已完成节点的精确元数据。此新恢复用例的观察窗口为30秒租约加60秒处理，普通两节点检查仍为15秒；首次45秒窗口内已接管但未处理完的失败证据保留，不把恢复测试当吞吐达标证据。该路径不代表外部副作用恰好一次、主机进程恢复、PG/多实例集群或整个图引擎验收；最新实际结果见 `MEMORY.md`，不由本文宣称通过。

后续执行器已扩展至 no_op、limit、sort、split、execution 范围 deduplicate、rename、
text_split、Filter/If/Switch 与 append Merge，实际验收证据见 MEMORY。core.stop 按
batch 求值，读取输入使用 `$input.first()`/`$input.all()`；其 active 模式通过同一
失败事务终结节点与执行，message/details 只存加密快照，公开仅短原因码与本地化通用
提示。mute/未选分支不求值并持久化零端口 skipped 输出；新快照从 manifest 冻结
outputless，普通及旧快照继续拒绝零端口包。always_output 不能吞掉 Stop 失败。
Stop 的 pin/bypass 因输出路由未定义明确拒绝；此处不声明其他错误策略或整个引擎完成。

# 预览穿透（Preview Holes）设计

> 2026-11-10：第17节为当前实现契约；前文路径代理、Host白名单和strip_prefix为历史设计，不再用于新预览。实际验收状态以MEMORY.md为准。

状态：IMPLEMENTATION。设计自审通过，用户授权按推荐实施（2026-11-09），§15 Q1–Q4 按推荐采纳。
日期：2026-11-09

修订提示（2026-11-09）：用户提出改用 `token.<预览域名>/path` 并移除路径前缀开关。§17 记录本轮方案；下文路径模式为已交付版本的历史契约，不代表域名模式已经实现。

## 1. 目标

把 Runner 所在主机上的本地 HTTP 服务（API、Web 原型）通过
`https://<Ferry 地址>/holes/{token}/...` 临时暴露出去，用于远程预览。
一条预览对应一个 Runner 主机上的 `127.0.0.1:{端口}`。

已确认的决策（用户，2026-11-09）：

| # | 决策 |
|---|---|
| D1 | 不引入 Rathole，复用现有的 Runner 会话通道（`Control::OpenSession` → `/api/runners/sessions/{id}`） |
| D2 | 采用路径路由，不分配额外域名或端口 |
| D3 | 预览页与控制台同源，因此**强制 CSP sandbox（不含 allow-same-origin）**。接受的代价是被预览应用不能使用 Cookie、localStorage 和 IndexedDB |
| D4 | 凭 token 公开访问，默认 24h 过期，可以撤销 |
| D5 | 只转发 Runner 本机回环地址，不能当作访问内网其他地址的跳板 |
| D6 | 连接自动释放、不泄漏；自动监控；远程状态与本地服务状态实时同步（用户补充要求） |

本文推荐、尚未被用户单独确认的默认值见 §15。评审时可以否决。

## 2. 范围

**本期做：**
- `holes` 资源：创建、列表、查看、编辑（名称、路径模式、续期）、撤销、删除。
- 公开路由 `/holes/{token}/...`：HTTP/1.1 反向代理，支持 WebSocket 升级（如 Vite HMR）。
- 新增 Runner 会话类型 `Forward`，带能力协商。
- 自动探测本地端口是否可达，状态写回列表。
- 连接上限、超时、撤销和过期断开、Runner 升级时断开。
- 六语言 i18n、权限、操作审计。

**不做：**
- 子域名或独立 origin。
- 改写 HTML/JS 里的绝对路径。
- Service Worker 注入。
- SSH 主机（`connection = ssh`）。
- 转发非回环地址。
- 原始 TCP/UDP 暴露。
- 上游是 HTTPS 的本地服务。
- 访问密码或登录保护。
- token 轮换（删除后重建即可）。
- 逐请求访问审计。
- 带宽限速与流量统计。
- 过期记录自动清理。
- 集群模式下跨节点汇总活跃连接数。

## 3. 架构

```
浏览器 ── GET /holes/{token}/x ──► Ferry server（任一节点）
   1. sha256(token) 查 holes：未撤销、未过期 → (workspace, host_id, port, strip_prefix)
   2. 连接池取空闲上游连接；没有就新建：
        RunnerChannel::session(Forward)  ── Control::OpenSession ──► runner
        runner 回拨 /api/runners/sessions/{id}，收到 ForwardStart{port}
        runner 连接 127.0.0.1:port（失败时再试 [::1]:port）→ 回 ForwardReady 或 Failed
   3. 改写请求头 → hyper http1 client 在会话 Socket 上发送请求 → 流式回传响应
   4. 改写响应头（强制 sandbox CSP 等，见 §6）
```

- **Runner 侧**只搬字节：TCP ↔ WS 二进制帧，不理解 HTTP。
- **Server 侧**通过 hyper-util 的 legacy `Client` 加一个自定义 connector 实现连接池。connector 的作用是“为某条 hole 开一个 Forward 会话，并把它包装成 `Read + Write`”。连接复用、keep-alive 和升级都由框架处理，不自己写 HTTP。
- **集群模式**：`RunnerChannel::session` 已经能跨节点 bridge，直接复用。
- **租约**：`RunnerChannel` 持有的 `reservations::Lease` 只覆盖“建立会话”这一步。会话建好后，channel 和租约立即释放，长连接不阻塞 Runner 升级。

## 4. 协议变更（`crates/protocol`）

| 变更 | 说明 |
|---|---|
| `Hello.supports_forward: bool`，`#[serde(default)]` | 旧 Runner 缺这个字段时视为 false |
| `SessionKind::Forward` | 新的会话类型 |
| `Session::ForwardStart { port: u16 }` | server → runner，第一帧 |
| `Session::ForwardReady` | runner → server，本地连接已建立，之后进入二进制透传 |
| 失败 | 沿用 `Session::Failed { error }` |

`wire::VERSION` 不变，因为全部是增量变更。`SessionKind::Forward` 只发给声明了 `supports_forward` 的 Runner。门控在 `channel.rs` 的 `session()` 和 `cluster_open()` 两处，与 `AgentProbe` 的做法一致。

**Runner 侧：**
- `port == 0` 直接返回 Failed。
- 本地连接超时 5s。
- 任一方向读到 EOF 或出错，就关闭双方。
- Runner 收到停止或升级信号时，所有 Forward 会话立即关闭。现在的实现在 `crates/runner/src/runtime.rs:155` 会等全部在途会话结束，必须让 Forward 会话监听停止信号，否则预览长连接会让 Runner 永远停不下来。

## 5. 公开路由与请求处理

**路由**注册在 `main.rs` 的顶层 app，和 `/api/runners/connect` 等公开路由放在一起，不放在 `/api/workspaces` 下：
- `ANY /holes/{token}`：308 重定向到 `/holes/{token}/`。
- `ANY /holes/{token}/{*rest}`：代理。

**`guard()` 调整**（`main.rs:1129` 起），只对 `/holes/` 前缀生效：
- 不要求 `X-Ferry-Request` 头。被预览应用自己的 POST 不会带这个头。
- 不做 Origin 校验。沙箱页面发出的请求带 `Origin: null`。
- Host 白名单（`FERRY_ALLOWED_HOSTS`）和 drain 期间返回 503 照常执行。

**请求头改写（发往上游前）：**

| 头 | 处理 | 原因 |
|---|---|---|
| `Cookie` | 整个删除 | 沙箱页面没有自己的 Cookie，这里只会是 Ferry 域的 Cookie（包括会话 Cookie），不能泄露给上游 |
| `X-Ferry-Request` | 删除 | 内部头 |
| `Referer` | 删除 | 沙箱页面本来就不发送；删除后上游也看不到预览链接 |
| `Host` | 改为 `localhost:{port}` | 开发服务器（如 Vite）会校验 Host 白名单 |
| `Origin` | 存在时改为 `http://localhost:{port}` | 开发服务器的 WebSocket Origin 校验 |
| `X-Forwarded-Host` / `X-Forwarded-Proto` / `X-Forwarded-Prefix` | 设置为对外地址和 `/holes/{token}` | 供支持这些头的应用生成正确链接 |
| 路径 | `strip_prefix = true`（默认）时，去掉 `/holes/{token}` 再转发；为 false 时原样转发 | API 和相对路径应用用 true；配置了 `--base /holes/{token}/` 的应用用 false |

**请求体**流式转发，上限 64 MiB，超出返回 413。

**日志脱敏**：请求日志（TraceLayer）和所有 tracing 字段中，`/holes/{token}` 一律记为 `/holes/***`，不记录 token 明文。

## 6. 响应头策略（安全核心）

| 头 | 处理 |
|---|---|
| `Content-Security-Policy` | **强制覆盖**为 `sandbox allow-scripts allow-forms allow-popups allow-modals allow-downloads`，删除上游自带的 CSP 和 CSP-Report-Only |
| `Set-Cookie`、`Service-Worker-Allowed`、`Clear-Site-Data` | 删除 |
| `Access-Control-Allow-Origin` | 强制设为 `*`，删除上游的 `Access-Control-Allow-Credentials`。沙箱页面是不透明 origin，`<script type=module>` 和 fetch 都需要 CORS |
| 预检请求（OPTIONS，且带 `Access-Control-Request-Method`） | 由 server 直接返回 204：回显请求的方法和头，`Max-Age: 600`，不转发给上游 |
| `Location` | 指向 `http://127.0.0.1:{port}`、`localhost:{port}` 或以 `/` 开头的绝对路径时，改写到 `/holes/{token}/...` 下 |
| `Cache-Control` | 保留上游的值；上游没给时由 `guard` 补 `no-store`（现有行为） |

**验收需要实测证明的三点：**
1. 预览页 `fetch('/api/...', {credentials:'include'})` 拿不到 Ferry 数据。
2. 上游收不到 Ferry 的 Cookie。
3. 预览页无法注册 Service Worker。

## 7. 状态与流转

`holes` 没有存储的状态列，状态由字段推导：

| 状态 | 条件 |
|---|---|
| `active` | `revoked_at = 0` 且 `expires_at > now` |
| `expired` | `revoked_at = 0` 且 `expires_at <= now` |
| `revoked` | `revoked_at > 0` |

| 操作 | 允许的状态 | 结果 | 非法时 |
|---|---|---|---|
| 创建 | — | active | — |
| 编辑名称或路径模式 | active、expired | 字段更新 | revoked 返回 409 `holes.revoked` |
| 续期（编辑时选择有效期） | active、expired | `expires_at = now + 所选时长` → active | revoked 返回 409 |
| 撤销 `POST …/holes/{id}/revocations` | active、expired | revoked（终态，不可恢复） | 已撤销返回 409 |
| 删除 | 任何状态 | 物理删除 | — |
| 探测 `POST …/holes/{id}/probes` | active | 立即探测一次，回写探测状态 | 非 active 返回 409 |

**探测状态**（`probe_state` 列）：
- `unknown`：还没探测过。
- `reachable`：本地端口可连接。
- `unreachable`：Runner 在线，但本地端口拒绝连接或超时。`probe_error` 记录原因。
- `runner_offline`：Runner 不在线。
- `unsupported`：Runner 版本不支持 Forward，需要升级。

## 8. 连接生命周期：自动释放与防泄漏（D6）

每条上游连接注册到进程内的 `HoleConnections` 表（按 hole 计数），用 RAII guard 确保连接结束时一定注销。

| 场景 | 行为 | 时限 |
|---|---|---|
| 浏览器或上游任一侧关闭 | 关闭另一侧，注销连接 | 立即 |
| 连接池中的空闲连接 | 关闭并注销 | 空闲 30s |
| 等待上游响应头 | 返回 504 并关闭 | 60s |
| 建立会话（复用现有 `DIAL_TIMEOUT`） | 返回 502，pending 项被移除（现有机制） | 20s |
| 长连接（WebSocket、流式响应、keep-alive）期间撤销、删除或过期 | 在上游连接这一层，每 30s 重新校验 hole 行，失效就关闭底层会话，普通请求、流式响应和 WebSocket 都覆盖（集群模式下同样有效） | ≤ 30s |
| Runner 断线或升级 | 会话 Socket 关闭，连接注销 | 立即 |
| 单条 hole 并发上游连接超过 32 | 返回 503 `holes.busy` | — |
| 单个 Runner 主机的 Forward 连接超过 128 | 返回 503 `holes.busy` | — |
| 新请求时 hole 无效（不存在、过期或撤销） | 返回 404，三种情况的响应体完全相同，避免泄露 token 是否存在 | 立即 |

以上上限和时限都是代码常量（见 §11 配置来源）。

## 9. 自动监控与状态同步（D6）

- **后台探测循环**：每 15s 一次，用 `lock::acquire` 保证集群里只有一个节点在执行，结构与 `hosts/monitor.rs` 的循环一致（`drain` 时退出）。
  - 每轮最多选 200 条 active 记录。
  - Runner 不在线或不支持 Forward 的，直接写对应状态，不发起会话。
  - 其余的发 `ForwardStart`，收到 Ready 或 Failed 后立即关闭会话。
  - 回写 `probe_state`、`probe_error`、`probed_at` 和 `probed_by = 0`（系统）。
- **被动更新**：真实代理请求建立连接成功或失败时，同样回写探测状态。只在状态变化时写库，避免每个请求都写库。
- **列表实时性**：前端没有推送通道，复用 ResourceTable 的自动刷新，本页默认每 5s 刷新一次。如果现有配置不支持设置页面默认刷新间隔，就给共享组件加这个配置项，不 fork 组件。
- **活跃连接数**：列表返回服务当前请求的节点在内存里的计数。单节点部署时准确；集群模式下只是该节点的数，界面会注明。

最坏情况下，本地服务停掉后状态在约 20s 内反映到界面上（15s 探测间隔加 5s 刷新间隔）。

## 10. 数据模型

表 `holes`：

| 字段 | 类型 | 必填 | 默认 | 约束和说明 |
|---|---|---|---|---|
| id | BIGINT | 是 | snowflake | PK；JSON 中是字符串 |
| workspace_id | BIGINT | 是 | — | 由 Scope 决定，不从请求体读取 |
| host_id | BIGINT | 是 | — | 必须是同一 workspace 下 `connection = 'runner'` 的主机；创建后不可改 |
| name | VARCHAR(128) | 是 | — | 去掉首尾空白后长度 1–128 |
| target_port | BIGINT | 是 | — | 整数 1–65535；创建后不可改 |
| strip_prefix | BOOLEAN | 是 | TRUE | |
| token_hash | VARCHAR(64) | 是 | — | sha256(token) 的十六进制；唯一 |
| token_sealed | TEXT | 是 | — | 用 `vault.seal(token, aad(workspace, "holes", id))` 加密，供有 `holes.read` 的人重复复制链接 |
| expires_at | BIGINT | 是 | — | 毫秒时间戳。**这是截止时间而不是操作事件，因此没有对应的 `_by` 列**（§15 Q1） |
| revoked_at | BIGINT | 是 | 0 | 0 表示未撤销 |
| revoked_by | BIGINT | 是 | 0 | |
| probe_state | VARCHAR(16) | 是 | 'unknown' | 取值 unknown / reachable / unreachable / runner_offline / unsupported |
| probe_error | VARCHAR(512) | 是 | '' | 超长截断 |
| probed_at | BIGINT | 是 | 0 | |
| probed_by | BIGINT | 是 | 0 | 0 表示系统 |
| created_at / created_by | BIGINT | 是 | — / 0 | |
| updated_at / updated_by | BIGINT | 是 | — / 0 | |
| version | BIGINT | 是 | 1 | 乐观锁；PATCH 必须带上，不一致返回 409 |

token：32 字节随机数的十六进制（64 个字符），复用 `hosts/runner/enrollment.rs` 中的 `secret()` 和 `fingerprint()`。

**索引：**
- `holes_token_hash`：唯一，`(token_hash)`。公开路由按它查找。
- `holes_workspace`：`(workspace_id, id)`。列表查询。
- `holes_host`：`(host_id)`。删除主机时检查引用；按主机计数。
- `holes_expires`：`(expires_at)`。探测循环选取 active 记录。

**DDL 示例（PostgreSQL；实际用 SeaQuery 生成）：**

```sql
CREATE TABLE IF NOT EXISTS holes (
  id BIGINT NOT NULL PRIMARY KEY,
  workspace_id BIGINT NOT NULL,
  host_id BIGINT NOT NULL,
  name VARCHAR(128) NOT NULL,
  target_port BIGINT NOT NULL,
  strip_prefix BOOLEAN NOT NULL DEFAULT TRUE,
  token_hash VARCHAR(64) NOT NULL,
  token_sealed TEXT NOT NULL,
  expires_at BIGINT NOT NULL,
  revoked_at BIGINT NOT NULL DEFAULT 0,
  revoked_by BIGINT NOT NULL DEFAULT 0,
  probe_state VARCHAR(16) NOT NULL DEFAULT 'unknown',
  probe_error VARCHAR(512) NOT NULL DEFAULT '',
  probed_at BIGINT NOT NULL DEFAULT 0,
  probed_by BIGINT NOT NULL DEFAULT 0,
  created_at BIGINT NOT NULL,
  created_by BIGINT NOT NULL DEFAULT 0,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT NOT NULL DEFAULT 0,
  version BIGINT NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX holes_token_hash ON holes (token_hash);
CREATE INDEX holes_workspace ON holes (workspace_id, id);
CREATE INDEX holes_host ON holes (host_id);
CREATE INDEX holes_expires ON holes (expires_at);
```

**方言差异**（由现有 helper 处理）：
- MySQL：`create_table` 加 `utf8mb4_bin` 排序规则，`long_text` 把 TEXT 映射为 `longtext`，BOOLEAN 映射为 `tinyint(1)`。
- SQLite：BOOLEAN 存为 INTEGER。
- 没有条件唯一索引，三种方言行为一致。

**迁移**：新文件 `crates/server/src/migration/holes.rs`，追加到 `Migrator::migrations()` 末尾，名称 `holes_v1`。
- up：建表和索引。
- down：按项目惯例是空操作。需要手工回滚时执行 `DROP TABLE holes;`。
- 只新增表，**不影响历史数据**。

**删除主机**：如果主机还有 holes 引用，删除时一并物理删除这些 holes，并为每条写审计。也可以改为禁止删除主机，见 §15 Q3。

## 11. API

前缀统一是 `/api/workspaces/{workspace}/holes`，返回统一的列表信封，用 Scope 做 workspace 隔离。

| 方法 | 路径 | 权限 | 说明 |
|---|---|---|---|
| GET | `/holes` | holes.read | 分页、搜索 `q`（name）；过滤 `host_id`、`state`；排序 `-created_at`（默认）、`name`、`expires_at` |
| POST | `/holes` | holes.create | body `{name, host_id, target_port, strip_prefix, ttl_hours}`，返回 201 和完整对象（含 `url`） |
| GET | `/holes/{id}` | holes.read | 含 `url`、`state`、`probe_*`、`host_name`、`host_online`、`active_connections` |
| PATCH | `/holes/{id}` | holes.update | body `{version, name?, strip_prefix?, ttl_hours?}`；传了 `ttl_hours` 就表示续期 |
| POST | `/holes/{id}/revocations` | holes.update | 撤销 |
| POST | `/holes/{id}/probes` | holes.update | 立即探测 |
| DELETE | `/holes/{id}` | holes.delete | 返回 204 |

- `ttl_hours` 取值限定为 `1 | 8 | 24 | 168`，默认 24（带建议值的必选项）。
- `url` 是根相对路径 `/holes/{token}/`。TLS 在反向代理终止时，后端无法知道对外的协议，所以由浏览器用 `new URL(url, location.origin)` 拼成完整链接。**不硬编码域名**。

**配置来源**：§8 和 §9 中的上限、时限、探测间隔和 64 MiB 请求体上限都是硬编码常量，属于开发配置，基本不变；ttl 选项是固定枚举。不新增环境变量。

## 12. 权限与审计

**权限**在 `PERMISSIONS` 中新增 `holes.read`、`holes.create`、`holes.update`、`holes.delete`，分组 `permission.group.holes`。

内置角色：
- owner 和 admin：全部权限。
- developer 和 operator：全部四项。
- viewer：只有 `holes.read`，按现有 `*.read` 规则自动获得。拿到 read 就能看到链接，因此也就能打开预览，见 §15 Q2。

`prepare_permissions` 中需要把 `holes.` 前缀加进回填列表，让已有的 workspace 也获得这些权限。

**审计**：通过 `activity_in` 记录以下操作的操作人、对象和变更字段：create、update、revoke、probe（手动触发的）、delete，以及删除主机时的级联删除。审计**不记录** token 明文。公开访问只写 tracing 日志，不写审计表。

## 13. 前端

- `src/features/holes/resource-config.tsx`：只写 `list` 和 `form` 配置，不手写表格。在 `src/app/resources.tsx` 注册，并在 `nav.ts` 的 host 分组里、hosts 之后加入口（权限 `holes.read`）。hosts 实际属于 host 分组，所以入口放在这里。
- **列表列**：名称、主机（附在线标记）、端口、状态、可达性（探测状态和最近检测时间）、活跃连接数、过期时间、创建人。
- **行操作**：复制链接、打开、立即探测、编辑、撤销（危险操作，需要确认）、删除（危险操作，需要确认）。
- **空态**：说明这个功能的用途，并给出创建按钮。
- **表单**：
  - 名称。
  - 主机：下拉，只列出当前 workspace 的 runner 主机。
  - 端口：数字。
  - 路径模式：开关，并说明“保留前缀需要应用配置 base 为 `/holes/{token}/`”。
  - 有效期：下拉，默认 24 小时。
  - 编辑时主机和端口只读。
- **查看页**：完整链接（可复制）、使用说明（沙箱限制、base 配置示例），以及所有状态字段。复用现有的查看页轮询机制，每 5s 刷新一次。
- **响应式**沿用共享组件：
  - Desktop：表格。
  - Pad：工具条换行，表单 1–2 列。
  - Phone：卡片化或在容器内滚动，单列表单。“复制链接”和“撤销”不能被隐藏。
- **i18n**：前端新增 `src/shared/i18n/locales/holes.json`，服务端新增 `crates/server/src/i18n/holes-<locale>.json`，六种语言全部覆盖。公开路由的错误页（404/502/503/504/413）是纯文本，按 `Accept-Language` 本地化，同样带上沙箱 CSP。文案保持中立，不写品牌名。

## 14. 测试与验收

**Rust 单元和集成测试：**
- protocol：新帧序列化往返；旧 Hello 缺 `supports_forward` 时为 false。
- runner：Forward 能和本地 TCP echo 双向透传；端口拒绝连接时返回 Failed；收到停止信号后会话在 1s 内结束。
- server，使用测试 Runner 加本地 HTTP 上游：
  - GET、POST（不带 `X-Ferry-Request`）、带 `Origin: null` 的请求都成功。
  - `strip_prefix` 两种模式下的路径正确。
  - Location 改写正确。
  - 强制 CSP、删除 Set-Cookie 和 Service-Worker-Allowed、ACAO 设为 `*`、OPTIONS 由 server 直接回 204。
  - 上游收不到 Cookie。
  - 不存在、过期、撤销三种 token 的 404 响应体完全相同。
  - 超过 64 MiB 返回 413。
  - 单 hole 第 33 个并发连接返回 503。
  - WebSocket 升级后能双向收发；撤销后在重新校验间隔内断开（测试时缩短间隔）。
  - **防泄漏**：并发请求结束、客户端中途断开、上游拒绝连接、超时这几种情况之后，`HoleConnections` 计数归零，`pending()` 为空，Runner 侧 TCP 连接已关闭。
  - 旧 Runner 返回 502 并写入 `unsupported`。
- API：CRUD；乐观锁冲突返回 409；对撤销的记录做续期返回 409；跨 workspace 访问返回 404；缺少权限返回 403；只能选 runner 主机；`ttl_hours` 和 `port` 越界返回 422。
- 迁移：在 SQLite、PostgreSQL、MySQL 上能建表（复用现有的三库迁移检查）。

**端到端（真实 server + 真实 Runner + Playwright）：**
- Vite dev（`--base /holes/{token}/`，`strip_prefix = false`）：页面加载成功，HMR 修改后能热更新。
- 静态构建页和一个 JSON API 在 `strip_prefix = true` 下都能访问。
- 安全三项（§6）：预览页对 Ferry `/api` 的请求拿不到数据，上游日志里没有 Ferry Cookie，`navigator.serviceWorker.register` 失败。
- 停掉本地服务后，列表在 20s 内变为 `unreachable`；重新启动后恢复为 `reachable`。停掉 Runner 后显示 `runner_offline`。
- 撤销后，已打开的 HMR WebSocket 在 30s 内断开，刷新页面得到 404。
- 在 1440、900、390、320 四种宽度下截图，测试 zh-CN、en-US、ms-MY 三种语言下列表和表单的长文案。

**交付检查**：`cargo test --workspace`、`npm run typecheck`、`npm run build` 全部通过。

**设计稿**：Figma 暂缺，响应式验收以截图为准，不当作已有设计稿覆盖。

## 15. 评审决定（已按推荐采纳，2026-11-09）

| # | 问题 | 推荐 |
|---|---|---|
| Q1 | `expires_at` 是截止时间，没有对应的 `_by` 列，是否作为时间审计规则的例外 | 作为例外。不存在“过期操作人”，硬加一个 `expires_by` 没有意义。续期操作已经由 `updated_by` 和审计记录覆盖 |
| Q2 | viewer 默认有 `holes.read`，能看到链接，也就能打开预览 | 接受。预览本来就是用来分享的，而且一个 workspace 里的 viewer 本就是被信任查看的人 |
| Q3 | 删除主机时，它的 holes 怎么处理 | 级联物理删除并审计。不阻止删除主机 |
| Q4 | 链接能否重复复制（为此加密保存 token） | 能。用 vault 加密保存，有 `holes.read` 就能复制。另一种做法是像注册令牌一样只在创建时显示一次，但不适合预览分享 |

## 16. 实施阶段

| 阶段 | 内容 | 执行方式 |
|---|---|---|
| P1 | protocol 新帧和能力位；Runner 实现 Forward 透传并响应停止信号；加测试 | 协议由主代理先定稿，之后可以交给子代理实现 |
| P2 | Server：迁移、实体、API、权限、审计、代理核心（connector、连接池、header 策略、guard 豁免、上限、重新校验）、探测循环、错误页、服务端 i18n；加测试 | 主代理直接实现（耦合紧、安全敏感） |
| P3 | 前端资源页和前端 i18n | 等 API 契约定稿后，可以交给子代理并行实现 |
| P4 | 端到端、安全实测、四种宽度截图、自审、验收 | 主代理 |

**新增依赖**：server 显式依赖 `hyper` 和 `hyper-util`（启用 `client-legacy`、`http1`）。这两个已经通过 axum 和 reqwest 存在于依赖树中，只是新增显式声明和 feature，不引入新的第三方库。

**版本兼容**：兼容升级。新 server 配旧 Runner 时，状态显示为“需要升级 Runner”；旧 server 配新 Runner 时，新字段被忽略。

**是否影响核心流程**：只影响 Runner 停止和升级时会话的收尾逻辑（新增“Forward 会话主动关闭”），其他会话类型的行为不变，需要回归测试。

## 17. 独立预览域名修订（设计中，未实现）

### 2026-11-10 配置规则修订（覆盖本节旧白名单与未配置待决描述）

- 平台业务域名统一在平台设置的中立“访问地址”分区配置；现有公开地址原值保留，不再要求env或安装脚本重复维护业务地址。平台公开访问地址是生成固定回调、通知等链接的主要地址，不作为控制台Host白名单；控制台默认接受网络可达的合法IP/域名请求，认证、权限、同源与跨站写入防护保留。公开地址为空时，依赖固定公开URL的功能沿用自身明确缺配置提示，不从不可信Host自动生成对外回调。
- 预览根域名为可选配置；未配置时不开放预览访问、不允许新建/自动托管预览，并提供配置入口，不回退到`/holes/token`模式。历史记录保留，具体历史链接失效和恢复规则在实现/迁移说明中覆盖。
- 预览Host须先按配置命名空间分流，未知/非法/过期/撤销token保持404，不因控制台默认开放而回退平台页面或API；与主要控制台Host冲突时配置校验和分流须明确保留控制台。此规则替代“控制台必须逐个登记Host”的旧前提。
- 安装脚本中用于生成反向代理站点/签发TLS证书的域名属于外部部署配置，不能由数据库业务配置自动替代；保留可选代理部署能力，不要求局域网HTTP访问先配域名。DNS、证书和代理路由依然需要对应域名匹配，修改平台字段不等于已配置外部设施。

用户依据：2026-11-09 提出由 `/holes/token` 改为 `token.host.domain/path`，认为剥离前缀选项不合适。此要求替代 D2；不自动扩大 Runner 转发目标、授权范围或来源生命周期。

### 17.1 已核对事实与改造范围

| 项目 | 项目内依据 | 改造契约 |
|---|---|---|
| 链接 | `holes.rs::link` 当前解密 token 并生成根相对地址 | 使用受信配置生成 `scheme://<DNS安全token>.<预览根域名>[:port]/`，不从任意请求的 Host/X-Forwarded-Host 推导公开域名 |
| token | `holes.rs::create_internal` 调用既有 `runner::secret`，代理要求64位hex | DNS单标签上限63字符。采用原32字节token的无填充小写Base32表示（52字符），反解回原hex后复用既有哈希索引、加密token和撤销记录；不截断token、不降低原熵 |
| 代理 | `holes/proxy.rs::serve` 解析路径、按token哈希查有效记录 | 在控制台路由前按预览域名分流，完整保留原始path与query；预览域名下的 `/api`、`/assets`、`/login` 均属于上游应用，不进入Ferry控制台 |
| 路径选项 | 手工预览表单、API、`host.preview`和Actions事件携带 `strip_prefix` | 手工和工作流配置移除可见开关；域名模式固定透传原路径。历史字段与旧Runner参数按兼容契约处理，不物理删除历史值 |
| 上游跳转 | `rewrite_response`、`location` 重写本机重定向 | 根相对Location直接保留；localhost/127.0.0.1/本地端口绝对Location转为同一预览域名路径；外部链接保留，不改写HTML/JS正文 |
| WebSocket | 既有Forward会话及HTTP Upgrade | `ws(s)://<同一token域名>/原始路径?query` 复用同一授权、连接计数、撤销及来源回收 |
| 浏览器隔离 | 既有CSP sandbox、Cookie剥离和guard豁免 | 本轮保留既有隔离策略。独立域名不等于自动开放Cookie、localStorage、Service Worker；不得在未知预览Host回退控制台页面/API |
| 生命周期 | 手工有效期、来源绑定、审核与终态回收已经实现 | 复用原记录、期限、审核和回收，不创建第二套预览资源或Runner协议 |

### 17.2 配置与兼容评审边界

- 公开部署需运维配置预览根域名、通配DNS和对应TLS；本地验收使用 `*.preview.test` 测试解析；CI逐个写入实际token域名到hosts，并由Node和浏览器使用系统解析。域名不能从页面语言、工作空间名或用户输入的完整URL随意派生。
- 配置归组（2026-11-10用户要求）：现有“平台公开访问地址”从“仓库关联”迁至独立、中立的“访问地址”分区，保留原配置值及已有回调/通知等消费者。同区提供独立“预览根域名”，不以控制台URL自动推导；输入纯域名，不带协议、`*.`、端口或路径。显示生成地址示例与所需通配DNS/TLS；保存不表示DNS/证书已就绪，不自动更改外部DNS或证书。具体新增配置键、后端存储及升级规则在实施时按项目内配置模型补齐，本段不代表已有入口。
- 用户确认兼容两种根域名：配置 `domain.xx` 时生成 `token.domain.xx/path`；配置 `subdomain.domain.xx` 时生成 `token.subdomain.domain.xx/path`。根域名可包含多级标签，token固定为其左侧紧邻的一段标签；不通过截取注册域名推导预览根域名。DNS/TLS须覆盖实际预览根域名对应的通配域名；`*.domain.xx`证书不能覆盖`token.subdomain.domain.xx`，后者须覆盖`*.subdomain.domain.xx`。分别测试两种有效域名、错误后缀、跨根域名token请求与TLS配置提示，不将保存配置等同证书验证成功。
- 控制台域名保留：控制台不再使用Host白名单，主要公开URL的完整Host保留为控制台。预览根域名采用 `domain.xx` 时，已登记的控制台Host（例如 `console.domain.xx`）继续进入控制台，不被通配分流吞掉；其余预览根域名下的单标签Host进入预览校验，未知或非法token返回404，不回退控制台。禁止生成与已登记控制台Host相同的预览链接；新增/修改配置不能让现有预览链接变成控制台地址。验收包括控制台与预览同父域、独立预览子域、控制台/API不受影响、未知子域/API不能回退控制台。
- 只接受配置域名下恰好一段合法token标签；拒绝后缀伪装、额外标签、非法端口、非法编码、未知token。未知、过期、撤销维持相同404语义，不回退控制台。
- 旧 `/holes/<hex>/path?query` 在已配置域名时验证记录后重定向至该记录域名的原路径。旧非剥离模式对应应用曾配置base path，不能声称无配置修改即可无缝迁移。
- 未配置时新建返回409，旧路径和token域名返回404；配置后旧路径按有效记录返回308；不能把“已填域名”当作DNS/TLS验证成功，不静默宣称公网可用。
- 数据模型：优先保持现有表、token哈希唯一索引和审计字段，无DDL变更；若实现引入持久配置，必须列出该配置的现有存储载体及up/down说明后再修改。

### 17.3 验收与交付边界

| 维度 | 可判定要求 |
|---|---|
| URL | 页面、根路径CSS/JS/图片、History API深链与query在无base配置的真实应用中可访问 |
| 跳转/连接 | 根相对和本机绝对Location、HTTP表单提交、WebSocket/HMR均保持token域名和原路径 |
| 权限 | 读取/创建仍按工作空间权限；未知预览Host不能读取平台API；复验既有控制台Cookie及内部请求标记剥离，不扩大工作空间访问权限 |
| 来源 | 手工、Actions自动预览、节点工作流自动预览生成同一格式，审核、取消、撤销、过期及Runner断连释放均复验 |
| 兼容 | 原token和历史记录不重置；旧链接行为、旧strip_prefix=false应用的迁移限制有独立用例 |
| 异常 | 未配置、DNS/TLS未就绪、端口不可达、Runner离线和重连分别覆盖；不给未知域名返回控制台壳 |
| UI | 列表/查看保持完整资源标题；创建/编辑无路径前缀开关；六语；1440/900/390/320长域名可换行或截断并可复制，无整页横向溢出 |
| 数据/审计 | 不删除历史记录、不重放执行；新增配置变更按既有平台审计；无逐请求落库扩容 |
| 设计追踪 | Figma暂缺，实际截图与测试证据待实现；实际测试与截图以MEMORY.md记录为准，Figma未交付 |

实施顺序：定稿配置与兼容边界 → Host分流/编码/链接与代理回归 → UI和两类自动来源统一 → 真实Runner/静态资源/HMR/隔离及四宽度验收。实现和发布状态分别见MEMORY.md，未经公开产物核验不视为发布完成。


### 17.4 配置存储与升级

- 复用platform_state单例行、平台设置版本锁和审计。新增access_config TEXT NULL，JSON内public_url和preview_domain均默认为空字符串。无新时间字段、索引、金额计算、删除策略或灰度。
- up：ALTER TABLE platform_state ADD COLUMN access_config TEXT NULL; 该值为NULL时读取原加密repository_oauth.public_url，首次保存写入新配置，不丢弃原地址。down：ALTER TABLE platform_state DROP COLUMN access_config; 三库由SeaQuery生成方言；降级丢失新地址配置，必须先备份。原holes数据/token不变。
- public_url为可选HTTP(S)根URL，最多2048字节，禁止凭据、query、hash、子路径及预览token标签作为控制台Host。preview_domain最多200字节，使用合法ASCII DNS标签，非IP、无协议/端口/路径/星号；空值禁用预览。预览协议及显式端口取主要公开URL，未配置主要地址时使用HTTPS。
- 主要控制台Host保留控制台路由；变更/清空预览根域名后，原token Host不回退控制台。已建连接按既有30秒重验周期断开。
- 设置Desktop/Pad短字段两列、Phone单列，复用CRUD与六语help，长URL可换行；预览列表/查看/编辑保留原响应式、错误/空态与权限策略。Figma和原生App未提供。
- 验收脚本：test-preview-holes.mjs、test-preview-automation.mjs；Actions preview-domains任务是发布门禁。CI使用hosts测试DNS解析，不代表公网DNS/证书已验证。

## 18. Token安全约束与入口诊断（2026-11-11）

用户明确要求子域名token足够长。复用现有实现，不改历史token：
- hosts/runner/enrollment.rs::secret使用rand 0.8密码学安全thread_rng生成32字节随机数（256位）。
- holes/domain.rs::label将完整随机值无损编码为52字符小写无填充Base32，不截断；低于DNS单标签63字符上限。禁止用序号、时间戳或短字符串替代。
- 解码要求固定长度、合法规范编码和32字节结果；授权继续查询原token哈希及有效记录，不将格式正确等同授权。
- 复用加密保存、哈希索引、有效期、撤销及来源回收；未知、过期、撤销均404。保留沙箱和Cookie剥离。
- 链接属于持有者凭证；256位随机性防猜测，不防泄漏。DNS、隧道和代理可见Host，不在诊断输出中打印真实token。
- 现有tokens_roundtrip_without_exceeding_dns_label_limit测试覆盖随机值及全0/全ff往返、长度及非法尾位。本轮只核对源码，未重跑Rust测试，不宣称完整安全审计。

只读诊断：本机4291返回alpha.18/server_mode=false；公开控制台返回alpha.12/server_mode=true，实例ID不同。alpha.13提交46284b3移除了旧Host允许列表逻辑；公网旧服务与报错吻合，具体出错预览Host及隧道路由仍未验证。没有重启或改配置。

后续设计仍为域名与LAN IP独立端口模式。Cloudflared支持泛域名ingress，需DNS/TLS及原始Host转发匹配。IP端口模式未实现；端口不是凭证。现有opaque-origin sandbox与Cookie剥离下，透明鉴权须先完成浏览器PoC。端口池/租约/集群入口/模型待设计，不恢复受限路径模式作自动降级。

## 19. 多入口配置与Quick Tunnel可行性（2026-11-11，设计草案）

范围：用户要求评估免费临时Cloudflare入口，支持平台默认、手工预览选择及切换。本节是待完成安全验证与评审的设计草案，不代表已有功能；本轮不改运行服务、三方配置、数据库或UI。

### 19.1 官方事实与适用性
来源：https://developers.cloudflare.com/tunnel/get-started/quick-tunnels/ （2026-11-11读取）。无需Cloudflare账号或自有域名，cloudflared输出临时trycloudflare.com地址；停止进程地址失效，每次创建域名变化。没有可用性保证；每隧道最多200个in-flight请求，额外请求429；不支持SSE。不能描述为无限制免费入口。官方还提供allowed-mail邮箱验证码访问，要求交互浏览器，不支持非交互客户端；项目所用cloudflared版本及兼容性尚未实测。

结论：可纳入临时开发预览候选，不作为长期固定地址的默认替代；SSE应用不适用。随机域名不等于本项目256位凭证，禁止凭“地址难猜”直接公开应用。不能假设可控制token.random.trycloudflare.com；每预览独立隧道/精确地址映射和鉴权适配待PoC，复用现有Runner转发与生命周期，不能把隧道直接绕过现有撤销/过期检查连到应用。

### 19.2 配置结构建议
泛域名和IP端口属于寻址；固定Cloudflared属于网络接入，两者不能设计为互斥枚举。平台设置增加预览穿透分区，展示可用入口配置：
| 入口方案 | 条件与用途 | 当前实现状态 |
|---|---|---|
| 泛域名入口 | 自建反代、LAN DNS或固定Cloudflare Tunnel均可承载；沿用52字符token标签 | 已有域名模式；隧道仍由运维配置 |
| IP独立端口入口 | LAN可达IP与受控端口池；无域名使用；公网需要额外端口网络支持 | 未实现，鉴权/隔离PoC前置 |
| Cloudflare临时入口 | 无自有域名，临时公网HTTP预览；标明200请求/SSE/域名变化限制 | 未实现，进程托管/安全/协议PoC前置 |

不把固定Cloudflared和泛域名重复做成两种独立路由实现。固定隧道使用现有泛域名入口，可记录接入说明；是否由平台管理固定隧道凭据与安装属于额外范围，本草案不包含。

建议允许多个入口配置同时启用，平台指定一个默认。手工创建默认选中该入口，并允许选择其他已启用且可用的入口。创建时固化实际入口，不因平台默认后来改变而搬迁。自动预览在创建时使用当时默认并固化；已存在执行不重放。首次升级保留现有域名行为，不自动启用公网临时隧道。

### 19.3 切换与异常闭环建议（待用户确认产品语义）
建议平台默认变更仅影响以后创建。单条活动预览显式切换：准备新入口、校验成功、原子切换、回收旧入口；失败保留原入口并显示失败原因。成功后旧地址不再访问，已有连接按明确关闭策略处理；具体连接排空规则待决。新入口准备期间不得未经授权公开。严禁启动失败后静默回退到另一公网方案。

临时隧道绑定预览TTL、撤销、来源执行终态和归属实例；进程退出显示入口离线，不显示上次URL仍可用。重建展示新URL，旧URL不承诺有效。集群入口绑定实际持有隧道/端口的节点，不能仅写共享数据库后由任意节点假装接管；所有权、租约、崩溃恢复与重启策略待设计。平台禁止停用/删除被活动预览使用的配置，先迁移或关闭；迁移权限与确认交互待设计。

### 19.4 开发门槛与验收追踪
待完成：入口模型、权限（复用现有平台设置管理与工作空间预览权限，不扩大匿名管理权）、审计/DDL/up-down、配置校验、六语文案、全状态及三端布局矩阵；Figma暂缺。本节不满足完整开发评审，不编造现有模型字段。

PoC必须验证：完整页面及子资源/深链接/query；HTTP表单；WebSocket/HMR；邮箱登录与sandbox兼容；未知/过期/撤销/跨预览拒绝；进程停止和服务重启；切换成功/失败及旧链接回收；集群节点掉线。SSE在临时入口明确不支持；200并发限制依官方事实，不以少量实测宣称无限额。不得为通过PoC移除sandbox、放开平台Cookie或用端口保密替代认证。

## 20. 三入口实施契约（2026-11-11，用户确认同时实施）

用户确认不安全方案明确提示、谨慎选择，三种入口同时实施。覆盖第19节暂缓新增入口的建议：domain继续256位持有者凭证；ip_port和cloudflare为显式公开访问模式，不声称有token鉴权。仍保留sandbox、Cookie剥离、TTL、撤销与来源回收，不增加HTML/JS重写或路径前缀兼容。

实施计划：Direct，接口、生命周期、表单耦合，不派生代理。修改accounts/addresses、holes及domain/proxy/owned、entities、migration、现有Platform与holes共享资源配置、六语文案及对应测试；新增holes/ingress.rs承担监听器/子进程生命周期，新增可重现验证脚本与迁移。protocol/runner、主工作树和共享4291服务不改。

| 评审维度 | 实施契约 |
|---|---|
| 事实来源 | 现有Configuration/access_config、holes记录/授权/版本/审计、Runner Forward、proxy隔离、lock模块；第18/19节官方资料及本轮真实Chrome/临时隧道PoC |
| 本期范围 / 不做范围 | 三入口设置/默认/手工选择/显式切换；不管理固定Cloudflare账号凭据，不自动安装CLI、不改外部DNS/TLS，不做自动公网降级 |
| 灰度策略 | 无；新增公开入口默认关闭，旧配置继续domain |
| 流程闭环 | 准备入口→事务校验版本/授权并保存→激活；准备或事务失败清理新资源并保留旧入口；撤销/过期/来源结束关闭连接并回收；服务重启不自动重放临时隧道，用户显式重连 |
| 异常覆盖 | 空配置、无CLI、端口池耗尽、超时、CLI退出、节点离线、非法模式/地址/端口、版本冲突、已撤销/自动记录限制；只显示脱敏错误 |
| 操作记录 | 沿用平台设置和holes create/update/revoke/delete；记录实际入口及公开确认，不记录token/完整临时URL |
| 删除保留 | holes沿用现有物理删除与审计；监听器/临时子进程随记录回收；端口可再次分配，不承诺IP端口地址永久唯一 |
| 权限规则 | 配置沿用平台管理员，手工沿用workspace holes权限；客户端不提供workspace_id。自动公开入口须管理员另行启用，手工每次选择公开方案确认 |
| 计算逻辑 | 无金额；端口闭区间分配且bind成功才占用；lease复用现有锁，15秒刷新/45秒过期；proxy连接按既有周期重验 |
| 模式差异 | 平台启用配置/默认；手工创建选择并固化；手工编辑显式选入口才切换/同模式重连；查看显示实际模式/访问级别/入口可用性；自动记录仍不可手工编辑 |
| 字段联动 | domain依赖根域名；ip_port依赖启用、可达IP、绑定IP与端口区间；cloudflare依赖启用和CLI路径。公开方案须平台风险确认，手工须独立确认；默认只影响以后创建 |
| 状态操作 | 仅未撤销手工记录可切换；成功轮换token以废止旧domain链接、关闭旧连接；失败不改旧记录；过期手工记录沿用现有延期规则，准备入口同时验证新有效期 |
| 校验规则 | 模式domain/ip_port/cloudflare；IPv4/IPv6由标准解析；公开IP禁未指定/组播；端口1..65535且起始<=结束；CLI仅文件名/路径、长度<=2048、无NUL，禁止拼shell/自定义参数 |
| 数据模型 | holes新增ingress_config TEXT NULL，NULL为旧domain行为；新值为沿用vault加密的JSON快照，含mode、origin、owner；IP/端口固化在origin，绑定地址与CLI路径只保留在平台配置；秘密不明文保存。platform_state.access_config增加带serde默认的设置键，无新增表 |
| 时间审计 | 无新增xx_at；沿用holes created_at/by、updated_at/by以及platform现有设置审计。运行存活复用locks而不新增时间字段 |
| 索引设计 | 无新增索引；沿用holes token_hash唯一/工作空间/有效期索引与locks主键；按已有记录ID操作 |
| SQL示例 | up: ALTER TABLE holes ADD COLUMN ingress_config TEXT NULL; down: 拒绝破坏性降级，须恢复升级前备份。SQLite/PG/MySQL用既有SeaQuery方言生成 |
| 文案完整 | 新文案六语zh-CN/en-US/ms-MY/zh-TW/ja-JP/ko-KR；公开风险、CLI/SSE/限额/地址变化、Cookie登录限制必须呈现；Figma暂缺 |
| 配置来源 | 运维平台设置：默认domain、公开入口关闭、绑定0.0.0.0、可达IP由用户填写、端口建议43000..43100、CLI默认cloudflared；均可配置；协议安全常量仍代码定义 |
| 状态覆盖 | 实际入口ready/offline，公开访问单独标识；尚无URL不显示可打开；错误不伪装成功；重启/进程退出不沿用在线状态 |
| 影响核心流程 | 预览创建/链接/切换；原Runner、控制台认证及转发限额不变 |
| 影响历史数据 | 加可空列不重置token；NULL旧记录复用旧行为；新记录固化方案，domain地址仍沿用现有全局根域名规则，旧记录保持兼容；不执行真实库迁移 |
| 是否依赖三方 | Quick模式需本机cloudflared和Cloudflare服务；200 in-flight/noSSE/无SLA/重建域名变化；无该依赖时其他入口正常 |
| 版本兼容 | 新快照须新版server；runner/protocol无变更；带新快照的数据拒绝破坏性down |
| 逻辑冲突 | Cookie/ServiceWorker透明鉴权PoC失败，公开模式明确无该鉴权；sandbox仍限制Cookie登录/存储/ServiceWorker应用，不承诺全应用兼容 |
| 可测试性 | 配置/确认/默认/隔离/版本/回收单测，真实HTTP端口占用/失败保留/切换旧入口404；Quick真实页面/资源/深链接/WS及退出回收；全量项目检查 |
| 验收追踪 | PoC证据Temp/preview-ingress-poc-ZgWTy3/report.json、preview-sw-poc-f7f6bA/report.json；仅测试夹具，非Ferry端到端。首轮真实资源/API/Runner/Chrome/Quick端到端25项PASS：data/maintenance/preview-holes-20261111-075107/result.json；最终含多语言/强杀回收26项PASS：data/maintenance/preview-holes-20261111-082533/result.json；Figma暂缺 |
| 响应式交付 | Desktop>=1200设置两列与共享列表；Pad768..1199分组折叠/控件换行；Phone320..767单列、长URL换行、确认与保存可达。导航/数据/滚动/安全区均沿用共享组件；新增区块须四宽度截图及核心操作验证 |
| 模糊描述 | 无；测试未覆盖明确标未验证，不宣称公网无限额或链接即身份 |

风险文案须直接表达：IP端口没有访问密码，网络可达者能访问，HTTP不加密且端口会复用；免费临时入口为公网公开地址，知道地址者能访问，随机域名不是本平台token保护。仅用于可公开的预览内容，不在其中处理敏感数据。切换前显示目标方案的实际风险。

### 20.1 实施补充与兼容规则
- 普通PATCH带回相同ingress_mode不得重连或轮换token；同模式显式重连必须reconnect_ingress=true。切换到不同模式则要求相应公开确认。
- 固化的是入口方案及IP/临时隧道实际地址；domain继续使用现有全局根域名和公开URL，根域名变更/清空的迁移/停用语义不改。默认方案改变不迁移已有预览。
- Quick使用独立空配置文件并排除继承的TUNNEL_*环境，传输选择已实测的HTTP/2；不改用户现有隧道。Windows用Job Object使服务进程终止时同时终止其连接器。
- IP入口仅接受当前服务节点拥有的LAN IP，不自动配置NAT映射；不同节点IP拒绝创建，避免虚假地址。创建请求须落在该IP所在节点。

### 20.2 地址回收边界
撤销/切换关闭的是当前入口绑定与连接，不保证公开地址永远不被复用。IP端口会重新分配；临时公网域名的后续分配由三方控制。两种公开模式都不把地址当作身份凭证。

启用开关控制新建及显式切换，不批量撤销当前记录；需要关闭访问时逐条撤销。创建中请求采用开始时配置快照。此规则在平台分区说明中六语展示。

入口可用性表示本平台入口资源/租约有效，不等于公网端到端探测成功。真实测试曾出现Quick连接器已注册但新域名尚未解析，以及后续连接器启动失败；不承诺DNS即时生效或三方服务稳定。测试脚本最多等待60秒公网DNS/HTTP就绪，再执行完整应用断言；平台不增加自动重试。

### 20.3 最终验证与交付边界
2026-11-11：最终cargo test --workspace通过（server668/9既有ignored、Runner54/7既有ignored及integration3+12、protocol7、sandbox16/3既有ignored）；前端82文件630通过/1既有skip；workspace check、主题生成校验、TypeScript及Vite生产构建、嵌入最终dist的server/Runner构建均通过。真实Runner/Chrome/Vite/Quick 26项通过，证据见20节验收追踪；包含Windows强制停止服务回收其连接器、六语文案、三语手机截图与四宽度布局。源码自审与原始授权范围验收通过。

仅Windows单机隔离环境实测，不冒充Linux强杀、多节点故障、PG/MySQL真实连接、Figma/原生App/物理键盘验收。未合并、推送、部署或迁移真实库；不会重启其他进程的服务。现有编译及前端大包警告不在本期修改范围。

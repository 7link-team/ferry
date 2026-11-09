# 预览穿透（Preview Holes）设计

状态：IMPLEMENTATION。设计自审通过，用户授权按推荐实施（2026-11-09），§15 Q1–Q4 按推荐采纳。
日期：2026-11-09

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

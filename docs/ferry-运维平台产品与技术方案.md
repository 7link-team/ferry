# Ferry：工作空间制运维平台 · 产品与技术方案

版本 V0.1，日期 2026-10-23。参考 `easy-erp` 的架构、部署形态与工程约定；业务域、多租户模型、主机通道与统一 DSL 组件是本项目新增的部分。

本轮交付为**方案与设计**，以及一个通过构建校验的空骨架。骨架不含业务逻辑；第 16 节的待确认项确认后才进入业务开发。

---

## 1. 产品定位与范围

Ferry 是一个**工作空间制的运维平台**：注册后创建自己的工作空间，或凭邀请码加入别人的工作空间。工作空间内按「仪表盘 / 部署 / 构建 / 主机 / 维护」五个模块组织资源，所有资源共享同一套列表、表单、权限、审计和定时任务机制。

和 `easy-erp` 的关系：**沿用全部工程基座，替换业务域**。

| 维度 | easy-erp | Ferry |
|---|---|---|
| 技术栈 | Tauri 2 + Rust/axum + React + SQLite/MySQL | 同 |
| 部署形态 | 桌面内嵌 + Docker Web | 同 |
| 租户模型 | 单实例单组织 | **工作空间多租户** |
| 页面构造 | 每页手写表格 | **列表/表单 DSL 配置驱动** |
| 外部连接 | 无 | **runner 代理 + SSH 直连** |
| 新增进程 | — | **`ferry-runner` 主机代理** |

第一版范围：账号与工作空间、角色权限、主机接入（两种方式）、远程终端、双向文件传输、主机组与标签、仓库与流水线、应用部署与模版、共享的定时任务与密钥变量、字典与配置项、操作记录、仪表盘。

不在第一版：多工作空间数据互通、跨云资源编排、日志检索平台、指标时序库、工单审批流。

---

## 2. 两种部署形态

一套界面、一套 API、一份数据。形态只决定「谁拉起服务进程」。

| 形态 | 组成 | 界面入口 | 适用 |
|---|---|---|---|
| **Docker** | `ferry-server` 单进程 | 仅浏览器 | 团队共用、放在服务器上 |
| **Desktop** | Tauri 外壳 + `ferry-server` sidecar | Tauri 内嵌 WebView（也可用浏览器打开同一地址） | 个人使用、无服务器环境 |

```
Docker 形态                        Desktop 形态
┌──────────────────────┐          ┌──────────────────────────────┐
│ ferry-server         │          │ Tauri 外壳（托盘/连接设置）    │
│  ├ 内嵌 Web 界面      │◀─浏览器   │   └ WebView → 127.0.0.1:4290 │
│  ├ REST API / WS     │          │ ferry-server（sidecar）       │
│  └ SQLite / Postgres │          │   └ SQLite                   │
└──────────────────────┘          └──────────────────────────────┘
          ▲                                     ▲
          └────────── ferry-runner（各主机）──────┘
```

关键约束，与 easy-erp 一致：

- Web 界面构建产物由服务用 `rust-embed` 内嵌，页面与 API 同源，运行时无 Node 依赖。
- 桌面外壳**只做**托盘、连接设置、服务生命周期；业务逻辑一行都不放在 `desktop/`。
- 具有原生权限的窗口只有 `launcher`；业务窗口是对服务 origin 的普通 WebView。
- Desktop 形态也可以只作客户端，连接另一台机器上的服务。

---

## 3. 总体架构

```
┌─────────────────────────────── crates/server ───────────────────────────────┐
│ HTTP 层    路由（REST + WS） · 会话中间件 · 工作空间提取器 · CSRF/Host 校验    │
│ 业务层     hosts groups tags · repositories pipelines · deployments ...      │
│            cron_jobs（共享） · variables（共享） · members roles activity     │
│ 通道层     HostChannel trait ── RunnerChannel │ SshChannel                   │
│ 数据层     db.rs 原生 SQL + dialect.rs 方言 + migration.rs 迁移               │
│            ── SQLite │ MySQL │ PostgreSQL                                  │
└──────────────────────────────────────────────────────────────────────────────┘
          ▲ WS 反向连接                      ▲ SSH 外拨
┌─────────┴──────────┐              ┌────────┴──────────┐
│ ferry-runner       │              │ 目标主机 sshd      │
│ 控制连接 + 会话连接  │              │ 账号密码 / 私钥    │
└────────────────────┘              └───────────────────┘
          ▲ 共用 crates/protocol 线上类型
```

分层规则：

1. **HTTP 层不碰 SQL**，业务层不碰 `axum` 的提取器之外的东西。
2. **所有主机操作只经过 `HostChannel`**。终端、文件、部署步骤、流水线步骤、定时任务执行，一律调这个 trait；谁都不准直接 `ssh` 或直接写 runner 协议。接第三种接入方式时只改工厂函数。
3. **`crates/protocol` 是 server 与 runner 的唯一契约**。协议类型不许在任一侧复制一份。
4. 数据访问全部走 `db.rs` 的 `execute/one/all`，不引入 ORM 实体宏——与 easy-erp 相同的取舍：SQL 看得见，迁移是纯 SQL。
5. **三种数据库的差异只在 `dialect.rs` 一个文件里**。业务代码一律写 SQLite 风味、`?` 占位符的 SQL，由 `dialect` 翻译。见 §3.1。

### 3.1 SQLite / MySQL / PostgreSQL

| | SQLite | MySQL | PostgreSQL |
|---|---|---|---|
| 用途 | 桌面形态、小型 Docker 部署的默认 | 已有 MySQL 运维体系的团队 | 多人共用、需要并发写入 |
| 配置 | 不填 `FERRY_DATABASE_URL` | `mysql://…` | `postgres://…` |

方言层只做两件事，刻意保持小：

- **占位符**：SQLite 和 MySQL 用 `?`，PostgreSQL 要 `$1 $2 …`，按出现顺序改写，字符串字面量里的 `?` 不动（`'it''s ? here'` 这种转义引号也要认对）。
- **DDL 类型**：MySQL 不能给 `TEXT` 列加索引，所以按列名分配宽度——`id` 和 `*_id` → `VARCHAR(64)`（UUID 36 字符），JSON 列 → `LONGTEXT`，其余 → `VARCHAR(255)`；建表追加 `utf8mb4_bin`，让大小写敏感性和 SQLite 的 BINARY 默认一致，否则 `Admin` 和 `admin` 会在唯一索引里撞车。MySQL 没有 `CREATE INDEX IF NOT EXISTS`，改为先查存在性——它的 DDL 各自提交，迁移中途失败必须能重跑。

**整数宽度是真踩过的坑**：PostgreSQL 的 `INTEGER` 是 i32，SQLite 给 i64，MySQL 的 `COUNT()` 给 u64。所以 (1) 表里所有整数列统一用 `BIGINT`，绑定的 i64 参数才不会和列类型冲突（`WHERE version = ?` 每次更新都会绑）；(2) `db::int` 逐个尝试 i64 / i32 / u64 / u32。单元测试抓不到这类问题，只有真服务连真数据库才会暴露——所以有 `scripts/test-backends.sh`，CI 每次推送都跑三遍。

不加第四种数据库之前，这一层不该再长大。遇到某个查询在某个库上不work，优先把查询改写成可移植的，而不是在方言层加一条特例——easy-erp 的 `mysql.rs` 就是反面教材，它为具体查询堆了几十条 `replace`。

---

## 4. 账号、工作空间与权限

### 4.0 平台初始化与全局管理（alpha.6）

首次使用由部署者创建平台超级管理员，无默认账号密码。`platform_state` 单行记录初始化完成状态、公开注册开关及乐观锁版本；`users.is_superadmin` 与工作空间角色独立。
初始化凭据存储于数据目录 `setup-token`（Unix 权限0600），服务器操作员手动输入，桌面内置服务通过 URL fragment 传递，前端立即从历史记录移除且不持久化。
升级时已有用户不自动升权；认领已有账号必须同时提供部署者凭据与原密码。初始化、账号权限变更和注册设置在单行锁下原子提交；重启和重复请求不能重新初始化。

平台页面使用 `/_platform/{users|workspaces|settings}`，API 使用 `/api/platform/...`，由独立 `Admin` 提取器授权；工作空间仍使用原 `Scope`，仅平台超管可跨空间获得完整权限目录。
平台列表及表单复用共享 DSL，通过显式 `endpoint` 配置连接全局接口。用户投影不含密码哈希；最后一位启用超管不可停用或降权，账号状态/权限变化吊销其全部会话。
平台审计复用 `activity`，`workspace_id=0`，不包含密码、凭据、邀请码等敏感内容。普通用户只读自己空间的审计记录。
初始化前禁止公开注册；初始化后默认允许，超管可关闭。首批平台管理提供用户状态/超管资格、工作空间列表/创建/改名/进入、注册设置，不提供删除工作空间或密码重置。

### 4.1 注册与进入

```
注册（邮箱 + 密码，argon2）
   └─ 无工作空间 → 引导页
        ├─ 创建工作空间（名称 + slug）→ 自己成为 owner，自动创建内置角色
        └─ 加入工作空间（邀请码）→ 成为成员，获得工作空间的默认角色
```

- 会话是 Cookie：`ferry_session`，HttpOnly、SameSite=Lax、`Secure` 由配置决定；服务端 `sessions` 表可随时吊销，滑动续期。
- `sessions.workspace_id` 记住最后使用的工作空间，访问 `/` 时直接跳回去。
- 一个用户可属于多个工作空间，顶栏右侧切换。切换只改 URL 第一段，不重新登录。

### 4.2 权限模型

扁平权限串 `{资源}.{动作}`，角色是权限集合，成员绑角色。沿用 easy-erp `roles.rs` 的三要素：**权限清单、依赖关系、内置角色**。

```rust
// 清单：(权限, 分组显示名, 动作显示名)
("hosts.read", "主机", "查看"),  ("hosts.create", "主机", "新增"),
("hosts.update", "主机", "修改"), ("hosts.delete", "主机", "删除"),
("hosts.terminal", "主机", "远程终端"), ("hosts.files", "主机", "文件传输"),
("hosts.enroll", "主机", "生成接入命令"),
("pipelines.read", "流水线", "查看"), ("pipelines.run", "流水线", "执行"),
("variables.reveal", "密钥变量", "查看明文"),
...
```

依赖关系自动补全：勾 `hosts.terminal` 自动带上 `hosts.read`；勾 `pipelines.run` 自动带上 `pipelines.read` + `repositories.read`。角色编辑页据此防止配出不可用的组合。

内置角色（每个工作空间创建时生成，可复制后修改，不可删除）：

| 角色 | 范围 |
|---|---|
| owner | 全部，含转让与删除工作空间 |
| admin | 全部业务 + 成员/角色管理 |
| developer | 构建、部署的读写执行；主机只读 |
| operator | 主机读写、终端、文件；构建部署只读 |
| viewer | 全部只读，不含密钥明文 |

### 4.3 多租户隔离：一个提取器管住全部

每张业务表都有 `workspace_id`。隔离**不靠每个 handler 自觉**，而靠一个提取器：

```rust
// 从路径取 slug → 校验会话用户是该空间的 active 成员 → 加载角色权限
pub struct Scope { pub workspace_id: String, pub user: User, pub permissions: HashSet<String> }
impl FromRequestParts<AppState> for Scope { ... }

impl Scope {
    pub fn require(&self, permission: &str) -> Result<()> { ... }
}
```

handler 只能拿到 `Scope`，`workspace_id` 永远来自路径校验结果，**不从请求体读**。这是整个系统最重要的不变量，审查时第一条就查它。

---

## 5. 导航与路由

### 5.1 菜单树即数据

全部菜单在 `src/nav.ts` 一个文件里。**用户已经说明「以后可能把主机/构建/部署合并成一个一级菜单」——这个合并必须是改这一个文件，不是改页面。** 路由由菜单树推导，没有第二份路由表。

| 一级（顶栏） | 左侧菜单 |
|---|---|
| 仪表盘 | 仪表盘 |
| 应用 | 应用部署 · 应用模版 · 应用市场 · 凭据 · 密钥变量 · 报警监控 · 定时任务 |
| 构建 | 仓库连接 · 流水线 · 流水线模版 · 凭据 · 密钥变量 · 定时任务 |
| 主机 | 节点（含分组目录） · 凭据 · 标签 · 定时任务 |
| 维护 | 成员管理 · 角色管理 · 密钥变量（全局） · 配置项 · 字典 · 操作记录 |

> 用户已确认一级菜单使用“应用”。主机、构建、应用的凭据分别管理，沿用同一 REST 资源与加密存储，以 `scope=host|build|deploy` 隔离页面及选择器。

出现在多个模块下的「定时任务」「密钥变量」是**同一个资源页 + 不同预置参数**，不是多份实现：

```ts
{ key: "cron-jobs", label: "定时任务", resource: "cron-jobs", params: { scope: "host" } }
```

### 5.2 URL 契约

工作空间 URL 和 API 都仅使用已有雪花 ID 的十进制字符串；名称及 slug 仅为展示/历史元数据。前端资源路径为 `/workspaces/{workspaceId}/{resource}`，接口为 `/api/workspaces/{workspaceId}/{resource}`。菜单分组不进入路径；共享资源的业务范围用 `scope` 查询参数表达。ID 全程使用字符串，禁止转成 JavaScript Number。登录后旧页面书签按可访问工作空间映射到 ID 并替换历史记录，保留详情、过滤条件和终端模式；API 不接受 slug。名称变更不改变资源地址，ID 不代表权限，成员状态和权限仍由服务端逐请求验证。

```
/workspaces/{workspaceId}/{resource}             列表
/workspaces/{workspaceId}/{resource}/new         创建
/workspaces/{workspaceId}/{resource}/{id}        详情（表单的只读模式）
/workspaces/{workspaceId}/{resource}/{id}/edit   编辑
```

查询串承载**全部视图状态**：

| 参数 | 含义 | 写入方式 |
|---|---|---|
| 过滤字段（如 `group_id`、`state`、`q`） | 搜索条件 | `replaceState` |
| `page` / `page_size` | 分页 | `replaceState` |
| `sort` | 排序，`-created_at` 表示降序 | `replaceState` |
| `at` | 列表滚动偏移 | `replaceState`，150ms 防抖 |

**一律 `replaceState`，不进历史**——否则输入框每敲一个字就多一条历史记录，浏览器后退键失效。只有切页面用 `pushState`。

### 5.3 链接分享与登录直达

未登录时**不跳转到登录页**，而是在当前 URL 上原地渲染登录层；登录成功后原地重渲染。URL 从头到尾没变过，所以分享链接 → 登录 → 直达目标页面和滚动位置，中间不丢状态。

滚动恢复的时序：等本页数据 resolve 之后再 `scrollTo(at)`，否则容器还是空的，设了也会被重排抹掉。

```ts
// ponytail: 用像素。换了页大小或字号会偏；真有人抱怨再换成「首个可见行的 id」。
export function useScrollSync(ready: boolean) { ... }
```

**自动刷新间隔不进 URL**——它是观看者的个人偏好，不是要分享的内容，存在 `localStorage` 里按资源区分。选中状态（全选框）是临时状态，两处都不存。

---

## 6. REST 资源契约

### 6.1 命名与方法

所有资源统一在工作空间下，资源名用**复数、小写、连字符**：

```
GET    /api/workspaces/{workspaceId}/nodes                 列表
POST   /api/workspaces/{workspaceId}/nodes                 创建       → 201
GET    /api/workspaces/{workspaceId}/nodes/{id}            详情
PATCH  /api/workspaces/{workspaceId}/nodes/{id}            局部修改
PUT    /api/workspaces/{workspaceId}/nodes/{id}            整体替换
DELETE /api/workspaces/{workspaceId}/nodes/{id}            删除       → 204
```

非 CRUD 操作用名词子资源表达，由 HTTP 方法表示操作；不使用 `/actions/{verb}`：

```
POST   /api/workspaces/{workspaceId}/nodes/{id}/connection-tests
DELETE /api/workspaces/{workspaceId}/nodes        批量（体内带 ids）
POST   /api/workspaces/{workspaceId}/pipelines/{id}/runs
```

> 批量删除不用带请求体的 `DELETE`——中间代理会丢掉 `DELETE` 的 body，这是实际踩过的坑。

导入导出也是固定形状：

```
GET    /api/workspaces/{workspaceId}/nodes/import-template?format=xlsx       下载导入模板
GET    /api/workspaces/{workspaceId}/nodes/exports?format=csv|xlsx     导出（沿用列表的全部过滤条件）
POST   /api/workspaces/{workspaceId}/nodes/imports                    上传 → 返回预览与逐行错误
POST   /api/workspaces/{workspaceId}/nodes/imports/{id}/commit        确认写入
```

导入**永远两步**：先预览再提交。一次性导入无法让人看清将要发生什么。

### 6.2 统一信封

列表响应只有这一种形状，前端 DSL 组件据此解析，任何资源都不许自定义：

```json
{ "items": [ ... ], "total": 137, "page": 1, "page_size": 20 }
```

单条响应直接是对象。错误响应沿用 easy-erp 的形状，message 是**给人看的中文整句**：

```json
{ "error": "主机名称已被占用，请换一个。", "fields": { "name": "已被占用" } }
```

`fields` 可选，用于把错误直接标在表单字段上。

### 6.3 并发与版本

每张业务表有 `version INTEGER NOT NULL DEFAULT 1`。`PATCH`/`PUT` 必须带 `version`，不一致返回 `409` 与一句「这条记录已被他人修改，请刷新后重试」。这是 easy-erp 已验证的做法，直接沿用。

### 6.4 查询参数约定

| 参数 | 规则 |
|---|---|
| `page` | 从 1 开始，缺省 1 |
| `page_size` | 缺省 20，上限 200 |
| `sort` | 字段名，前缀 `-` 为降序；只接受白名单字段 |
| `q` | 资源自定的模糊搜索字段集合 |
| 其它 | 一个过滤字段一个参数；多值用逗号分隔 |

---

## 7. 统一列表 DSL 组件

### 7.1 为什么是配置而不是组件库

需求里「搜索过滤 / 搜索 / 清空 / 刷新下拉 / 导入 / 导出 / 创建 / 序号列 / 操作列 / 全选 / 分页 / 四边 sticky」是**每个资源都一样**的。手写一遍三十行，手写二十遍就是二十处会各自长歪的实现。所以是一个组件 `<ResourceTable config={...} />`，配置是资源的全部差异。

**这是硬约定：资源页不手写表格。** 表达不了的需求是扩展配置的理由，不是另开一份实现的理由；第二份表格实现算缺陷。

### 7.2 配置契约

```ts
export interface ListConfig<T> {
  /** REST 资源名，拼成 /api/workspaces/{workspaceId}/{resource} */
  resource: string;
  rowKey: (row: T) => string;

  /** 搜索过滤区。每项自动与 URL 同步，key 即查询参数名 */
  filters?: Filter[];

  columns: Column<T>[];

  /** 行尾操作列。无权限的项自动不渲染 */
  actions?: RowAction<T>[];
  /** 底部批量操作，依赖选中项 */
  bulk?: BulkAction[];

  toolbar?: {
    create?: boolean | { label: string };
    import?: boolean;
    export?: boolean;
    /** 定时刷新可选秒数，0 为关闭；默认 [0, 10, 30, 60] */
    refresh?: number[];
  };

  /** 预置的固定条件（共享资源的 scope 等），不出现在过滤区 */
  fixed?: Record<string, string>;
  /** 空列表时的一句话指引 */
  empty?: string;
}

export interface Filter {
  key: string;
  label: string;
  type: "text" | "select" | "multi-select" | "date-range" | "tag";
  options?: Option[] | (() => Promise<Option[]>);
  placeholder?: string;
}

export interface Column<T> {
  key: string;
  label: string;
  /** 固定宽度；省略则参与剩余空间分配 */
  width?: number;
  align?: "left" | "right" | "center";
  sortable?: boolean;
  /** 自定义渲染：状态徽标、相对时间、标签组等 */
  render?: (row: T) => ReactNode;
}

export interface RowAction<T> {
  label: string;
  permission?: string;
  /** 跳转型：去详情/编辑 */
  to?: (row: T) => Partial<Route>;
  /** 执行型：调 actions 端点；destructive 的会先确认 */
  action?: (row: T) => Promise<void>;
  destructive?: boolean;
  /** 条件显示，如「已停用的主机不显示终端」 */
  when?: (row: T) => boolean;
}
```

一个资源页因此就是一个常量加一行 JSX：

```tsx
const hosts: ListConfig<Host> = {
  resource: "hosts",
  rowKey: (h) => h.id,
  filters: [
    { key: "q", label: "名称/地址", type: "text" },
    { key: "group_id", label: "主机组", type: "select", options: loadGroups },
    { key: "connection", label: "接入方式", type: "select",
      options: [{ value: "runner", label: "代理" }, { value: "ssh", label: "SSH" }] },
    { key: "state", label: "状态", type: "select", options: hostStates },
  ],
  columns: [
    { key: "name", label: "名称" },
    { key: "address", label: "地址", width: 180 },
    { key: "state", label: "状态", width: 100, render: (h) => <State value={h.state} /> },
    { key: "seen_at", label: "最近心跳", width: 160, sortable: true, render: (h) => <Ago at={h.seen_at} /> },
  ],
  actions: [
    { label: "终端", permission: "hosts.terminal", to: (h) => ({ id: h.id, mode: "view" }) },
    { label: "编辑", permission: "hosts.update", to: (h) => ({ id: h.id, mode: "edit" }) },
    { label: "删除", permission: "hosts.delete", destructive: true, action: remove },
  ],
  toolbar: { create: true, import: true, export: true },
};

export const HostsPage = () => <ResourceTable config={hosts} />;
```

组件自己负责：URL 同步、分页、定时刷新、选中集合、导入导出、加载/空/错误态、权限过滤、以及下面的 sticky 布局。

### 7.3 页面骨架与 sticky 规则

```
┌─ 顶栏 ─────────────────────────────────────────────────────┐
│ Ferry │ 仪表盘 部署 构建 主机 维护 │              工作空间 ▾ │
├───────┬────────────────────────────────────────────────────┤
│ 主机组 │ 工作空间 / 主机 / 主机            ← 面包屑          │
│ 主机  │ 主机                              ← 页面标题        │
│ 标签  │ ┌────────────────────────────────────────────────┐ │
│ 定时  │ │ 名称 [    ] 组 [ ▾] 状态 [ ▾]                  │ │  ← 过滤区（固定）
│       │ │ [搜索] [清空]   [刷新|▾] [导入] [导出] [+ 创建]  │ │
│       │ ├────┬──────────┬────────┬──────────┬───────────┤ │
│       │ │ #  │ 名称      │ 状态   │ 最近心跳  │ 操作       │ │  ← 表头（sticky top）
│       │ ├────┼──────────┼────────┼──────────┼───────────┤ │
│       │ │ 1  │ web-01   │ 在线   │ 3 秒前    │ 终端 编辑… │ │  ← 仅此区域滚动
│       │ │ 2  │ web-02   │ 离线   │ 2 天前    │ 终端 编辑… │ │
│       │ │ ↕  │          │        │          │           │ │
│       │ ├────┴──────────┴────────┴──────────┴───────────┤ │
│       │ │ ☐ 全选 已选 2 项 [批量删除]   ‹ 1 2 3 … 99 ›  20▾│ │  ← 底部（固定）
│       │ └────────────────────────────────────────────────┘ │
└───────┴────────────────────────────────────────────────────┘
    ↑ 第一列（序号）sticky left，最后一列（操作）sticky right
```

实现要点，写死在组件的样式里：

```css
/* 内容区是唯一的滚动所有者：过滤区与底部 auto，表格区 1fr。
   min-height:0 不能省——grid 子项默认 min-height:auto，省了就不会收缩，
   整页会一起滚，四边固定全部失效。 */
.list        { display: grid; grid-template-rows: auto 1fr auto; min-height: 0; }
.list-scroll { overflow: auto; min-height: 0; }

table        { border-collapse: separate; border-spacing: 0; width: 100%; }
thead th     { position: sticky; top: 0;    z-index: 3; }
th:first-child, td:first-child { position: sticky; left: 0;  z-index: 2; }
th:last-child,  td:last-child  { position: sticky; right: 0; z-index: 2; }
/* 表头与首/末列的交叉格要盖住两者 */
thead th:first-child, thead th:last-child { z-index: 4; }
/* sticky 单元格默认透明，必须给不透明底色，否则滚动时字叠字 */
th, td:first-child, td:last-child { background: var(--panel); }
td:first-child { box-shadow: 1px 0 0 var(--line); }
td:last-child  { box-shadow: -1px 0 0 var(--line); }
```

「撑满剩余空间」的做法：`table` 宽 100%，固定宽度的列写 `width`，**留一列不写**（通常是名称列）吸收剩余宽度；同时给表格一个 `min-width`（各列最小宽之和），窗口更窄时横向滚动，而不是把列挤变形。

底部是 grid 的第三行，在滚动容器**外面**——用 sticky 做底部在 iOS Safari 的橡皮筋回弹里会抖，放外面没有这个问题。

### 7.4 工具栏行为

| 控件 | 行为 |
|---|---|
| 搜索 | 提交过滤区，写 URL，回到第 1 页 |
| 清空 | 清掉所有过滤参数与 `page`，保留 `page_size` |
| 刷新 ▾ | 左半立即刷新；右半下拉选「关闭 / 10 秒 / 30 秒 / 1 分钟」，存 `localStorage`，页面不可见时自动暂停 |
| 导入 | 上传 → 预览（逐行错误）→ 确认提交 |
| 导出 | 带上当前全部过滤条件，服务端流式生成 |
| 创建 | 跳 `/new`，同一套表单组件 |

---

## 8. 统一表单 DSL 组件

**详情页就是编辑页的只读模式**，一个组件三种模式，由路由决定：`new` / `edit` / `view`。

```ts
export interface FormConfig<T> {
  resource: string;
  /** 分区标题 + 字段 */
  sections: { title?: string; fields: FormField[] }[];
  /** 详情页标题取自哪个字段 */
  heading: (row: T) => string;
  /** 详情页下方的关联子列表，直接复用 ListConfig */
  related?: { label: string; config: ListConfig<unknown> }[];
}

export interface FormField {
  key: string;
  label: string;
  type: "text" | "textarea" | "number" | "select" | "multi-select" | "switch"
      | "password" | "secret" | "date" | "code" | "tags" | "key-value";
  required?: boolean;
  help?: string;
  placeholder?: string;
  options?: Option[] | (() => Promise<Option[]>);
  /** 条件显示：SSH 接入才显示地址/端口/凭据 */
  when?: (draft: Partial<T>) => boolean;
  /** 创建后不可改，如主机的接入方式 */
  immutable?: boolean;
  /** 栅格占位，1~2 */
  span?: 1 | 2;
}
```

只读模式**把值渲染成文本，不是渲染成 disabled 的输入框**——禁用态的输入框对比度低、不好读、也不好复制。`secret` 类型在只读模式只显示「已设置」或「未设置」，永不显示明文；要看明文需要 `variables.reveal` 权限并单独调一次接口，这次调用会进操作记录。

离开有未保存内容的表单时弹确认，并拦住浏览器后退键——easy-erp 的 `useNavigation` 已经把这件事做对了（`popstate` 时按历史 index 差值 `history.go(offset)` 回滚），直接搬过来。

---

## 9. 主机接入：runner 与 SSH

这是本项目相对 easy-erp 全新的部分，也是设计上最需要想清楚的地方。

### 9.1 两种方式，一个抽象

| | runner 代理 | SSH 直连 |
|---|---|---|
| 连接方向 | 主机 → 服务（反向） | 服务 → 主机 |
| 能否穿 NAT | 能 | 不能，要求服务可达主机 22 |
| 主机上要装东西 | 要，一条命令 | 不要 |
| 凭据存哪 | 主机持有自己的 key | 服务保管密码/私钥 |
| 终端 / 文件传输 | 支持 | 支持 |

两者的差异**只存在于通道层**。上面所有功能（终端、文件、部署步骤、流水线步骤、定时任务执行）只认一个 trait：

```rust
#[async_trait]
pub trait HostChannel: Send + Sync {
    async fn exec(&self, command: &str, timeout: Duration) -> Result<Output>;
    async fn pty(&self, cols: u16, rows: u16) -> Result<PtySession>;
    async fn read_file(&self, path: &str) -> Result<(FileMeta, ByteStream)>;
    async fn write_file(&self, path: &str, mode: u32, body: ByteStream) -> Result<()>;
    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>>;
}

/// 唯一的分派点。接第三种接入方式时只改这里。
pub async fn open(s: &AppState, host: &Host) -> Result<Box<dyn HostChannel>> {
    match host.connection {
        Connection::Runner => RunnerChannel::open(s, host).await,
        Connection::Ssh    => SshChannel::open(s, host).await,
    }
}
```

### 9.2 runner：一条命令加主机

主机页「添加主机 → 代理方式」给出一条命令，带一个有时效的一次性注册令牌：

```sh
curl -fsSL https://ops.example.com/api/runners/install.sh | sudo sh -s -- \
  --server https://ops.example.com --token 7f3a…
```

`install.sh` 由服务动态生成（内含 server 地址与令牌），脚本做四件事：识别 os/arch → 从 `/api/runners/binary/{os}-{arch}` 下载对应二进制 → 装到 `/usr/local/bin/ferry-runner` 并写 systemd unit → 启动。

注册与保活：

```
runner                                    server
  │── WS /api/runners/connect ───────────▶│
  │   Hello{version,token,hostname,os,…}  │  校验令牌 → upsert hosts 行
  │◀── Welcome{host_id, key, heartbeat} ──│  令牌作废，下发长期 key
  │   （落盘 /var/lib/ferry-runner/runner.json，之后用 key 重连）
  │◀── Ping ──────────────────────────────│
  │── Pong / Metrics{load,mem,disk} ─────▶│  仪表盘的健康数据来源
```

**控制连接不承载任何批量数据。** 需要终端、上传、下载时，服务下发 `OpenSession{session, kind}`，runner 为它**单独拨一条连接**：

```
server ── OpenSession{session:"a1b2", kind:Terminal} ──▶ runner
runner ── WS /api/runners/sessions/a1b2 ──────────────▶ server
          （此后这条连接只跑这一个会话：文本帧是控制，二进制帧是数据）
```

```rust
// ponytail: 一会话一连接，不写多路复用器。运维主机同时开的会话是个位数；
// 真的不够了再上 yamux。
```

这样换来的好处是实打实的：没有 stream id、没有流控、没有队头阻塞、没有半关闭状态机——这几样是自研多路复用最容易出错的地方，而代价只是多几个 TCP 连接。

### 9.3 SSH 直连

- `hosts` 存 `address`、`port`、`username`、`credential_id`；凭据（密码或私钥）存在 `credentials` 表，加密后落库，见 9.6。
- 用 `russh` 外拨，`russh-sftp` 做文件；两者都是纯 Rust 异步，不依赖系统 ssh 客户端，容器里不用额外装东西。
- **主机密钥 TOFU**：首次连接记下指纹存 `hosts.host_key`；之后指纹变了直接拒绝并要求人工确认。不做「永远接受」的开关——那等于关掉中间人防护。
- 连接按需建立，不长连；带超时和失败退避，失败原因原样回显给操作者（「认证失败」和「网络不可达」要能区分）。

### 9.4 远程终端

```
浏览器 xterm.js ──WS──▶ /api/workspaces/{workspaceId}/nodes/{id}/terminal-sessions ──▶ HostChannel::pty
```

- 浏览器到服务的 WS 与服务到主机的通道**对拷字节**，中间不解析、不缓冲整屏。
- 二进制帧 = pty 字节流；文本帧 = `{"type":"resize","cols":120,"rows":32}`。xterm.js 配 fit addon，窗口变化就发 resize。
- 开终端要 `hosts.terminal` 权限，开启/关闭都写操作记录（谁、哪台、什么时候、多久）。
- 会话录像第一版不做，见第 16 节待确认 6。

### 9.5 双向文件传输

```
GET    /api/workspaces/{workspaceId}/nodes/{id}/files?path=/etc            目录列表
GET    /api/workspaces/{workspaceId}/nodes/{id}/files/content?path=/etc/nginx.conf   下载
PUT    /api/workspaces/{workspaceId}/nodes/{id}/files/content?path=…        上传
DELETE /api/workspaces/{workspaceId}/nodes/{id}/files?path=…                删除
```

- **两个方向都流式**，服务不把整个文件读进内存，也不在服务端落盘中转。大文件传输不应该让服务的内存随文件大小走。
- 需要 `hosts.files` 权限；每次传输写操作记录（路径、字节数、方向、结果）。
- 界面是左右双栏文件管理器 + 拖拽上传，和终端同一个页面的两个标签页。

### 9.5.1 多实例连接与监控采集

可选 NATS JetStream 模式通过 KV 维护节点连接所属服务、单次会话路由和分布式升级预约。
浏览器请求、Runner 控制连接、专用会话回连可以分别落在三个服务副本；业务授权仍由工作空间 Scope 校验，
回连仍校验节点密钥与一次性会话绑定。消息服务仅保存短期协调状态，实时流使用有界队列、分片与逐包确认，
不持久化、不重放命令。服务或消息链路故障会明确结束会话，重新连接后由用户打开新会话。
连接归属 30 秒过期、5 秒续约，SQL 状态写入包含同语句归属条件，防止旧服务覆盖新连接状态。
升级预约覆盖完整任务及交互会话；写预约阻止新任务，状态不确定时继续阻止派发，交由当前所属服务核实。

所有副本使用同一业务数据库、主密钥、NATS 命名空间，但使用独立数据目录及实例身份；
部署参数与 NATS 副本要求见 README「多实例节点通信」。NATS 访问属于可信服务边界。
监控由唯一所属服务每 5 秒后台采集，最近一小时的真实时间戳样本仅在该服务内存保留，
其他副本通过 RPC 查询；HTTP 读取不触发远程采集或写入监控数值。
状态摘要每 30 秒写入业务库以兼容列表与报警。重启或归属切换重新积累历史，不补零或复制单点。
图表按真实时间坐标显示 5 分钟 / 15 分钟 / 1 小时，可暂停或选择 5 / 15 / 30 / 60 秒刷新。
单点只显示点，缺失样本断开连线。长期历史 Redis / TSDB 不在本轮范围。

### 9.6 凭据与密钥的保管

密码、私钥、密钥变量走同一个保管机制，不许各自实现一遍：

```
FERRY_MASTER_KEY（Docker：环境变量；Desktop：data_dir 下 0600 的密钥文件）
   └─ 包裹每个工作空间的数据密钥
        └─ 加密 credentials.secret / variables.value
```

- 密文只在真正要用的那一刻解开，解开的明文不进日志、不进错误消息、不进审计详情。
- 列表与详情接口**从不返回密文或明文**，只返回 `has_value: true`。
- 看明文要 `variables.reveal` 权限 + 单独一次接口调用，这次调用必定进操作记录。
- 没配 `FERRY_MASTER_KEY` 的 Docker 部署，服务启动时就拒绝，不静默退化成明文存储。

---

## 10. 部署：一份通用 manifest，多种部署模式

需求是「一个通用的 manifest，然后可以选择部署模式为 native / docker / docker compose / docker stack / k8s / helm」。这一节是它的落地设计。

### 10.1 结论先行：manifest 描述意图，模式是渲染器

把「要跑什么」和「怎么跑」彻底分开：

```
应用 manifest（意图：服务、镜像、端口、环境、卷、健康检查、副本）
        │
        ├─ 能力校验（这个模式支不支持 manifest 用到的东西）
        ├─ 渲染（纯函数：manifest + 目标配置 → 产物）
        │     native  → systemd unit + 环境文件
        │     docker  → docker run 参数
        │     compose → compose.yaml
        │     stack   → stack.yaml（Swarm）
        │     k8s     → Deployment/Service/Ingress/ConfigMap/Secret
        │     helm    → values.yaml（+ 指定的 chart）
        ├─ 预览与差异（apply 前看到逐字产物和与线上的 diff）
        └─ 应用（native/docker/compose/stack 经 HostChannel；k8s/helm 经集群凭据）
```

**渲染必须是纯函数**：同样的 manifest 加同样的目标配置，永远渲染出同样的字节。这样预览才可信，差异才有意义，回滚才只是「重新应用上一版产物」。

### 10.2 manifest 长什么样

一份 YAML（库里存 JSON），描述一个应用，不描述它跑在哪：

```yaml
apiVersion: ferry/v1
kind: Application
metadata:
  name: web-api
  labels: { team: platform }

services:
  - name: api
    # 两种来源二选一：容器镜像，或可执行产物（native 模式用）
    image: ghcr.io/acme/web-api:1.4.2
    artifact:
      source: pipeline            # 来自构建模块的产物
      pipeline: web-api-release
      path: dist/web-api
    command: ["--config", "/etc/web-api.toml"]
    replicas: 2
    ports:
      - { name: http, container: 8080, published: 80, protocol: tcp }
    env:
      - { name: LOG_LEVEL, value: info }
      - { name: DATABASE_URL, valueFrom: variable/db-url }      # 密钥变量引用
    files:
      - { path: /etc/web-api.toml, fromTemplate: web-api-config }
    volumes:
      - { name: data, path: /var/lib/web-api, size: 10Gi }
    health:
      readiness: { httpGet: { path: /healthz, port: http }, periodSeconds: 10 }
      liveness:  { httpGet: { path: /healthz, port: http }, periodSeconds: 30 }
    resources:
      requests: { cpu: 100m, memory: 128Mi }
      limits:   { cpu: "1",  memory: 512Mi }
    dependsOn: [redis]

  - name: redis
    image: redis:7-alpine
    ports: [{ name: redis, container: 6379 }]

strategy:
  type: rolling            # rolling | recreate
  maxUnavailable: 0
  healthTimeout: 120s

# 逃生舱：渲染结果上的原样合并，按模式分开。没有它，所有抽象迟早被绕开。
overrides:
  k8s:
    spec:
      template:
        spec:
          nodeSelector: { disktype: ssd }
  compose:
    services:
      api:
        logging: { driver: json-file, options: { max-size: 10m } }
```

要点：

- **`valueFrom: variable/db-url` 是唯一的取密方式**，manifest 里永不写明文。渲染时按模式落到对应载体（k8s Secret / compose 的 `env_file` / systemd 的 `EnvironmentFile`，文件权限 0600），密文在服务端解开，不进产物预览，不进操作记录。
- **`artifact` 与 `image` 二选一**，让 native 模式和容器模式共用一份描述。
- **`overrides` 按模式分开**，是原样合并到渲染结果上的。这是整套设计能不能活下去的关键：任何一个「通用模型」都一定有覆盖不到的字段，没有逃生舱的人会直接放弃这套工具去手写 YAML。

### 10.3 能力矩阵：差异要提前报错，不要悄悄降级

六种模式的能力不一样。**在保存 manifest 的时候就校验，不要等到 apply**；也绝不悄悄按最低能力降级——「我配了 2 个副本，线上只有 1 个」是最难查的那类问题。

| 能力 | native | docker | compose | stack | k8s | helm |
|---|:--:|:--:|:--:|:--:|:--:|:--:|
| 多服务 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `replicas > 1` | ✗ | ✗ | ✓¹ | ✓ | ✓ | ✓ |
| 滚动更新 | ✗ | ✗ | ✗ | ✓ | ✓ | ✓ |
| 健康检查 | ✓² | ✓ | ✓ | ✓ | ✓ | ✓ |
| `resources.limits` | ✓³ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 持久卷 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 服务发现 | ✗⁴ | ✗⁴ | ✓ | ✓ | ✓ | ✓ |
| 部署目标 | 主机 | 主机 | 主机 | Swarm | 集群 | 集群 |

¹ 同主机多实例，端口要用范围或不发布。² systemd 的 `ExecStartPost` + 重启策略，不是真正的就绪探针。³ systemd 的 `CPUQuota` / `MemoryMax`。⁴ 只能用固定地址或主机名。

校验信息要具体：不是「不支持」，而是「native 模式一台主机只跑一个实例；要 2 个副本请改用 compose/stack/k8s，或把副本部署到主机组的多台主机上」。

### 10.4 部署目标不总是主机

这是容易设计错的地方：**native / docker / compose / stack 部署到主机（或主机组），k8s / helm 部署到集群**。所以部署记录上的目标是一个二选一：

```
deployments.target_kind  ∈ host | host-group | cluster
deployments.target_ref   → hosts.id | host_groups.id | clusters.id
```

- 主机类目标经 §9 的 `HostChannel` 执行，runner 和 SSH 两种接入方式都能用，不需要额外通道。
- 集群类目标需要新增一个 `clusters` 资源（kubeconfig 存进 §9.6 的保管机制），走 Kubernetes API，不经过 `HostChannel`。
- 部署到**主机组**时，逐台滚动，失败即停，已成功的不自动回滚——自动回滚一半主机会留下更难理解的混合状态，由人决定继续还是回退。

### 10.5 接口

```
GET    /api/workspaces/{workspaceId}/deployments                      列表
POST   /api/workspaces/{workspaceId}/deployments                      创建（manifest + 模式 + 目标）
GET    /api/workspaces/{workspaceId}/deployments/{id}                 详情
PATCH  /api/workspaces/{workspaceId}/deployments/{id}                 修改
POST   /api/workspaces/{workspaceId}/deployments/{id}/renderings  纯渲染，不接触目标
POST   /api/workspaces/{workspaceId}/deployments/{id}/previews  渲染 + 与线上对比
POST   /api/workspaces/{workspaceId}/deployments/{id}/executions   执行
POST   /api/workspaces/{workspaceId}/deployments/{id}/rollbacks 回滚到指定 revision
GET    /api/workspaces/{workspaceId}/deployments/{id}/revisions       历史
```

`render` 不碰目标，所以给所有能读的人用；`apply` 要 `deployments.apply`，`rollback` 要 `deployments.rollback`。

**每次 apply 把渲染产物整份存进 `deployment_revisions`**，不只存 manifest。镜像 tag 会被覆盖、chart 会更新、模板会变——只存 manifest 的「回滚」可能渲染出和当时不一样的东西。存产物，回滚才是确定的。

### 10.6 应用模版与应用市场

- **应用模版**（`app_templates`）＝ 参数化的 manifest。`{{ .Values.image.tag }}` 这类占位符 + 参数定义（类型、默认值、校验、是否密钥）。创建部署时选模版、填参数、生成 manifest。
- **应用市场**（`marketplace`）＝ 模版的来源目录。第一版建议用**仓库索引**：一个 Git 仓库里放模版清单，工作空间订阅它，定时同步。这样「市场」不需要中心化服务，自建和公共源是同一套机制。见第 16 节待确认 7。

Helm 是个特例：它自己就是模版系统。Ferry 不重新实现它——helm 模式下 manifest 只负责产出 `values.yaml` 和 chart 引用，渲染交给 helm。**不要把 Go template 再包一层。**

### 10.7 实现顺序

能力矩阵上从窄到宽，但每一步都要能真正跑通：

1. `compose` —— 最常用，产物可读，本地就能验证渲染器和 `overrides` 合并逻辑。
2. `docker` —— 同一渲染器去掉多服务编排，验证单服务路径。
3. `native` —— systemd，验证非容器路径和 `artifact` 来源。
4. `k8s` —— 引入 `clusters` 资源和集群通道。
5. `stack` —— 在 `compose` 渲染器上加 Swarm 字段。
6. `helm` —— 在 `k8s` 的集群通道上加 helm 客户端。

```
// ponytail: 渲染器一个模式一个函数，共用一个 manifest 解析和一个 overrides 合并。
// 不要先造「渲染器插件框架」——六个函数还不值得一个注册表。
```

## 11. 共享资源：定时任务与密钥变量

需求明确说了「定时任务虽然多个菜单有，但是接口和逻辑都是一套」。落地方式：

### 10.1 定时任务

一张表、一个模块、一个 API、一个调度器。三个菜单项是**同一个页面的三种预置过滤**。

```
GET  /api/workspaces/{workspaceId}/cron-jobs?scope=host|build|deploy
POST /api/workspaces/{workspaceId}/cron-jobs/{id}/runs
GET  /api/workspaces/{workspaceId}/cron-jobs/{id}/runs        执行历史
```

`scope` 只影响两件事：在哪个菜单下列出，以及创建时可选的动作类型（主机 scope 给「在主机/主机组上执行命令」，构建 scope 给「触发流水线」，部署 scope 给「重启/重新部署应用」）。**调度、并发控制、超时、重试、历史记录、错误通知全部是同一份代码。**

调度器一个 tokio task 轮询 `next_run_at`，到点取任务、乐观加锁（`UPDATE … WHERE next_run_at = ?` 看影响行数）、执行、写 `cron_runs`、算下一次。

```
// ponytail: 单调度器 + 数据库乐观锁。一个服务实例跑得动；
// 真要多实例了再换成带租约的抢占。
```

### 10.2 密钥变量

```
GET /api/workspaces/{workspaceId}/variables?scope=global|deploy|build&scope_ref={id}
```

同一张表、同一套接口。「维护 → 密钥变量（全局）」就是 `scope=global`。读取时按「全局 → 模块 → 具体对象」逐层覆盖合并，越具体的优先；界面在详情里标出每个值来自哪一层，不然排查「为什么这个变量不是我设的值」会很痛苦。

### 10.3 字典与配置项

- **字典**：枚举值的集中管理（主机状态、环境名、应用分类…），其它资源的 `select` 选项从这里取，避免散落的硬编码。
- **配置项**：工作空间级的运行参数（默认页大小、心跳超时、保留多少执行历史…），键值对 + 类型 + 校验。

---

## 12. 数据模型

全部业务表共有：`id TEXT PK`、`workspace_id TEXT`、`created_at BIGINT`、`updated_at BIGINT`、`version BIGINT`；时间统一毫秒时间戳。**整数列一律 `BIGINT`**，原因见 §3.1。骨架已建立身份与租户基座（`users` `sessions` `workspaces` `workspace_roles` `workspace_members` `activity`），其余按模块分批迁移。

| 模块 | 表 | 要点 |
|---|---|---|
| 身份 | `users` `sessions` | 全局，不属于任何工作空间 |
| 租户 | `workspaces` `workspace_members` `workspace_roles` | `join_code` 承载邀请；`permissions` 存 JSON 数组 |
| 主机 | `hosts` | `connection` ∈ runner/ssh；`state` `seen_at` `host_key` `runner_key_hash` `metrics` |
| 主机 | `host_groups` `tags` `host_tags` | 组是树（`parent_id`），标签是多对多 |
| 主机 | `runner_tokens` | 一次性注册令牌，带 TTL 与使用痕迹 |
| 构建 | `repositories` `pipelines` `pipeline_templates` `pipeline_runs` `pipeline_steps` | 流水线定义存 JSON，运行实例拆行便于查询 |
| 分模块管理 | `credentials` | `scope` ∈ host/build/deploy，创建后固定；各模块独立列表及选择器，底层统一加密；`kind` ∈ password/ssh-key/token |
| 部署 | `deployments` `app_templates` `deployment_revisions` | `target_kind`/`target_ref` 指向主机、主机组或集群；revision 存**渲染产物**而非 manifest（§10.5） |
| 部署 | `clusters` | k8s/helm 的目标；kubeconfig 走 §9.6 的保管机制 |
| 部署 | `alert_rules` `alert_events` | 规则 + 触发记录 |
| 共享 | `cron_jobs` `cron_runs` | 一套，`scope` 区分归属 |
| 共享 | `variables` | 一套，`scope` + `scope_ref` 分层 |
| 维护 | `settings` `dictionaries` `dictionary_items` | 工作空间级 |
| 维护 | `activity` | 操作记录，所有写操作与敏感读操作都落这里 |

迁移规则，与 easy-erp 相同：**纯 SQL、按顺序追加、发布后不再改动已有迁移**。两种数据库的 DDL 差异集中在迁移模块一处处理。

---

## 13. 仪表盘

一个接口回全部数据，不让首页发十几个请求：

```
GET /api/workspaces/{workspaceId}/dashboard
→ {
    counts:  { hosts: 42, deployments: 18, pipelines: 7, cron_jobs: 12, ... },
    states:  { hosts: { online: 38, offline: 3, unknown: 1 }, deployments: {...} },
    health:  { runner_lag_seconds: 3, failed_cron_runs_24h: 1, alerts_open: 2 },
    recent:  [ ...最近活动与失败事件... ]
  }
```

- 资源数量、状态分布、健康监控三块，对应需求里的「所有资源数量、状态分布、健康监控」。
- 健康数据来自 runner 上报的 `Metrics` 与各模块的失败计数，不另起采集链路。
- 页面可见时按配置的间隔刷新，不可见时暂停——和列表的自动刷新用同一个 hook。

---

## 14. 安全

沿用 easy-erp 已验证的那套，加上多租户与远程通道特有的部分。

| 面 | 措施 |
|---|---|
| 会话 | Cookie HttpOnly + SameSite=Lax + 可配 Secure；服务端可吊销；滑动续期 |
| CSRF | 所有非幂等请求必须带 `X-Ferry-Request: 1`；校验 `Origin` 与 `Host` |
| Host 头 | 允许列表（`FERRY_ALLOWED_HOSTS`），不在列表内直接 403 |
| 响应头 | CSP、`X-Content-Type-Options`、`Referrer-Policy`、`Cache-Control: no-store` |
| 租户隔离 | 唯一入口是 `Scope` 提取器；`workspace_id` 绝不取自请求体 |
| 密码 | argon2；登录失败按来源计数退避 |
| 密钥 | 分层加密（9.6）；列表/详情不回明文；查看明文要权限且进审计 |
| 主机密钥 | SSH 首次 TOFU，变更即拒；runner 用长期 key，可在界面吊销重签 |
| 注册令牌 | 一次性、带 TTL、可撤销；用过即作废 |
| 审计 | 所有写操作 + 终端开启/关闭 + 文件传输 + 密钥明文查看，全部进 `activity` |
| 桌面形态 | 只有 `launcher` 窗口有原生权限；业务窗口是普通 WebView |

---

## 15. 交付阶段

| 阶段 | 内容 | 验收 |
|---|---|---|
| 0 ✅ | 项目骨架：三个 crate + Tauri 外壳 + 界面外壳 + Docker | `cargo check --workspace`、`npm run build`、服务起得来并应答 |
| 1 ✅ | 注册登录、工作空间创建/加入、角色权限、`Scope` 提取器 | 两个账号互不可见（陌生人访问他人空间返回 404）；内置角色依赖闭包有测试 |
| 2 ✅ | 列表 DSL + 表单 DSL，shadcn 主题（默认 + erp），主机组/主机/标签/操作记录 | 四边 sticky 浏览器实测通过；URL 含过滤/分页/排序/滚动；分享链接登录后直达原位置 |
| 1 | 注册登录、工作空间创建/加入、角色权限、`Scope` 提取器 | 两个账号两个空间，互相看不到对方数据 |
| 2 | 列表 DSL + 表单 DSL 组件，用主机组/标签两个资源验证 | 四边 sticky 在三种窗口宽度下正确；URL 含全部视图状态；分享链接登录后直达含滚动位置 |
| 3 | 主机 CRUD + runner 接入 + SSH 接入 + 终端 + 文件传输 | 同一台主机两种方式接入，终端与双向传输都通；断网重连；审计齐全 |
| 4 | 仓库、流水线、流水线模版、凭据 | 一条真实流水线跑通 |
| 5 | 应用部署（manifest + 六种模式，按 §10.7 顺序）、应用模版、应用市场、报警监控 | compose 模式一次部署加一次回滚；渲染产物逐字可预览 |
| 6 | 共享定时任务与密钥变量、字典、配置项、操作记录 | 三个菜单入口确认是同一套接口与逻辑 |
| 7 | 仪表盘 | 数量/分布/健康与各模块实际数据一致 |
| 8 | 桌面打包与自动更新、发布流程 | 三平台安装包；升级不丢数据 |

阶段 2 是整个项目的杠杆点：**DSL 组件的配置能力决定后面每个资源页是三十行还是三百行**，宁可在这一阶段多花时间。

---

## 16. 待确认项

1. **「部署」与「应用」是不是同一个一级菜单？** 需求顶部列的是「部署」，左侧菜单描述的是「应用」。本文按同一模块处理、标签取「部署」。如果是两个独立模块，`nav.ts` 加一组即可，但两者的左侧菜单要重新划分。
2. ~~数据库选 PostgreSQL 还是 MySQL？~~ **已定：三种都支持**（SQLite / MySQL / PostgreSQL），见 §3.1。`scripts/test-backends.sh` 对三者跑同一套 30 项功能测试，CI 每次推送都跑。
3. **加入工作空间要不要审批？** 本文按「邀请码加入即生效，获得空间默认角色」。要审批就给 `workspace_members.state` 加 `pending` 流程与一个待审列表。
4. **滚动位置用像素还是首行 id？** 本文用像素（实现简单）。跨设备分享、或页大小不同会有偏差；要精确就改成记首个可见行的 id，组件内部一处改动。
5. **runner 二进制怎么分发？** 本文按「随服务镜像一起带，由服务提供下载」。另一条路是从 GitHub Release 拉，镜像更小但主机需要外网。
6. **终端会话录像与命令级审计第一版要不要？** 本文只记开启/关闭，不记录击键与输出。合规场景常要全量录像，加上会影响存储与通道设计，要早定。
7. **应用市场的内容从哪来？** 内置清单 / 远程索引 / 自建仓库，三种的权限与更新机制都不一样。

第 1 条会影响骨架，建议先定；其余可以在对应阶段开始前定。

## 17. 阶段 4 设计：构建模块的最小闭环

目标是跑通一次真实的「拉代码 → 执行步骤 → 产出日志」，而不是先造一套编排抽象。模版、并行、条件、矩阵等能力等这条路通了再加——先造编排容易造出用不上的东西。

### 17.1 数据模型

| 表 | 作用 | 要点 |
|---|---|---|
| `repositories` | Git 仓库登记 | `url`、`default_branch`、`credential_id`（复用 §9.6 的凭据保管） |
| `pipelines` | 流水线定义 | `definition` 存 JSON；`target_kind`/`target_ref` 指向主机或主机组 |
| `pipeline_runs` | 一次执行 | 工作空间内自增 `number`、`state`、触发方式与触发人 |
| `pipeline_run_steps` | 每步的结果 | `index`、`state`、`exit_code`、起止时间 |
| `pipeline_run_logs` | 日志分片 | `seq` 单调递增，前端按 `seq` 增量拉取 |

T4 落地字段：

- `pipeline_runs`：`id/workspace_id/pipeline_id/host_id/credential_id/triggered_by` 为 BIGINT；`number` 工作空间内递增；`state` 为 queued/running/succeeded/failed/cancelled；`trigger_kind`、定义与仓库 URL/branch/commit 快照、创建/开始/结束时间。唯一索引 `(workspace_id, number)`，队列索引 `(state, created_at)`。
- `pipeline_run_steps`：雪花 `id`、`run_id`、`step_index`（从 0 开始）、名称、状态、可空 `exit_code`、起止时间；唯一索引 `(run_id, step_index)`。状态包含 `timed_out` 和 `skipped`，超时无退出码。
- `pipeline_run_logs`：雪花 `id`、`run_id`、`seq`、`step_index`、`stream`、`chunk`；唯一索引 `(run_id, seq)`。脱敏后按 UTF-8 边界切分，每片最多 16 KiB。
- `workspaces.pipeline_run_number` 初始 0，事务内先原子 `UPDATE counter = counter + 1` 再读取本事务的值，与 run 和步骤一起提交；不使用 `MAX(number)+1`。

日志单独成表而不是塞进 run 的一个大字段：一次构建的日志可以是几十 MB，整行更新会让每次追加都重写整列。分片之后追加是插入，增量读取是一次按 `seq` 的范围查询。

### 17.2 流水线定义

```json
{
  "workdir": "/tmp/ferry/{{run}}",
  "env": [{ "name": "LOG_LEVEL", "value": "info" }],
  "steps": [
    { "name": "拉取代码", "run": "git clone --depth 1 -b {{branch}} {{repo_url}} ." },
    { "name": "构建",     "run": "cargo build --release", "timeout_seconds": 1800 }
  ]
}
```

- `workdir` **必填**。不给默认值是有意的：默认一个目录意味着两条流水线可能悄悄写到同一处，而「构建产物莫名被另一条流水线覆盖」极难排查。让运维人员明确写出来。
- **第一版只有线性步骤**，没有并行、条件、矩阵。主机组目标在触发时从本组直接成员中按 ID 选第一台在线主机，固定为 run.host_id；不展开成多机执行，空组或没有在线成员时拒绝触发。
- 模板变量只有 `{{run}}`、`{{branch}}`、`{{repo_url}}`、`{{commit}}` 这几个，由服务端替换；**不做通用模板引擎**。
- 仓库凭据在执行时由服务端解密，通过 `GIT_ASKPASS` 或临时 `~/.git-credentials` 注入，**不拼进命令行**——命令行会出现在目标主机的进程列表里。

### 17.3 执行引擎

执行只经过 §9.1 的 `HostChannel`，所以 runner 主机和 SSH 主机跑同一套逻辑：

```
排队 → 取一条 run → 打开通道 → 逐步 exec → 每步写 steps 与 logs
     → 任一步非零退出即停止，后续步骤标记 skipped → 标记 run 结束
```

- **失败即停**，不自动重试：构建失败重跑一次通常只是把问题推迟。
- 每步有超时（默认 1800 秒），超时步骤记 `timed_out`、run 记 `failed`，不是普通非零退出的 `failed` 步骤。
- **取消只停止服务端等待并标记 cancelled，不保证远端命令立即停止。** HostChannel 捕获完整输出后返回，远端进程可能继续运行直到自身退出。取消后的临时凭据清理由执行器继续等待完成，因此 cancelled 状态可能先于清理完成；界面不可将其表述为“远端已停止”。
- 凭据通过 HostChannel 文件传输写入随机的 0700 临时目录；密码/令牌文件权限 0600，GIT_ASKPASS 脚本 0700。私钥同样写入 0600 文件。步骤脚本也上传为文件，exec 参数仅引用路径。成功、失败、超时、取消均清理临时目录，日志在分片前脱敏，activity 只记录 run ID/序号。
- 单进程每次领取一条 queued run；条件更新防止重复领取。服务重启时将遗留 running 标记失败，不自动重试远端可能已执行的命令。
- `HostChannel::exec` 捕获完整输出后才返回，所以第一版的日志是**按步落库**，不是边跑边流。实时性由 17.4 的轮询提供。真正的流式需要 `HostChannel` 增加一个流式 exec，等这条路通了再说。

```
// ponytail: 单进程调度器 + 数据库乐观锁取任务，和定时任务用同一套思路。
// 一个服务实例够用；要多实例了再换成带租约的抢占。
```

### 17.4 接口

```
GET    /api/workspaces/{workspaceId}/repositories            仓库（标准 CRUD）
GET    /api/workspaces/{workspaceId}/pipelines               流水线（标准 CRUD）
POST   /api/workspaces/{workspaceId}/pipelines/{id}/runs     触发一次执行
GET    /api/workspaces/{workspaceId}/pipeline-runs                  执行历史
GET    /api/workspaces/{workspaceId}/pipeline-runs/{id}             一次执行的详情与各步状态
GET    /api/workspaces/{workspaceId}/pipeline-runs/{id}/logs?after={seq}   增量日志
POST   /api/workspaces/{workspaceId}/pipeline-runs/{id}/cancellations     取消
```

触发接口可省略请求体；可传 `{ "branch": "main", "commit": "<sha>" }` 作为模板上下文。branch 缺省使用登记仓库的默认分支，commit 缺省为空字符串；引擎不自动追加 checkout 命令，实际拉取行为由步骤定义控制。

`logs` 返回 `{items:[{seq,step_index,stream,chunk}], next_seq}`，每页最多 200 片，空页保持传入游标；前端带着 `next_seq` 轮询，运行中 1 秒一次，结束后停止。这是最小实现；换成 SSE 时接口形状不变。

### 17.5 验收标准

必须用**真实的 Git 仓库和真实的主机**跑通，不接受只有单元测试：

1. 登记一个仓库，用凭据拉取一个私有仓库成功
2. 一条两步流水线在 runner 主机上跑通，日志完整、顺序正确
3. 同一条流水线在 SSH 主机上跑通，结果一致
4. 第一步失败时第二步不执行，run 标记为 failed，失败步骤的退出码正确
5. 超时的步骤被标记为超时，不是普通失败
6. 取消一个运行中的 run，服务端停止等待并标记为 cancelled（不保证远端进程立刻停止）
7. 凭据明文不出现在日志、不出现在目标主机的进程列表、不出现在操作记录里
8. 跨工作空间：A 的流水线不能引用 B 的仓库或主机

## 18. 数据层重做：雪花 ID、分布式锁、SeaORM 强类型

两项基础改动,越早做越便宜。项目尚未发版、没有任何部署,现在换掉主键类型和数据访问方式只是改代码;有了真实数据再换就是迁移工程。

### 18.1 为什么现在动

- 主键从 `TEXT`(UUID v4)换成 `BIGINT`(雪花):UUID 是随机的,作为聚簇索引键会让 B+ 树持续页分裂,写入放大且索引体积大;雪花 ID 单调递增,插入总在右侧。16 字节文本 → 8 字节整数,每个索引都变小。
- 手写 SQL 换成 SeaORM 强类型:列名拼错、占位符数量对不上、改表忘改某条语句——这几类错误在本项目都真实发生过,编译期能挡住。

### 18.2 雪花 ID

```
 1 bit  符号位,恒为 0(保证正数)
41 bit  毫秒时间戳,相对 FERRY_EPOCH = 2026-01-01T00:00:00Z,够用约 69 年
10 bit  worker id,0..1023
12 bit  序列号,同一毫秒内 0..4095
```

单实例每毫秒 4096 个,够用。关键细节:

- **时钟回拨**:检测到当前毫秒小于上次发号的毫秒时,**等待**直到追上;回拨超过 5 秒则拒绝发号并报错,而不是继续发——继续发会产生重复 ID,比停机严重。
- **序列号耗尽**:同一毫秒内用满 4096 个就自旋等到下一毫秒,不跨毫秒借用。
- 生成器用一个 `Mutex<State>`,不是无锁结构。每秒几千次调用,锁的开销完全不是瓶颈,而无锁实现的正确性难验证得多。

**JSON 必须把 ID 序列化成字符串。** 这是本节最容易踩的坑:JavaScript 的 `Number` 只能精确表示到 2^53,而雪花 ID 是 64 位,超出部分会被**静默**舍入——前端拿到的 id 和数据库里的不是同一个,而且不报错。所以 `id` 和一切 `*_id` 字段在 JSON 里一律是字符串。前端的 TypeScript 类型本来就写的 `id: string`,不用改。

### 18.3 基于数据库的分布式锁

一张表,不依赖任何外部组件:

```sql
CREATE TABLE locks (
    name       VARCHAR(64) PRIMARY KEY,
    owner      VARCHAR(64) NOT NULL,   -- 持有者的 instance id
    acquired_at BIGINT NOT NULL,
    expires_at  BIGINT NOT NULL)
```

获取锁要在三种数据库上都能用,所以**不用 upsert 语法**(SQLite/PostgreSQL 是 `ON CONFLICT`,MySQL 是 `ON DUPLICATE KEY`),改成两步:

1. `UPDATE locks SET owner=?, acquired_at=?, expires_at=? WHERE name=? AND (owner=? OR expires_at<?)`
   —— 影响行数为 1 表示拿到(要么本来就是自己的,要么原持有者已过期)。
2. 影响行数为 0 时再 `INSERT`;主键冲突说明别人刚插进去,即未获得。

释放是 `DELETE WHERE name=? AND owner=?`,续期是 `UPDATE ... WHERE name=? AND owner=?`。**都带 owner 条件**,否则会删掉或续期别人的锁。

锁必须有 TTL 并且自动过期,否则一个崩溃的实例会把锁永久锁死。

### 18.4 worker id 的动态分配

```sql
CREATE TABLE worker_ids (
    worker_id    BIGINT PRIMARY KEY,   -- 0..1023
    instance_id  VARCHAR(64) NOT NULL,
    heartbeat_at BIGINT NOT NULL)
```

启动时:取 18.3 的锁 → 优先复用本 `instance_id` 上次占用的槽位(重启后 ID 不跳变)→ 否则取最小的空闲槽位(无行,或心跳超过 TTL)→ 写入并释放锁。之后按固定间隔心跳。

**这里有一个必须守住的安全性质:**

> 一旦本实例的心跳距今超过 TTL,就必须**立即停止发号**,因为此时别的实例可能已经回收了这个槽位。继续发号会产生重复 ID。

这条比"能分配到 worker id"重要得多,而且最容易被忽略——正常路径永远测不到它。实现上:发号前检查距上次成功心跳的时间,超限则返回错误,让请求失败而不是发出可能重复的 ID。心跳恢复后自动恢复发号。

TTL 取心跳间隔的若干倍(例如心跳 10 秒、TTL 60 秒),留足网络抖动和 GC 停顿的余量。

### 18.5 SeaORM 强类型

数据访问改用 SeaORM 的实体与查询构建器,手写 SQL 只保留在 SeaORM 确实表达不了的地方(并在注释里说明是哪一点表达不了)。

这个改动会**删掉**现有的不少代码,这是它的主要收益之一:

- `dialect.rs` 的占位符改写(`?` → `$1`)不再需要,SeaORM 自己处理。
- `dialect.rs` 的 MySQL DDL 改写(TEXT 定宽、LONGTEXT、索引 `IF NOT EXISTS`)在迁移改用 SeaORM 的 `Table::create()` 后大部分不再需要——schema builder 自己按后端生成正确的 DDL。
- `db::statement` 里的占位符/参数计数断言随之失去意义,因为那类错误不再可能发生。

保留的是:
- `resource::list` 的**白名单**。查询条件仍然来自请求,列名仍然只能来自 `Spec`——换成 `Condition` 构建,白名单的作用不变。这一层是安全边界,不因为换了 ORM 就取消。
- `db::now()` / 审计写入等薄封装。

**迁移策略**:项目未发版,直接重写 `Initial` 等已有迁移为 SeaORM schema builder 形式,不做兼容性迁移。这是明知故犯地修改已发布迁移,理由是没有任何部署存在;之后恢复"已发布迁移不可改"的规则。

### 18.6 验收标准

1. 三种数据库(SQLite/MySQL/PostgreSQL)全部通过现有的端到端矩阵。
2. 雪花 ID:单调递增;同一毫秒内不重复;多线程并发生成 10 万个无重复。
3. 时钟回拨:模拟回拨后不产生重复 ID(要么等待,要么报错)。
4. 分布式锁:两个并发持有者只有一个成功;过期后可被抢占;非持有者无法释放或续期。
5. worker id:两个实例拿到不同的槽位;同一 `instance_id` 重启后复用原槽位;**心跳过期后发号必须失败**(这条要有测试,它是正确性的核心)。
6. JSON 里的 ID 是字符串,且前端 `npm test` 全过。
7. 手写 SQL 的残留处都有注释说明 SeaORM 为何表达不了。

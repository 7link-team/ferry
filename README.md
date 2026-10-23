# Ferry · 运维平台

一个工作空间制的运维工具：主机、构建、部署、维护四个模块加一个仪表盘，两种部署形态
（Docker 纯 Web / Tauri 桌面内嵌），两种主机接入方式（runner 代理 / SSH 直连），
两者都支持远程终端与双向文件传输。

**方案与设计见 [`docs/ferry-运维平台产品与技术方案.md`](docs/ferry-运维平台产品与技术方案.md)。**

已经能跑：注册登录、工作空间创建与加入、角色权限、主机组 / 主机 / 标签 / 操作记录、
仪表盘计数。尚未实现：runner 与 SSH 通道、终端与文件传输、构建与部署模块。

## 结构

| 位置 | 内容 |
|---|---|
| `crates/server` | 业务服务：HTTP API、内嵌 Web 界面、数据库与迁移 |
| `crates/runner` | 主机代理：一条命令注册主机，持有反向控制连接 |
| `crates/protocol` | server 与 runner 的线上协议，双方共用 |
| `src` | React + TypeScript 界面，构建产物由服务内嵌 |
| `desktop` | Tauri 2 外壳：托盘、连接设置，服务作为 sidecar |
| `docs` | 产品与技术方案 |

界面只有两个页面组件：`src/components/ResourceTable.tsx`（列表）和
`ResourceForm.tsx`（表单，详情是它的只读模式）。一个资源页就是
`src/resources.tsx` 里的一份配置，**不手写表格**；配置契约在
`src/components/dsl.ts`。服务端对应的一份实现是 `crates/server/src/resource.rs`，
过滤、排序、分页和列名白名单都只在那里。

主题是 shadcn/ui + Tailwind v4 的语义令牌，两套配色：默认（shadcn zinc）和
`erp`（沿用 easy-erp 的配色）。加一套配色＝在 `src/style.css` 加两个值块、在
`src/theme.ts` 加一行，组件不动。

## 开发

```sh
cargo run -p ferry-server          # 127.0.0.1:4290
npm install && npm run dev         # 5173，/api 代理到 4290
npm run build                      # 产物进 dist/，被服务内嵌

cargo fmt --all -- --check && cargo check --workspace && cargo test --workspace
npm run typecheck && npm test

# 三种数据库跑同一套功能测试（不带 URL 则只测 SQLite）
cargo build -p ferry-server
FERRY_TEST_MYSQL='mysql://root:pw@127.0.0.1:3306/ferry' \
FERRY_TEST_POSTGRES='postgres://postgres:pw@127.0.0.1:5432/ferry' \
  ./scripts/test-backends.sh
```

## 数据库

SQLite、MySQL、PostgreSQL 都支持,用 `FERRY_DATABASE_URL` 选择:

| 不填 | SQLite,数据在 `FERRY_DATA_DIR/ferry.sqlite`(桌面形态与小型部署的默认) |
|---|---|
| `mysql://…` | MySQL 8+ |
| `postgres://…` | PostgreSQL |

三者的差异只在 `crates/server/src/dialect.rs`:业务代码一律写 SQLite 风味、
`?` 占位符的 SQL。新增查询请保持可移植,不要在方言层加按查询的特例。

Docker 部署：

```sh
docker compose up -d                       # 拉 ghcr.io/7link-team/ferry-server
docker compose up -d --build               # 或本地构建
```

> GHCR 的包默认是私有的（即使仓库公开）。首次发布后需要在
> GitHub → Packages → ferry-server → Package settings 里把可见性改成 Public，
> 匿名 `docker pull` 才能用。

桌面部署：`desktop/` 需要先把 `ferry-server` 可执行文件按目标三元组放进
`desktop/binaries/`，然后 `npm run desktop:dev`。

**只构建桌面端（Windows / macOS / Linux），不构建移动应用。** `tauri.conf.json`
的 `targets: "all"` 指的是当前桌面平台的全部安装包格式；Android/iOS 需要另外执行
`tauri android init` / `tauri ios init`，本项目不执行，也不保留移动端图标与配置。

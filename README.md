# Ferry · 运维平台

一个工作空间制的运维工具：主机、构建、部署、维护四个模块加一个仪表盘，两种部署形态
（Docker 纯 Web / Tauri 桌面内嵌），两种主机接入方式（runner 代理 / SSH 直连），
两者都支持远程终端与双向文件传输。

**方案与设计见 [`docs/ferry-运维平台产品与技术方案.md`](docs/ferry-运维平台产品与技术方案.md)。**

当前为 **alpha 预发布，功能尚未完整**。已接入界面的能力包括：超管初始化与平台管理、注册登录、工作空间创建与加入、成员邀请与角色管理、真实数据仪表盘、主机组 / 主机 / 标签 / 操作记录、runner / SSH 终端与文件传输、构建流水线和 Compose 单主机部署。

**本轮已验收 13/25 个工作空间入口，12 个仍未交付完整功能**：应用模板、流水线模板、应用市场、密钥变量、报警监控、定时任务、配置项和字典（部分功能有多个入口）。成员管理、角色管理和仪表盘已补齐；此项变化尚未进入已发布的 alpha.6。表格导入/导出也未实现。

Compose 当前支持镜像来源、单主机部署及历史回滚，尚未接入密钥变量和配置模板。平台级注册设置不等同于工作空间配置项。详细边界见 [功能完成度核对](docs/tasks/T10-functional-completeness.md)。

## 结构

| 位置 | 内容 |
|---|---|
| `crates/server` | 业务服务：HTTP API、内嵌 Web 界面、数据库与迁移 |
| `crates/runner` | 主机代理：一条命令注册主机，持有反向控制连接 |
| `crates/protocol` | server 与 runner 的线上协议，双方共用 |
| `src` | React + TypeScript 界面，构建产物由服务内嵌 |
| `desktop` | Tauri 2 外壳：托盘、连接设置，服务作为 sidecar |
| `docs` | 产品与技术方案 |

资源界面复用两个共享组件：`src/components/ResourceTable.tsx`（列表）和
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

## 首次初始化与平台管理

没有默认管理员账号或密码。首次打开需由部署者设置平台超级管理员，完成后直接进入平台管理页。
桌面使用内置服务时自动传递初始化凭据，只需填写账号、密码和确认密码；连接远程服务时按服务器部署流程初始化。
服务器部署需读取 `FERRY_DATA_DIR/setup-token`，在初始化页面填写该凭据。该文件仅用于首次初始化，成功后删除；重启不会重新开放初始化。

升级已有安装不会自动提升旧账号权限。可使用初始化凭据及旧账号的原邮箱、原密码认领超管身份；原姓名和密码保留，当前初始化会话自动登录，其他旧会话需要重新登录。
平台超级管理员可在 `/_platform` 管理用户的启用状态和超管资格、创建或改名工作空间、进入全部工作空间，以及开启/关闭公开注册。
必须保留至少一位启用的超管；账号停用或权限变化会使该账号的会话失效。普通工作空间拥有者没有平台管理权限。
初始化后默认允许注册，超管可在「注册设置」关闭；关闭不会阻止已有账号登录。

## 凭据加密

SSH 密码/私钥、Git 令牌等由服务端加密后落库,任何接口都不会把明文返回。

主密钥优先取 `FERRY_MASTER_KEY`(64 位十六进制,`openssl rand -hex 32` 生成);
未设置时在数据目录生成 `master.key`(权限 0600)并一直复用。

两种方式都可用,区别在备份边界:生成的密钥和密文在同一个目录里,
备份该目录等于同时备份了两者;要把密钥和数据分开保管,就用环境变量。
**无论哪种方式,丢了密钥等于丢了所有已存凭据** —— 数据库里只有密文。

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

## Compose 部署与桌面更新

在“部署 → 应用”创建应用，选择单台主机并填写绝对工作目录与 Application manifest。
目标需安装 Docker Compose v2+、`flock`、`sha256sum`，并允许主机接入账号管理 Docker。
先预览目标差异，再执行：预览固定镜像摘要，执行使用同一份产物。
“部署历史”可以回滚到成功 revision，直接复用保存的产物；卷数据不会回滚。
失败不会自动回滚；服务中断后的“需核实”状态需填写核实说明后才能再次部署。
删除应用仅删除平台记录与历史，保留远端容器和卷。首批拒绝密钥变量、配置模板及非镜像来源。

Windows 安装程序支持“仅为我安装”和“所有用户”；后者需管理员权限。
桌面连接设置窗口提供“检查更新”和“下载并安装更新”，安装前要求用户确认，下载后验证签名与版本。
alpha 通道包含预发布版本。`alpha.1` 没有更新入口，需手动安装 `alpha.3`，之后才能使用应用内更新。
服务数据库与数据目录升级时继续复用，发布只追加迁移。

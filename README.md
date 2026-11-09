# Ferry

**自托管的运维与工作流平台。** 在同一个工作空间中管理主机、仓库、构建、节点工作流和应用部署。

[![Build](https://github.com/7link-team/ferry/actions/workflows/build.yml/badge.svg)](https://github.com/7link-team/ferry/actions/workflows/build.yml)
[![Release](https://img.shields.io/github/v/release/7link-team/ferry?include_prereleases)](https://github.com/7link-team/ferry/releases)

支持 Docker Web 服务与 Windows / macOS / Linux 桌面应用，使用 Rust、React、TypeScript 和 Tauri 构建。

[快速开始](#快速开始) · [主要功能](#主要功能) · [部署配置](#部署配置) · [本地开发](#本地开发) · [文档](#文档)

> 当前处于 **Alpha 预发布阶段**。升级前请备份数据库、数据目录和加密主密钥；功能范围与兼容边界以发布说明和文档为准。

## 主要功能

| 模块 | 能力 |
| --- | --- |
| 主机 | Runner Agent / SSH 接入，分组与标签，远程终端、文件传输和资源监控 |
| 构建流水线 | GitHub Actions 格式，可视化表单与高级 YAML 编辑，任务依赖、执行日志和构建产物 |
| 节点工作流 | 可视化画布，应用与集成节点、表达式、AI / 文件处理，独立执行视图与版本记录 |
| 仓库与代码洞察 | 外部代码平台授权、仓库事件与通知，提交统计、作者昵称映射和提交热力图 |
| 应用部署 | Compose、Docker、systemd、Swarm，主机与主机组部署，差异预览、历史版本和回滚 |
| 系统维护 | 定时任务、报警监控，字典与配置项，按类型编辑、模板和 CSV / XLSX 导入导出 |
| 平台与协作 | 工作空间隔离、成员与角色、凭据，平台账号、邮件模板、登录防护与系统监控 |

提供简体中文、繁体中文、英语、日语、韩语和马来语，以及多套主题、明暗模式和跟随系统设置。

## 快速开始

### Docker 一键部署

需要 Docker 与 Compose v2.20+，在 Linux、macOS 或 Windows WSL / Git Bash 中执行：

```sh
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh
```

脚本创建 `ferry/` 部署目录，启动 **Ferry + PostgreSQL + NATS JetStream + Redis**，固定最新发布版本并生成随机密钥。健康检查通过后会输出初始化令牌。

打开 **http://localhost:4290**，使用令牌创建第一个平台超级管理员。**没有默认账号或密码。**

域名、HTTPS、MySQL、备份与升级方法见 [Compose 部署指南](docs/compose-quickstart.md)。

### 轻量单机部署

只想运行一个服务，可以使用仓库中的 Compose 配置，默认采用 SQLite 和本地文件存储：

```sh
git clone https://github.com/7link-team/ferry.git
cd ferry
docker compose up -d
docker compose exec ferry cat /data/setup-token
```

访问 **http://localhost:4290** 完成初始化。数据保存在 `ferry-data` 卷中；单机基础功能不依赖 Redis 或 NATS。

### 桌面应用

从 [Releases](https://github.com/7link-team/ferry/releases) 下载对应平台的安装包。桌面应用可启动内置服务，也可连接远程 Ferry 服务。

### 接入第一台主机

创建工作空间，在主机页面选择 **Runner Agent** 或 **SSH**。Runner 安装命令由界面生成，可选系统服务或当前用户方式；终端与任务以接入账号的身份执行。

## 部署配置

| 配置 | 默认 / 用途 |
| --- | --- |
| `FERRY_DATA_DIR` | 数据目录；容器内为 `/data` |
| `FERRY_DATABASE_URL` | 留空使用 SQLite；也支持 MySQL 8+、PostgreSQL |
| `FERRY_MASTER_KEY` | 64 位十六进制加密主密钥；未指定时在数据目录生成并复用 `master.key` |
| ???? ? ???? | ??????URL?????????????????IP/?????????? |
| `FERRY_TRUSTED_PROXIES` | 可信代理 CIDR，控制登录 IP 等记录是否采信转发头 |
| `FERRY_REDIS_URL` | 可选 Redis 连接，系统监控读取该实例 |
| `FERRY_NATS_URL` | 可选 NATS JetStream 连接，启用多实例节点通信 |

文件存储可在平台设置中选择 **本地文件（单机）** 或 **S3（单机 / 集群）**。多实例部署使用共享 PostgreSQL / MySQL、相同主密钥与集群命名空间、独立数据目录，并配置 NATS JetStream 和共享文件存储；不要跨机器共享 SQLite 文件。反向代理需要支持 WebSocket。

凭据在服务端加密保存。**丢失主密钥后，数据库中的已存凭据将无法解密**；备份与迁移时请一并保管。OAuth 应用、邮箱、登录验证和存储等可在平台界面配置。

## 兼容范围

- **构建与节点工作流**：借鉴 Actions、n8n 等工具的使用方式，不保证兼容全部第三方 Action、n8n 节点或 ComfyUI 插件。扩展方式见 [工作流扩展示例](examples/workflow-extensions/README.md)。
- **表达式沙箱**：支持 Windows 与 Linux；Linux 需要可用的隔离能力，禁止非特权用户命名空间的环境会拒绝执行。macOS 暂不支持沙箱表达式执行。
- **主机执行**：Windows 支持主机模式 Actions 任务，容器镜像任务需要 Linux；所需工具与权限以目标主机实际安装情况为准。
- **部署**：目前提供 Compose、Docker、systemd、Swarm；Kubernetes / Helm 暂未提供。回滚恢复平台保存的配置与产物，不回滚业务卷数据；删除应用记录不会停止远端服务。

## 本地开发

需要 Rust stable、Node.js 24 和 npm。先构建 Web 资源，再启动内嵌服务：

```sh
npm ci
npm run build
cargo build -p ferry-sandbox
cargo run -p ferry-server
```

服务默认监听 `127.0.0.1:4290`。另开终端运行前端开发服务：

```sh
npm run dev
```

打开 **http://localhost:5173**，`/api` 请求会代理至 `4290`。运行检查：

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
npm run typecheck
npm test
```

| 目录 | 内容 |
| --- | --- |
| `src/app` | 页面组合、路由与资源注册 |
| `src/features` | 主机、工作流、仓库、部署等业务模块 |
| `src/shared` | 公共 UI、表单、表格、API、主题与语言 |
| `crates/server` | HTTP 服务、数据库、迁移与内嵌 Web 资源 |
| `crates/runner` | 主机代理与任务执行 |
| `crates/protocol` | 服务与 Runner 的共享协议 |
| `crates/sandbox` | 表达式隔离执行进程 |
| `desktop` | Tauri 桌面应用 |

前端依赖方向为 `app → features → shared`。桌面打包、Runner 生命周期与多数据库验收参见下方文档。

## 文档

- [产品与技术方案](docs/ferry-运维平台产品与技术方案.md)
- [Compose 部署、HTTPS、备份与升级](docs/compose-quickstart.md)
- [仓库连接与授权](docs/repository-connections.md)
- [部署与数据导入导出](docs/deployments-and-transfers.md)
- [工作流能力清单](docs/ferry-工作流能力清单.md) · [工作流总体设计](docs/ferry-工作流平台v3总体设计.md)
- [工作流扩展示例](examples/workflow-extensions/README.md)
- [Runner 安装、运行与升级](docs/tasks/T16-runner-lifecycle.md)
- [桌面应用更新](docs/tasks/T17-desktop-unattended-updates.md)
- [阶段实现与验收记录](docs/tasks/T12-product-completion.md)

## 反馈与参与

欢迎通过 [Issues](https://github.com/7link-team/ferry/issues) 报告问题或提出建议。反馈时请附上版本、操作系统、部署方式、复现步骤和脱敏日志；提交代码前请运行相关检查。

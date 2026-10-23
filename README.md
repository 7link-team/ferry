# Ferry · 运维平台

一个工作空间制的运维工具：主机、构建、部署、维护四个模块加一个仪表盘，两种部署形态
（Docker 纯 Web / Tauri 桌面内嵌），两种主机接入方式（runner 代理 / SSH 直连），
两者都支持远程终端与双向文件传输。

**方案与设计见 [`docs/ferry-运维平台产品与技术方案.md`](docs/ferry-运维平台产品与技术方案.md)。**
当前仓库是通过构建校验的骨架，业务模块尚未实现。

## 结构

| 位置 | 内容 |
|---|---|
| `crates/server` | 业务服务：HTTP API、内嵌 Web 界面、数据库与迁移 |
| `crates/runner` | 主机代理：一条命令注册主机，持有反向控制连接 |
| `crates/protocol` | server 与 runner 的线上协议，双方共用 |
| `src` | React + TypeScript 界面，构建产物由服务内嵌 |
| `desktop` | Tauri 2 外壳：托盘、连接设置，服务作为 sidecar |
| `docs` | 产品与技术方案 |

## 开发

```sh
cargo run -p ferry-server          # 127.0.0.1:4290
npm install && npm run dev         # 5173，/api 代理到 4290
npm run build                      # 产物进 dist/，被服务内嵌
cargo check --workspace && npm run typecheck
```

Docker 部署：`docker compose up -d --build`。

桌面部署：`desktop/` 需要先把 `ferry-server` 可执行文件按目标三元组放进
`desktop/binaries/`，然后 `npm run desktop:dev`。

**只构建桌面端（Windows / macOS / Linux），不构建移动应用。** `tauri.conf.json`
的 `targets: "all"` 指的是当前桌面平台的全部安装包格式；Android/iOS 需要另外执行
`tauri android init` / `tauri ios init`，本项目不执行，也不保留移动端图标与配置。

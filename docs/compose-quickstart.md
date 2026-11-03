# Docker Compose 快速部署

需要 Docker Engine/Desktop 与 Compose v2.20+，以及 `curl`（或 `wget`）。无需克隆仓库，一条命令完成：

```sh
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh
```

也可以写成 `bash <(curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh)`。脚本在当前目录创建部署目录 `ferry/`，下载最新已发布版本的 Compose 文件，镜像固定为同一版本，生成随机密钥，启动 Ferry、PostgreSQL、带认证及磁盘持久化的 NATS JetStream、Redis，等待服务健康后输出首次初始化令牌。打开 http://localhost:4290 完成平台初始化。Windows 请在 WSL 或 Git Bash 中执行。

传参使用 `sh -s --`。改用 MySQL，在**首次启动时**执行：

```sh
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh -s -- --database mysql
```

数据库二选一，不会同时启动两套业务数据库。已生成配置后，在同一位置重复执行会复用原密钥和版本，不再联网；重新传入的访问参数（`--domain`、`--allow-host`、`--bind`、`--port`）会替换原有设置，密钥不变。切换数据库会拒绝执行，需要独立部署目录及显式数据迁移。所有部署共用 Compose 项目名 `ferry`：如果本机已有其他目录的 Ferry 部署（容器或数据卷），脚本会拒绝新建第二套部署，并提示原部署所在的目录。请回到那里执行，否则新密钥会接管原有容器而无法连接原有数据库。`--version v0.1.0-alpha.29` 可指定版本，`--dir 路径` 可指定部署目录。版本固定为一个已发布版本，不会隐式跟随 `latest`。

## 域名与 HTTPS（自动证书）

为一个或多个域名开启 HTTPS：

```sh
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh -s -- --domain ops.example.com
# 多个域名，重复 --domain
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh -s -- --domain ops.example.com --domain ops.example.org
```

- **证书**：脚本在部署目录生成 `compose.https.yaml` 和 `caddy/Caddyfile`，启动 Caddy 作为 HTTPS 入口。Caddy 为每个域名自动申请 Let's Encrypt 证书，到期前自动续期，并把 HTTP 自动跳转到 HTTPS。证书保存在 `caddy-data` 数据卷中，重启不会重新申请。
- **前提**：
  - 每个域名都已解析（A/AAAA 记录）到这台服务器；
  - 云安全组或防火墙放行 80 和 443 端口；
  - 本机没有其他程序占用 80 和 443 端口。

  证书验证不需要手动配置 DNS。
- **访问**：登录 Cookie 只通过 HTTPS 发送。新安装默认发布在 `0.0.0.0:4290`；如仅使用本机反向代理，可显式添加 `--bind 127.0.0.1`。已有部署保留原监听地址。
- **按访问域名生成命令**：从哪个域名打开 Ferry，Runner 安装命令和节点连接命令就显示哪个域名。
- **不在列表内的域名**：Ferry 拒绝访问。
- **已有部署加域名、换域名**：直接用新的完整域名列表重新执行上述命令。不需要先升级，也适用于更早版本的部署。
- **还没有域名时**：可以用 [sslip.io](https://sslip.io) 测试。它把 IP 写进域名即可解析，例如服务器 IP 为 `203.0.113.10` 时，使用 `--domain 203-0-113-10.sslip.io`。sslip.io 是第三方公共服务，正式使用请换成自己的域名。
- **排查证书问题**：查看 Caddy 日志，`docker compose ... -f compose.https.yaml logs caddy`。

## 局域网 HTTP

新安装默认发布在 `0.0.0.0:4290`，访问白名单初始只包含本机名称。允许局域网通过 IP 访问（换成服务器实际 IP）：

```sh
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh -s -- --allow-host 192.168.0.10 --bind 0.0.0.0
```

`--allow-host` 可重复使用，但不能与 `--domain` 同时使用：开启 HTTPS 后登录 Cookie 只通过 HTTPS 发送。从 HTTPS 切换到 `--allow-host` 时，脚本会停止并移除 Caddy。

如果改用自己的反向代理，代理需要满足：
- 保留 Host 头；
- 支持 WebSocket Upgrade；
- 设置足够长的终端会话空闲超时。

## Cloudflare Tunnel / 外部 HTTPS

HTTPS 由 cloudflared、Nginx 等外部代理提供时，用 `--proxy-domain`，不使用 `--domain`：

```sh
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh -s -- --proxy-domain ops.example.com
```

在原来执行安装命令的目录中，同一命令适用于已安装的部署：保留当前版本、数据库和密钥，更新访问白名单、监听地址与 HTTPS Cookie 设置，不启动 Caddy；原来配置了 Caddy 时会移除其入口。`--dir` 必须指向原部署目录。多个访问域名可重复传入 `--proxy-domain`，不能与 `--domain` 或 `--allow-host` 混用。

cloudflared 运行在宿主机时，源站填 `http://127.0.0.1:4290`；运行在独立容器时，填该容器能访问的宿主机地址。容器里的 `127.0.0.1` 指向容器自身。外部 HTTPS 在代理终止，代理到 Ferry 的连接使用 HTTP，并保留原始 Host 和 WebSocket Upgrade。

## 配置与维护

`--init-only` 只生成配置；`--without-redis` 不启动 Redis。随机生成的数据库密码、NATS/Redis 密码及主密钥保存在部署目录的 `.env.production`，重复运行不会重置。新建文件权限为 0600。

PostgreSQL 与 MySQL 的应用账号均不使用数据库管理员身份。数据库、Redis、NATS 客户端端口及管理端口不发布到宿主机；仅应用入口发布。日志滚动限制大小，数据使用命名卷，应用有健康检查、依赖就绪等待、优雅退出时间及重启策略。

Redis **当前未被 Ferry 使用**，是随全套方案提供的可选基础服务。现有连接、终端和文件跨实例路由实际使用 SQL + NATS。没有设置虚假的 Redis 应用配置。

以下维护命令在部署目录中执行，以 PostgreSQL 为例，MySQL 替换第二个文件名：

```sh
docker compose --env-file .env.production -f compose.production.yaml -f compose.postgres.yaml --profile redis ps
docker compose --env-file .env.production -f compose.production.yaml -f compose.postgres.yaml --profile redis logs -f --tail 100 ferry
docker compose --env-file .env.production -f compose.production.yaml -f compose.postgres.yaml --profile redis down
```

`down` 保留数据卷；**不要给已有部署随意加 `-v`**。不要修改已有 `FERRY_MASTER_KEY`，否则已有凭证无法解密。

## 一键升级

在部署目录的**上一级目录**执行（与安装时相同的位置，或用 `--dir` 指定部署目录）：

```sh
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh -s -- --upgrade
# 升级到指定版本
curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh -s -- --upgrade --version v0.1.0-alpha.29
```

升级步骤：

1. 读取当前版本。目标版本与当前版本相同时，不做任何操作。
2. 拒绝降级（包括降到当前版本的预发布版本），因为数据库迁移只能向前执行。
3. 数据库服务需处于运行状态。用 `pg_dump` 或 `mysqldump` 导出数据库；导出失败时立即停止，不做任何改动。
4. 将数据库导出文件、`.env.production` 和全部 Compose 文件备份到部署目录下的 `backups/<时间>-<旧版本>-to-<新版本>/`，该目录仅当前用户可读。
5. 下载新版本标签下的全部 Compose 文件。全部下载完成后才替换旧文件。
6. `.env.production` 中只修改 `FERRY_IMAGE` 的版本号；镜像仓库地址、密钥、地址和端口保持不变。
7. 校验配置并拉取镜像。任一步骤失败时还原原有文件，正在运行的服务不受影响。
8. 重启服务并等待恢复健康。原部署启用了 Redis 时继续启用。

如果新版本启动后不健康，脚本会输出该版本的日志查看命令，以及具体的回滚命令：用备份覆盖文件、导入数据库导出文件、重新启动。

`--upgrade` 只接受 `--dir` 和 `--version`。

备份不包括应用数据卷和 JetStream 卷，需按日常备份策略另行备份。

`FERRY_IMAGE` 使用 digest 固定或非版本标签时，脚本拒绝升级。这种情况需要手动升级：

1. 将 `FERRY_IMAGE` 改为目标镜像。
2. 若新版本发布说明提到 Compose 文件变化，用该版本标签下的文件替换部署目录中的同名文件。
3. 执行 `pull` 和 `up -d --wait`。

## 生产边界

这是完整的**单机持久化部署**，不是跨宿主机高可用方案。单台机器故障会同时影响数据库、单副本 NATS 和应用；Redis AOF 也不替代备份。上线仍需配置实际域名/TLS、数据库定期备份和恢复演练、宿主机磁盘/容量监控。

跨宿主机高可用使用外部数据库高可用集群、至少三节点 NATS JetStream（`FERRY_NATS_REPLICAS=3`）、应用负载均衡。各应用副本共享数据库、主密钥及 NATS 命名空间，但必须有**独立数据卷和实例 ID**；不要直接对本文件的 Ferry 服务执行 `--scale`，因为其发布端口和数据卷是单实例定义。详见 README「多实例节点通信」。

本地环境未安装 Docker 时，只能执行配置/启动脚本测试，不能将其宣称为实际容器启动验收。

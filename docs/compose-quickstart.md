# Docker Compose 快速部署

需要 Docker Engine/Desktop、Compose v2.20+、Python 3.9+。在仓库根目录执行：

```sh
python scripts/quickstart.py
```

默认启动 Ferry、PostgreSQL、带认证及磁盘持久化的 NATS JetStream、Redis；等待服务健康后输出首次初始化令牌。打开 http://localhost:4290 完成平台初始化。Windows 也执行相同命令，Python 启动器也可使用 `py -3`。

改用 MySQL，在**首次启动时**执行：

```sh
python scripts/quickstart.py --database mysql
```

数据库二选一，不会同时启动两套业务数据库。已生成配置后，重复启动复用原密钥和地址；切换数据库会拒绝执行，需要独立部署目录及显式数据迁移。默认镜像固定为已发布的 `v0.1.0-alpha.24`，不会隐式跟随 `latest`；该镜像不包含尚未发布的本地修改。

## 公网、局域网与 HTTPS

默认仅绑定本机回环地址。局域网示例（换成服务器实际 IP）：

```sh
python scripts/quickstart.py --public-url http://192.168.0.10:4290 --bind 0.0.0.0
```

公网示例（由现有 HTTPS 反向代理转发到本机 4290）：

```sh
python scripts/quickstart.py --public-url https://ops.example.com
```

HTTPS 地址会开启安全 Cookie；代理必须保留 Host、支持 WebSocket Upgrade，并设置足够的终端会话空闲超时。主机上的 Runner 必须能访问所填公共地址。已有部署要改地址、端口、镜像版本，请编辑 `.env.production` 后重新执行启动命令；重新传入参数不会覆盖该文件。

## 配置与维护

`--init-only` 只生成配置；`--without-redis` 不启动 Redis。随机生成的数据库密码、NATS/Redis 密码及主密钥保存在忽略提交的 `.env.production`，重复运行不会重置。Unix 新建文件权限为 0600；Windows 请将部署目录 ACL 限制为运维账号和管理员。

PostgreSQL 与 MySQL 的应用账号均不使用数据库管理员身份。数据库、Redis、NATS 客户端端口及管理端口不发布到宿主机；仅应用入口发布。日志滚动限制大小，数据使用命名卷，应用有健康检查、依赖就绪等待、优雅退出时间及重启策略。

Redis **当前未被 Ferry 使用**，是随全套方案提供的可选基础服务。现有连接、终端和文件跨实例路由实际使用 SQL + NATS。没有设置虚假的 Redis 应用配置。

以下维护命令以 PostgreSQL 为例，MySQL 替换第二个文件名：

```sh
docker compose --env-file .env.production -f compose.production.yaml -f compose.postgres.yaml --profile redis ps
docker compose --env-file .env.production -f compose.production.yaml -f compose.postgres.yaml --profile redis logs -f --tail 100 ferry
docker compose --env-file .env.production -f compose.production.yaml -f compose.postgres.yaml --profile redis down
```

`down` 保留数据卷；**不要给已有部署随意加 `-v`**。升级时先备份 SQL 数据库、`.env.production`、应用卷和 JetStream 卷，再将 `FERRY_IMAGE` 改为已验证版本，执行 `pull`、`up -d --wait`。不要修改已有 `FERRY_MASTER_KEY`，否则已有凭证无法解密。镜像变量支持替换为内部镜像及固定 digest。

## 生产边界

这是完整的**单机持久化部署**，不是跨宿主机高可用方案。单台机器故障会同时影响数据库、单副本 NATS 和应用；Redis AOF 也不替代备份。上线仍需配置实际域名/TLS、数据库定期备份和恢复演练、宿主机磁盘/容量监控。

跨宿主机高可用使用外部数据库高可用集群、至少三节点 NATS JetStream（`FERRY_NATS_REPLICAS=3`）、应用负载均衡。各应用副本共享数据库、主密钥及 NATS 命名空间，但必须有**独立数据卷和实例 ID**；不要直接对本文件的 Ferry 服务执行 `--scale`，因为其发布端口和数据卷是单实例定义。详见 README「多实例节点通信」。

本地环境未安装 Docker 时，只能执行配置/启动脚本测试，不能将其宣称为实际容器启动验收。

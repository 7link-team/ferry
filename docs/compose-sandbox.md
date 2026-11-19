# Docker Compose 表达式沙箱

工作流保存时，服务端调用独立的 `ferry-sandbox` 校验表达式；执行时也用它解析表达式。主机 Runner 不承担这一步服务端校验。

容器中的服务以 UID/GID `10001:10001` 运行。Linux 沙箱的子进程先创建用户命名空间，再清除权限、环境与继承的文件描述符，设置资源限制和自己的 seccomp 过滤器，最后才运行表达式。Docker 默认 seccomp 拒绝这个非 root 进程的 `unshare(CLONE_NEWUSER)`，导致服务返回 `workflow.sandboxUnavailable` / HTTP 503。

## 配置

两套 Compose 文件的应用服务都使用同一份策略：

```yaml
services:
  ferry:
    security_opt:
      - no-new-privileges:true
      - seccomp=./sandbox-seccomp.json
```

`sandbox-seccomp.json` 放在 Compose 文件旁边。它保留 Moby 默认策略，只额外允许参数恰为 `CLONE_NEWUSER` 的 `unshare`；不授予 `CAP_SYS_ADMIN`，不关闭容器 seccomp 或 AppArmor，不改用 root 运行服务。表达式子进程自己的过滤器继续拒绝文件打开、网络、进程创建、ptrace 等操作。

一键部署按目标版本的 Compose 内容决定是否下载该策略及 `sandbox-seccomp.LICENSE`；旧版本未引用策略时不下载不存在的文件。升级先备份旧策略，完整暂存新文件后才替换；拉取失败时还原原文件。

## 已有部署

更新镜像本身不会改变已有容器的安全配置。部署修复时，把匹配版本的策略、许可证和 Compose 文件一起更新，再重建应用容器；数据库、卷、账号和 Runner 不需要重建。自行维护 Compose 时，将上面的 `security_opt` 合入实际的应用服务，然后执行：

```sh
docker compose up -d --no-deps --force-recreate ferry
printf '%s' '{"version":1,"timeout_ms":1000,"memory_bytes":16777216,"deterministic":true,"evaluations":[{"expression":"1+1","context":{}}]}' | docker compose exec -T ferry /usr/local/bin/ferry-sandbox
```

预期返回 `{"status":"ok","version":1,"values":[2]}`。请使用实际部署的 Compose 参数和服务名。不要把沙箱直接作为 PID 1 启动来代替该检测；它与服务端启动子进程的路径不同。

若宿主机完全禁用用户命名空间，或额外的 AppArmor/SELinux 策略仍拒绝此操作，沙箱继续拒绝运行。先核对宿主机和容器的策略，不使用 `privileged`、`seccomp=unconfined` 或同权限执行表达式的降级方案。

## 来源与验证

基线来自 Moby Profiles 的 `seccomp/default.json`，提交 `9ee3204f0d0cfa19ca2c6a7b0fcf5a2b353954ae`，原文件 SHA256 为 `3320527d4568265ac97a077609b1bfd4cfd0d789eeb9239b7e094b1526675406`，许可证随策略分发。

`.github/scripts/test-sandbox-container.py --check-profile` 验证唯一策略差异；传入镜像时还验证实际 Compose 部署和原有完整沙箱隔离测试。镜像冒烟测试通过真实认证接口创建含表达式的工作流、执行并检查输出为 `2`。发布镜像的两种 Linux 架构均执行这些检查。

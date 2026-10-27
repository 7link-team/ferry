# 外部仓库授权与应用市场

平台连接已有 GitHub、GitLab、Gitee、Codeup 仓库，不托管 Git。先配置仓库授权或 API 凭据，再创建外部仓库；应用市场来源关联该仓库并读取指定分支的索引文件。

## 网页授权

GitHub、GitLab、Gitee 支持网页授权。管理员先在提供方创建 OAuth 应用，将回调地址登记为：

```text
https://<平台域名>/api/repository-auth/callback
```

服务端配置以下环境变量并重启。`PUBLIC_URL` 必须是用户实际访问平台的地址，授权前后使用同一登录会话。

```text
REPOSITORY_OAUTH_PUBLIC_URL=https://<平台域名>
REPOSITORY_OAUTH_GITHUB_CLIENT_ID=<应用 ID>
REPOSITORY_OAUTH_GITHUB_CLIENT_SECRET=<应用密钥>
```

GitLab、Gitee 分别将变量名中的 `GITHUB` 替换为 `GITLAB`、`GITEE`。密钥仅配置在服务端，不放入前端、仓库索引或版本库。

| 提供方 | 默认网页地址 | 默认 API 地址 | 请求权限 |
| --- | --- | --- | --- |
| GitHub | `https://github.com` | `https://api.github.com` | `repo read:user` |
| GitLab | `https://gitlab.com` | `https://gitlab.com/api/v4` | `read_user read_api read_repository` |
| Gitee | `https://gitee.com` | `https://gitee.com/api/v5` | `user_info projects` |

自托管实例可配置对应前缀的 `_BASE_URL` 与 `_API_URL`。GitHub Enterprise 默认 API 路径为 `/api/v3`，GitLab 为 `/api/v4`。更换应用 ID、网页地址或 API 地址后，已有凭据需要重新授权，避免旧令牌被发送到新的接入点。

用户进入工作空间的“仓库 → 平台授权”，点击网页授权并在提供方确认。GitHub、GitLab、Gitee 的按钮在未配置 OAuth 应用时仍显示，但不可点击，旁边说明缺少的配置；超管可展开“配置网页授权”查看部署步骤。成功后生成加密保管的令牌凭据；创建外部仓库时选择该凭据。授权将要过期且提供方发放了刷新令牌时，在实际使用前自动续期。续期失败或服务在续期中断开时，通过“重新授权”恢复；原凭据 ID 保留，已有仓库引用无需重建。网页管理的令牌不可手工覆盖。

## API 凭据

Codeup 当前使用官方个人访问令牌。未配置 OAuth 应用的平台也可以使用 API 凭据入口；该入口位于“仓库 → 平台授权 → 使用个人访问令牌”，默认创建“令牌”类型，不是独立的 API 令牌模块。这是代码平台签发的仓库访问凭据，不是调用 Ferry API 的令牌。令牌必须具有目标仓库的读取权限，私有仓库还需满足提供方的组织访问要求。

创建外部仓库时填写仓库 URL、分支并选择凭据。市场来源的 API 主机必须与仓库主机一致，官方例外为 GitHub 的 `api.github.com` 和 Codeup 的 `openapi-rdc.aliyuncs.com`。不允许通过重定向转发认证信息。

## 市场索引

在外部仓库提交 JSON 文件，例如 `marketplace.json`：

```json
{
  "version": 1,
  "templates": [
    {
      "key": "web",
      "name": "Web 示例",
      "description": "单服务示例",
      "content": {
        "apiVersion": "ferry/v1",
        "kind": "Application",
        "metadata": { "name": "web" },
        "services": [{ "name": "web", "image": "nginx:stable" }]
      },
      "parameters": []
    }
  ]
}
```

在“市场来源”选择仓库，设置提供方、API 接入点、项目路径、分支及索引路径。GitHub/Gitee 使用 `owner/repository`；GitLab 可使用项目路径或项目 ID；Codeup 填写仓库 ID 和组织 ID，API 接入点为 `https://openapi-rdc.aliyuncs.com`。

点击“立即同步”后才更新目录。索引最多 500 项，解码后最多 2 MiB；全部模板校验成功后原子替换目录，失败保留上次成功内容并显示错误。定时同步在共享定时任务中选择部署范围、市场来源及同步动作。

在“应用市场”查看模板并导入，得到独立的应用模板。后续来源同步不会覆盖已导入模板，导入也不会自动部署。

## 验证范围

当前已用隔离 HTTP 提供方及 Chromium 验证授权、令牌续期、市场同步和导入流程。真实第三方账号、组织权限策略以及安装版 WebView 的授权兼容性尚未验收；接入真实部署后需使用实际测试仓库完成授权、私有文件读取和重新授权验证。

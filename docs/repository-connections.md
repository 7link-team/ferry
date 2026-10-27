# 外部仓库连接与应用市场

只连接已有 GitHub、GitLab、Gitee、Gitea、Codeup 或普通 Git 仓库，不托管 Git。所有连接步骤都在工作空间的“仓库 → 创建”完成，没有独立的“平台授权”或“仓库访问令牌”Tab。

## 创建仓库

1. 选择代码来源：GitHub、GitLab、Gitee、Gitea、Codeup 或普通 Git。
2. 代码平台选择网页授权或个人访问令牌。网页授权在弹窗完成，返回后保留原表单；个人令牌必须通过平台接口验证并成功读取仓库列表，才会保存为连接。
3. 从平台返回的列表选择仓库，确认名称、默认分支和备注后创建。访问凭据自动绑定，无需跳到凭据页面。

令牌错误或没有读取仓库的权限时，界面显示错误，不会仅创建一个凭据就显示“已连接”。仓库列表按页读取；可使用已有连接或在表单中重新授权。重新授权保留原凭据引用。授权记录按工作空间和操作者隔离，仓库绑定的凭据可供该工作空间中有相应权限的任务使用。

普通 Git 直接填写 HTTP(S) 或 SSH 地址，选择无需授权、账号密码、SSH 私钥或令牌。授权信息与仓库在同一事务中保存并加密，失败不会留下孤立凭据。已存在的仓库可保持原授权，也可在编辑时替换。

## 平台动态配置

超管在“平台管理 → 平台设置”配置公开访问地址及 GitHub、GitLab、Gitee、Gitea 的 OAuth 应用。**保存后立即生效，无需重启**。各工作空间仅授权自己的账号，不重复配置应用密钥。

先在代码平台创建 OAuth 应用，将平台设置页面显示的回调地址登记到提供方：

```text
https://<平台域名>/api/repository-auth/callback
```

公开访问地址必须是用户浏览器实际访问平台的地址，授权前后使用同一登录会话。应用配置加密存储，密钥从不回显；不勾选替换或留空均保留原密钥。关闭某提供方的网页授权后，该提供方的网页授权凭据不再用于新执行；配置仍保留。个人访问令牌连接独立于 OAuth 应用开关。

| 提供方 | 默认网站地址 | 默认 API 地址 | 网页授权请求权限 |
| --- | --- | --- | --- |
| GitHub | `https://github.com` | `https://api.github.com` | `repo read:user` |
| GitLab | `https://gitlab.com` | `https://gitlab.com/api/v4` | `read_user read_api read_repository` |
| Gitee | `https://gitee.com` | `https://gitee.com/api/v5` | `user_info projects` |
| Gitea | 必须填写实例地址 | 网站地址加 `/api/v1` | `read:user read:repository` |

自建 GitLab/GitHub Enterprise/Gitea 可填写网站和 API 地址；GitHub Enterprise 默认 API 路径为 `/api/v3`。更换应用 ID、网站或 API 接入点后，原网页授权凭据需要重新授权，不能将旧令牌发送到新接入点。更换公开地址会拒绝尚未完成的旧授权流程，请重新发起。

兼容原有环境变量：尚未在网页保存设置时，读取 `REPOSITORY_OAUTH_PUBLIC_URL` 与 `REPOSITORY_OAUTH_{GITHUB|GITLAB|GITEE|GITEA}_{CLIENT_ID|CLIENT_SECRET|BASE_URL|API_URL}`。网页保存后以数据库平台设置为准，关闭网页授权不会重新启用环境变量里的配置。

## 个人访问令牌

令牌由代码平台签发，需具备目标仓库读取权限；不是调用本平台 API 的令牌。GitHub/GitLab/Gitee/Gitea 会验证账号并读取仓库列表，未配置 OAuth 应用也可以使用令牌连接。自建平台在创建表单中填写自定义接入点。

Codeup 使用个人访问令牌。中心版填写组织 ID；默认网站为 `https://codeup.aliyun.com`，API 为 `https://openapi-rdc.aliyuncs.com`。Region 版填写对应的网站和 API 地址，组织 ID 可留空。由于 Codeup 令牌接口不返回 Git 登录用户名，还需填写用于实际拉取的 Git 用户名。连接以成功读取组织内可访问仓库为准。

市场来源的 API 主机必须与仓库主机一致，官方例外为 GitHub 的 `api.github.com` 和 Codeup 的 `openapi-rdc.aliyuncs.com`。不会通过重定向转发认证信息。

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

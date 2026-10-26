/**
 * The whole menu tree as data. Moving an item between groups — the planned
 * merge of 主机 / 构建 / 部署 into one top-level module — is an edit here and
 * nowhere else. Routes are derived: /{workspace}/{group}/{item}.
 */
export interface NavItem {
  key: string;
  label: string;
  /** Detail/history routes available through resource actions, not extra menu entries. */
  hidden?: boolean;
  /** Resource page this item opens. Several items may share one resource. */
  resource: string;
  /** Preset query applied on entry, e.g. the scope of a shared resource. */
  params?: Record<string, string>;
  /** Permission required to see the item at all. */
  permission?: string;
}
export interface NavGroup {
  key: string;
  label: string;
  items: NavItem[];
}

export const nav: NavGroup[] = [
  {
    key: "dashboard",
    label: "仪表盘",
    items: [
      { key: "overview", label: "仪表盘", resource: "dashboard", permission: "dashboard.read" },
    ],
  },
  {
    key: "deploy",
    label: "部署",
    items: [
      { key: "apps", label: "应用", resource: "deployments", permission: "deployments.read" },
      { key: "revisions", label: "部署历史", resource: "deployment-revisions", permission: "deployments.read" },
      { key: "config-templates", label: "配置文件模板", resource: "config-templates", permission: "config-templates.read" },
      { key: "app-templates", label: "应用模版", resource: "app-templates", permission: "app-templates.read" },
      { key: "marketplace", label: "应用市场", resource: "marketplace", permission: "marketplace.read" },
      { key: "marketplace-sources", label: "市场来源", resource: "marketplace-sources", permission: "marketplace-sources.read" },
      { key: "variables", label: "密钥变量", resource: "variables", params: { scope: "deploy" }, permission: "variables.read" },
      { key: "alerts", label: "报警监控", resource: "alert-rules", permission: "alerts.read" },
      { key: "alert-events", label: "告警事件", resource: "alert-events", permission: "alerts.read" },
      { key: "cron-jobs", label: "定时任务", resource: "cron-jobs", params: { scope: "deploy" }, permission: "cron-jobs.read" },
      { key: "cron-runs", label: "定时任务执行历史", resource: "cron-runs", params: {scope:"deploy"}, permission:"cron-jobs.read", hidden:true },
    ],
  },
  {
    key: "build",
    label: "构建",
    items: [
      { key: "repositories", label: "仓库", resource: "repositories", permission: "repositories.read" },
      { key: "repository-auth", label: "仓库授权", resource: "repository-auth", permission: "credentials.read" },
      { key: "api-tokens", label: "API 令牌", resource: "credentials", params:{kind:"token"}, permission:"credentials.read", hidden:true },
      { key: "pipelines", label: "流水线", resource: "pipelines", permission: "pipelines.read" },
      { key: "artifacts", label: "构建产物", resource: "pipeline-artifacts", permission: "pipelines.read" },
      { key: "runs", label: "执行历史", resource: "pipeline-runs", permission: "pipelines.read" },
      { key: "pipeline-templates", label: "流水线模版", resource: "pipeline-templates", permission: "pipeline-templates.read" },
      { key: "credentials", label: "授权认证", resource: "credentials", permission: "credentials.read" },
      { key: "variables", label: "密钥变量", resource: "variables", params: { scope: "build" }, permission: "variables.read" },
      { key: "cron-jobs", label: "定时任务", resource: "cron-jobs", params: { scope: "build" }, permission: "cron-jobs.read" },
      { key: "cron-runs", label: "定时任务执行历史", resource: "cron-runs", params: {scope:"build"}, permission:"cron-jobs.read", hidden:true },
    ],
  },
  {
    key: "host",
    label: "主机",
    items: [
      { key: "host-groups", label: "主机组", resource: "host-groups", permission: "host-groups.read" },
      { key: "hosts", label: "主机", resource: "hosts", permission: "hosts.read" },
      { key: "tags", label: "标签", resource: "tags", permission: "tags.read" },
      { key: "cron-jobs", label: "定时任务", resource: "cron-jobs", params: { scope: "host" }, permission: "cron-jobs.read" },
      { key: "cron-runs", label: "定时任务执行历史", resource: "cron-runs", params: {scope:"host"}, permission:"cron-jobs.read", hidden:true },
    ],
  },
  {
    key: "ops",
    label: "维护",
    items: [
      { key: "members", label: "成员管理", resource: "members", permission: "members.read" },
      { key: "roles", label: "角色管理", resource: "roles", permission: "roles.read" },
      { key: "variables", label: "密钥变量", resource: "variables", params: { scope: "global" }, permission: "variables.read" },
      { key: "settings", label: "配置项", resource: "settings", permission: "settings.read" },
      { key: "dictionaries", label: "字典", resource: "dictionaries", permission: "dictionaries.read" },
      { key: "activity", label: "操作记录", resource: "activity", permission: "activity.read" },
    ],
  },
];

export const findGroup = (key: string) => nav.find((g) => g.key === key);
export const findItem = (group: string, key: string) =>
  findGroup(group)?.items.find((i) => i.key === key);

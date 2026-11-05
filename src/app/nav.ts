import { commonText, workflowText } from "../shared/i18n/i18n";
/**
 * The whole menu tree as data. Moving an item between groups — the planned
 * merge of 主机 / 构建 / 部署 into one top-level module — is an edit here and
 * nowhere else. Routes are derived: /workspaces/{workspaceId}/{resource}.
 */
export interface NavItem {
  key: string;
  label: string;
  /** Detail/history routes available through resource actions, not extra menu entries. */
  hidden?: boolean;
  /** Related page shown inside this parent menu entry. */
  parent?: string;
  description?: string;
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
    get label() { return commonText("nav.dashboard"); },
    items: [
      { key: "overview", get label() { return commonText("nav.dashboard"); }, resource: "dashboard", permission: "dashboard.read" },
    ],
  },
  {
    key: "deploy",
    get label() { return commonText("nav.applications"); },
    items: [
      { key: "apps", get label() { return commonText("nav.deployments"); }, resource: "deployments", permission: "deployments.read" },
      { key: "revisions", get label() { return commonText("nav.deploymentHistory"); }, resource: "deployment-revisions", hidden: true, parent: "apps", permission: "deployments.read" },
      { key: "config-templates", get label() { return commonText("nav.configTemplates"); }, resource: "config-templates", hidden: true, parent: "apps", get description() { return commonText("nav.configTemplatesHelp"); }, permission: "config-templates.read" },
      { key: "app-templates", get label() { return commonText("nav.appTemplates"); }, resource: "app-templates", permission: "app-templates.read" },
      { key: "marketplace", get label() { return commonText("nav.marketplace"); }, resource: "marketplace", permission: "marketplace.read" },
      { key: "marketplace-sources", get label() { return commonText("nav.marketplaceSources"); }, resource: "marketplace-sources", permission: "marketplace-sources.read" },
      { key: "credentials", get label() { return commonText("nav.credentials"); }, resource: "credentials", params: { scope: "deploy" }, permission: "credentials.read" },
      { key: "variables", get label() { return commonText("nav.variables"); }, resource: "variables", params: { scope: "deploy" }, permission: "variables.read" },
      { key: "alerts", get label() { return commonText("nav.alertRules"); }, resource: "alert-rules", permission: "alerts.read" },
      { key: "alert-events", get label() { return commonText("nav.alertEvents"); }, resource: "alert-events", permission: "alerts.read" },
      { key: "cron-jobs", get label() { return commonText("nav.cronJobs"); }, resource: "cron-jobs", params: { scope: "deploy" }, permission: "cron-jobs.read" },
      { key: "cron-runs", get label() { return commonText("nav.cronRuns"); }, resource: "cron-runs", params: {scope:"deploy"}, permission:"cron-jobs.read", hidden:true },
    ],
  },
  {
    key: "workflow",
    get label() { return workflowText("workflows"); },
    items: [
      { key: "workflows", get label() { return workflowText("kind.nodes"); }, resource: "workflows", permission: "workflows.read" },
      { key: "pipelines", get label() { return workflowText("kind.actions"); }, resource: "pipelines", permission: "pipelines.read" },
      { key: "credentials", get label() { return commonText("nav.credentials"); }, resource: "credentials", permission: "credentials.read" },
      { key: "ai-credentials", get label() { return workflowText("aiCredentials"); }, resource: "credentials", params: { scope: "ai", kind: "token" }, hidden: true, parent: "credentials", permission: "credentials.read" },
      { key: "integration-credentials", get label() { return workflowText("integration.credentials"); }, resource: "credentials", params: { scope: "integration", kind: "token" }, hidden: true, parent: "credentials", permission: "credentials.read" },
      { key: "workflow-groups", get label() { return commonText("table.groups"); }, resource: "workflow-groups", permission: "workflow-groups.read", hidden: true, parent: "workflows" },
      {key:"pipeline-groups",get label() { return commonText("table.groups"); },resource:"pipeline-groups",permission:"pipeline-groups.read",hidden:true,parent:"pipelines"},
      {key:"pipeline-environments",get label() { return commonText("nav.environments"); },resource:"pipeline-environments",permission:"pipeline-environments.read",hidden:true,parent:"pipelines"},
      {key:"environment-variables",get label() { return commonText("nav.environmentVariables"); },resource:"variables",permission:"variables.read",params:{scope:"environment"},hidden:true,parent:"pipelines"},
      { key: "artifacts", get label() { return commonText("nav.artifacts"); }, resource: "pipeline-artifacts", permission: "pipelines.read" },
      { key: "runs", get label() { return commonText("nav.runHistory"); }, resource: "pipeline-runs", permission: "pipelines.read" },
      { key: "pipeline-templates", get label() { return workflowText("templates"); }, resource: "pipeline-templates", params:{workflow_kind:"actions"}, hidden:true,parent:"pipelines",permission: "pipeline-templates.read" },
      { key: "workflow-templates", get label() { return workflowText("templates"); }, resource: "pipeline-templates", params:{workflow_kind:"nodes"}, hidden:true,parent:"workflows",permission: "pipeline-templates.read" },
      { key: "variables", get label() { return commonText("nav.variables"); }, resource: "variables", params: { scope: "build" }, permission: "variables.read" },
      { key: "cron-jobs", get label() { return commonText("nav.cronJobs"); }, resource: "cron-jobs", params: { scope: "build" }, permission: "cron-jobs.read" },
      { key: "cron-runs", get label() { return commonText("nav.cronRuns"); }, resource: "cron-runs", params: {scope:"build"}, permission:"cron-jobs.read", hidden:true },
    ],
  },
  {
    key: "repository",
    get label() { return commonText("nav.repository"); },
    items: [
      { key: "repositories", get label() { return commonText("nav.repositories"); }, resource: "code-connections", permission: "repositories.read" },
      { key: "repository-events", get label() { return commonText("nav.repositoryEvents"); }, resource: "repository-events", permission: "repositories.read" },
      { key: "code-insights", get label() { return commonText("nav.insights"); }, resource: "code-insights", permission: "code-insights.read" },
      { key: "credentials", get label() { return commonText("nav.credentials"); }, resource: "credentials", params: { scope: "build" }, permission: "credentials.read" },
    ],
  },
  {
    key: "host",
    get label() { return commonText("nav.hosts"); },
    items: [
      { key: "hosts", get label() { return commonText("nav.nodes"); }, resource: "hosts", permission: "hosts.read" },
      { key: "host-groups", get label() { return commonText("nav.hostGroups"); }, resource: "host-groups", permission: "host-groups.read", hidden: true, parent: "hosts" },
      { key: "credentials", get label() { return commonText("nav.credentials"); }, resource: "credentials", params: { scope: "host" }, permission: "credentials.read" },
      { key: "tags", get label() { return commonText("nav.tags"); }, resource: "tags", permission: "tags.read" },
      { key: "cron-jobs", get label() { return commonText("nav.cronJobs"); }, resource: "cron-jobs", params: { scope: "host" }, permission: "cron-jobs.read" },
      { key: "cron-runs", get label() { return commonText("nav.cronRuns"); }, resource: "cron-runs", params: {scope:"host"}, permission:"cron-jobs.read", hidden:true },
    ],
  },
  {
    key: "ops",
    get label() { return commonText("nav.maintenance"); },
    items: [
      { key: "members", get label() { return commonText("nav.members"); }, resource: "members", permission: "members.read" },
      { key: "roles", get label() { return commonText("nav.roles"); }, resource: "roles", permission: "roles.read" },
      { key: "variables", get label() { return commonText("nav.variables"); }, resource: "variables", params: { scope: "global" }, permission: "variables.read" },
      { key: "settings", get label() { return commonText("nav.settings"); }, resource: "settings", permission: "settings.read" },
      { key: "dictionaries", get label() { return commonText("nav.dictionaries"); }, resource: "dictionaries", permission: "dictionaries.read" },
      { key: "activity", get label() { return commonText("nav.activity"); }, resource: "activity", permission: "activity.read" },
    ],
  },
];

export const findGroup = (key: string) => nav.find((g) => g.key === key);
export const findItem = (group: string, key: string) =>
  findGroup(group)?.items.find((i) => i.key === key);

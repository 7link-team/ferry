import { deleteResource, runItemAction } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import type { ResourceConfigContext } from "../../shared/resources/config";
import {
  type AnyRow,
  cronDate,
  form,
  list,
  namedOptions,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import type { Navigation } from "../../shared/routing/types";
import { marketProviders, trustLabel } from "./labels";

export interface MarketSource {
  id: string;
  name: string;
  repository_id: string;
  provider: string;
  api_base: string;
  project: string;
  organization: string;
  branch: string;
  index_path: string;
  state: string;
  last_error: string;
  synced_at: number;
  version: number;
}

export interface MarketEntry {
  id: string;
  source_id: string;
  source_name: string;
  trust: string;
  entry_key: string;
  name: string;
  description: string;
  content: string;
  parameters: string;
  version: number;
  package_version: string;
  kind: string;
  compatible: boolean;
  incompatible_reason: string | null;
  /** Spec §15.2: "verified" only for official sources; other sources' records are declarations. */
  verification: "verified" | "declared" | "none";
  verified_drivers: string[];
  readme: string;
  files: string[];
}

export { marketProviders } from "./labels";

export const marketInstall = (
  workspace: string,
  navigate: Navigation["navigate"],
) => ({
  label: "导入应用模板",
  labelKey: "resource-domain:template.importApp",
  permission: "marketplace.install",
  disabled: (r: MarketEntry) => !r.compatible,
  disabledReason: (r: MarketEntry) => r.incompatible_reason ?? "",
  action: async (r: MarketEntry) => {
    const created = await runItemAction<{ id: string }>(
      workspace,
      "marketplace",
      r.id,
      "install",
      { version: r.version },
    );
    navigate(
      { group: "deploy", item: "app-templates", id: created.id, mode: "view" },
      {},
    );
  },
});

export function marketplaceLists({
  workspace,
  permissions,
  navigate,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    "marketplace-sources": list<MarketSource>({
      resource: "marketplace-sources",
      rowKey: (r) => r.id,
      filters: [
        {
          key: "q",
          label: "名称",
          labelKey: "common-domain:common.name",
          type: "text",
        },
      ],
      columns: [
        {
          key: "name",
          label: "名称",
          labelKey: "common-domain:common.name",
          sortable: true,
        },
        {
          key: "provider",
          label: "平台",
          labelKey: "resource-domain:repository.provider",
        },
        {
          key: "state",
          label: "同步状态",
          labelKey: "resource-domain:market.syncState",
          render: (r) =>
            ({
              idle: commonText("resource-domain:market.idle"),
              syncing: commonText("resource-domain:market.syncing"),
              failed: commonText("common-domain:dashboard.failed"),
            })[r.state] ?? r.state,
        },
        {
          key: "synced_at",
          label: "上次成功",
          labelKey: "resource-domain:market.lastSuccess",
          render: (r) => cronDate(r.synced_at),
        },
        {
          key: "last_error",
          label: "同步错误",
          labelKey: "resource-domain:market.syncError",
          render: r => r.last_error ? <span className="text-destructive">{r.last_error}</span> : "—",
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "marketplace-sources.update",
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
        {
          label: "立即同步",
          labelKey: "resource-domain:market.syncNow",
          permission: "marketplace-sources.sync",
          disabled: r => r.state === "syncing",
          disabledReason: () => commonText("resource-domain:market.syncing"),
          action: async (r) => {
            await runItemAction(
              workspace,
              "marketplace-sources",
              r.id,
              "sync",
              {},
            );
          },
        },
        {
          label: "目录",
          labelKey: "resource-domain:market.catalog",
          to: () => ({ group: "deploy", item: "marketplace", mode: "list" }),
          query: (r) => ({ source_id: r.id }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "marketplace-sources.delete",
          destructive: true,
          action: (r) => deleteResource(workspace, "marketplace-sources", r.id),
        },
      ],
      toolbar: { import: false, export: false },
    }),
    marketplace: list<MarketEntry>({
      resource: "marketplace",
      rowKey: (r) => r.id,
      filters: [
        {
          key: "q",
          label: "名称 / 说明",
          labelKey: "resource-domain:resource.nameDescription",
          type: "text",
        },
        !permissions || permissions.has("marketplace-sources.read")
          ? {
              key: "source_id",
              label: "来源",
              labelKey: "resource-domain:market.source",
              type: "select",
              options: namedOptions(workspace, "marketplace-sources"),
            }
          : {
              key: "source_id",
              label: "来源 ID",
              labelKey: "resource-domain:market.sourceId",
              type: "text",
            },
        {
          key: "trust",
          label: "来源类型",
          labelKey: "resource-domain:market.sourceKind",
          type: "select",
          options: [
            { value: "official", label: "官方", labelKey: "resource-domain:market.official" },
            { value: "community", label: "社区", labelKey: "resource-domain:market.community" },
            { value: "workspace", label: "本空间", labelKey: "resource-domain:market.workspaceSource" },
          ],
        },
      ],
      columns: [
        {
          key: "name",
          label: "应用模板",
          labelKey: "resource-domain:market.appTemplate",
          sortable: true,
        },
        {
          key: "description",
          label: "说明",
          labelKey: "common-domain:common.description",
        },
        {
          key: "source_name",
          label: "来源",
          labelKey: "resource-domain:market.source",
          render: (r) => (
            <span className="inline-flex min-w-0 flex-wrap items-center gap-2">
              <span className="rounded border px-1.5 py-0.5 text-xs text-muted-foreground">{trustLabel(r.trust)}</span>
              <span className="min-w-0 break-words">{r.source_name}</span>
            </span>
          ),
        },
        {
          key: "package_version",
          label: "版本",
          labelKey: "resource-domain:market.packageVersion",
          render: (r) => (
            <span className="inline-flex min-w-0 flex-wrap items-center gap-2">
              <span>{r.package_version === "0.0.0" ? "—" : r.package_version}</span>
              {r.verification === "verified" && (
                <span className="rounded border border-emerald-600/40 px-1.5 py-0.5 text-xs text-emerald-700 dark:text-emerald-400" title={commonText("resource-domain:market.verifiedHelp")}>
                  {commonText("resource-domain:market.verified")} · {r.verified_drivers.join(", ")}
                </span>
              )}
              {r.verification === "declared" && (
                <span className="rounded border px-1.5 py-0.5 text-xs text-muted-foreground" title={commonText("resource-domain:market.declaredHelp")}>
                  {commonText("resource-domain:market.declared")} · {r.verified_drivers.join(", ")}
                </span>
              )}
            </span>
          ),
        },
        {
          key: "entry_key",
          label: "目录标识",
          labelKey: "resource-domain:market.catalogId",
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        marketInstall(workspace, navigate),
      ],
      toolbar: { create: false, import: false, export: false },
    }),
  };
}

export function marketplaceForms({
  workspace,
  navigate,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    "marketplace-sources": form<MarketSource>({
      resource: "marketplace-sources",
      heading: (r) => r.name,
      blank: {
        name: "",
        repository_id: "",
        provider: "github",
        api_base: "https://api.github.com",
        project: "",
        organization: "",
        branch: "main",
        index_path: "marketplace.json",
      },
      sections: [
        {
          fields: [
            {
              key: "name",
              label: "名称",
              labelKey: "common-domain:common.name",
              type: "text",
              required: true,
            },
            {
              key: "repository_id",
              label: "外部仓库",
              labelKey: "resource-domain:market.externalRepository",
              type: "marketplace-repository",
              span: 2,
              options: namedOptions(workspace, "repositories"),
              help: "从代码连接选择仓库，或保留已有仓库绑定；同步平台与 API 配置在下方填写。",
              helpKey: "resource-domain:market.repositoryHelp",
            },
            {
              key: "provider",
              label: "平台",
              labelKey: "resource-domain:repository.provider",
              type: "select",
              required: true,
              options: marketProviders,
            },
            {
              key: "api_base",
              label: "API 接入点",
              labelKey: "resource-domain:repository.apiEndpoint",
              type: "text",
              required: true,
              help: "GitHub: https://api.github.com；GitLab: https://gitlab.com/api/v4；Gitee: https://gitee.com/api/v5；Codeup中心版: https://openapi-rdc.aliyuncs.com。自托管使用所选仓库同主机的API。",
              helpKey: "resource-domain:market.apiHelp",
            },
            {
              key: "project",
              label: "项目路径 / ID",
              labelKey: "resource-domain:repository.project",
              type: "text",
              required: true,
              help: "GitHub/Gitee填写owner/repository；GitLab/Codeup支持项目完整路径或ID。",
              helpKey: "resource-domain:market.projectHelp",
            },
            {
              key: "organization",
              label: "组织 ID",
              labelKey: "resource-domain:repository.organization",
              type: "text",
              when: (d) => d.provider === "codeup",
              help: "Codeup中心版需要组织ID；Region版可留空。",
              helpKey: "resource-domain:market.organizationHelp",
            },
            {
              key: "branch",
              label: "分支 / 标签 / Commit",
              labelKey: "resource-domain:repository.ref",
              type: "text",
              required: true,
            },
            {
              key: "index_path",
              label: "索引文件路径",
              labelKey: "resource-domain:market.indexPath",
              type: "text",
              required: true,
              help: "仓库内JSON文件，格式为version:1及templates数组；所有模板验证成功后才替换目录。",
              helpKey: "resource-domain:market.indexHelp",
            },
          ],
        },
      ],
    }),
    marketplace: form<MarketEntry>({
      resource: "marketplace",
      heading: (r) => r.name,
      blank: {},
      readOnly: () => true,
      viewActions: [marketInstall(workspace, navigate)],
      sections: [
        {
          fields: [
            {
              key: "name",
              label: "名称",
              labelKey: "common-domain:common.name",
              type: "text",
            },
            {
              key: "description",
              label: "说明",
              labelKey: "common-domain:common.description",
              type: "textarea",
              span: 2,
            },
            {
              key: "package_version",
              label: "版本",
              labelKey: "resource-domain:market.packageVersion",
              type: "text",
            },
            {
              key: "readme",
              label: "说明文档",
              labelKey: "resource-domain:market.readme",
              type: "textarea",
              span: 2,
              when: (d) => !!d.readme,
            },
            {
              key: "files",
              label: "附带配置文件",
              labelKey: "resource-domain:market.files",
              type: "custom",
              span: 2,
              when: (d) => Array.isArray(d.files) && d.files.length > 0,
              render: ({ draft }) => <ul className="list-disc space-y-1 pl-5 text-sm">{(draft.files as string[]).map((f) => <li key={f} className="break-all">{f}</li>)}</ul>,
            },
            {
              key: "content",
              label: "应用配置",
              labelKey: "resource-domain:deployment.config",
              type: "code",
              span: 2,
            },
            {
              key: "parameters",
              label: "模板参数",
              labelKey: "resource-domain:template.parameters",
              type: "code",
              span: 2,
            },
          ],
        },
      ],
    }),
  };
}

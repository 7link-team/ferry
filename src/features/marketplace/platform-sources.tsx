import { api } from "../../shared/api/api";
import { commonText } from "../../shared/i18n/i18n";
import { cronDate } from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import { marketProviders, trustLabel } from "./labels";

/** Platform-level public template source (superadmin only, no credentials). */
export interface PlatformSource {
  id: string;
  name: string;
  trust: "official" | "community";
  enabled: boolean;
  sync_interval_hours: number;
  provider: string;
  api_base: string;
  project: string;
  organization: string;
  branch: string;
  index_path: string;
  state: string;
  last_error: string;
  synced_at: number;
  next_sync_at: number;
  entry_count: number;
  version: number;
}

const endpoint = "/platform/marketplace-sources";
export { trustLabel };

export function platformMarketplaceList(): ListConfig<PlatformSource> {
  return {
    resource: "marketplace-sources",
    endpoint,
    rowKey: r => r.id,
    filters: [
      { key: "q", label: "名称", labelKey: "common-domain:common.name", type: "text" },
      { key: "trust", label: "标识", labelKey: "resource-domain:market.trust", type: "select", options: [
        { value: "official", label: "官方", labelKey: "resource-domain:market.official" },
        { value: "community", label: "社区", labelKey: "resource-domain:market.community" },
      ] },
    ],
    columns: [
      { key: "name", label: "名称", labelKey: "common-domain:common.name", sortable: true },
      { key: "trust", label: "标识", labelKey: "resource-domain:market.trust", render: r => trustLabel(r.trust) },
      { key: "enabled", label: "启用", labelKey: "resource-domain:market.enabled", render: r => r.enabled ? commonText("resource-domain:status.enabled") : commonText("resource-domain:status.disabled") },
      { key: "provider", label: "平台", labelKey: "resource-domain:repository.provider" },
      { key: "state", label: "同步状态", labelKey: "resource-domain:market.syncState", render: r => ({ idle: commonText("resource-domain:market.idle"), syncing: commonText("resource-domain:market.syncing"), failed: commonText("common-domain:dashboard.failed") })[r.state] ?? r.state },
      { key: "entry_count", label: "模板版本数", labelKey: "resource-domain:market.entryCount" },
      { key: "synced_at", label: "上次成功", labelKey: "resource-domain:market.lastSuccess", render: r => cronDate(r.synced_at) },
      { key: "last_error", label: "同步错误", labelKey: "resource-domain:market.syncError", render: r => r.last_error ? <span className="break-words text-destructive">{r.last_error}</span> : "—" },
    ],
    actions: [
      { label: "查看", labelKey: "common-domain:common.view", to: r => ({ id: r.id, mode: "view" }) },
      { label: "编辑", labelKey: "common-domain:common.edit", to: r => ({ id: r.id, mode: "edit" }) },
      {
        label: "立即同步", labelKey: "resource-domain:market.syncNow",
        disabled: r => r.state === "syncing",
        disabledReason: () => commonText("resource-domain:market.syncing"),
        action: async r => { await api(`${endpoint}/${encodeURIComponent(r.id)}/synchronizations`, { method: "POST", body: "{}", headers: { "Content-Type": "application/json" } }); },
      },
      { label: "删除", labelKey: "common-domain:common.delete", destructive: true, action: async r => { await api(`${endpoint}/${encodeURIComponent(r.id)}`, { method: "DELETE" }); } },
    ],
    toolbar: { create: { label: "添加来源", labelKey: "resource-domain:market.addSource" }, import: false, export: false },
    empty: "还没有平台模板来源。添加官方或社区的公开模板仓库后，所有工作空间都能在应用市场看到。",
    emptyKey: "resource-domain:market.noPlatformSources",
  };
}

export function platformMarketplaceForm(): FormConfig<PlatformSource> {
  return {
    resource: "marketplace-sources",
    endpoint,
    heading: r => r.name,
    blank: { name: "", trust: "official", enabled: true, sync_interval_hours: 6, provider: "github", api_base: "https://api.github.com", project: "", organization: "", branch: "main", index_path: "marketplace/index.json" },
    sections: [{ fields: [
      { key: "name", label: "名称", labelKey: "common-domain:common.name", type: "text", required: true, max: 60 },
      { key: "trust", label: "标识", labelKey: "resource-domain:market.trust", type: "select", required: true, options: [
        { value: "official", label: "官方", labelKey: "resource-domain:market.official" },
        { value: "community", label: "社区", labelKey: "resource-domain:market.community" },
      ], helpKey: "resource-domain:market.trustHelp" },
      { key: "enabled", label: "启用", labelKey: "resource-domain:market.enabled", type: "switch", helpKey: "resource-domain:market.enabledHelp" },
      { key: "sync_interval_hours", label: "同步间隔（小时）", labelKey: "resource-domain:market.syncInterval", type: "number", required: true, min: 1, max: 168, helpKey: "resource-domain:market.syncIntervalHelp" },
      { key: "provider", label: "平台", labelKey: "resource-domain:repository.provider", type: "select", required: true, options: marketProviders },
      { key: "api_base", label: "API 接入点", labelKey: "resource-domain:repository.apiEndpoint", type: "text", required: true, helpKey: "resource-domain:market.platformApiHelp" },
      { key: "project", label: "项目路径 / ID", labelKey: "resource-domain:repository.project", type: "text", required: true, helpKey: "resource-domain:market.projectHelp" },
      { key: "organization", label: "组织 ID", labelKey: "resource-domain:repository.organization", type: "text", when: d => d.provider === "codeup", helpKey: "resource-domain:market.organizationHelp" },
      { key: "branch", label: "分支 / 标签 / Commit", labelKey: "resource-domain:repository.ref", type: "text", required: true },
      { key: "index_path", label: "索引文件路径", labelKey: "resource-domain:market.indexPath", type: "text", required: true, helpKey: "resource-domain:market.indexHelp" },
    ] }],
  };
}

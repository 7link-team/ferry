import { Eye, History, BellPlus, Trash2, Settings2 } from "lucide-react";
import { deploymentState } from "../deployments/deployment";
import { deleteResource } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import { form, list, namedOptions, type AnyRow, type ResourceConfigContext } from "../../shared/resources/config";
import type { FormConfig, FormField, ListConfig } from "../../shared/resources/types";

const text = (key: string) => commonText(`monitoring:${key}`);
const option = (value: string) => ({ value, label: value, labelKey: `monitoring:${value}` });
const date = (value: unknown) => typeof value === "number" && value > 0 ? new Date(value).toLocaleString(currentLocale()) : "—";
export const healthState = (value: unknown) => typeof value === "string" ? text(value) : text("unknown");
const status = (value: unknown) => <span className="inline-flex min-h-6 items-center gap-1.5"><span aria-hidden className={`size-2 shrink-0 rounded-full ${value === "up" ? "bg-emerald-600" : value === "down" ? "bg-destructive" : value === "degraded" ? "bg-amber-600" : "bg-muted-foreground"}`} />{healthState(value)}</span>;
const reason = (value: unknown) => typeof value === "string" && value.startsWith("monitoring.") ? text(value.slice(11)) : "—";
const outcome = (row: AnyRow) => (row.result as { observation?: AnyRow } | undefined)?.observation ?? {};
const evidence = (value: unknown) => typeof value === "string" ? text(value) : "—";

export function monitoringLists({ workspace }: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    "health-checks": list<AnyRow>({
      resource: "health-checks", permissionResource: "alerts", rowKey: row => String(row.id), defaultRefresh: 10,
      emptyKey: "monitoring:empty",
      filters: [
        { key: "q", label: "", labelKey: "common-domain:common.name", type: "text" },
        { key: "kind", label: "", labelKey: "monitoring:kind", type: "select", options: ["http", "tcp", "runtime"].map(option) },
        { key: "deployment_id", label: "", labelKey: "monitoring:deployment_id", type: "select", options: namedOptions(workspace, "deployments") },
        { key: "host_id", label: "", labelKey: "monitoring:host_id", type: "select", options: namedOptions(workspace, "hosts") },
      ],
      columns: [
        { key: "name", label: "", labelKey: "common-domain:common.name", sortable: true, width: 180, render: row => <span className="block max-w-44 truncate" title={String(row.name)}>{String(row.name)}</span> },
        { key: "state", label: "", labelKey: "monitoring:state", render: row => status(row.state) },
        { key: "kind", label: "", labelKey: "monitoring:kind", render: row => text(String(row.kind)) },
        { key: "role", label: "", labelKey: "monitoring:role", render: row => text(String(row.role)) },
        { key: "position", label: "", labelKey: "monitoring:position", render: row => row.kind === "runtime" ? String(row.host_name || row.host_id) : text("server") },
        { key: "deployment_name", label: "", labelKey: "monitoring:deployment_id", render: row => String(row.deployment_name || row.deployment_id || "—") },
        { key: "checked_at", label: "", labelKey: "monitoring:checked", render: row => date(row.checked_at) },
        { key: "latency", label: "", labelKey: "monitoring:latency", render: row => row.checked_at ? String(outcome(row).latency_ms ?? "—") : "—" },
        { key: "reason", label: "", labelKey: "monitoring:reason", render: row => reason(outcome(row).reason) },
      ],
      actions: [
        { label: "View", icon: <Eye />, labelKey: "common-domain:common.view", to: row => ({ id: String(row.id), mode: "view" }) },
        { label: "Delete", icon: <Trash2 />, labelKey: "common-domain:common.delete", permission: "alerts.delete", destructive: true, action: row => deleteResource(workspace, "health-checks", String(row.id)) },
      ], toolbar: { create: true },
    }),
    "health-observations": list<AnyRow>({
      resource: "health-observations", permissionResource: "alerts", rowKey: row => String(row.id), defaultRefresh: 10,
      filters: [{ key: "check_id", label: "", labelKey: "monitoring:checks", type: "select", options: namedOptions(workspace, "health-checks") }],
      columns: [
        { key: "observed_at", label: "", labelKey: "monitoring:checked", sortable: true, render: row => date(row.observed_at) },
        { key: "outcome", label: "", labelKey: "monitoring:state", render: row => status((row.result as AnyRow)?.outcome) },
        { key: "latency", label: "", labelKey: "monitoring:latency", render: row => String((row.result as AnyRow)?.latency_ms ?? "—") },
        { key: "reason", label: "", labelKey: "monitoring:reason", render: row => reason((row.result as AnyRow)?.reason) },
      ], toolbar: { create: false },
    }),
    "application-health": list<AnyRow>({
      resource: "application-health", permissionResource: "alerts", rowKey: row => String(row.id), defaultRefresh: 10,
      filters: [{ key: "q", label: "", labelKey: "common-domain:common.name", type: "text" }],
      columns: [
        { key: "name", label: "", labelKey: "common-domain:common.name", sortable: true, width: 180, render: row => <span className="block max-w-44 truncate" title={String(row.name)}>{String(row.name)}</span> },
        { key: "health", label: "", labelKey: "monitoring:state", render: row => status(row.health) },
        { key: "entry_state", label: "", labelKey: "monitoring:entry_state", render: row => status(row.entry_state) },
        { key: "replicas", label: "", labelKey: "monitoring:replicas", render: row => `${row.healthy_instances} / ${row.expected_instances}` },
        { key: "unknown_instances", label: "", labelKey: "monitoring:unknown_instances" },
        { key: "unmonitored_hosts", label: "", labelKey: "monitoring:unmonitored_hosts" },
        { key: "deployment_state", label: "", labelKey: "monitoring:deployment_state", render: row => deploymentState(String(row.deployment_state)) },
      ], actions: [
        { label: "View", icon: <Eye />, labelKey: "common-domain:common.view", to: row => ({ id: String(row.id), mode: "view" }) },
        { label: "Configure", icon: <Settings2 />, labelKey: "monitoring:configure", to: () => ({ group: "deploy", item: "health-checks", mode: "list" }), query: row => ({ deployment_id: String(row.id) }) },
        { label: "Create alert policy", icon: <BellPlus />, labelKey: "monitoring:createPolicy", permission: "alerts.create", to: () => ({ group: "deploy", item: "alerts", mode: "new" }), query: row => ({ target_kind: "application-health", target_ref: String(row.id), metric: "health-failed" }) },
      ], toolbar: { create: false },
    }),
  };
}

export function monitoringForms(context: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  const { workspace } = context;
  const field = (key: string, type: FormField<AnyRow>["type"] = "text", extra: Partial<FormField<AnyRow>> = {}): FormField<AnyRow> => ({ key, type, label: "", labelKey: `monitoring:${key}`, ...extra });
  const instanceField: FormField<AnyRow> = field("instances", "records", {
    record: { title: item => `${item.service} · ${item.node} · ${item.id}`, meta: item => <span>{healthState(item.state)} · {evidence(item.evidence)}</span>, body: item => <span className="break-words text-muted-foreground">{String(item.runtime_state ?? "")}</span> },
  });
  return {
    "health-checks": form<AnyRow>({
      resource: "health-checks", permissionResource: "alerts", heading: row => String(row.name), presentation: "sections", descriptionKey: "monitoring:help",
      queryDefaults: ["deployment_id", "host_id"],
      blank: { name: "", description: "", kind: "http", role: "entry", deployment_id: "", host_id: "", service_name: "", enabled: false, address: "", expected_status: 200, body_contains: "", interval_seconds: 30, timeout_seconds: 5, failure_threshold: 3, recovery_threshold: 2, stale_seconds: 90, retention_days: 7 },
      pollMs: row => row.enabled ? 5000 : 0,
      sections: [
        { titleKey: "monitoring:settings", fields: [
          { key: "name", type: "text", label: "", labelKey: "common-domain:common.name", required: true },
          { key: "enabled", type: "switch", label: "", labelKey: "resource-domain:status.enabled" },
          field("kind", "select", { required: true, options: ["http", "tcp", "runtime"].map(option), resetOnChange: ["address", "body_contains", "host_id", "service_name" ] }),
          field("role", "select", { required: true, options: ["entry", "service"].map(option), when: row => row.kind !== "runtime", resetOnChange: ["service_name"] }),
          field("deployment_id", "select", { options: namedOptions(workspace, "deployments"), width: "full", required: row => row.kind === "runtime" || row.role === "service", resetOnChange: ["host_id", "service_name"] }),
          field("host_id", "select", { options: namedOptions(workspace, "hosts"), width: "full", when: row => row.kind === "runtime", required: true, helpKey: "monitoring:runtimeHelp" }),
          field("service_name", "text", { when: row => row.kind !== "runtime" && row.role === "service", required: true }),
          field("address", "text", { when: row => row.kind !== "runtime", required: true, span: 2 }),
          field("expected_status", "number", { when: row => row.kind === "http", min: 100, max: 599, required: true }),
          field("body_contains", "text", { when: row => row.kind === "http" }),
          { key: "description", type: "textarea", label: "", labelKey: "common-domain:common.description", span: 2 },
        ] },
        { titleKey: "monitoring:timing", fields: [
          field("interval_seconds", "number", { min: 5, max: 86400, required: true }), field("timeout_seconds", "number", { min: 1, max: 300, required: true }),
          field("failure_threshold", "number", { min: 1, max: 100, required: true }), field("recovery_threshold", "number", { min: 1, max: 100, required: true }),
          field("stale_seconds", "number", { min: 6, max: 604800, required: true }), field("retention_days", "number", { min: 1, max: 365, required: true }),
        ] },
        { titleKey: "monitoring:result", fields: [
          field("state", "text", { when: row => !!row.id, disabled: () => true, display: row => healthState(row.state) }),
          field("checked_at", "text", { labelKey: "monitoring:checked", when: row => !!row.id, disabled: () => true, display: row => date(row.checked_at) }),
          field("streak", "text", { when: row => !!row.id, disabled: () => true, display: row => { const result = row.result as AnyRow | undefined; return `${result?.successes ?? 0} / ${result?.failures ?? 0}`; } }),
          { ...instanceField, when: row => !!row.id && row.kind === "runtime", span: 2 },
        ] },
      ], viewActions: [
        { label: "History", icon: <History />, labelKey: "monitoring:history", to: () => ({ group: "deploy", item: "health-observations", mode: "list" }), query: row => ({ check_id: String(row.id) }) },
        { label: "Create alert policy", icon: <BellPlus />, labelKey: "monitoring:createPolicy", permission: "alerts.create", to: () => ({ group: "deploy", item: "alerts", mode: "new" }), query: row => ({ target_kind: "health-check", target_ref: String(row.id), metric: "health-failed" }) },
      ],
    }),
    "application-health": form<AnyRow>({
      resource: "application-health", permissionResource: "alerts", heading: row => String(row.name), readOnly: () => true, pollMs: () => 5000, blank: {}, descriptionKey: "monitoring:summaryHelp",
      sections: [{ fields: [
        field("health", "text", { labelKey: "monitoring:state", display: row => healthState(row.health) }),
        field("entry_state", "text", { display: row => healthState(row.entry_state) }),
        field("replicas", "text", { display: row => `${row.healthy_instances} / ${row.expected_instances}` }),
        field("unknown_instances", "number"), field("unmonitored_hosts", "number"),
        field("services", "records", { labelKey: "monitoring:service", span: 2, record: { title: item => String(item.name), meta: item => `${item.healthy} / ${item.expected} · ${text("unknown")}: ${item.unknown}` } }),
        { ...instanceField, span: 2 },
        field("checks", "records", { labelKey: "monitoring:checks", span: 2, record: { title: item => String(item.name), meta: item => <span>{healthState(item.state)} · {date(item.checked_at)}</span>, href: item => context.href?.({ workspace, group: "deploy", item: "health-checks", id: String(item.id), mode: "view" }) ?? "#" } }),
      ] }], viewActions: [
        { label: "Add health check", labelKey: "monitoring:addCheck", permission: "alerts.create", to: () => ({ group: "deploy", item: "health-checks", mode: "new" }), query: row => ({ deployment_id: String(row.id) }) },
        { label: "Create alert policy", labelKey: "monitoring:createPolicy", permission: "alerts.create", to: () => ({ group: "deploy", item: "alerts", mode: "new" }), query: row => ({ target_kind: "application-health", target_ref: String(row.id), metric: "health-failed" }) },
        { label: "Configure", icon: <Settings2 />, labelKey: "monitoring:configure", to: () => ({ group: "deploy", item: "health-checks", mode: "list" }), query: row => ({ deployment_id: String(row.id) }) }],
    }),
  };
}

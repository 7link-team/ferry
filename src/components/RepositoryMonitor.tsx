import { hostRepositoryText, useCommonTranslation } from "../i18n";
import { selectDisplay } from "./auto-form";
import { currentLocale } from "../locale";
import { useCallback, useMemo } from "react";
import { Bell, Pencil } from "lucide-react";
import type { FormConfig, ListConfig } from "./dsl";
import { ResourceForm } from "./ResourceForm";
import { ResourceTable } from "./ResourceTable";
import { patchResource, base, listResource, named } from "../resource";

export const repositoryEventOptions = [
  ["repository.created", "repository-monitor.001"], ["repository.deleted", "repository-monitor.002"],
  ["branch.created", "repository-monitor.003"], ["branch.deleted", "repository-monitor.004"],
  ["push", "repository-monitor.005"], ["pull_request", "repository-monitor.006"], ["merge", "repository-monitor.007"],
  ["tag.created", "repository-monitor.008"], ["tag.deleted", "repository-monitor.009"],
].map(([value, key]) => ({ value: value!, get label() { return hostRepositoryText(key!); } }));

export const gitRepositoryEventOptions = [
  ...repositoryEventOptions.filter(option => ["push", "branch.created", "branch.deleted", "tag.created", "tag.deleted"].includes(option.value)),
  { value: "tag.updated", get label() { return hostRepositoryText("repository-monitor.010"); } },
];
const eventLabel = (value: unknown) => selectDisplay(value, { options: [...repositoryEventOptions, ...gitRepositoryEventOptions] });

const time = (value: unknown) => {
  if (value == null || value === "" || value === 0 || value === "0") return "—";
  if (typeof value === "number" && value > 0) return new Date(value).toLocaleString(currentLocale());
  const parsed = typeof value === "string" && value ? Date.parse(value) : NaN;
  return Number.isNaN(parsed) ? String(value ?? "") : new Date(parsed).toLocaleString(currentLocale());
};
const has = (key: string) => (row: Record<string, unknown>) => row[key] !== undefined && row[key] !== null && row[key] !== "";
const commitRecord = {
  title: (item: Record<string, unknown>) => String(item.short_id ?? item.id ?? ""),
  meta: (item: Record<string, unknown>) => [item.author_username && item.author_username !== item.author ? `${item.author}（${item.author_username}）` : item.author, item.time ? time(item.time) : ""].filter(Boolean).join(" · "),
  body: (item: Record<string, unknown>) => String(item.message ?? ""),
  href: (item: Record<string, unknown>) => String(item.url ?? ""),
};

/** Read-only detail page of one repository event (shared form engine). */
export function repositoryEventForm(): FormConfig<Record<string, unknown>> {
  const text = (key: string, label: string, extra: Partial<FormConfig<Record<string, unknown>>["sections"][number]["fields"][number]> = {}) =>
    ({ key, label, type: "text" as const, when: has(key), ...extra });
  return {
    resource: "repository-events", permissionResource: "repositories", width: "full", blank: {}, readOnly: () => true,
    heading: row => `${eventLabel(row.event) || hostRepositoryText("repository-monitor.011")} · ${String(row.repository ?? "")}`,
    viewActions: [{ label: hostRepositoryText("repository-monitor.012"), to: row => ({ group: "build", item: "repositories", mode: "view", id: String(row.connection_id) }), query: () => ({ tab: "events" }) }],
    sections: [
      { title: hostRepositoryText("repository-monitor.013"), fields: [
        { key: "event", label: hostRepositoryText("repository-monitor.014"), type: "select", options: [...repositoryEventOptions, { value: "tag.updated", label: hostRepositoryText("repository-monitor.010") }] },
        text("repository", hostRepositoryText("repository.004")), text("reference", hostRepositoryText("repository-monitor.015")),
        text("actor", hostRepositoryText("repository-monitor.016"), { when: has("actor") }),
        text("action", hostRepositoryText("repository-monitor.017")),
        text("created_at", hostRepositoryText("repository-monitor.018"), { display: row => time(row.created_at) }),
        { key: "source", label: hostRepositoryText("repository-monitor.019"), type: "select", options: [{ value: "webhook", label: hostRepositoryText("repository-monitor.020") }, { value: "polling", label: hostRepositoryText("repository-monitor.021") }], when: has("source") },
        text("delivery", hostRepositoryText("repository-monitor.022")),
      ] },
      { title: hostRepositoryText("repository-monitor.023"), fullWidth: true, fields: [
        text("total_commits", hostRepositoryText("repository-monitor.024")), text("before", hostRepositoryText("repository-monitor.025")), text("after", hostRepositoryText("repository-monitor.026")),
        { key: "forced", label: hostRepositoryText("repository-monitor.027"), type: "switch", when: has("forced") },
        text("compare_url", hostRepositoryText("repository-monitor.028"), { link: true, span: 2 }),
        { key: "commits", label: hostRepositoryText("repository-monitor.029"), type: "records", record: commitRecord, span: 2, when: has("commits") },
        text("commits_truncated", hostRepositoryText("repository-monitor.030"), { span: 2 }),
        { key: "head_commit", label: hostRepositoryText("repository-monitor.031"), type: "records", record: commitRecord, span: 2, when: has("head_commit") },
      ] },
      { title: hostRepositoryText("repository-monitor.032"), fullWidth: true, fields: [
        text("request_number", hostRepositoryText("repository-monitor.033")), text("request_title", hostRepositoryText("repository-monitor.034")), text("request_state", hostRepositoryText("repository-monitor.035")), text("request_branches", hostRepositoryText("repository-monitor.036")),
        text("request_author", hostRepositoryText("repository-monitor.037")), text("request_merged_by", hostRepositoryText("repository-monitor.038")), text("request_reviewers", hostRepositoryText("repository-monitor.039")), text("request_merge_commit", hostRepositoryText("repository-monitor.040")),
        text("request_created_at", hostRepositoryText("repository-monitor.041"), { display: row => time(row.request_created_at) }),
        text("request_updated_at", hostRepositoryText("repository-monitor.042"), { display: row => time(row.request_updated_at) }),
        text("request_merged_at", hostRepositoryText("repository-monitor.043"), { display: row => time(row.request_merged_at) }),
        text("request_closed_at", hostRepositoryText("repository-monitor.044"), { display: row => time(row.request_closed_at) }),
        text("request_url", hostRepositoryText("repository-monitor.045"), { link: true, span: 2 }),
        { key: "request_description", label: hostRepositoryText("repository-monitor.030"), type: "textarea", span: 2, when: has("request_description") },
      ] },
      { title: hostRepositoryText("repository-monitor.046"), fields: [
        text("tag_target", hostRepositoryText("repository-monitor.047")), text("tagger", hostRepositoryText("repository-monitor.048")),
        { key: "tag_message", label: hostRepositoryText("repository-monitor.049"), type: "textarea", span: 2, when: has("tag_message") },
      ] },
      { title: hostRepositoryText("repository-monitor.050"), fields: [
        text("repository_visibility", hostRepositoryText("repository-monitor.051")), text("repository_default_branch", hostRepositoryText("repository-monitor.052")),
        text("repository_url", hostRepositoryText("repository-monitor.053"), { link: true, span: 2 }),
        { key: "repository_description", label: hostRepositoryText("repository-monitor.054"), type: "textarea", span: 2, when: has("repository_description") },
      ] },
      { title: hostRepositoryText("repository-monitor.030"), fullWidth: true, fields: [{ key: "note", label: hostRepositoryText("repository-monitor.030"), type: "textarea", span: 2, when: has("note") }] },
    ],
  };
}

export function RepositoryMonitor({ workspace, connectionId, permissions, provider }: { workspace: string; connectionId: string; permissions: Set<string>; provider?: string | undefined }) {
  const { i18n } = useCommonTranslation();
  const canSelectRunner = permissions.has("hosts.read") && permissions.has("hosts.terminal");
  const runnerOptions = useCallback(async () => {
    const options: { value: string; label: string }[] = [];
    if (!canSelectRunner) return options;
    for (let page = 1; ; page++) {
      const result = await listResource<{ id: string; name: string; connection: string }>(workspace, "nodes", { page, page_size: 100 });
      options.push(...result.items.filter(node => node.connection === "runner").map(node => ({ value: node.id, label: node.name })));
      if (!result.items.length || page * 100 >= result.total) break;
    }
    return options;
  }, [workspace, canSelectRunner]);
  const form = useMemo<FormConfig<Record<string, unknown>>>(() => ({
    resource: "code-connection-monitors", permissionResource: "repositories", width: "full", blank: {}, heading: () => hostRepositoryText("repository-monitor.055"), readOnly: () => true,
    pollMs: row => row.enabled && row.mode === "poll" ? 15000 : 0,
    sections: [{ title: hostRepositoryText("repository-monitor.056"), fullWidth: true, fields: [
      { key: "mode", label: hostRepositoryText("repository-monitor.057"), type: "select", options: [{ value: "push", label: hostRepositoryText("repository-monitor.020") }, { value: "poll", label: hostRepositoryText("repository-monitor.021") }] },
      { key: "poll_interval_minutes", label: hostRepositoryText("repository-monitor.058"), type: "number", when: row => row.mode === "poll" },
      { key: "poll_status", label: hostRepositoryText("repository-monitor.059"), type: "text", when: row => row.mode === "poll" },
      { key: "last_poll_error", label: hostRepositoryText("repository-monitor.060"), type: "text", when: row => row.mode === "poll" && !!row.last_poll_error, span: 2 },
      { key: "enabled", label: hostRepositoryText("repository-monitor.061"), type: "switch" },
      { key: "runner_id", label: hostRepositoryText("repository-monitor.062"), type: "text", when: row => row.provider === "git", display: row => named(typeof row.runner_name === "string" ? row.runner_name : undefined, String(row.runner_id ?? "")), help: canSelectRunner ? hostRepositoryText("repository-monitor.063") : hostRepositoryText("repository-monitor.064") },
      { key: "all_repositories", label: hostRepositoryText("repository-monitor.065"), type: "switch", when: row => row.provider !== "git" },
      { key: "repositories", label: hostRepositoryText("repository-monitor.066"), type: "textarea", when: row => !row.all_repositories },
      { key: "events", label: hostRepositoryText("repository-monitor.067"), type: "multiselect", options: repositoryEventOptions, when: row => row.provider !== "git", span: 2 },
      { key: "events", label: hostRepositoryText("repository-monitor.067"), type: "multiselect", options: gitRepositoryEventOptions, when: row => row.provider === "git", span: 2 },
      { key: "capability_note", label: hostRepositoryText("repository-monitor.068"), type: "text", span: 2 },
      { key: "callback_path", when: row => row.provider !== "git" && row.mode !== "poll", label: hostRepositoryText("repository-monitor.069"), type: "text", span: 2, help: hostRepositoryText("repository-monitor.070", { origin: location.origin }) },
      { key: "secret", label: hostRepositoryText("repository-monitor.071"), type: "secret", stored: "has_secret", when: row => row.provider !== "git" && row.mode !== "poll" },
      { key: "webhook", label: hostRepositoryText("repository-monitor.072"), type: "secret", stored: "has_webhook" },
      { key: "provider_changed", label: hostRepositoryText("repository-monitor.073"), type: "switch", when: row => !!row.provider_changed },
    ] }],
    viewActions: [{
      label: hostRepositoryText("repository-monitor.074"), section: hostRepositoryText("repository-monitor.056"), iconOnly: true, icon: <Pencil className="size-4" />, permission: "repositories.update",
      when: row => row.provider !== "git" || canSelectRunner,
      description: hostRepositoryText("repository-monitor.075"),
      initialValues: row => ({ enabled: row.enabled, mode: row.provider === "git" ? "poll" : row.mode ?? "push", ...(row.provider === "git" ? { runner_id: row.runner_id ?? "" } : {}), poll_interval_minutes: row.poll_interval_minutes ?? 1, all_repositories: row.provider === "git" ? false : row.all_repositories, repositories: Array.isArray(row.repositories) ? row.repositories.join("\n") : row.repositories, events: Array.isArray(row.events) ? row.events.filter(event => ![...repositoryEventOptions, ...gitRepositoryEventOptions].some(option => option.value === event) || (row.provider === "git" ? gitRepositoryEventOptions.some(option => option.value === event) : row.mode === "poll" || row.provider !== "codeup" || !String(event).startsWith("repository."))) : [] }),
      parameters: row => row.provider === "git" ? [
        { key: "runner_id", label: hostRepositoryText("repository-monitor.062"), type: "select", required: true, options: runnerOptions, help: hostRepositoryText("repository-monitor.076") },
        { key: "poll_interval_minutes", label: hostRepositoryText("repository-monitor.058"), type: "number", min: 1, required: true, help: hostRepositoryText("repository-monitor.077") },
        { key: "enabled", label: hostRepositoryText("repository-monitor.061"), type: "switch" },
        { key: "repositories", label: hostRepositoryText("repository-monitor.078"), type: "textarea", required: true, help: hostRepositoryText("repository-monitor.079") },
        { key: "events", label: hostRepositoryText("repository-monitor.067"), type: "multiselect", options: gitRepositoryEventOptions, required: true },
        { key: "webhook", label: hostRepositoryText("repository-monitor.080"), type: "password", help: hostRepositoryText("repository-monitor.081") },
        { key: "clear_webhook", label: hostRepositoryText("repository-monitor.082"), type: "switch", when: () => !!row.has_webhook },
      ] : [
        { key: "mode", label: hostRepositoryText("repository-monitor.057"), type: "select", required: true, options: [{ value: "push", label: hostRepositoryText("repository-monitor.020") }, { value: "poll", label: hostRepositoryText("repository-monitor.021") }], resetOnChange: ["events"], help: hostRepositoryText("repository-monitor.083") },
        { key: "poll_interval_minutes", label: hostRepositoryText("repository-monitor.058"), type: "number", min: 1, required: true, when: value => value.mode === "poll", help: hostRepositoryText("repository-monitor.084") },
        { key: "enabled", label: hostRepositoryText("repository-monitor.061"), type: "switch" },
        { key: "all_repositories", label: hostRepositoryText("repository-monitor.065"), type: "switch" },
        { key: "repositories", label: hostRepositoryText("repository-monitor.085"), type: "textarea", when: value => !value.all_repositories, required: true, help: hostRepositoryText("repository-monitor.086") },
        { key: "events", label: hostRepositoryText("repository-monitor.067"), type: "multiselect", options: repositoryEventOptions.filter(option => row.provider !== "codeup" || !option.value.startsWith("repository.")), required: true, when: value => value.mode !== "poll" },
        { key: "events", label: hostRepositoryText("repository-monitor.067"), type: "multiselect", options: repositoryEventOptions, required: true, when: value => value.mode === "poll" },
        { key: "secret", label: hostRepositoryText("repository-monitor.071"), type: "password", when: value => value.mode !== "poll", required: !row.has_secret, help: hostRepositoryText("repository-monitor.087") },
        { key: "webhook", label: hostRepositoryText("repository-monitor.080"), type: "password", help: hostRepositoryText("repository-monitor.088") },
        { key: "clear_webhook", label: hostRepositoryText("repository-monitor.082"), type: "switch", when: () => !!row.has_webhook },
      ],
      action: async (row, values) => { await patchResource(workspace, "code-connection-monitors", connectionId, { ...values, ...(row.provider === "git" ? { mode: "poll", all_repositories: false } : {}), version: row.version }); },
    }],
  }), [workspace, connectionId, canSelectRunner, runnerOptions, i18n.resolvedLanguage]);
  return <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-auto">
    <div className="rounded-lg border bg-muted/30 p-4 text-sm leading-6 text-muted-foreground">
      <p className="flex items-center gap-2 font-medium text-foreground"><Bell className="size-4" />{hostRepositoryText("repository-monitor.089")}</p>
      {provider === "git" && !canSelectRunner && <p>{hostRepositoryText("repository-monitor.064")}</p>}
      <p>{hostRepositoryText("repository-monitor.090")}</p>
      {provider !== "git" && <><p>{hostRepositoryText("repository-monitor.091")}</p>
      <p>{hostRepositoryText("repository-monitor.092")}</p></>}
    </div>
    <ResourceForm config={form} workspace={workspace} permissions={permissions} showViewNavigation={false} />

  </div>;
}

/**
 * Repository event list: one connection's history, or (without a connection)
 * every connection the user owns in the workspace, with its connection name.
 */
export function repositoryEventList(workspace: string, connectionId?: string, provider?: string): ListConfig<Record<string, unknown>> {
  return {
    resource: "repository-events", permissionResource: "repositories", rowKey: row => String(row.id), defaultRefresh: 15,
    ...(connectionId ? { endpoint: `${base(workspace, "code-connections")}/${encodeURIComponent(connectionId)}/events` } : {}),
    filters: [{ key: "q", label: hostRepositoryText("repository-monitor.093"), type: "text" }, { key: "event", label: hostRepositoryText("repository-monitor.014"), type: "select", options: provider === "git" ? gitRepositoryEventOptions : repositoryEventOptions }],
    columns: [
      { key: "created_at", label: hostRepositoryText("repository-monitor.018"), sortable: true, render: row => time(row.created_at) },
      ...(connectionId ? [] : [{ key: "connection_name", label: hostRepositoryText("repository.002") }]),
      { key: "repository", label: hostRepositoryText("repository.004"), sortable: true },
      { key: "event", label: hostRepositoryText("repository-monitor.014"), render: row => eventLabel(row.event) },
      { key: "reference", label: hostRepositoryText("repository-monitor.015") }, { key: "summary", label: hostRepositoryText("repository-monitor.094") }, { key: "actor", label: hostRepositoryText("repository-monitor.016") }, { key: "action", label: hostRepositoryText("repository-monitor.017") }, { key: "delivery", label: hostRepositoryText("repository-monitor.022") },
    ], actions: [{ label: hostRepositoryText("repository-monitor.095"), to: row => ({ group: "build", item: "repository-events", mode: "view", id: String(row.id) }) }],
    toolbar: { create: false, import: false, export: false }, empty: provider === "git" ? hostRepositoryText("repository-monitor.096") : hostRepositoryText("repository-monitor.097"),
  };
}

export function RepositoryEventHistory({ workspace, connectionId, permissions, provider }: { workspace: string; connectionId: string; permissions: Set<string>; provider?: string | undefined }) {
  const { i18n } = useCommonTranslation();
  const events = useMemo(() => repositoryEventList(workspace, connectionId, provider), [workspace, connectionId, provider, i18n.resolvedLanguage]);
  return <section className="flex min-h-80 flex-1 flex-col gap-3" aria-label={hostRepositoryText("repository-monitor.098")}><h3 className="text-base font-semibold">{hostRepositoryText("repository-monitor.099")}</h3><ResourceTable config={events} workspace={workspace} permissions={permissions} /></section>;
}

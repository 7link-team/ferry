import { Bell,Pencil } from "lucide-react";
import { useCallback,useMemo } from "react";
import { gitRepositoryEventOptions,repositoryEventList,repositoryEventOptions } from "../../features/repositories/event-config";
import { listResource,named,patchResource } from "../../shared/api/resource";
import { hostRepositoryText,useCommonTranslation } from "../../shared/i18n/i18n";
import { localeNames,supportedLocales } from "../../shared/i18n/locale";
import type { FormConfig } from "../../shared/resources/types";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
export * from "../../features/repositories/event-config";

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
      { key: "webhook", label: hostRepositoryText("repository-monitor.072"), type: "text" },
      { key: "notification_language", label: hostRepositoryText("repository-monitor.notificationLanguage"), type: "select", options: supportedLocales.map(value => ({ value, label: localeNames[value] })) },
      { key: "provider_changed", label: hostRepositoryText("repository-monitor.073"), type: "switch", when: row => !!row.provider_changed },
    ] }],
    viewActions: [{
      label: hostRepositoryText("repository-monitor.074"), section: hostRepositoryText("repository-monitor.056"), iconOnly: true, icon: <Pencil className="size-4" />, permission: "repositories.update",
      when: row => row.provider !== "git" || canSelectRunner,
      description: hostRepositoryText("repository-monitor.075"),
      initialValues: row => ({ notification_language: row.notification_language || "en-US", webhook: row.webhook ?? "", clear_webhook: false, enabled: row.enabled, mode: row.provider === "git" ? "poll" : row.mode ?? "push", ...(row.provider === "git" ? { runner_id: row.runner_id ?? "" } : {}), poll_interval_minutes: row.poll_interval_minutes ?? 1, all_repositories: row.provider === "git" ? false : row.all_repositories, repositories: Array.isArray(row.repositories) ? row.repositories.join("\n") : row.repositories, events: Array.isArray(row.events) ? row.events.filter(event => ![...repositoryEventOptions, ...gitRepositoryEventOptions].some(option => option.value === event) || (row.provider === "git" ? gitRepositoryEventOptions.some(option => option.value === event) : row.mode === "poll" || row.provider !== "codeup" || !String(event).startsWith("repository."))) : [] }),
      parameters: row => row.provider === "git" ? [
        { key: "runner_id", label: hostRepositoryText("repository-monitor.062"), type: "select", required: true, options: runnerOptions, help: hostRepositoryText("repository-monitor.076") },
        { key: "poll_interval_minutes", label: hostRepositoryText("repository-monitor.058"), type: "number", min: 1, required: true, help: hostRepositoryText("repository-monitor.077") },
        { key: "enabled", label: hostRepositoryText("repository-monitor.061"), type: "switch" },
        { key: "repositories", label: hostRepositoryText("repository-monitor.078"), type: "textarea", required: true, help: hostRepositoryText("repository-monitor.079") },
        { key: "events", label: hostRepositoryText("repository-monitor.067"), type: "multiselect", options: gitRepositoryEventOptions, required: true },
        { key: "webhook", label: hostRepositoryText("repository-monitor.080"), type: "text", help: hostRepositoryText("repository-monitor.081") },
        { key: "notification_language", label: hostRepositoryText("repository-monitor.notificationLanguage"), type: "select", required: true, options: supportedLocales.map(value => ({ value, label: localeNames[value] })) },
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
        { key: "webhook", label: hostRepositoryText("repository-monitor.080"), type: "text", help: hostRepositoryText("repository-monitor.088") },
        { key: "notification_language", label: hostRepositoryText("repository-monitor.notificationLanguage"), type: "select", required: true, options: supportedLocales.map(value => ({ value, label: localeNames[value] })) },
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

export function RepositoryEventHistory({ workspace, connectionId, permissions, provider }: { workspace: string; connectionId: string; permissions: Set<string>; provider?: string | undefined }) {
  const { i18n } = useCommonTranslation();
  const events = useMemo(() => repositoryEventList(workspace, connectionId, provider), [workspace, connectionId, provider, i18n.resolvedLanguage]);
  return <section className="flex min-h-80 flex-1 flex-col gap-3" aria-label={hostRepositoryText("repository-monitor.098")}><h3 className="text-base font-semibold">{hostRepositoryText("repository-monitor.099")}</h3><ResourceTable config={events} workspace={workspace} permissions={permissions} /></section>;
}

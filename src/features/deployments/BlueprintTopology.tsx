import { useId, useMemo, useState } from "react";
import { api } from "../../shared/api/api";
import { named } from "../../shared/api/resource";
import { commonText, useCommonTranslation } from "../../shared/i18n/i18n";
import { Badge } from "../../shared/ui/badge";
import { Button } from "../../shared/ui/button";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";
import { failoverClass, readBlueprint, type BlueprintSummary } from "./blueprint-spec";
import { driverLabel } from "./BlueprintWizard";
import { useHostNames, useInstanceHealth, type InstanceHealth } from "./blueprint-hooks";
import { deploymentActive, instanceResource, type BlueprintBinding, type Deployment } from "./deployment";

const t = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:topology.${key}`, values);
const w = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:wizard.${key}`, values);
const f = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:failover.${key}`, values);

export type InstanceState = "up" | "down" | "unknown" | "loading";
export interface TopologyRow { instance: string; hostId: string; hostName?: string; state: InstanceState; replicas?: number; healthy?: number; expected?: number }
export interface TopologyGroup { role: string; identity: string; rows: TopologyRow[] }

/**
 * Role → instance → host, in blueprint role order, merged with health by instance name
 * (`<role>-<index>`). A stateless Swarm role is one service replicated from the manager,
 * so it is one row there. Instances without a health entry are unknown, never healthy.
 */
export function topologyGroups(binding: BlueprintBinding, blueprint: BlueprintSummary | undefined, health: InstanceHealth[] | undefined, loading = false): TopologyGroup[] {
  const byName = new Map((health ?? []).map(entry => [entry.name, entry]));
  const state = (instance: string): Pick<TopologyRow, "state" | "hostName" | "healthy" | "expected"> => {
    const entry = byName.get(instance);
    if (!entry) return { state: loading ? "loading" : "unknown" };
    return {
      state: entry.state === "up" || entry.state === "down" ? entry.state : "unknown",
      ...(entry.host_name ? { hostName: entry.host_name } : {}),
      ...(typeof entry.healthy === "number" ? { healthy: entry.healthy } : {}),
      ...(typeof entry.expected === "number" ? { expected: entry.expected } : {}),
    };
  };
  const ordered = blueprint?.roles.map(role => role.name).filter(role => role in binding.roles) ?? [];
  const roles = [...ordered, ...Object.keys(binding.roles).filter(role => !ordered.includes(role))];
  return roles.map(role => {
    const list = binding.roles[role] ?? [];
    const identity = blueprint?.roles.find(r => r.name === role)?.identity ?? "";
    if (binding.driver === "stack" && binding.manager && identity === "stateless")
      return { role, identity, rows: [{ instance: role, hostId: binding.manager, replicas: list.length, ...state(role) }] };
    return { role, identity, rows: list.map((item, index) => ({ instance: `${role}-${index}`, hostId: item.hostId, ...state(`${role}-${index}`) })) };
  });
}

const STATE_CLASS: Record<InstanceState, string> = {
  up: "border-emerald-600/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  down: "border-destructive/40 bg-destructive/10 text-destructive",
  unknown: "border-border text-muted-foreground",
  loading: "border-border text-muted-foreground",
};

function HealthBadge({ state }: { state: InstanceState }) {
  return <Badge variant="outline" data-health={state} className={STATE_CLASS[state]}>{t(`state.${state}`)}</Badge>;
}

/** The detail header's target of a blueprint deployment: driver and distinct bound hosts (and the Swarm manager). */
export function BlueprintTarget({ workspace, deployment, permissions }: { workspace: string; deployment: Deployment; permissions: Set<string> }) {
  useCommonTranslation();
  const binding = deployment.blueprint_binding;
  const manager = binding?.driver === "stack" ? binding.manager : undefined;
  const names = useHostNames(workspace, permissions.has("hosts.read") && !!manager);
  if (!binding) return <>—</>;
  const hosts = new Set(Object.values(binding.roles).flatMap(list => list.map(item => item.hostId))).size;
  return <>{[driverLabel(binding.driver), t("hostCount", { count: hosts }), manager && t("manager", { host: named(names.get(manager), manager) })].filter(Boolean).join(" · ")}</>;
}

/** Where a blueprint deployment's instances run and how healthy each one is. */
export function BlueprintTopology({ workspace, deployment, permissions, onDone }: { workspace: string; deployment: Deployment; permissions: Set<string>; onDone?: () => void }) {
  useCommonTranslation();
  const titleId = useId();
  const [replacing, setReplacing] = useState<string>();
  const [operation, setOperation] = useState<string>();
  const binding = deployment.blueprint_binding;
  const canHealth = permissions.has("alerts.read") && permissions.has("deployments.read");
  const names = useHostNames(workspace, permissions.has("hosts.read") && !!binding);
  const health = useInstanceHealth(workspace, deployment.id, canHealth && !!binding, `${deployment.current_revision}/${deployment.state}`);
  const blueprint = useMemo(() => readBlueprint(deployment.manifest ?? ""), [deployment.manifest]);
  const groups = useMemo(() => binding ? topologyGroups(binding, blueprint, health.placement, health.loading) : [], [binding, blueprint, health.placement, health.loading]);
  const hostName = (row: TopologyRow) => row.hostName ?? named(names.get(row.hostId), row.hostId);
  const replaceable = ["native", "docker"].includes(deployment.mode) && permissions.has("deployments.apply") && !deploymentActive(deployment.state) && deployment.state !== "needs-review";

  return <section aria-labelledby={titleId} className="min-w-0 space-y-3">
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      <h3 id={titleId} className="font-semibold">{t("title")}</h3>
      {binding?.driver === "stack" && binding.manager && <span className="min-w-0 break-all text-sm text-muted-foreground">{t("manager", { host: named(names.get(binding.manager), binding.manager) })}</span>}
    </div>
    {operation && <p role="status" className="break-words text-sm text-muted-foreground">{f("accepted", { id: operation })}</p>}
    {!binding ? <p className="rounded-md border p-4 text-sm text-muted-foreground">{t("noBinding")}</p>
      : !groups.length ? <p className="rounded-md border p-4 text-sm text-muted-foreground">{t("empty")}</p>
      : <>
        {health.loading && <p role="status" className="text-sm text-muted-foreground">{t("healthLoading")}</p>}
        {!canHealth && <p className="text-sm text-muted-foreground">{t("healthUnavailable")}</p>}
        {health.error && <div role="alert" className="flex min-w-0 flex-wrap items-center gap-2 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">
          <span className="min-w-0 break-words">{t("healthFailed", { detail: health.error })}</span>
          <Button type="button" size="sm" variant="outline" className="min-h-11 lg:min-h-9" onClick={health.retry}>{commonText("common-domain:common.retry")}</Button>
        </div>}
        <div className="grid min-w-0 gap-3 xl:grid-cols-2">{groups.map(group => {
          // A replicated role counts Swarm replicas (healthy/expected); others count instances.
          const replicated = group.rows.length === 1 && group.rows[0]!.replicas !== undefined ? group.rows[0]! : undefined;
          const up = replicated ? replicated.healthy ?? 0 : group.rows.filter(row => row.state === "up").length;
          const total = replicated ? replicated.expected ?? replicated.replicas ?? 0 : group.rows.length;
          return <section key={group.role} data-topology-role={group.role} aria-label={t("labeled", { label: w("role"), value: group.role })} className="min-w-0 rounded-md border">
            <header className="flex min-w-0 flex-wrap items-center gap-2 border-b px-3 py-2 text-sm">
              <h4 className="min-w-0 break-all font-medium">{group.role}</h4>
              {group.identity && <Badge variant="outline">{w(group.identity === "stable" ? "identityStable" : "identityStateless")}</Badge>}
              <span className="text-muted-foreground">{t("instanceCount", { count: group.rows.reduce((sum, row) => sum + (row.replicas ?? 1), 0) })}</span>
              <span className="ms-auto text-muted-foreground tabular-nums">{t("summary", { up, total })}</span>
            </header>
            <div className="hidden grid-cols-[minmax(0,1fr)_minmax(0,1fr)_7rem] gap-3 px-3 pt-2 text-xs text-muted-foreground md:grid" aria-hidden>
              <span>{w("instance")}</span><span>{w("host")}</span><span>{t("health")}</span>
            </div>
            {/* Long role lists scroll inside a named region; the page never scrolls sideways. */}
            <div role="region" aria-label={t("instancesOf", { role: group.role })} tabIndex={0} className="max-h-96 overflow-y-auto">
              <ul className="divide-y">{group.rows.map(row => <li key={row.instance} data-topology-instance={row.instance}
                className="grid min-w-0 gap-1 px-3 py-2 text-sm md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_7rem] md:items-center md:gap-3">
                <span className="min-w-0">
                  <span className="break-all font-medium">{row.instance}</span>
                  {row.replicas !== undefined && <span className="block text-xs text-muted-foreground">{t("replicated", { count: row.replicas })}</span>}
                </span>
                <span className="min-w-0 break-all"><span className="me-1 text-muted-foreground md:sr-only">{t("label", { label: w("host") })}</span>{hostName(row)}</span>
                <span className="flex min-w-0 flex-wrap items-center gap-2"><span className="text-muted-foreground md:sr-only">{t("label", { label: t("health") })}</span><HealthBadge state={row.state} />
                  {deployment.failover?.fences.some(item => item.instance === row.instance && item.host_id === row.hostId) && <Badge variant="outline" className="max-w-full whitespace-normal break-words">{t("fenced")}</Badge>}
                  {deployment.failover?.attention.some(item => item.instance === row.instance) && <Badge variant="destructive" className="max-w-full whitespace-normal break-words">{t("needsAttention")}</Badge>}
                  {replaceable && blueprint?.roles.some(role => role.name === group.role && failoverClass(role) !== "manual") && instanceResource(workspace, deployment.id, row.instance) && <Button type="button" size="sm" variant="outline" className="min-h-11 lg:min-h-9" data-replace-instance={row.instance} onClick={() => setReplacing(row.instance)}>{f("replace")}</Button>}
                </span>
              </li>)}</ul>
            </div>
          </section>;
        })}</div>
      </>}
    {replacing && <DeleteConfirmDialog action={f("replace")} target={replacing} description={f("replaceHelp")} onClose={() => setReplacing(undefined)} onConfirm={async () => {
      const path = instanceResource(workspace, deployment.id, replacing);
      if (!path || !replaceable) return;
      const result = await api<{ operation_id: string }>(`${path}/replace`, { method: "POST", body: JSON.stringify({ version: deployment.version }) });
      setOperation(result.operation_id); onDone?.();
    }} />}
  </section>;
}

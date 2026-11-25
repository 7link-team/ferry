import { WorkspaceDate, formatDate, useWorkspaceTimezone } from "../../shared/timezone";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { useEffect,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { currentLocale, localizedPath } from "../../shared/i18n/locale";
import { Button } from "../../shared/ui/button";
import { InlineAlert } from "../../shared/ui/inline-alert";

interface Summary {
  counts: Record<string, number>;
  states: Record<string, Record<string, number>>;
  failures_24h: Record<string, number>;
  runners:
    | {
        id: string;
        name: string;
        seen_at: number | null;
        lag_ms: number | null;
      }[]
    | null;
  recent:
    | {
        id: string;
        actor_name: string;
        action: string;
        resource: string;
        created_at: number;
      }[]
    | null;
  as_of: number;
}
const labels: Record<string, string> = {
  "alert-rules": "shell.rules",
  "alert-events": "nav.alertEvents",
  "cron-jobs": "nav.cronJobs",
  "cron-runs": "dashboard.cronRuns",
  firing: "dashboard.firing",
  recovered: "dashboard.recovered",
  "alert-triggered": "dashboard.triggered",
  "alert-recovered": "dashboard.alertRecovered",
  "needs-review": "dashboard.review",
  skipped: "dashboard.skipped",
  hosts: "nav.hosts",
  "host-groups": "nav.hostGroups",
  tags: "nav.tags",
  credentials: "nav.credentials",
  repositories: "dashboard.repositories",
  pipelines: "shell.pipeline",
  deployments: "nav.deployments",
  members: "dashboard.members",
  roles: "dashboard.roles",
  "pipeline-runs": "dashboard.pipelineRuns",
  "deployment-revisions": "dashboard.revisions",
  online: "dashboard.online",
  offline: "dashboard.offline",
  unknown: "dashboard.unknown",
  pending: "dashboard.pending",
  queued: "dashboard.queued",
  running: "dashboard.running",
  succeeded: "dashboard.succeeded",
  failed: "dashboard.failed",
  cancelled: "dashboard.cancelled",
  idle: "dashboard.idle",
  create: "common.create",
  update: "dashboard.updatedAction",
  delete: "common.delete",
  "rotate-invitation": "dashboard.rotateInvitation",
  "invite-email": "invitation.email",
};
const label = (key: string) => labels[key] ? commonText(labels[key]) : key;

type Recent = { id: string; title: string; kind?: string; agent_name?: string; unread?: number; last_message?: { text: string; sender_name: string; created_at: number } | null };
/** Phones open on what people act on: their latest conversations. */
function RecentConversations({ workspace }: { workspace: string }) {
  const [rows, setRows] = useState<Recent[]>();
  useEffect(() => {
    const controller = new AbortController();
    void api<{ items: Recent[] }>(`/workspaces/${workspace}/conversations?page=1&page_size=5`, { signal: controller.signal })
      .then(result => setRows(result.items))
      .catch(() => { if (!controller.signal.aborted) setRows([]); });
    return () => controller.abort();
  }, [workspace]);
  if (!rows?.length) return null;
  return <section data-recent-conversations aria-label={commonText("dashboard.recentConversations")} className="rounded-xl border bg-card md:hidden">
    <h2 className="px-4 pt-3 text-sm font-medium">{commonText("dashboard.recentConversations")}</h2>
    <ul>{rows.map(row => <li key={row.id}>
      <a className="flex min-h-14 flex-col justify-center gap-0.5 px-4 py-2" href={localizedPath(`/workspaces/${encodeURIComponent(workspace)}/conversations/${row.id}`, currentLocale())}>
        <span className="flex min-w-0 items-baseline gap-2"><span className="min-w-0 flex-1 truncate font-medium">{row.title}</span>
          {!!row.unread && <span className="shrink-0 rounded-full bg-primary px-1.5 text-xs text-primary-foreground">{row.unread}</span>}</span>
        <span className="truncate text-xs text-muted-foreground">{row.last_message ? row.last_message.text : row.agent_name}</span>
      </a>
    </li>)}</ul>
  </section>;
}
export function Dashboard({ workspace }: { workspace: string }) {
  useCommonTranslation();
  const timezone = useWorkspaceTimezone();
  const [data, setData] = useState<Summary>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(true);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setBusy(true);
    setError("");
    void api<Summary>(`/workspaces/${workspace}/dashboard`, {
      signal: controller.signal,
    })
      .then(setData)
      .catch((cause) => {
        if (!controller.signal.aborted) setError(errorText(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setBusy(false);
      });
    return () => controller.abort();
  }, [workspace, attempt]);
  return (
    <div className="min-h-0 space-y-5 overflow-auto pb-6">
      <div className="flex items-center gap-3 max-md:hidden">
        <Button
          variant="outline"
          disabled={busy}
          onClick={() => setAttempt((n) => n + 1)}
        >
          {busy ? commonText("common.loading") : commonText("common.refresh")}
        </Button>
        {data && (
          <span className="text-xs text-muted-foreground">
            {commonText("dashboard.updated", { time: formatDate(data.as_of, timezone) })}
          </span>
        )}
      </div>
      {error && <InlineAlert>{error}</InlineAlert>}
      <RecentConversations workspace={workspace} />
      {!data && busy && !error && <LoadingSkeleton variant="page" />}
      {data && (
        <>
          {!Object.keys(data.counts).length && (
            <p className="text-sm text-muted-foreground">
              {commonText("dashboard.noPermission")}</p>
          )}
          <section
            aria-label={commonText("dashboard.overview")}
            className="grid grid-cols-3 gap-2 md:grid-cols-2 md:gap-3 lg:grid-cols-4"
          >
            {Object.entries(data.counts).map(([key, count]) => (
              // Phones skip empty counters; they say nothing at a glance.
              <div key={key} className={`rounded-md border p-2.5 md:p-4 ${count ? "" : "max-md:hidden"}`}>
                <h2 className="truncate text-xs text-muted-foreground md:text-sm">{label(key)}</h2>
                <p className="mt-1 text-xl font-semibold md:mt-2 md:text-2xl">{count}</p>
              </div>
            ))}
          </section>
          <div className="grid gap-4 lg:grid-cols-2">
            {Object.entries(data.states).map(([key, states]) => (
              <section key={key} className={`rounded-md border p-4 ${Object.keys(states).length ? "" : "max-md:hidden"}`}>
                <h2 className="mb-3 font-medium">{commonText("dashboard.states", { name: label(key) })}</h2>
                {Object.keys(states).length ? (
                  <dl className="flex flex-wrap gap-5">
                    {Object.entries(states).map(([state, count]) => (
                      <div key={state}>
                        <dt className="text-sm text-muted-foreground">
                          {label(state)}
                        </dt>
                        <dd className="text-xl">{count}</dd>
                      </div>
                    ))}
                  </dl>
                ) : (
                  <p className="text-sm text-muted-foreground">{commonText("common.noRecords")}</p>
                )}
              </section>
            ))}
          </div>
          {!!Object.keys(data.failures_24h).length && (
            <section className="rounded-md border p-4">
              <h2 className="mb-3 font-medium">{commonText("dashboard.failures")}</h2>
              <dl className="flex gap-6">
                {Object.entries(data.failures_24h).map(([key, value]) => (
                  <div key={key}>
                    <dt className="text-sm text-muted-foreground">
                      {label(key)}
                    </dt>
                    <dd className="text-xl">{value}</dd>
                  </div>
                ))}
              </dl>
            </section>
          )}
          {data.runners && (
            <section className="rounded-md border p-4">
              <h2 className="mb-3 font-medium">
                {commonText("dashboard.heartbeat")}</h2>
              {data.runners.length ? (
                <ul className="space-y-2">
                  {data.runners.map((r) => (
                    <li
                      key={r.id}
                      className="flex justify-between gap-4 text-sm"
                    >
                      <span>{r.name}</span>
                      <span className="text-muted-foreground">
                        {r.lag_ms === null
                          ? commonText("dashboard.unreported")
                          : commonText("dashboard.secondsAgo", { count: Math.floor(r.lag_ms / 1000) })}
                      </span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-muted-foreground">
                  {commonText("dashboard.noRunners")}</p>
              )}
            </section>
          )}
          {data.recent && (
            <section className="rounded-md border p-4">
              <h2 className="mb-3 font-medium">{commonText("dashboard.recent")}</h2>
              {data.recent.length ? (
                <ul className="space-y-2">
                  {data.recent.map((a) => (
                    <li
                      key={a.id}
                      className="flex flex-wrap justify-between gap-2 text-sm"
                    >
                      <span>
                        {a.actor_name} · {label(a.action)} · {label(a.resource)}
                      </span>
                      <time className="text-muted-foreground">
                        {<WorkspaceDate value={a.created_at} locale={currentLocale()} />}
                      </time>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-muted-foreground">{commonText("dashboard.noActivity")}</p>
              )}
            </section>
          )}
        </>
      )}
    </div>
  );
}

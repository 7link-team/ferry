import { currentLocale } from "../locale";
import { commonText, useCommonTranslation } from "../i18n";
import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Button } from "@/components/ui/button";

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
};
const label = (key: string) => labels[key] ? commonText(labels[key]) : key;
export function Dashboard({ workspace }: { workspace: string }) {
  useCommonTranslation();
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
      <div className="flex items-center gap-3">
        <Button
          variant="outline"
          disabled={busy}
          onClick={() => setAttempt((n) => n + 1)}
        >
          {busy ? commonText("common.loading") : commonText("common.refresh")}
        </Button>
        {data && (
          <span className="text-xs text-muted-foreground">
            {commonText("dashboard.updated", { time: new Date(data.as_of).toLocaleString(currentLocale()) })}
          </span>
        )}
      </div>
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
      {data && (
        <>
          {!Object.keys(data.counts).length && (
            <p className="text-sm text-muted-foreground">
              {commonText("dashboard.noPermission")}</p>
          )}
          <section
            aria-label={commonText("dashboard.overview")}
            className="grid grid-cols-2 gap-3 lg:grid-cols-4"
          >
            {Object.entries(data.counts).map(([key, count]) => (
              <div key={key} className="rounded-md border p-4">
                <h2 className="text-sm text-muted-foreground">{label(key)}</h2>
                <p className="mt-2 text-2xl font-semibold">{count}</p>
              </div>
            ))}
          </section>
          <div className="grid gap-4 lg:grid-cols-2">
            {Object.entries(data.states).map(([key, states]) => (
              <section key={key} className="rounded-md border p-4">
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
                        {new Date(a.created_at).toLocaleString(currentLocale())}
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

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Tooltip } from "radix-ui";
import { AnalysisTooltip } from "../../features/code-insights/AnalysisTooltip";
import { api, errorText } from "../../shared/api/api";
import { base, type Envelope } from "../../shared/api/resource";
import type { ListConfig } from "../../shared/resources/types";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "../../shared/ui/tabs";
import { Button } from "../../shared/ui/button";
import { ResourceForm, ResourceActionDialog } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { href, navigate, useQueryValue } from "../router";
import {
  reportForm,
  reportActions,
  date,
  options,
  periods,
} from "../../features/insight-reports/config";
import { text, key, statusText } from "../../features/insight-reports/messages";
import type { Row } from "../../features/insight-reports/ReportScope";
import { reportTrend } from "../../features/insight-reports/trend";

export function EvidenceDetail({
  workspace,
  id,
  permissions,
}: {
  workspace: string;
  id: string;
  permissions: Set<string>;
}) {
  useTranslation("reports-reviews-domain");
  const resource = "insight-reports";
  const endpoint = `${base(workspace, resource)}/${encodeURIComponent(id)}`;
  const [tab, setTab] = useQueryValue("tab", "overview");
  const [runId, setRunId] = useQueryValue("run", "");
  const [definition, setDefinition] = useState<Row>();
  const [actionOpen, setActionOpen] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [selected, setSelected] = useState<Row>();
  const [error, setError] = useState("");
  useEffect(() => {
    let live = true;
    void api<Row>(endpoint)
      .then((data) => {
        if (live) setDefinition(data);
      })
      .catch((cause) => {
        if (live) setError(errorText(cause));
      });
    return () => {
      live = false;
    };
  }, [endpoint, refresh]);
  const form = useMemo(
    () => reportForm(workspace, { href, navigate }),
    [workspace],
  );
  useEffect(() => {
    let live = true;
    setSelected(undefined);
    setError("");
    if (runId)
      void api<Row>(`${endpoint}/runs/${encodeURIComponent(runId)}`)
        .then((row) => {
          if (live) setSelected(row);
        })
        .catch((cause) => {
          if (live) setError(errorText(cause));
        });
    return () => {
      live = false;
    };
  }, [endpoint, runId, refresh]);
  useEffect(() => {
    if (
      !runId ||
      (selected &&
        ![
          "queued",
          "running",
          "pending",
          "launching",
          "waiting",
          "cancelling",
        ].includes(String(selected.state)))
    )
      return;
    let live = true;
    const timer = window.setInterval(() => {
      void api<Row>(`${endpoint}/runs/${encodeURIComponent(runId)}`)
        .then((row) => {
          if (live) {
            setSelected(row);
            if (
              ![
                "queued",
                "running",
                "pending",
                "launching",
                "waiting",
                "cancelling",
              ].includes(String(row.state))
            )
              void api<Row>(endpoint)
                .then((data) => {
                  if (live) setDefinition(data);
                })
                .catch((cause) => {
                  if (live) setError(errorText(cause));
                });
          }
        })
        .catch((cause) => {
          if (live) setError(errorText(cause));
        });
    }, 3000);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [endpoint, runId, selected]);
  const runs = useMemo<ListConfig<Row>>(
    () => ({
      resource: `${resource}/${id}/runs`,
      endpoint: `${endpoint}/runs`,
      permissionResource: resource,
      queryPrefix: "runs",
      rowKey: (row) => String(row.id),
      defaultRefresh: 10,
      ...{
        filters: [
          {
            key: "period",
            label: "period",
            labelKey: key("period"),
            type: "select" as const,
            options: options(periods),
          },
          {
            key: "period_start",
            label: "periodStart",
            labelKey: key("periodStart"),
            type: "date" as const,
          },
        ],
      },
      columns: [
        {
          key: "created_at",
          label: "history",
          labelKey: key("history"),
          render: (row) => (
            <Button
              type="button"
              variant="link"
              onClick={() => {
                setRunId(String(row.id));
                setTab("overview");
              }}
            >
              {date(row.created_at)}
            </Button>
          ),
        },

        {
          key: "period",
          label: "period",
          labelKey: key("period"),
          render: (row: Row) => text(String(row.period)),
        },
        {
          key: "period_start",
          label: "periodStart",
          labelKey: key("periodStart"),
          render: (row: Row) => date(row.period_start),
        },
        {
          key: "definition_version",
          label: "version",
          labelKey: key("version"),
        },

        {
          key: "state",
          label: "state",
          labelKey: key("state"),
          render: (row) => statusText(row.state),
        },
        { key: "error", label: "error", labelKey: key("error") },
      ],
      toolbar: { create: false, import: false, export: false },
      empty: "empty",
      emptyKey: key("empty"),
    }),
    [resource, id, endpoint, setRunId, setTab],
  );
  const operation = reportActions(workspace)[0]!;
  const reportOverview = useMemo<ListConfig<Row>>(
    () => ({
      ...runs,
      summary: (data) => (
        <ReportSummary
          items={data.items}
          runId={runId}
          selected={selected}
          workspace={workspace}
          endpoint={endpoint}
          onChoose={setRunId}
        />
      ),
    }),
    [runs, runId, selected, workspace, endpoint, setRunId],
  );
  return (
    <Tabs
      value={
        ["overview", "history", "configuration"].includes(tab)
          ? tab
          : "overview"
      }
      onValueChange={setTab}
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="min-w-0 break-words text-lg font-semibold">
          {String(definition?.name ?? text("reports"))}
        </h2>
        {definition && permissions.has(`${resource}.generate`) && (
          <Button type="button" onClick={() => setActionOpen(true)}>
            {text("generate")}
          </Button>
        )}
      </div>
      {definition?.trigger_error ? (
        <p role="alert" className="break-words text-destructive">
          {text("trigger_error")}: {String(definition.trigger_error)}
        </p>
      ) : null}
      {actionOpen && definition && (
        <ResourceActionDialog
          action={operation}
          row={definition}
          workspace={workspace}
          permissions={permissions}
          onClose={() => setActionOpen(false)}
          onDone={() => {
            setActionOpen(false);
            setRefresh((value) => value + 1);
            setRunId("");
            void api<Envelope<Row>>(`${endpoint}/runs?page=1&page_size=1`)
              .then((data) => {
                if (data.items[0]) setRunId(String(data.items[0].id));
              })
              .catch((cause) => setError(errorText(cause)));
          }}
        />
      )}
      <div className="min-w-0 overflow-x-auto border-b pb-2">
        <TabsList aria-label={text("reports")}>
          {["overview", "history", "configuration"].map((name) => (
            <TabsTrigger key={name} value={name}>
              {text(name)}
            </TabsTrigger>
          ))}
        </TabsList>
      </div>
      <TabsContent value="configuration" className="min-h-0 overflow-auto">
        <ResourceForm
          config={form}
          workspace={workspace}
          permissions={permissions}
        />
      </TabsContent>
      <TabsContent value="history" className="flex min-h-0 flex-1 flex-col">
        <ResourceTable
          config={runs}
          workspace={workspace}
          permissions={permissions}
          refreshKey={refresh}
        />
      </TabsContent>
      <TabsContent
        value="overview"
        className="flex min-h-0 min-w-0 flex-1 flex-col gap-4 overflow-auto"
      >
        {error && (
          <p role="alert" className="text-destructive">
            {error}
          </p>
        )}

        <ResourceTable
          config={reportOverview}
          workspace={workspace}
          permissions={permissions}
          refreshKey={refresh}
        />
      </TabsContent>
    </Tabs>
  );
}

export function ReportSummary({
  items,
  runId,
  selected,
  endpoint,
  onChoose,
}: {
  items: Row[];
  runId: string;
  selected: Row | undefined;
  workspace: string;
  endpoint: string;
  onChoose: (id: string) => void;
}) {
  const visible = items.find((row) => String(row.id) === runId);
  useEffect(() => {
    if (!visible) {
      const next = String(items[0]?.id ?? "");
      if (next !== runId) onChoose(next);
    }
  }, [items, runId, visible, onChoose]);
  if (!items.length)
    return <p className="text-sm text-muted-foreground">{text("empty")}</p>;
  if (!visible || !selected || String(selected.id) !== String(visible.id))
    return <p role="status">{text("loading")}</p>;
  return (
    <div className="min-w-0 space-y-4">
      <RunEvidence row={selected} />
      {selected.state === "complete" && (
        <div className="flex flex-wrap gap-4">
          {["html", "csv", "json"].map((format) => (
            <a
              key={format}
              className="text-sm text-primary underline"
              href={`/api${endpoint}/runs/${encodeURIComponent(String(selected.id))}/export?format=${format}`}
              download
            >
              {text("exportReport")} · {format.toUpperCase()}
            </a>
          ))}
        </div>
      )}
    </div>
  );
}

function RunEvidence({ row }: { row: Row }) {
  const snapshot = (row.snapshot ?? {}) as Row;
  const coverage = snapshot.coverage as Row | undefined;
  const summary = snapshot.summary as Row | undefined;
  const previous = snapshot.previous as Row | undefined;
  const trend = reportTrend(snapshot);
  const max = Math.max(1, ...trend.map((point) => Number(point.commits) || 0));
  const facts = [
    "period",
    "definition_version",
    "period_start",
    "period_end",
    "generated_at",
  ];
  return (
    <section className="min-w-0 space-y-5 rounded-md border p-4">
      <dl className="grid min-w-0 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {facts.map((name) => (
          <div key={name} className="min-w-0">
            <dt className="text-sm text-muted-foreground">
              {text(
                (
                  {
                    test_state: "tests_enabled",
                    ai_state: "ai_enabled",
                    definition_version: "version",
                    period_start: "periodStartRead",
                    period_end: "periodEnd",
                    generated_at: "last",
                  } as Record<string, string>
                )[name] ?? name,
              )}
            </dt>
            <dd className="mt-1 break-all">
              {name.endsWith("_at") || name.startsWith("period_")
                ? date(row[name])
                : ["state", "test_state", "ai_state", "period"].includes(name)
                  ? name === "period"
                    ? text(String(row[name]))
                    : statusText(
                        row[name],
                        name === "test_state"
                          ? "test"
                          : name === "ai_state"
                            ? "ai"
                            : undefined,
                      )
                  : String(row[name] ?? "—")}
            </dd>
          </div>
        ))}
      </dl>
      {
        <p className="text-sm text-muted-foreground">
          {text("evidence")}
          {snapshot.preview ? ` ${text("preview")}` : ""}
        </p>
      }
      {summary && (
        <dl className="grid gap-4 sm:grid-cols-3">
          {Object.entries(summary)
            .filter(([, value]) => typeof value === "number")
            .map(([name, value]) => (
              <div key={name}>
                <dt className="text-sm text-muted-foreground">
                  {name === "failed" ? text("testsFailed") : text(name)}
                </dt>
                <dd className="text-xl font-semibold tabular-nums">
                  {String(value)}
                  {previous && (
                    <small className="ml-2 text-xs font-normal text-muted-foreground">
                      {text("comparison")}: {String(previous[name] ?? "—")}
                    </small>
                  )}
                </dd>
              </div>
            ))}
        </dl>
      )}
      {trend.length > 0 && (
        <div>
          <h3 className="mb-3 font-medium">{text("trend")}</h3>
          <div className="flex min-w-0 gap-2" aria-label={text("commits")}>
            <div className="relative mt-3 h-36 w-8 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
              {[max, max / 2, 0].map((tick, index) => (
                <span
                  key={index}
                  className="absolute right-0 -translate-y-1/2"
                  style={{ top: `${index * 50}%` }}
                >
                  {tick.toLocaleString(undefined, { maximumFractionDigits: 1 })}
                </span>
              ))}
            </div>
            <Tooltip.Provider delayDuration={150}>
              <ol
                className="flex h-44 min-w-0 flex-1 items-end gap-1 overflow-x-auto border-l border-muted-foreground/30"
                aria-label={text("trend")}
              >
                {trend.map((point, index) => (
                  <li
                    key={index}
                    className="flex h-full min-w-12 flex-1 flex-col justify-end text-center text-xs"
                  >
                    <AnalysisTooltip date={String(point.date)} row={point}>
                      <button
                        type="button"
                        aria-label={`${point.date}: ${text("commits")} ${point.commits}`}
                        className="relative flex h-36 w-full shrink-0 items-end justify-center border-b border-muted-foreground/30 outline-offset-2"
                      >
                        <span
                          className="block w-5 bg-primary"
                          style={{
                            height: `${Math.max(1, (Number(point.commits) / max) * 144)}px`,
                          }}
                        />
                      </button>
                    </AnalysisTooltip>
                    <span className="mt-1 truncate">
                      {String(point.date).slice(5, 10)}
                    </span>
                  </li>
                ))}
              </ol>
            </Tooltip.Provider>
          </div>
        </div>
      )}
      {coverage && (
        <div className="space-y-2">
          <h3 className="font-medium">
            {text("coverage")}:{" "}
            {text(coverage.complete ? "complete" : "incomplete")}
          </h3>
          {
            <ul className="space-y-1 text-sm">
              {((coverage.sources ?? []) as Row[]).map((source, index) => (
                <li key={index} className="break-all">
                  {String(source.repository)} · {statusText(source.state)} ·{" "}
                  {date(source.synced_at)}
                </li>
              ))}
              {((coverage.missing_repositories ?? []) as unknown[]).map(
                (item, index) => (
                  <li key={`missing${index}`} className="text-destructive">
                    {text("incomplete")}:{" "}
                    {typeof item === "string"
                      ? item
                      : String((item as Row)?.repository ?? "—")}
                  </li>
                ),
              )}
            </ul>
          }
        </div>
      )}
      {
        <>
          <ul className="space-y-2">
            {((snapshot.groups ?? []) as Row[]).map((group, index) => (
              <li
                key={index}
                className="flex flex-wrap gap-4 border-b py-2 text-sm"
              >
                <strong>{String(group.name)}</strong>
                {[
                  "commits",
                  "additions",
                  "deletions",
                  "effectives",
                  "active_days",
                  "repository_count",
                  "contributor_count",
                ]
                  .filter((name) => typeof group[name] === "number")
                  .map((name) => (
                    <span key={name}>
                      {text(name)}: {String(group[name])}
                    </span>
                  ))}
              </li>
            ))}
          </ul>
          <h3 className="font-medium">{text("contributions")}</h3>
          <ol className="space-y-3">
            {((snapshot.commits ?? []) as Row[]).map((commit, index) => (
              <li key={index} className="min-w-0 border-b pb-3 text-sm">
                <p className="font-medium">
                  {String(commit.nickname || commit.author_name)} ·{" "}
                  {String(commit.repository_id)}
                </p>
                <p className="break-all font-mono text-xs">
                  {String(commit.sha)} · {date(commit.author_date)}
                </p>
                <p className="mt-1 whitespace-pre-wrap break-words">
                  {String(commit.message)}
                </p>
              </li>
            ))}
          </ol>
          <p className="text-xs text-muted-foreground">
            {text("snapshot")}: {text("version")}{" "}
            {String(row.definition_version)} · {text("identity_version")}{" "}
            {String(snapshot.identity_version ?? "—")} ·{" "}
            {text("repository_scope_version")}{" "}
            {String(snapshot.repository_scope_version ?? "—")} ·{" "}
            {String(snapshot.timezone ?? "—")}
          </p>
        </>
      }
      {snapshot.config ? (
        <ConfigurationSnapshot config={snapshot.config as Row} />
      ) : null}
    </section>
  );
}

function ConfigurationSnapshot({ config }: { config: Row }) {
  const scalarFields = ["name", "enabled", "dimension", "timezone"];
  return (
    <details>
      <summary className="cursor-pointer font-medium">
        {text("snapshot")}
      </summary>
      <dl className="mt-3 grid min-w-0 gap-3 sm:grid-cols-2">
        {scalarFields
          .filter((name) => config[name] !== undefined)
          .map((name) => (
            <div key={name} className="min-w-0">
              <dt className="text-sm text-muted-foreground">{text(name)}</dt>
              <dd className="whitespace-pre-wrap break-all text-sm">
                {typeof config[name] === "boolean"
                  ? text(config[name] ? "enabled" : "not_enabled")
                  : ["dimension"].includes(name)
                    ? text(String(config[name]))
                    : String(config[name] ?? "—")}
              </dd>
            </div>
          ))}
        {["periods", "author_keys"]
          .filter(
            (name) =>
              Array.isArray(config[name]) && (config[name] as unknown[]).length,
          )
          .map((name) => (
            <div key={name}>
              <dt className="text-sm text-muted-foreground">
                {text(name === "author_keys" ? "people" : name)}
              </dt>
              <dd className="break-all text-sm">
                {(config[name] as string[])
                  .map((value) => (name === "periods" ? text(value) : value))
                  .join(" · ")}
              </dd>
            </div>
          ))}
      </dl>
      {[
        ...((config.repositories ?? []) as Row[]),
        ...((config.applications ?? []) as Row[]).flatMap(
          (app) => (app.repositories ?? []) as Row[],
        ),
      ].map((repo, index) => (
        <p key={index} className="mt-2 break-all text-sm">
          {String(repo.repository)}
        </p>
      ))}
    </details>
  );
}

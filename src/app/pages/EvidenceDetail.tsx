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
  reviewForm,
  reviewActions,
  reviewCancelAction,
} from "../../features/code-reviews/config";
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
  review = false,
}: {
  workspace: string;
  id: string;
  permissions: Set<string>;
  review?: boolean;
}) {
  useTranslation("reports-reviews-domain");
  const resource = review ? "code-reviews" : "insight-reports";
  const endpoint = `${base(workspace, resource)}/${encodeURIComponent(id)}`;
  const [tab, setTab] = useQueryValue("tab", "overview");
  const [runId, setRunId] = useQueryValue("run", "");
  const [definition, setDefinition] = useState<Row>();
  const [actionOpen, setActionOpen] = useState(false);
  const [cancelRun, setCancelRun] = useState<Row>();
  const cancelAction = useMemo(
    () => reviewCancelAction(workspace, id),
    [workspace, id],
  );
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
    () =>
      review
        ? reviewForm(workspace, { href, navigate })
        : reportForm(workspace, { href, navigate }),
    [review, workspace],
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
  useEffect(() => {
    if (!review || runId) return;
    let live = true;
    void api<Envelope<Row>>(`${endpoint}/runs?page=1&page_size=1`)
      .then((data) => {
        if (live && data.items[0]) setRunId(String(data.items[0].id));
      })
      .catch((cause) => {
        if (live) setError(errorText(cause));
      });
    return () => {
      live = false;
    };
  }, [endpoint, runId, setRunId, refresh, review]);
  const runs = useMemo<ListConfig<Row>>(
    () => ({
      resource: `${resource}/${id}/runs`,
      endpoint: `${endpoint}/runs`,
      permissionResource: resource,
      queryPrefix: "runs",
      rowKey: (row) => String(row.id),
      defaultRefresh: 10,
      ...(!review
        ? {
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
          }
        : {}),
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
        ...(review
          ? [
              {
                key: "repository",
                label: "repositories",
                labelKey: key("repositories"),
              },
              {
                key: "head_sha",
                label: "head_sha",
                labelKey: key("head_sha"),
                render: (row: Row) => (
                  <span className="break-all font-mono">
                    {String(row.head_sha ?? "—")}
                  </span>
                ),
              },
              {
                key: "test_state",
                label: "tests_enabled",
                labelKey: key("tests_enabled"),
                render: (row: Row) => statusText(row.test_state, "test"),
              },
              {
                key: "ai_state",
                label: "ai_enabled",
                labelKey: key("ai_enabled"),
                render: (row: Row) => statusText(row.ai_state, "ai"),
              },
            ]
          : [
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
            ]),
        {
          key: "state",
          label: "state",
          labelKey: key("state"),
          render: (row) => statusText(row.state),
        },
        { key: "error", label: "error", labelKey: key("error") },
        ...(review && permissions.has("code-reviews.run")
          ? [
              {
                key: "cancel",
                label: "cancelRun",
                labelKey: key("cancelRun"),
                render: (row: Row) =>
                  cancelAction.when?.(row) ? (
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => setCancelRun(row)}
                    >
                      {text("cancelRun")}
                    </Button>
                  ) : null,
              },
            ]
          : []),
      ],
      toolbar: { create: false, import: false, export: false },
      empty: "empty",
      emptyKey: key("empty"),
    }),
    [
      resource,
      id,
      endpoint,
      setRunId,
      setTab,
      review,
      permissions,
      cancelAction,
    ],
  );
  const operation = (
    review ? reviewActions(workspace) : reportActions(workspace)
  )[0]!;
  const findings = useMemo<ListConfig<Row>>(
    () => ({
      resource: `code-reviews/${id}/findings`,
      endpoint: `${endpoint}/findings`,
      permissionResource: resource,
      queryPrefix: "findings",
      rowKey: (row) => String(row.id),
      filters: [
        {
          key: "status",
          label: "state",
          labelKey: key("state"),
          type: "select",
          options: options(["open", "fixed", "ignored", "false_positive"]),
        },
      ],
      columns: [
        { key: "severity", label: "severity", labelKey: key("severity") },
        {
          key: "repository",
          label: "repositories",
          labelKey: key("repositories"),
        },
        {
          key: "file",
          label: "file",
          labelKey: key("file"),
          render: (row) => (
            <span className="break-all">
              {String(row.file ?? "—")}
              {Number(row.line) > 0 ? `:${row.line}` : ""}
            </span>
          ),
        },
        {
          key: "message",
          label: "message",
          labelKey: key("message"),
          render: (row) => (
            <div className="max-w-xl whitespace-pre-wrap break-words">
              <p>{String(row.message ?? "")}</p>
              <p className="mt-2 text-muted-foreground">
                {text("suggestion")}: {String(row.suggestion ?? "—")}
              </p>
              <p className="mt-2">{String(row.evidence ?? "")}</p>
            </div>
          ),
        },
        {
          key: "status",
          label: "state",
          labelKey: key("state"),
          render: (row) => text(String(row.status)),
        },
        {
          key: "last_seen",
          label: "last",
          labelKey: key("last"),
          render: (row) => date(row.last_seen),
        },
      ],
      actions: ["open", "fixed", "ignored", "false_positive"].map((status) => ({
        label: status,
        labelKey: key(status),
        permission: "code-reviews.update",
        when: (row) => row.status !== status,
        action: async (row) => {
          await api(
            `${endpoint}/findings/${encodeURIComponent(String(row.id))}`,
            {
              method: "PATCH",
              body: JSON.stringify({ status, version: row.version }),
            },
          );
        },
      })),
      toolbar: { create: false, import: false, export: false },
    }),
    [id, endpoint, resource],
  );
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
        [
          "overview",
          "history",
          "configuration",
          ...(review ? ["findings"] : []),
        ].includes(tab)
          ? tab
          : "overview"
      }
      onValueChange={setTab}
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="min-w-0 break-words text-lg font-semibold">
          {String(definition?.name ?? text(review ? "reviews" : "reports"))}
        </h2>
        {definition &&
          permissions.has(`${resource}.${review ? "run" : "generate"}`) && (
            <Button type="button" onClick={() => setActionOpen(true)}>
              {text(review ? "run" : "generate")}
            </Button>
          )}
      </div>
      {definition?.trigger_error ? (
        <p role="alert" className="break-words text-destructive">
          {text("trigger_error")}: {String(definition.trigger_error)}
        </p>
      ) : null}
      {cancelRun && (
        <ResourceActionDialog
          action={cancelAction}
          row={cancelRun}
          workspace={workspace}
          permissions={permissions}
          onClose={() => setCancelRun(undefined)}
          onDone={() => {
            setCancelRun(undefined);
            setRefresh((value) => value + 1);
          }}
        />
      )}
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
            if (!review)
              void api<Envelope<Row>>(`${endpoint}/runs?page=1&page_size=1`)
                .then((data) => {
                  if (data.items[0]) setRunId(String(data.items[0].id));
                })
                .catch((cause) => setError(errorText(cause)));
          }}
        />
      )}
      <div className="min-w-0 overflow-x-auto border-b pb-2">
        <TabsList aria-label={text(review ? "reviews" : "reports")}>
          {[
            "overview",
            "history",
            ...(review ? ["findings"] : []),
            "configuration",
          ].map((name) => (
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
      {review && (
        <TabsContent value="findings" className="flex min-h-0 flex-1 flex-col">
          <ResourceTable
            config={findings}
            workspace={workspace}
            permissions={permissions}
          />
        </TabsContent>
      )}
      <TabsContent
        value="overview"
        className="flex min-h-0 min-w-0 flex-1 flex-col gap-4 overflow-auto"
      >
        {error && (
          <p role="alert" className="text-destructive">
            {error}
          </p>
        )}
        {!review ? (
          <ResourceTable
            config={reportOverview}
            workspace={workspace}
            permissions={permissions}
            refreshKey={refresh}
          />
        ) : (
          <>
            {runId && !selected && !error && (
              <p role="status">{text("loading")}</p>
            )}
            {selected ? (
              <>
                <RunEvidence
                  row={selected}
                  review={review}
                  workspace={workspace}
                />
                {permissions.has("code-reviews.run") &&
                  cancelAction.when?.(selected) && (
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => setCancelRun(selected)}
                    >
                      {text("cancelRun")}
                    </Button>
                  )}
                {!review && selected.state === "complete" && (
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
              </>
            ) : (
              !runId && (
                <p className="text-sm text-muted-foreground">
                  {text("history")}
                </p>
              )
            )}
            <div className="flex min-h-80 min-w-0 flex-1 flex-col">
              <ResourceTable
                config={runs}
                workspace={workspace}
                permissions={permissions}
                refreshKey={refresh}
              />
            </div>
          </>
        )}
      </TabsContent>
    </Tabs>
  );
}

export function ReportSummary({
  items,
  runId,
  selected,
  workspace,
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
      <RunEvidence row={selected} review={false} workspace={workspace} />
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

function RunEvidence({
  row,
  review,
  workspace,
}: {
  row: Row;
  review: boolean;
  workspace: string;
}) {
  const snapshot = (row.snapshot ?? {}) as Row;
  const coverage = (review ? row.coverage : snapshot.coverage) as
    Row | undefined;
  const summary = (review ? row.test_summary : snapshot.summary) as
    Row | undefined;
  const previous = snapshot.previous as Row | undefined;
  const trend = reportTrend(snapshot);
  const max = Math.max(1, ...trend.map((point) => Number(point.commits) || 0));
  const facts = review
    ? ["state", "test_state", "ai_state", "base_sha", "head_sha", "error"]
    : [
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
      {!review && (
        <p className="text-sm text-muted-foreground">
          {text("evidence")}
          {snapshot.preview ? ` ${text("preview")}` : ""}
        </p>
      )}
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
      {!review && snapshot.review_evidence ? (
        <ReviewEvidence
          value={snapshot.review_evidence as Row}
          workspace={workspace}
        />
      ) : null}
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
            {!review && text(coverage.complete ? "complete" : "incomplete")}
          </h3>
          {review ? (
            ["scanned", "skipped"].map((name) => (
              <div key={name}>
                <h4 className="text-sm font-medium">{text(name)}</h4>
                <ul className="space-y-1 text-sm">
                  {((coverage[name] ?? []) as unknown[]).map((item, index) => (
                    <li key={index} className="break-all">
                      {typeof item === "string"
                        ? item
                        : Object.entries(item as Row)
                            .map(([k, v]) => `${k}: ${String(v)}`)
                            .join(" · ")}
                    </li>
                  ))}
                </ul>
              </div>
            ))
          ) : (
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
          )}
        </div>
      )}
      {!review && (
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
                {group.review_evidence ? (
                  <ReviewEvidence
                    value={group.review_evidence as Row}
                    workspace={workspace}
                  />
                ) : null}
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
      )}
      {review && (
        <>
          {["test_error", "ai_error"].map((name) =>
            row[name] ? (
              <p
                key={name}
                role="alert"
                className="break-words text-destructive"
              >
                {text(name === "test_error" ? "tests_enabled" : "ai_enabled")}:{" "}
                {String(row[name])}
              </p>
            ) : null,
          )}
        </>
      )}
      {(review ? row.config_snapshot : snapshot.config) ? (
        <ConfigurationSnapshot
          config={(review ? row.config_snapshot : snapshot.config) as Row}
        />
      ) : null}
      {review && row.pipeline_run_id ? (
        <a
          className="text-primary underline"
          href={href({
            workspace,
            group: "build",
            item: "runs",
            id: String(row.pipeline_run_id),
            mode: "view",
          })}
        >
          {text("logs")}
        </a>
      ) : null}
    </section>
  );
}

export function ReviewEvidence({
  value,
  workspace,
}: {
  value: Row;
  workspace: string;
}) {
  const runs = (Array.isArray(value.runs) ? value.runs : []) as Row[];
  return (
    <section
      className="w-full min-w-0 space-y-3 border-t pt-3"
      aria-label={text("reviewEvidence")}
    >
      <h4 className="font-medium">{text("reviewEvidence")}</h4>
      {value.available !== true ? (
        <p className="text-sm text-muted-foreground">
          {text("unavailable")} · {text("evidence.permission")}
        </p>
      ) : (
        <>
          {value.total === 0 ? (
            <p className="text-sm text-muted-foreground">
              {text("noReviewEvidence")}
            </p>
          ) : (
            <dl className="flex flex-wrap gap-x-6 gap-y-2 text-sm">
              {["total", "tests_passed", "tests_failed", "findings"]
                .filter((name) => typeof value[name] === "number")
                .map((name) => (
                  <div key={name}>
                    <dt className="text-muted-foreground">
                      {text(`evidence.${name}`)}
                    </dt>
                    <dd className="font-medium tabular-nums">
                      {String(value[name])}
                    </dd>
                  </div>
                ))}
            </dl>
          )}
          <ul className="space-y-3 text-sm">
            {runs.map((run, index) => (
              <li
                key={String(run.id ?? index)}
                className="min-w-0 space-y-1 border-l pl-3"
              >
                <div className="flex flex-wrap gap-x-4 gap-y-1">
                  <a
                    className="text-primary underline"
                    href={href(
                      {
                        workspace,
                        group: "build",
                        item: "code-reviews",
                        id: String(run.review_id),
                        mode: "view",
                      },
                      { tab: "overview", run: String(run.id) },
                    )}
                  >
                    {text("reviews")} · {String(run.repository)}
                  </a>
                  <span>{statusText(run.state)}</span>
                  {run.pipeline_run_id &&
                  String(run.pipeline_run_id) !== "0" ? (
                    <a
                      className="text-primary underline"
                      href={href({
                        workspace,
                        group: "build",
                        item: "runs",
                        id: String(run.pipeline_run_id),
                        mode: "view",
                      })}
                    >
                      {text("logs")}
                    </a>
                  ) : null}
                </div>
                <p className="break-all font-mono text-xs">
                  {String(run.head_sha ?? "—")}
                </p>
                <p>
                  {text("tests_enabled")}: {statusText(run.test_state, "test")}{" "}
                  · {text("ai_enabled")}: {statusText(run.ai_state, "ai")}
                  {["completed_clear", "completed_findings"].includes(
                    String(run.ai_state),
                  ) && typeof run.finding_count === "number"
                    ? ` · ${text("evidence.findings")}: ${run.finding_count}`
                    : ""}
                </p>
              </li>
            ))}
          </ul>
          {((value.unavailable_repositories ?? []) as Row[]).map(
            (repo, index) => (
              <p key={index} className="break-all text-sm text-destructive">
                {String(repo.repository)} ·{" "}
                {text(
                  `evidence.${repo.reason === "credential" ? "credential" : "authorization"}`,
                )}
              </p>
            ),
          )}
        </>
      )}
    </section>
  );
}

function ConfigurationSnapshot({ config }: { config: Row }) {
  const scalarFields = [
    "name",
    "enabled",
    "trigger",
    "branch",
    "dimension",
    "timezone",
    "tests_enabled",
    "ai_enabled",
    "test_command",
    "junit_path",
    "runner_kind",
    "runner_ref",
    "ai_endpoint",
    "ai_model",
    "ai_credential_id",
    "ai_timeout_seconds",
    "ai_concurrency",
    "ai_max_tokens",
    "max_files",
    "max_bytes",
  ];
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
                  : ["trigger", "dimension", "runner_kind"].includes(name)
                    ? text(String(config[name]))
                    : String(config[name] ?? "—")}
              </dd>
            </div>
          ))}
        {["periods", "author_keys", "runner_labels", "include", "exclude"]
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

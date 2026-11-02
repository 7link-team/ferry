import { Pencil } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Envelope } from "../../shared/api/resource";
import {
  base,
  createResource,
  listResource,
  patchResource,
} from "../../shared/api/resource";
import { commonText, insightText } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import type {
  Filter,
  FormConfig,
  FormField,
  ListConfig,
} from "../../shared/resources/types";

export type Row = Record<string, unknown>;

export const analysisReports = [
  ["overview", "总览"],
  ["contributors", "贡献者"],
  ["ranking", "贡献排名"],
  ["branches", "分支"],
  ["commits", "提交明细"],
  ["heatmap", "每日热力数据"],
  ["period", "周期统计"],
] as const;

export const timestamp = (value: unknown) =>
  typeof value === "number" && value > 0
    ? new Date(value).toLocaleString(currentLocale())
    : "—";

export const coverage = (value: unknown) =>
  value === 0 ? insightText("完整历史") : timestamp(value);

export const lines = (value: unknown) =>
  Array.isArray(value) ? value.join("\n") : String(value ?? "");

export const states: Record<string, string> = {
  pending: "等待初始化",
  queued: "已排队",
  running: "同步中",
  complete: "同步完成",
  failed: "同步失败",
  partial: "部分覆盖",
  unsupported: "平台不支持",
};

export const analysisState = (value: unknown) =>
  states[String(value)]
    ? insightText(states[String(value)]!)
    : value === null || value === undefined || value === ""
      ? "—"
      : commonText("state.unknown", { value: String(value) });

export const analysisDatePresets = [
  ["default", "默认（已采集范围）"],
  ["all", "全部"],
  ["today", "今天"],
  ["yesterday", "昨天"],
  ["this-week", "本周"],
  ["last-week", "上周"],
  ["two-weeks-ago", "上上周"],
  ["this-month", "本月"],
  ["last-month", "上月"],
  ["two-months-ago", "上上月"],
] as const;

export type DatePreset = (typeof analysisDatePresets)[number][0];

export const dayMs = 86400000;

export function analysisDateRange(
  key: DatePreset,
  now = new Date(),
): Record<string, string> {
  if (key === "default" || key === "all") return { since: "", until: "" };
  const today = Date.UTC(
    now.getUTCFullYear(),
    now.getUTCMonth(),
    now.getUTCDate(),
  );
  const monday = today - ((now.getUTCDay() + 6) % 7) * dayMs;
  const month = (offset: number) =>
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth() + offset, 1);
  const [start, end] =
    key === "today"
      ? [today, today + dayMs]
      : key === "yesterday"
        ? [today - dayMs, today]
        : key === "this-week"
          ? [monday, monday + 7 * dayMs]
          : key === "last-week"
            ? [monday - 7 * dayMs, monday]
            : key === "two-weeks-ago"
              ? [monday - 14 * dayMs, monday - 7 * dayMs]
              : key === "this-month"
                ? [month(0), month(1)]
                : key === "last-month"
                  ? [month(-1), month(0)]
                  : [month(-2), month(-1)];
  return {
    since: new Date(start).toISOString().slice(0, 10),
    until: new Date(end - dayMs).toISOString().slice(0, 10),
  };
}

export function AnalysisSummary({
  data,
  kind,
}: {
  data: Envelope<Row>;
  kind: string;
}) {
  useTranslation("insight-domain");
  const reportCoverage = data.coverage as
    { complete?: boolean; start?: number; sources?: Row[] } | undefined;
  const totals = new Map<string, { commits: number; effectives: number }>();
  for (const row of data.series ?? []) {
    const date = String(row.period ?? row.date ?? "");
    const previous = totals.get(date) ?? { commits: 0, effectives: 0 };
    totals.set(date, {
      commits: previous.commits + Number(row.commits ?? 0),
      effectives: previous.effectives + Number(row.effectives ?? 0),
    });
  }
  const points = [...totals].sort(([a], [b]) => a.localeCompare(b));
  const authorName = (row: Row) =>
    String(row.nickname ?? row.name ?? insightText("未命名作者"));
  const authorKey = (row: Row) =>
    String(row.authorKey ?? JSON.stringify([authorName(row), row.email ?? ""]));
  const distinctAuthors = [
    ...new Map(
      (data.series ?? []).map((row) => [authorKey(row), row]),
    ).values(),
  ];
  const authors = distinctAuthors
    .map((row) => ({
      key: authorKey(row),
      name:
        distinctAuthors.filter((other) => authorName(other) === authorName(row))
          .length > 1
          ? `${authorName(row)}（${String(row.email ?? "")}）`
          : authorName(row),
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
  const authorPoints = authors.map((author) => ({
    ...author,
    values: new Map(
      (data.series ?? [])
        .filter((row) => authorKey(row) === author.key)
        .map((row) => [
          String(row.period ?? row.date ?? ""),
          {
            commits: Number(row.commits ?? 0),
            effectives: Number(row.effectives ?? 0),
          },
        ]),
    ),
  }));
  const colors = [
    "var(--primary)",
    "var(--success)",
    "var(--warning)",
    "var(--destructive)",
    "var(--muted-foreground)",
  ];
  const dash = (index: number) =>
    index < colors.length
      ? undefined
      : `${2 + Math.floor(index / colors.length) * 2} 3`;
  const max = Math.max(
    1,
    ...(kind === "heatmap"
      ? points.map(([, value]) => value.commits)
      : authorPoints.flatMap((author) =>
          [...author.values.values()].map((value) => value.effectives),
        )),
  );
  const first = points.length ? Date.parse(points[0]![0]) : NaN;
  const last = points.length ? Date.parse(points[points.length - 1]![0]) : NaN;
  const start = Number.isFinite(first)
    ? first - ((new Date(first).getUTCDay() + 6) % 7) * 86400000
    : NaN;
  const days =
    Number.isFinite(last) && Number.isFinite(start)
      ? Math.floor((last - start) / 86400000) + 1
      : 0;
  return (
    <div className="space-y-3 rounded-md border p-3">
      {reportCoverage && (
        <p role="status" className="text-sm">
          {insightText("coverageSummary", {
            status: insightText(
              reportCoverage.complete
                ? "所选仓库同步完成"
                : "覆盖尚未完成，报告仅包含已采集事实",
            ),
            start: coverage(reportCoverage.start),
            count: reportCoverage.sources?.length ?? 0,
          })}
        </p>
      )}
      {(kind === "heatmap" || kind === "period") &&
        (points.length ? (
          <figure className="space-y-2">
            <figcaption className="text-sm font-medium">
              {kind === "heatmap"
                ? insightText("每日提交热力图（UTC，周一至周日）")
                : insightText("每位作者的有效变更趋势")}
            </figcaption>
            <div className="overflow-x-auto">
              {kind === "heatmap" ? (
                <svg
                  role="img"
                  aria-label={insightText(
                    "每日提交热力图，深色表示更多提交，下方表格提供精确数值",
                  )}
                  width={Math.max(280, Math.ceil(days / 7) * 16)}
                  height="128"
                  className="text-primary"
                >
                  {Array.from({ length: days }, (_, index) => {
                    const date = new Date(start + index * 86400000)
                      .toISOString()
                      .slice(0, 10);
                    const commits = totals.get(date)?.commits ?? 0;
                    return (
                      <rect
                        key={date}
                        x={Math.floor(index / 7) * 16}
                        y={(index % 7) * 16}
                        width="12"
                        height="12"
                        rx="2"
                        fill="currentColor"
                        opacity={commits ? 0.25 + (commits / max) * 0.75 : 0.08}
                      >
                        <title>
                          {insightText("commitCount", { date, count: commits })}
                        </title>
                      </rect>
                    );
                  })}
                </svg>
              ) : (
                <svg
                  role="img"
                  aria-label={insightText(
                    "周期有效变更趋势，下方表格提供作者和周期精确数值",
                  )}
                  viewBox="0 0 800 180"
                  className="h-44 w-full text-primary"
                >
                  <line
                    x1="20"
                    x2="780"
                    y1="155"
                    y2="155"
                    stroke="currentColor"
                    opacity="0.2"
                  />
                  {authorPoints.map((author, authorIndex) => (
                    <g
                      key={author.key}
                      data-author={author.name}
                      data-author-key={author.key}
                      stroke={colors[authorIndex % colors.length]}
                      fill={colors[authorIndex % colors.length]}
                    >
                      <polyline
                        fill="none"
                        strokeWidth="2"
                        strokeDasharray={dash(authorIndex)}
                        points={points
                          .map(
                            ([date], index) =>
                              `${20 + (index / Math.max(1, points.length - 1)) * 760},${145 - ((author.values.get(date)?.effectives ?? 0) / max) * 125}`,
                          )
                          .join(" ")}
                      >
                        <title>{author.name}</title>
                      </polyline>
                      {points.map(([date], index) => {
                        const value = author.values.get(date);
                        return (
                          <circle
                            key={date}
                            cx={
                              20 +
                              (index / Math.max(1, points.length - 1)) * 760
                            }
                            cy={145 - ((value?.effectives ?? 0) / max) * 125}
                            r="3"
                          >
                            <title>
                              {insightText("trendPoint", {
                                author: author.name,
                                date,
                                effectives: value?.effectives ?? 0,
                                commits: value?.commits ?? 0,
                              })}
                            </title>
                          </circle>
                        );
                      })}
                    </g>
                  ))}
                  <text x="20" y="176" fill="currentColor" fontSize="12">
                    {points[0]![0]}
                  </text>
                  <text
                    x="780"
                    y="176"
                    textAnchor="end"
                    fill="currentColor"
                    fontSize="12"
                  >
                    {points[points.length - 1]![0]}
                  </text>
                </svg>
              )}
            </div>
            {kind === "period" && (
              <ul
                aria-label={insightText("趋势作者图例")}
                className="flex flex-wrap gap-x-4 gap-y-2 text-xs"
              >
                {authors.map((author, index) => (
                  <li key={author.key} className="flex items-center gap-1.5">
                    <svg width="24" height="8" aria-hidden="true">
                      <line
                        x1="0"
                        y1="4"
                        x2="24"
                        y2="4"
                        stroke={colors[index % colors.length]}
                        strokeDasharray={dash(index)}
                        strokeWidth="2"
                      />
                    </svg>
                    {author.name}
                  </li>
                ))}
              </ul>
            )}
            <p className="text-xs text-muted-foreground">
              {insightText(kind === "heatmap" ? "heatmapScale" : "trendScale", {
                max,
              })}
              {insightText("图表使用全部筛选结果，表格分页不影响图表。")}
            </p>
          </figure>
        ) : (
          <p className="text-sm text-muted-foreground">
            {insightText("尚无可绘制的已采集数据。")}
          </p>
        ))}
    </div>
  );
}

export function analysisConfig(
  workspace: string,
  connectionId: string,
  permissions = new Set<string>(),
): FormConfig<Row> {
  const canConfigurePipeline =
    permissions.has("pipelines.read") && permissions.has("pipelines.run");
  const endpoint = `${base(workspace, "code-connections")}/${encodeURIComponent(connectionId)}`;
  const fields: FormField<Row>[] = [
    {
      key: "enabled",
      label: "启用贡献分析",
      labelKey: "insight-domain:启用贡献分析",
      type: "switch",
    },
    {
      key: "all_repositories",
      label: "全部可见仓库",
      labelKey: "insight-domain:全部可见仓库",
      type: "switch",
    },
    {
      key: "repositories",
      label: "指定仓库",
      labelKey: "insight-domain:指定仓库",
      type: "textarea",
      required: true,
      when: (row) => !row.all_repositories,
      display: (row) => lines(row.repositories),
      help: "每行一个完整仓库路径；普通 Git 连接填写完整仓库地址，并关闭全部仓库。",
      helpKey:
        "insight-domain:每行一个完整仓库路径；普通 Git 连接填写完整仓库地址，并关闭全部仓库。",
      span: 2,
    },
    {
      key: "full_history",
      label: "初始化完整历史",
      labelKey: "insight-domain:初始化完整历史",
      type: "switch",
    },
    {
      key: "initial_days",
      label: "初始化最近天数",
      labelKey: "insight-domain:初始化最近天数",
      type: "number",
      required: true,
      min: 1,
      when: (row) => !row.full_history,
      help: "默认最近 90 天；扩大范围会补采历史，缩小范围不会删除已有事实。",
      helpKey:
        "insight-domain:默认最近 90 天；扩大范围会补采历史，缩小范围不会删除已有事实。",
    },
    {
      key: "reconcile_minutes",
      label: "分支头核对间隔（分钟）",
      labelKey: "insight-domain:分支头核对间隔（分钟）",
      type: "number",
      required: true,
      min: 1,
      help: "默认 1440 分钟。事件或轮询发现变化后增量同步。",
      helpKey: "insight-domain:默认 1440 分钟。事件或轮询发现变化后增量同步。",
    },
    {
      key: "pipeline_id",
      label: "act 分析流水线",
      labelKey: "insight-domain:act 分析流水线",
      type: canConfigurePipeline ? "select" : "text",
      display: (row) =>
        String(row.pipeline_name ?? row.pipeline_id ?? "") ||
        insightText("使用平台接口"),
      options: canConfigurePipeline
        ? async () => {
            const options: {
              value: string;
              label: string;
              labelKey?: string;
            }[] = [
              {
                value: "",
                label: "使用平台接口",
                labelKey: "insight-domain:使用平台接口",
              },
            ];
            for (let page = 1; ; page++) {
              const result = await listResource<{ id: string; name: string }>(
                workspace,
                "pipelines",
                { page, page_size: 100 },
              );
              options.push(
                ...result.items.map((row) => ({
                  value: row.id,
                  label: row.name,
                })),
              );
              if (!result.items.length || page * 100 >= result.total) break;
            }
            return options;
          }
        : [],
      help: canConfigurePipeline
        ? insightText(
            "普通 Git 或平台接口无法提供完整差异时，选择配置了仓库检出与分析产物的 act 流水线。",
          )
        : insightText("更换分析流水线需要流水线读取及执行权限。"),
    },
    {
      key: "aliases",
      label: "作者昵称映射",
      labelKey: "insight-domain:作者昵称映射",
      type: "textarea",
      span: 2,
      display: (row) => JSON.stringify(row.aliases ?? [], null, 2),
      get help() {
        return insightText("legacyAliasHelp", {
          example:
            '[{"name":"Alice","email":"alice@example.com","nickname":"小艾"}]',
        });
      },
    },
  ];
  return {
    resource: "code-connection-analyses",
    permissionResource: "repositories",
    width: "full",
    blank: {},
    readOnly: () => true,
    heading: () => insightText("贡献分析配置与同步覆盖"),
    pollMs: () => 15000,
    sections: [
      {
        title: "分析配置",
        titleKey: "insight-domain:分析配置",
        fullWidth: true,
        fields,
      },
      {
        title: "同步覆盖",
        titleKey: "insight-domain:同步覆盖",
        fullWidth: true,
        fields: [
          {
            key: "coverage_start",
            label: "已请求覆盖起点",
            labelKey: "insight-domain:已请求覆盖起点",
            type: "text",
            display: (row) => coverage(row.coverage_start),
          },
          {
            key: "last_error",
            label: "发现 / 核对错误",
            labelKey: "insight-domain:发现 / 核对错误",
            type: "textarea",
            when: (row) => !!row.last_error,
            span: 2,
          },
          {
            key: "sources",
            label: "各仓库覆盖与任务状态",
            labelKey: "insight-domain:各仓库覆盖与任务状态",
            type: "records",
            span: 2,
            record: {
              title: (row) => String(row.repository ?? ""),
              meta: (row) =>
                insightText("sourceMeta", {
                  state: analysisState(row.state),
                  start: coverage(row.coverage_start),
                  time: timestamp(row.synced_at),
                }),
              body: (row) => {
                const job = row.job as Row | undefined;
                return [
                  row.last_error,
                  job &&
                    insightText("jobProgress", {
                      state: analysisState(job.state),
                      completed: job.completed_generation ?? 0,
                      generation: job.generation ?? 0,
                    }),
                  job?.last_error,
                ]
                  .filter(Boolean)
                  .join("\n");
              },
            },
          },
        ],
      },
    ],
    viewActions: [
      {
        label: "修改分析配置",
        labelKey: "insight-domain:修改分析配置",
        section: insightText("分析配置"),
        iconOnly: true,
        icon: <Pencil className="size-4" />,
        permission: "repositories.update",
        description:
          "保存后自动初始化并持续采集。配置流水线需要流水线执行权限，分析采集需要读取连接凭据的权限。",
        descriptionKey:
          "insight-domain:保存后自动初始化并持续采集。配置流水线需要流水线执行权限，分析采集需要读取连接凭据的权限。",
        initialValues: (row) => ({
          ...row,
          repositories: lines(row.repositories),
          aliases: JSON.stringify(row.aliases ?? [], null, 2),
        }),
        parameters: () =>
          fields.filter(
            (field) => field.key !== "pipeline_id" || canConfigurePipeline,
          ),
        action: async (row, values) => {
          const draft = values ?? {};
          let aliases: unknown;
          try {
            aliases = JSON.parse(String(draft.aliases || "[]"));
          } catch {
            throw new Error(insightText("作者昵称映射须为有效的 JSON 数组。"));
          }
          if (
            !Array.isArray(aliases) ||
            aliases.some(
              (alias) =>
                !alias ||
                typeof alias !== "object" ||
                typeof alias.nickname !== "string" ||
                !alias.nickname.trim() ||
                (!String(alias.name ?? "").trim() &&
                  !String(alias.email ?? "").trim()),
            )
          )
            throw new Error(
              insightText("每条作者映射须填写姓名或邮箱，以及显示昵称。"),
            );
          const config = {
            enabled: !!draft.enabled,
            all_repositories: !!draft.all_repositories,
            repositories: String(draft.repositories ?? "")
              .split(/\r?\n/)
              .map((item) => item.trim())
              .filter(Boolean),
            full_history: !!draft.full_history,
            initial_days: Number(draft.initial_days ?? 90),
            reconcile_minutes: Number(draft.reconcile_minutes ?? 1440),
            pipeline_id: String(draft.pipeline_id ?? ""),
            aliases: aliases.map((alias) => ({
              name: String(alias.name ?? ""),
              email: String(alias.email ?? ""),
              nickname: alias.nickname,
              ...(alias.contributor_id
                ? { contributor_id: String(alias.contributor_id) }
                : {}),
            })),
          };
          await patchResource(
            workspace,
            "code-connections",
            "analysis",
            { config, version: row.version },
            endpoint,
          );
        },
      },
      {
        label: "重新核对 / 重试同步",
        labelKey: "insight-domain:重新核对 / 重试同步",
        permission: "repositories.update",
        when: (row) => !!row.enabled,
        action: async () => {
          await createResource(
            workspace,
            "code-connections",
            {},
            `${endpoint}/analysis-runs`,
          );
        },
      },
    ],
  };
}

export const analysisFilters: Filter[] = [
  {
    key: "repos",
    label: "仓库（逗号分隔）",
    labelKey: "insight-domain:仓库（逗号分隔）",
    type: "text",
  },
  {
    key: "authors",
    label: "作者昵称（逗号分隔）",
    labelKey: "insight-domain:作者昵称（逗号分隔）",
    type: "text",
  },
  {
    key: "branches",
    label: "分支（逗号分隔）",
    labelKey: "insight-domain:分支（逗号分隔）",
    type: "text",
  },
  {
    key: "since",
    label: "起始日期（UTC）",
    labelKey: "insight-domain:起始日期（UTC）",
    type: "text",
    placeholder: "YYYY-MM-DD",
  },
  {
    key: "until",
    label: "截止日期（UTC）",
    labelKey: "insight-domain:截止日期（UTC）",
    type: "text",
    placeholder: "YYYY-MM-DD",
  },
  {
    key: "history",
    label: "分支历史",
    labelKey: "insight-domain:分支历史",
    type: "select",
    options: [
      {
        value: "observed",
        label: "已观察历史",
        labelKey: "insight-domain:已观察历史",
      },
      {
        value: "current",
        label: "当前可达提交",
        labelKey: "insight-domain:当前可达提交",
      },
    ],
  },
  {
    key: "isMerge",
    label: "合并提交",
    labelKey: "insight-domain:合并提交",
    type: "select",
    options: [
      {
        value: "0",
        label: "排除合并（默认）",
        labelKey: "insight-domain:排除合并（默认）",
      },
      { value: "1", label: "仅合并", labelKey: "insight-domain:仅合并" },
      { value: "0,1", label: "包含合并", labelKey: "insight-domain:包含合并" },
    ],
  },
  {
    key: "messageType",
    label: "提交类型（逗号分隔）",
    labelKey: "insight-domain:提交类型（逗号分隔）",
    type: "text",
    placeholder: "feat,fix,docs",
  },
  {
    key: "commitHash",
    label: "完整提交 SHA",
    labelKey: "insight-domain:完整提交 SHA",
    type: "text",
  },
  {
    key: "geEffective",
    label: "有效变更下限",
    labelKey: "insight-domain:有效变更下限",
    type: "text",
  },
  {
    key: "leEffective",
    label: "有效变更上限",
    labelKey: "insight-domain:有效变更上限",
    type: "text",
  },
];

export function analysisReport(
  workspace: string,
  connectionId: string,
  kind: string,
  scope: "connection" | "insight" = "connection",
): ListConfig<Row> {
  const metrics = [
    { key: "commits", label: "提交数", labelKey: "insight-domain:提交数" },
    { key: "additions", label: "新增行", labelKey: "insight-domain:新增行" },
    { key: "deletions", label: "删除行", labelKey: "insight-domain:删除行" },
    {
      key: "effectives",
      label: "有效变更",
      labelKey: "insight-domain:有效变更",
    },
  ];
  return {
    resource: `code-contributions/${kind}`,
    permissionResource: scope === "insight" ? "code-insights" : "repositories",
    endpoint:
      scope === "insight"
        ? `${base(workspace, "code-insights")}/${encodeURIComponent(connectionId)}/reports/${kind}`
        : `${base(workspace, "code-contributions")}/${kind}`,
    fixed: scope === "insight" ? {} : { connection_id: connectionId },
    rowKey: (row) => String(row.id),
    defaultRefresh: 30,
    summary: (data) => <AnalysisSummary data={data} kind={kind} />,
    filterPresets: [
      {
        label: "日期快捷条件（UTC）",
        labelKey: "insight-domain:日期快捷条件（UTC）",
        presets: analysisDatePresets.map(([key, label]) => ({
          label,
          labelKey: `insight-domain:${label}`,
          values: () => analysisDateRange(key),
        })),
      },
      ...(kind === "heatmap"
        ? [
            {
              label: "热力图提交类型",
              labelKey: "insight-domain:热力图提交类型",
              presets: [
                {
                  label: "全部提交",
                  labelKey: "insight-domain:全部提交",
                  values: { messageType: "", isMerge: "0,1" },
                },
                {
                  label: "修复 fix",
                  labelKey: "insight-domain:修复 fix",
                  values: { messageType: "fix", isMerge: "0" },
                },
                {
                  label: "功能 feat",
                  labelKey: "insight-domain:功能 feat",
                  values: { messageType: "feat", isMerge: "0" },
                },
                {
                  label: "合并 merge",
                  labelKey: "insight-domain:合并 merge",
                  values: { messageType: "", isMerge: "1" },
                },
              ],
            },
          ]
        : []),
    ],
    filters: [
      ...analysisFilters,
      ...(kind === "period"
        ? [
            {
              key: "period",
              label: "统计周期",
              labelKey: "insight-domain:统计周期",
              type: "select" as const,
              options: [
                { value: "day", label: "日", labelKey: "insight-domain:日" },
                { value: "week", label: "周", labelKey: "insight-domain:周" },
                { value: "month", label: "月", labelKey: "insight-domain:月" },
              ],
            },
          ]
        : []),
    ],
    columns:
      kind === "commits"
        ? [
            {
              key: "repository",
              label: "仓库",
              labelKey: "insight-domain:仓库",
            },
            {
              key: "branches",
              label: "分支",
              labelKey: "insight-domain:分支",
              render: (row) => lines(row.branches),
            },
            {
              key: "commitHash",
              label: "提交 SHA",
              labelKey: "insight-domain:提交 SHA",
              render: (row) => {
                const sha = (
                  <span className="font-mono" title={String(row.commitHash)}>
                    {String(row.commitHash)}
                  </span>
                );
                return typeof row.commitUrl === "string" &&
                  /^https?:\/\//i.test(row.commitUrl) ? (
                  <a
                    href={row.commitUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-primary underline underline-offset-4"
                    aria-label={insightText("viewCommit", {
                      sha: String(row.commitHash),
                    })}
                  >
                    {sha}
                  </a>
                ) : (
                  sha
                );
              },
            },
            {
              key: "message",
              label: "提交信息",
              labelKey: "insight-domain:提交信息",
            },
            {
              key: "messageType",
              label: "类型",
              labelKey: "insight-domain:类型",
            },
            {
              key: "isMerge",
              label: "合并",
              labelKey: "insight-domain:合并",
              render: (row) =>
                row.isMerge ? insightText("是") : insightText("否"),
            },
            {
              key: "nickname",
              label: "作者昵称",
              labelKey: "insight-domain:作者昵称",
            },
            {
              key: "authorName",
              label: "作者姓名",
              labelKey: "insight-domain:作者姓名",
            },
            {
              key: "authorEmail",
              label: "作者邮箱",
              labelKey: "insight-domain:作者邮箱",
            },
            {
              key: "date",
              label: "作者时间",
              labelKey: "insight-domain:作者时间",
              render: (row) => timestamp(row.date),
            },
            {
              key: "committerDate",
              label: "提交时间",
              labelKey: "insight-domain:提交时间",
              render: (row) => timestamp(row.committerDate),
            },
            ...metrics.filter((column) => column.key !== "commits"),
            {
              key: "languageStats",
              label: "语言变更",
              labelKey: "insight-domain:语言变更",
              render: (row) => (
                <pre className="whitespace-pre-wrap text-xs">
                  {JSON.stringify(row.languageStats ?? {}, null, 2)}
                </pre>
              ),
            },
          ]
        : kind === "branches"
          ? [
              {
                key: "repository",
                label: "仓库",
                labelKey: "insight-domain:仓库",
              },
              {
                key: "branchName",
                label: "分支",
                labelKey: "insight-domain:分支",
              },
              {
                key: "present",
                label: "仍然存在",
                labelKey: "insight-domain:仍然存在",
                render: (row) =>
                  row.present ? insightText("是") : insightText("否"),
              },
              {
                key: "nicknames",
                label: "贡献者",
                labelKey: "insight-domain:贡献者",
                render: (row) => lines(row.nicknames),
              },
              ...metrics,
            ]
          : [
              ...(kind === "period" || kind === "heatmap"
                ? [
                    {
                      key: "period",
                      label: "日期 / 周期（UTC）",
                      labelKey: "insight-domain:日期 / 周期（UTC）",
                    },
                  ]
                : []),
              ...(kind !== "overview" && kind !== "heatmap"
                ? [
                    {
                      key: "nickname",
                      label: "作者昵称",
                      labelKey: "insight-domain:作者昵称",
                    },
                    {
                      key: "name",
                      label: "作者姓名",
                      labelKey: "insight-domain:作者姓名",
                    },
                    {
                      key: "email",
                      label: "邮箱",
                      labelKey: "insight-domain:邮箱",
                    },
                  ]
                : []),
              {
                key: "projects",
                label: "参与仓库",
                labelKey: "insight-domain:参与仓库",
              },
              ...metrics,
            ],
    toolbar: { create: false, import: false, export: false },
    empty:
      "暂无匹配的完整提交事实。请检查分析配置、同步覆盖和筛选条件；同步未完成不代表没有贡献。",
    emptyKey:
      "insight-domain:暂无匹配的完整提交事实。请检查分析配置、同步覆盖和筛选条件；同步未完成不代表没有贡献。",
  };
}

export const overviewPeriods = [
  "today",
  "yesterday",
  "this-week",
  "last-week",
  "this-month",
  "last-month",
] as const;

export function overviewRanking(
  workspace: string,
  connectionId: string,
  period: (typeof overviewPeriods)[number],
  now = new Date(),
  scope: "connection" | "insight" = "connection",
): ListConfig<Row> {
  const report = analysisReport(workspace, connectionId, "ranking", scope);
  return {
    ...report,
    resource: `code-contributions/overview-${period}`,
    queryPrefix: `overview.${period}`,
    filterPresets: [],
    fixed: { ...report.fixed, ...analysisDateRange(period, now) },
    filters: report.filters!.filter(
      (filter) => filter.key !== "since" && filter.key !== "until",
    ),
    defaultPageSize: 10,
  };
}

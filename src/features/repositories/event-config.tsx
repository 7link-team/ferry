import { base } from "../../shared/api/resource";
import { selectDisplay } from "../../shared/forms/auto-form";
import { commonText, hostRepositoryText } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import type { FormConfig, ListConfig } from "../../shared/resources/types";

export const repositoryEventOptions = [
  ["repository.created", "repository-monitor.001"],
  ["repository.deleted", "repository-monitor.002"],
  ["branch.created", "repository-monitor.003"],
  ["branch.deleted", "repository-monitor.004"],
  ["push", "repository-monitor.005"],
  ["pull_request", "repository-monitor.006"],
  ["merge", "repository-monitor.007"],
  ["tag.created", "repository-monitor.008"],
  ["tag.deleted", "repository-monitor.009"],
].map(([value, key]) => ({
  value: value!,
  get label() {
    return hostRepositoryText(key!);
  },
}));

export const gitRepositoryEventOptions = [
  ...repositoryEventOptions.filter((option) =>
    [
      "push",
      "branch.created",
      "branch.deleted",
      "tag.created",
      "tag.deleted",
    ].includes(option.value),
  ),
  {
    value: "tag.updated",
    get label() {
      return hostRepositoryText("repository-monitor.010");
    },
  },
];

export const eventLabel = (value: unknown) =>
  selectDisplay(value, {
    options: [...repositoryEventOptions, ...gitRepositoryEventOptions],
  });

export const time = (value: unknown) => {
  if (value == null || value === "" || value === 0 || value === "0") return "—";
  if (typeof value === "number" && value > 0)
    return new Date(value).toLocaleString(currentLocale());
  const parsed = typeof value === "string" && value ? Date.parse(value) : NaN;
  return Number.isNaN(parsed)
    ? String(value ?? "")
    : new Date(parsed).toLocaleString(currentLocale());
};

export const has = (key: string) => (row: Record<string, unknown>) =>
  row[key] !== undefined && row[key] !== null && row[key] !== "";

function eventSource(row: Record<string, unknown>) {
  const detail = row.detail && typeof row.detail === "object" ? row.detail as Record<string, unknown> : {};
  return String(row.source || detail.source || "");
}

function committerOf(commit: unknown) {
  const item = Array.isArray(commit) ? commit[0] : commit;
  if (!item || typeof item !== "object") return "";
  const { committer, author } = item as Record<string, unknown>;
  return String(committer || author || "").trim();
}

/** Polling cannot see the pusher, so it falls back to the newest commit's committer, labeled as such. */
export function repositoryEventActor(row: Record<string, unknown>) {
  const actor = typeof row.actor === "string" ? row.actor.trim() : "";
  if (actor) return actor;
  const detail = row.detail && typeof row.detail === "object" ? row.detail as Record<string, unknown> : row;
  const commits = Array.isArray(detail.commits) ? detail.commits : [];
  const committer = committerOf(detail.head_commit) || committerOf(commits[0]) || committerOf(detail.commit);
  if (committer) return hostRepositoryText("repository.actorCommitter", { name: committer });
  return hostRepositoryText(eventSource(row) === "polling" ? "repository.actorPollingUnavailable" : "repository.actorUnavailable");
}

export const commitRecord = {
  title: (item: Record<string, unknown>) =>
    String(item.short_id ?? item.id ?? ""),
  meta: (item: Record<string, unknown>) =>
    [
      item.author_username && item.author_username !== item.author
        ? `${item.author}（${item.author_username}）`
        : item.author,
      item.time ? time(item.time) : "",
    ]
      .filter(Boolean)
      .join(" · "),
  body: (item: Record<string, unknown>) => String(item.message ?? ""),
  href: (item: Record<string, unknown>) => String(item.url ?? ""),
};

export function repositoryEventForm(): FormConfig<Record<string, unknown>> {
  const text = (
    key: string,
    label: string,
    extra: Partial<
      FormConfig<Record<string, unknown>>["sections"][number]["fields"][number]
    > = {},
  ) => ({ key, label, type: "text" as const, when: has(key), ...extra });
  return {
    resource: "repository-events",
    permissionResource: "repositories",
    width: "full",
    blank: {},
    readOnly: () => true,
    heading: (row) =>
      `${eventLabel(row.event) || hostRepositoryText("repository-monitor.011")} · ${String(row.repository ?? "")}`,
    viewActions: [
      {
        label: commonText("common.backList"),
        when: row => new URLSearchParams(location.search).get("from_connection") !== String(row.connection_id),
        to: () => ({ group: "repository", item: "repository-events", mode: "list" }),
        query: () => Object.fromEntries([...new URLSearchParams(location.search)].filter(([key]) => key !== "from_connection")),
      },
      {
        label: hostRepositoryText("repository-monitor.012"),
        when: row => new URLSearchParams(location.search).get("from_connection") === String(row.connection_id),
        to: (row) => ({
          group: "repository",
          item: "repositories",
          mode: "view",
          id: String(row.connection_id),
        }),
        query: () => ({ ...Object.fromEntries([...new URLSearchParams(location.search)].filter(([key]) => key !== "from_connection")), tab: "events" }),
      },
    ],
    sections: [
      {
        title: hostRepositoryText("repository-monitor.013"),
        fields: [
          {
            key: "event",
            label: hostRepositoryText("repository-monitor.014"),
            type: "select",
            options: [
              ...repositoryEventOptions,
              {
                value: "tag.updated",
                label: hostRepositoryText("repository-monitor.010"),
              },
            ],
          },
          text("repository", hostRepositoryText("repository.004")),
          text("reference", hostRepositoryText("repository-monitor.015")),
          text("actor", hostRepositoryText("repository-monitor.016"), {
            when: () => true,
            display: repositoryEventActor,
            help: hostRepositoryText("repository.actorSourceHelp"),
          }),
          text("action", hostRepositoryText("repository-monitor.017")),
          text("created_at", hostRepositoryText("repository-monitor.018"), {
            display: (row) => time(row.created_at),
          }),
          {
            key: "source",
            label: hostRepositoryText("repository-monitor.019"),
            type: "select",
            options: [
              {
                value: "webhook",
                label: hostRepositoryText("repository-monitor.020"),
              },
              {
                value: "polling",
                label: hostRepositoryText("repository-monitor.021"),
              },
            ],
            when: row => !!eventSource(row),
            display: row => eventSource(row) === "polling" ? hostRepositoryText("repository-monitor.021") : eventSource(row) === "webhook" ? hostRepositoryText("repository-monitor.020") : eventSource(row),
          },
          text("delivery", hostRepositoryText("repository-monitor.022")),
        ],
      },
      {
        title: hostRepositoryText("repository-monitor.023"),
        fullWidth: true,
        fields: [
          text("total_commits", hostRepositoryText("repository-monitor.024")),
          text("before", hostRepositoryText("repository-monitor.025")),
          text("after", hostRepositoryText("repository-monitor.026")),
          {
            key: "forced",
            label: hostRepositoryText("repository-monitor.027"),
            type: "switch",
            when: has("forced"),
          },
          text("compare_url", hostRepositoryText("repository-monitor.028"), {
            link: true,
            span: 2,
          }),
          {
            key: "commits",
            label: hostRepositoryText("repository-monitor.029"),
            type: "records",
            record: commitRecord,
            span: 2,
            when: has("commits"),
          },
          text(
            "commits_truncated",
            hostRepositoryText("repository-monitor.030"),
            { span: 2 },
          ),
          {
            key: "head_commit",
            label: hostRepositoryText("repository-monitor.031"),
            type: "records",
            record: commitRecord,
            span: 2,
            when: has("head_commit"),
          },
        ],
      },
      {
        title: hostRepositoryText("repository-monitor.032"),
        fullWidth: true,
        fields: [
          text("request_number", hostRepositoryText("repository-monitor.033")),
          text("request_title", hostRepositoryText("repository-monitor.034")),
          text("request_state", hostRepositoryText("repository-monitor.035")),
          text(
            "request_branches",
            hostRepositoryText("repository-monitor.036"),
          ),
          text("request_author", hostRepositoryText("repository-monitor.037")),
          text(
            "request_merged_by",
            hostRepositoryText("repository-monitor.038"),
          ),
          text(
            "request_reviewers",
            hostRepositoryText("repository-monitor.039"),
          ),
          text(
            "request_merge_commit",
            hostRepositoryText("repository-monitor.040"),
          ),
          text(
            "request_created_at",
            hostRepositoryText("repository-monitor.041"),
            { display: (row) => time(row.request_created_at) },
          ),
          text(
            "request_updated_at",
            hostRepositoryText("repository-monitor.042"),
            { display: (row) => time(row.request_updated_at) },
          ),
          text(
            "request_merged_at",
            hostRepositoryText("repository-monitor.043"),
            { display: (row) => time(row.request_merged_at) },
          ),
          text(
            "request_closed_at",
            hostRepositoryText("repository-monitor.044"),
            { display: (row) => time(row.request_closed_at) },
          ),
          text("request_url", hostRepositoryText("repository-monitor.045"), {
            link: true,
            span: 2,
          }),
          {
            key: "request_description",
            label: hostRepositoryText("repository-monitor.030"),
            type: "textarea",
            span: 2,
            when: has("request_description"),
          },
        ],
      },
      {
        title: hostRepositoryText("repository-monitor.046"),
        fields: [
          text("tag_target", hostRepositoryText("repository-monitor.047")),
          text("tagger", hostRepositoryText("repository-monitor.048")),
          {
            key: "tag_message",
            label: hostRepositoryText("repository-monitor.049"),
            type: "textarea",
            span: 2,
            when: has("tag_message"),
          },
        ],
      },
      {
        title: hostRepositoryText("repository-monitor.050"),
        fields: [
          text(
            "repository_visibility",
            hostRepositoryText("repository-monitor.051"),
          ),
          text(
            "repository_default_branch",
            hostRepositoryText("repository-monitor.052"),
          ),
          text("repository_url", hostRepositoryText("repository-monitor.053"), {
            link: true,
            span: 2,
          }),
          {
            key: "repository_description",
            label: hostRepositoryText("repository-monitor.054"),
            type: "textarea",
            span: 2,
            when: has("repository_description"),
          },
        ],
      },
      {
        title: hostRepositoryText("repository-monitor.030"),
        fullWidth: true,
        fields: [
          {
            key: "note",
            label: hostRepositoryText("repository-monitor.030"),
            type: "textarea",
            span: 2,
            when: has("note"),
          },
        ],
      },
    ],
  };
}

export function repositoryEventList(
  workspace: string,
  connectionId?: string,
  provider?: string,
): ListConfig<Record<string, unknown>> {
  return {
    resource: "repository-events",
    permissionResource: "repositories",
    rowKey: (row) => String(row.id),
    defaultRefresh: 15,
    ...(connectionId
      ? {
          endpoint: `${base(workspace, "code-connections")}/${encodeURIComponent(connectionId)}/events`,
        }
      : {}),
    filters: [
      {
        key: "q",
        label: hostRepositoryText("repository-monitor.093"),
        type: "text",
      },
      {
        key: "event",
        label: hostRepositoryText("repository-monitor.014"),
        type: "select",
        options:
          provider === "git"
            ? gitRepositoryEventOptions
            : repositoryEventOptions,
      },
    ],
    columns: [
      {
        key: "created_at",
        label: hostRepositoryText("repository-monitor.018"),
        sortable: true,
        render: (row) => time(row.created_at),
      },
      ...(connectionId
        ? []
        : [
            {
              key: "connection_name",
              label: hostRepositoryText("repository.002"),
            },
          ]),
      {
        key: "repository",
        label: hostRepositoryText("repository.004"),
        sortable: true,
      },
      {
        key: "event",
        label: hostRepositoryText("repository-monitor.014"),
        render: (row) => eventLabel(row.event),
      },
      { key: "reference", label: hostRepositoryText("repository-monitor.015") },
      { key: "summary", label: hostRepositoryText("repository-monitor.094") },
      { key: "actor", label: hostRepositoryText("repository-monitor.016"), render: row => <span title={hostRepositoryText("repository.actorSourceHelp")}>{repositoryEventActor(row)}</span> },
      { key: "action", label: hostRepositoryText("repository-monitor.017") },
      { key: "delivery", label: hostRepositoryText("repository-monitor.022") },
    ],
    actions: [
      {
        label: hostRepositoryText("repository-monitor.095"),
        query: () => ({ ...Object.fromEntries(new URLSearchParams(location.search)), from_connection: connectionId ?? "" }),
        to: (row) => ({
          group: "repository",
          item: "repository-events",
          mode: "view",
          id: String(row.id),
        }),
      },
    ],
    toolbar: { create: false, import: false, export: false },
    empty:
      provider === "git"
        ? hostRepositoryText("repository-monitor.096")
        : hostRepositoryText("repository-monitor.097"),
  };
}

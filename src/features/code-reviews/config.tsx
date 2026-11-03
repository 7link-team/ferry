import { api } from "../../shared/api/api";
import { base, deleteResource } from "../../shared/api/resource";
import { namedOptions } from "../../shared/resources/config";
import type {
  FormConfig,
  ListConfig,
  RowAction,
} from "../../shared/resources/types";
import type { Navigation } from "../../shared/routing/types";
import { text, key } from "../insight-reports/messages";
import { field, options, date } from "../insight-reports/config";
import type { Row } from "../insight-reports/ReportScope";
export function reviewCancelAction(
  workspace: string,
  reviewId: string,
): RowAction<Row> {
  return {
    label: "cancelRun",
    labelKey: key("cancelRun"),
    permission: "code-reviews.run",
    destructive: true,
    descriptionKey: key("cancelRunHelp"),
    parameters: () => [],
    when: (row) =>
      ["queued", "launching", "waiting", "running"].includes(String(row.state)),
    action: async (row) => {
      await api(
        `${base(workspace, "code-reviews")}/${encodeURIComponent(reviewId)}/runs/${encodeURIComponent(String(row.id))}/cancel`,
        { method: "POST" },
      );
    },
  };
}
export function reviewActions(workspace: string): RowAction<Row>[] {
  return [
    {
      label: "run",
      labelKey: key("run"),
      permission: "code-reviews.run",
      initialValues: () => ({ full_scan: false }),
      parameters: () => [
        field("repository"),
        field("base_sha"),
        field("head_sha"),
        field("full_scan", "switch"),
      ],
      action: async (row, values) => {
        await api(
          `${base(workspace, "code-reviews")}/${encodeURIComponent(String(row.id))}/run`,
          { method: "POST", body: JSON.stringify(values) },
        );
      },
    },
  ];
}
export function reviewList(workspace: string): ListConfig<Row> {
  return {
    resource: "code-reviews",
    rowKey: (row) => String(row.id),
    filters: [{ key: "q", label: "name", labelKey: key("name"), type: "text" }],
    columns: [
      { key: "name", label: "name", labelKey: key("name") },
      {
        key: "repositories",
        label: "repositories",
        labelKey: key("repositories"),
        render: (row) =>
          (row.repositories as { repository: string }[])
            ?.map((repo) => repo.repository || "*")
            .join(" · "),
      },
      {
        key: "trigger",
        label: "trigger",
        labelKey: key("trigger"),
        render: (row) => text(String(row.trigger)),
      },
      {
        key: "tests_enabled",
        label: "tests_enabled",
        labelKey: key("tests_enabled"),
        render: (row) => text(row.tests_enabled ? "enabled" : "not_enabled"),
      },
      {
        key: "ai_enabled",
        label: "ai_enabled",
        labelKey: key("ai_enabled"),
        render: (row) => text(row.ai_enabled ? "enabled" : "not_enabled"),
      },
      {
        key: "last_run_at",
        label: "last",
        labelKey: key("last"),
        render: (row) => date(row.last_run_at),
      },
      {
        key: "trigger_error",
        label: "trigger_error",
        labelKey: key("trigger_error"),
      },
      {
        key: "enabled",
        label: "state",
        labelKey: key("state"),
        render: (row) => text(row.enabled ? "enabled" : "not_enabled"),
      },
    ],
    actions: [
      ...["view", "edit"].map((mode) => ({
        label: mode,
        labelKey: key(mode),
        permission: `code-reviews.${mode === "view" ? "read" : "update"}`,
        to: (row: Row) => ({
          group: "build",
          item: "code-reviews",
          mode: mode as "view" | "edit",
          id: String(row.id),
        }),
      })),
      ...reviewActions(workspace),
      {
        label: "delete",
        labelKey: key("delete"),
        permission: "code-reviews.delete",
        destructive: true,
        action: async (row) => {
          await deleteResource(workspace, "code-reviews", String(row.id));
        },
      },
    ],
    toolbar: {
      create: { label: "create", labelKey: key("create") },
      import: false,
      export: false,
    },
    empty: "empty",
    emptyKey: key("empty"),
  };
}
export function reviewForm(
  workspace: string,
  navigation?: Navigation,
): FormConfig<Row> {
  const lines = (name: string) =>
    field(name, "custom", {
      span: 2,
      render: ({ draft, readOnly, onPatch }) => (
        <label className="block space-y-2 text-sm">
          <span>{text(name)}</span>
          <textarea
            aria-label={text(name)}
            disabled={readOnly}
            className="min-h-24 w-full rounded-md border bg-background p-3"
            value={
              Array.isArray(draft[name])
                ? (draft[name] as string[]).join("\n")
                : ""
            }
            onChange={(event) =>
              onPatch({ [name]: event.target.value.split("\n") })
            }
          />
        </label>
      ),
    });
  return {
    resource: "code-reviews",
    width: "full",
    presentation: "sections",
    heading: (row) => String(row.name ?? text("reviews")),
    description: "pipelineHelp",
    descriptionKey: key("pipelineHelp"),
    blank: {
      name: "",
      description: "",
      enabled: true,
      trigger: "manual",
      branch: "main",
      repositories: [],
      tests_enabled: true,
      ai_enabled: false,
      test_command: "",
      junit_path: "junit.xml",
      runner_kind: "pool",
      runner_ref: "",
      runner_labels: [],
      ai_endpoint: "",
      ai_model: "",
      ai_credential_id: "",
      ai_timeout_seconds: 120,
      ai_concurrency: 1,
      ai_max_tokens: 4096,
      max_files: 100,
      max_bytes: 1048576,
      include: [],
      exclude: [],
    },
    viewActions: reviewActions(workspace),
    sections: [
      {
        title: "configuration",
        titleKey: key("configuration"),
        fields: [
          field("name", "text", { required: true }),
          field("description", "textarea", { labelKey: key("message") }),
          field("trigger", "select", {
            options: options(["manual", "push"]),
            required: true,
          }),
          field("branch", "text", { required: true }),
          field("enabled", "switch"),
          field("pipeline_id", "custom", {
            labelKey: key("pipeline"),
            render: ({ draft }) =>
              draft.pipeline_id ? (
                <a
                  className="text-primary underline"
                  href={navigation?.href({
                    group: "build",
                    item: "pipelines",
                    id: String(draft.pipeline_id),
                    mode: "view",
                  })}
                >
                  {text("pipeline")}
                </a>
              ) : (
                <p className="text-sm text-muted-foreground">
                  {text("pipelineHelp")}
                </p>
              ),
          }),
        ],
      },
      {
        title: "repositories",
        titleKey: key("repositories"),
        fullWidth: true,
        fields: [
          field("repositories", "connection-repositories", {
            required: true,
            span: 2,
            repositoryAddLabelKey: key("addRepository"),
          }),
        ],
      },
      {
        title: "tests_enabled",
        titleKey: key("tests_enabled"),
        fields: [
          field("tests_enabled", "switch"),
          field("test_command", "textarea", {
            required: true,
            span: 2,
            when: (row) => !!row.tests_enabled,
          }),
          field("junit_path", "text", { when: (row) => !!row.tests_enabled }),
          field("runner_kind", "select", {
            required: true,
            options: options(["pool", "host", "host-group"]),
            resetOnChange: ["runner_ref"],
          }),
          field("runner_ref", "select", {
            required: true,
            when: (row) => row.runner_kind === "host",
            options: namedOptions(workspace, "hosts"),
          }),
          field("runner_ref", "select", {
            required: true,
            when: (row) => row.runner_kind === "host-group",
            options: namedOptions(workspace, "host-groups"),
          }),
          lines("runner_labels"),
        ],
      },
      {
        title: "ai_enabled",
        titleKey: key("ai_enabled"),
        fields: [
          field("ai_enabled", "switch"),
          ...(["ai_endpoint", "ai_model"] as const).map((name) =>
            field(name, "text", {
              required: true,
              when: (row) => !!row.ai_enabled,
            }),
          ),
          field("ai_credential_id", "select", {
            required: true,
            when: (row) => !!row.ai_enabled,
            options: namedOptions(workspace, "credentials"),
          }),
          ...[
            "ai_timeout_seconds",
            "ai_concurrency",
            "ai_max_tokens",
            "max_files",
            "max_bytes",
          ].map((name) =>
            field(name, "number", {
              required: true,
              min: 1,
              ...(name === "ai_concurrency" ? { max: 16 } : {}),
              when: (row) => !!row.ai_enabled,
            }),
          ),
          lines("include"),
          lines("exclude"),
        ],
      },
    ],
  };
}

import type { Navigation } from "../../shared/routing/types";
import { api } from "../../shared/api/api";
import { currentLocale } from "../../shared/i18n/locale";
import { base, deleteResource } from "../../shared/api/resource";
import { namedOptions } from "../../shared/resources/config";
import type {
  FormConfig,
  FormField,
  ListConfig,
  RowAction,
} from "../../shared/resources/types";
import { text, key } from "./messages";
import { ReportScope, type Row } from "./ReportScope";
export const periods = ["day", "week", "month", "quarter", "year"];
export const options = (values: string[]) =>
  values.map((value) => ({ value, label: value, labelKey: key(value) }));
export const date = (value: unknown) =>
  typeof value === "number" && value > 0
    ? new Date(value).toLocaleString(currentLocale())
    : "—";
export const field = (
  name: string,
  type: FormField<Row>["type"] = "text",
  extra: Partial<FormField<Row>> = {},
): FormField<Row> => ({
  key: name,
  label: name,
  labelKey: key(name),
  type,
  ...extra,
});
export function reportActions(workspace: string): RowAction<Row>[] {
  return [
    {
      label: "generate",
      labelKey: key("generate"),
      permission: "insight-reports.generate",
      initialValues: (row) => ({
        period: (row.periods as string[])?.[0] ?? "week",
      }),
      parameters: (row) => [
        field("period", "select", {
          required: true,
          options: options((row.periods as string[]) ?? periods),
        }),
        field("period_start", "text", {
          labelKey: key("periodStart"),
          placeholder: "YYYY-MM-DD",
        }),
      ],
      action: async (row, values) => {
        await api(
          `${base(workspace, "insight-reports")}/${encodeURIComponent(String(row.id))}/generate`,
          { method: "POST", body: JSON.stringify(values) },
        );
      },
    },
  ];
}
export function reportList(workspace: string): ListConfig<Row> {
  return {
    resource: "insight-reports",
    rowKey: (row) => String(row.id),
    filters: [{ key: "q", label: "name", labelKey: key("name"), type: "text" }],
    columns: [
      { key: "name", label: "name", labelKey: key("name") },
      {
        key: "dimension",
        label: "dimension",
        labelKey: key("dimension"),
        render: (row) => text(String(row.dimension)),
      },
      {
        key: "subjects",
        label: "objects",
        labelKey: key("objects"),
        render: (row) =>
          String(
            (
              (row.dimension === "application"
                ? row.applications
                : row.author_keys) as unknown[]
            )?.length ?? 0,
          ),
      },
      {
        key: "periods",
        label: "periods",
        labelKey: key("periods"),
        render: (row) => (row.periods as string[])?.map(text).join(" · "),
      },
      {
        key: "last_generated_at",
        label: "last",
        labelKey: key("last"),
        render: (row) => date(row.last_generated_at),
      },
      {
        key: "next_run_at",
        label: "next",
        labelKey: key("next"),
        render: (row) => date(row.next_run_at),
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
        permission: `insight-reports.${mode === "view" ? "read" : "update"}`,
        to: (row: Row) => ({
          group: "repository",
          item: "insight-reports",
          mode: mode as "view" | "edit",
          id: String(row.id),
        }),
      })),
      ...reportActions(workspace),
      {
        label: "delete",
        labelKey: key("delete"),
        permission: "insight-reports.delete",
        destructive: true,
        action: async (row) => {
          await deleteResource(workspace, "insight-reports", String(row.id));
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
export function reportForm(
  workspace: string,
  navigation?: Navigation,
): FormConfig<Row> {
  return {
    resource: "insight-reports",
    width: "full",
    presentation: "sections",
    heading: (row) => String(row.name ?? text("reports")),
    blank: {
      name: "",
      insight_id: new URLSearchParams(location.search).get("insight_id") ?? "",
      dimension: "person",
      periods: ["week"],
      author_keys: [],
      applications: [],
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      enabled: true,
    },
    incompleteMessage: "required",
    incompleteMessageKey: key("required"),
    viewActions: reportActions(workspace),
    sections: [
      {
        title: "configuration",
        titleKey: key("configuration"),
        fields: [
          field("name", "text", { required: true }),
          field("insight_id", "select", {
            labelKey: key("source"),
            required: true,
            options: namedOptions(workspace, "code-insights"),
            resetOnChange: ["author_keys", "applications"],
          }),
          field("periods", "multiselect", {
            required: true,
            options: options(periods).map((option) => ({
              ...option,
              group: "",
            })),
            span: 2,
          }),
          field("timezone", "text", { required: true }),
          field("enabled", "switch"),
        ],
      },
      {
        title: "dimension",
        titleKey: key("dimension"),
        fullWidth: true,
        fields: [
          field("scope", "custom", {
            span: 2,
            render: (props) => (
              <ReportScope {...props} {...(navigation ? { navigation } : {})} />
            ),
          }),
        ],
      },
    ],
  };
}

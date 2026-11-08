import { useEffect, useRef, useState } from "react";
import { Eye, Pencil } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import {
  form,
  list,
  type AnyRow,
  type ResourceConfigContext,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { WorkflowAIParameters } from "../workflows/WorkflowAIParameters";
import { WorkflowAssetContext } from "../workflows/WorkflowAssets";

const key = (s: string) => `models:${s}`;
const t = (s: string) => commonText(key(s));
function Catalog({
  workspace,
  draft,
  readOnly,
  onPatch,
}: {
  workspace: string;
  draft: Partial<AnyRow>;
  readOnly: boolean;
  onPatch: (p: Partial<AnyRow>) => void;
}) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const latest = useRef(draft),
    alive = useRef(true);
  latest.current = draft;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  async function discover() {
    const provider = draft.provider_id;
    setBusy(true);
    setError("");
    try {
      const r = await api<{ models: string[]; has_more: boolean }>(
        `/workspaces/${workspace}/ai-models/model-discoveries`,
        {
          method: "POST",
          body: JSON.stringify({ provider_id: provider }),
        },
      );
      if (!alive.current || latest.current.provider_id !== provider) return;
      const models = [
        ...new Set([...(latest.current.models ?? []), ...r.models]),
      ];
      onPatch({ models });
      if (r.has_more) setError(t("partialCatalog"));
    } catch (e) {
      if (alive.current && latest.current.provider_id === provider)
        setError(errorText(e));
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  return (
    <AutoField label={t("catalog")}>
      <div className="space-y-3">
        <textarea
          aria-label={t("catalog")}
          className="min-h-36 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
          disabled={readOnly}
          value={(draft.models ?? []).join("\n")}
          onChange={(e) => onPatch({ models: e.target.value.split("\n") })}
        />
        {!readOnly && (
          <Button
            type="button"
            variant="outline"
            disabled={busy || !draft.provider_id || draft.provider_id === "0"}
            onClick={() => void discover()}
          >
            {busy ? t("discovering") : t("discover")}
          </Button>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </div>
    </AutoField>
  );
}
function DefaultModel({
  draft,
  readOnly,
  onPatch,
}: {
  draft: Partial<AnyRow>;
  readOnly: boolean;
  onPatch: (p: Partial<AnyRow>) => void;
}) {
  return (
    <AutoField label={t("defaultModel")}>
      <div>
        <Input
          aria-label={t("defaultModel")}
          disabled={readOnly}
          list="model-catalog-options"
          value={draft.default_model ?? ""}
          onChange={(e) => onPatch({ default_model: e.target.value })}
        />
        <datalist id="model-catalog-options">
          {(draft.models ?? [])
            .filter((m: string) => m.trim())
            .map((m: string, i: number) => (
              <option key={i} value={m.trim()} />
            ))}
        </datalist>
      </div>
    </AutoField>
  );
}
export function modelsLists(
  _: ResourceConfigContext,
): Record<string, ListConfig<AnyRow>> {
  return {
    "ai-models": list<AnyRow>({
      resource: "ai-models",
      rowKey: (r) => r.id,
      filters: [
        { key: "q", label: "Search", labelKey: key("search"), type: "text" },
        {
          key: "kind",
          label: "Type",
          labelKey: key("kind"),
          type: "select",
          options: [
            { value: "api", label: "API" },
            { value: "cli", label: "CLI" },
          ],
        },
      ],
      columns: [
        { key: "name", label: "Name", labelKey: key("name"), sortable: true },
        {
          key: "kind",
          label: "Type",
          labelKey: key("kind"),
          render: (r) => r.kind.toUpperCase(),
        },
        {
          key: "runtime",
          label: "CLI",
          render: (r) =>
            r.runtime === "claude"
              ? "Claude Code"
              : r.runtime === "codex"
                ? "Codex"
                : "—",
        },
        {
          key: "default_model",
          label: "Default model",
          labelKey: key("defaultModel"),
          render: (r) => r.default_model || t("nativeDefault"),
        },
        {
          key: "state",
          label: "State",
          labelKey: key("state"),
          render: (r) => t(r.state),
        },
      ],
      actions: [
        {
          label: "View",
          icon: <Eye className="size-4" />,
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        {
          label: "Edit",
          icon: <Pencil className="size-4" />,
          labelKey: "common-domain:common.edit",
          permission: "ai-models.update",
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
      ],
      toolbar: { import: false, export: false },
      empty: "",
      emptyKey: key("empty"),
    }),
  };
}
export function modelsForms({
  permissions,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    "ai-models": form<AnyRow>({
      resource: "ai-models",
      heading: (r) => r.name,
      blank: {
        name: "",
        kind: "api",
        runtime: "",
        provider_id: "0",
        models: [],
        default_model: "",
        tool_version: "",
        state: "enabled",
      },
      sections: [
        {
          fields: [
            {
              key: "name",
              label: "Name",
              labelKey: key("name"),
              type: "text",
              required: true,
            },
            {
              key: "kind",
              label: "Type",
              labelKey: key("kind"),
              type: "select",
              required: true,
              resetOnChange: [
                "runtime",
                "tool_version",
                "provider_id",
                "models",
                "default_model",
              ],
              options: [
                { value: "api", label: "API" },
                { value: "cli", label: "CLI" },
              ],
            },
            {
              key: "runtime",
              label: "CLI tool",
              labelKey: key("runtime"),
              type: "select",
              required: true,
              when: (r) => r.kind === "cli",
              options: [
                { value: "codex", label: "Codex" },
                { value: "claude", label: "Claude Code" },
              ],
            },
            {
              key: "tool_version",
              label: "CLI version",
              labelKey: key("toolVersion"),
              type: "text",
              when: (r) => r.kind === "cli",
            },
            {
              key: "provider_id",
              label: "Provider",
              labelKey: key("provider"),
              type: "custom",
              span: 2,
              render: ({ workspace, draft, readOnly, onPatch }) =>
                !permissions?.has("credentials.read") ? (
                  <AutoField label={t("provider")}>
                    <p>
                      {draft.provider_id && draft.provider_id !== "0"
                        ? draft.provider_id
                        : t("nativeDefault")}
                    </p>
                  </AutoField>
                ) : (
                  <WorkflowAssetContext.Provider
                    value={{ workspace, workflow: "" }}
                  >
                    <WorkflowAIParameters
                      type="ai.llm.chat"
                      providerOnly
                      value={{
                        provider_id: draft.provider_id,
                        model: draft.default_model,
                      }}
                      readOnly={readOnly}
                      change={(v) => onPatch({ provider_id: v.provider_id })}
                    />
                  </WorkflowAssetContext.Provider>
                ),
            },
            {
              key: "models",
              label: "Model catalog",
              labelKey: key("catalog"),
              type: "custom",
              span: 2,
              render: (p) => <Catalog {...p} />,
            },
            {
              key: "default_model",
              label: "Default model",
              labelKey: key("defaultModel"),
              type: "custom",
              render: (p) => <DefaultModel {...p} />,
            },
            {
              key: "state",
              label: "State",
              labelKey: key("state"),
              type: "select",
              options: ["enabled", "disabled"].map((value) => ({
                value,
                label: value,
                labelKey: key(value),
              })),
            },
          ],
        },
      ],
    }),
  };
}

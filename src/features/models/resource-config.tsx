import { useEffect, useRef, useState } from "react";
import { Eye, Pencil } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { AutoField, FieldControl } from "../../shared/forms/auto-form";
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
import { Switch } from "../../shared/ui/switch";
import {
  ProviderFields,
  type Provider,
} from "../../shared/forms/ai-provider-fields";
import { matchPreset, ProviderPresetPicker, type ProviderPreset } from "./ProviderPresets";

const key = (s: string) => `models:${s}`;
const t = (s: string) => commonText(key(s));
const newConnection = (): Omit<Provider, "id"> => ({
  name: "",
  protocol: "openai",
  credential_id: "0",
  base_url: "https://api.openai.com/v1",
  currency: "USD",
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  monthly_limit_micros: 0,
  qps: 1,
  concurrency: 1,
  prices: [
    {
      model: "",
      operation: "ai.llm.chat",
      input_micros: 0,
      output_micros: 0,
      unit_size: 1,
    },
  ],
  version: 0,
});
type ApiModel = { id: string; name: string; provider_id: string; models: string[]; default_model: string };
/** A CLI with a custom API may only reuse the connection of an existing API model. */
function ApiModelSource({
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
  const [items, setItems] = useState<ApiModel[] | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    api<{ items: ApiModel[] }>(`/workspaces/${encodeURIComponent(workspace)}/ai-models?kind=api&page_size=100`)
      .then((r) => alive && setItems(r.items))
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [workspace]);
  const provider = String(draft.provider_id ?? "0");
  const current = items?.find((m) => m.provider_id === provider);
  if (readOnly)
    return (
      <AutoField label={t("apiModelSource")}>
        <p className="break-words text-sm">
          {items ? current?.name ?? t("apiModelMissing") : t("loadingConnection")}
        </p>
      </AutoField>
    );
  return (
    <AutoField label={t("apiModelSource")} required help={t("apiModelSourceHelp")}>
      <div className="space-y-2">
        <FieldControl
          name="api-model-source"
          field={{
            type: "select",
            name: t("apiModelSource"),
            width: "full",
            config: {
              placeholder: t("apiModelPlaceholder"),
              options: (items ?? []).map((m) => ({ value: m.id, label: m.name })),
            },
          }}
          value={current?.id ?? ""}
          required
          onChange={(id) => {
            const picked = items?.find((m) => m.id === id);
            if (picked)
              onPatch({
                provider_id: picked.provider_id,
                connection: null,
                models: [...picked.models],
                default_model: picked.default_model,
              });
          }}
        />
        {items && !items.length && <p className="text-sm text-muted-foreground">{t("apiModelNone")}</p>}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </div>
    </AutoField>
  );
}
function ModelConnection({
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
  useEffect(() => {
    if (!draft.connection && draft.kind === "api" && !readOnly)
      onPatch({ connection: newConnection() });
  }, [draft.connection, draft.kind, readOnly, onPatch]);
  const [preset, setPreset] = useState<ProviderPreset | undefined>(() =>
    matchPreset(draft.connection?.protocol, draft.connection?.base_url),
  );
  const custom = draft.kind === "cli" && (!!draft.provider_id && draft.provider_id !== "0" || draft.customApi === true);
  if (draft.kind === "cli")
    return (
      <div className="space-y-4">
        {!readOnly && (
          <AutoField label={t("customApi")}>
            <Switch
              checked={custom}
              onCheckedChange={(checked) =>
                onPatch({ customApi: checked, connection: null, provider_id: "0", models: [], default_model: "" })
              }
            />
          </AutoField>
        )}
        {custom ? (
          <ApiModelSource workspace={workspace} draft={draft} readOnly={readOnly} onPatch={onPatch} />
        ) : (
          readOnly && <p className="text-sm">{t("nativeDefault")}</p>
        )}
      </div>
    );
  if (!draft.connection) return null;
  return (
    <div className="space-y-4">
      {!readOnly && (
        <ProviderPresetPicker
          selected={preset}
          onSelect={(next) => {
            setPreset(next ?? undefined);
            if (!next) return;
            onPatch({
              connection: { ...draft.connection, protocol: next.protocol, base_url: next.base_url },
              ...(next.models.length ? { models: [...next.models], default_model: next.models[0] } : {}),
              ...(!String(draft.name ?? "").trim() ? { name: next.name } : {}),
            });
          }}
        />
      )}
      <ProviderFields
        draft={{
          ...draft.connection,
          id: draft.provider_id === "0" ? "" : draft.provider_id,
        }}
        onChange={({ id: _, ...connection }) => onPatch({ connection })}
        readOnly={readOnly}
        hideName
      />
    </div>
  );
}
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
  const native = draft.kind === "cli" && !draft.connection && (!draft.provider_id || draft.provider_id === "0");
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
    if (native) return;
    const provider = draft.provider_id;
    const connection = draft.connection;
    const runtime = draft.runtime;
    setBusy(true);
    setError("");
    try {
      const r = await api<{ models: string[]; has_more: boolean }>(
        `/workspaces/${workspace}/ai-models/model-discoveries`,
        {
          method: "POST",
          body: JSON.stringify({
            provider_id: provider || "0",
            ...(connection
              ? { connection: { ...connection, name: draft.name || t("name") } }
              : {}),
          }),
        },
      );
      if (
        !alive.current ||
        latest.current.provider_id !== provider ||
        latest.current.connection !== connection ||
        latest.current.runtime !== runtime
      )
        return;
      const models = [
        ...new Set([...(latest.current.models ?? []), ...r.models]),
      ];
      onPatch({ models });
      if (r.has_more) setError(t("partialCatalog"));
    } catch (e) {
      if (
        alive.current &&
        latest.current.provider_id === provider &&
        latest.current.connection === connection &&
        latest.current.runtime === runtime
      )
        setError(errorText(e));
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  return (
    <AutoField label={t("catalog")}>
      <div className="space-y-3">
        {readOnly ? (
          <p className="whitespace-pre-wrap break-words text-sm">
            {(draft.models ?? []).join("\n") || "\u2014"}
          </p>
        ) : (
          <div className="space-y-2">
            {(draft.models ?? []).map((model: string, index: number) => (
              <div key={index} className="flex min-w-0 items-center gap-2">
                <Input
                  aria-label={`${t("catalog")} ${index + 1}`}
                  value={model}
                  className="min-w-0 flex-1"
                  onChange={(e) =>
                    onPatch({
                      models: draft.models.map((v: string, i: number) =>
                        i === index ? e.target.value : v,
                      ),
                    })
                  }
                />
                <Button
                  type="button"
                  variant="outline"
                  className="min-h-11 shrink-0"
                  aria-label={`${commonText("common.delete")} ${index + 1}`}
                  onClick={() =>
                    onPatch({
                      models: draft.models.filter(
                        (_: string, i: number) => i !== index,
                      ),
                      ...(draft.default_model === model
                        ? { default_model: "" }
                        : {}),
                    })
                  }
                >
                  {commonText("common.delete")}
                </Button>
              </div>
            ))}
            <Button
              type="button"
              variant="outline"
              className="min-h-11"
              onClick={() => onPatch({ models: [...(draft.models ?? []), ""] })}
            >
              {commonText("common.add")}
            </Button>
          </div>
        )}
        {!readOnly && native && <p className="text-sm text-muted-foreground">{t("nativeCatalogAtExecution")}</p>}
        {!readOnly && !native && (
          <Button
            type="button"
            variant="outline"
            disabled={
              busy ||
              (!draft.connection && (!draft.provider_id || draft.provider_id === "0"))
            }
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
        {readOnly ? (
          <p className="break-words text-sm">
            {draft.default_model || t("nativeDefault")}
          </p>
        ) : (
          <Input
            aria-label={t("defaultModel")}
            disabled={readOnly}
            list="model-catalog-options"
            value={draft.default_model ?? ""}
            onChange={(e) => onPatch({ default_model: e.target.value })}
          />
        )}
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
            ({
              codex: "Codex",
              claude: "Claude Code",
              gemini: "Gemini CLI",
              qwen: "Qwen Code",
              opencode: "OpenCode",
              pi: "Pi",
              dsh: "DeepSeek Harness",
            })[r.runtime as string] ?? "—",
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
      width: "full",
      heading: (r) => r.name,
      // A CLI reuses an API model's connection by reference and must never resave it.
      editValues: (r) => (r.kind === "cli" ? { ...r, connection: null } : r),
      blank: {
        name: "",
        kind: "api",
        runtime: "",
        provider_id: "0",
        connection: newConnection(),
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
              type: "custom",
              width: "full",
              required: true,
              render: ({ draft, readOnly, onPatch }) => (
                <AutoField label={t("kind")} required={!readOnly}>
                  <FieldControl
                    name="kind"
                    field={{
                      type: "select",
                      name: t("kind"),
                      width: "full",
                      config: {
                        options: [
                          { value: "api", label: "API" },
                          { value: "cli", label: "CLI" },
                        ],
                      },
                    }}
                    value={draft.kind}
                    readonly={readOnly}
                    required
                    onChange={(kind) =>
                      onPatch({
                        kind,
                        runtime: "",
                        tool_version: "",
                        provider_id: "0",
                        connection: kind === "api" ? newConnection() : null,
                        models: [],
                        default_model: "",
                      })
                    }
                  />
                </AutoField>
              ),
            },
            {
              key: "runtime",
              label: "CLI tool",
              labelKey: key("runtime"),
              type: "select",
              width: "full",
              required: true,
              when: (r) => r.kind === "cli",
              options: [
                { value: "codex", label: "Codex" },
                { value: "claude", label: "Claude Code" },
                { value: "gemini", label: "Gemini CLI" },
                { value: "qwen", label: "Qwen Code" },
                { value: "opencode", label: "OpenCode" },
                { value: "pi", label: "Pi" },
                { value: "dsh", label: "DeepSeek Harness" },
                { value: "kimi", label: "Kimi CLI" },
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
              render: (props) =>
                !permissions?.has("credentials.read") ? (
                  <AutoField label={t("provider")}>
                    <p>{t("connectionRestricted")}</p>
                  </AutoField>
                ) : (
                  <ModelConnection {...props} />
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
              width: "full",
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

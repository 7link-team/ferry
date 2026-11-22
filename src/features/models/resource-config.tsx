import { EffectiveTimezone } from "../../shared/timezone";
import { useEffect, useRef, useState } from "react";
import { Eye, Pencil } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { AutoField, FieldControl } from "../../shared/forms/auto-form";
import { TimezoneField } from "../../shared/forms/TimezoneField";
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
import {
  ProviderFields,
  type Provider,
} from "../../shared/forms/ai-provider-fields";
import { matchPreset, ProviderPresetPicker, type ProviderPreset } from "./ProviderPresets";
import { CLI_TOOLS, CliConnection, cliToolOptions } from "./CliConnection";
import { ModelHosts } from "./ModelHosts";
import { CliCheck } from "./CliCheck";

const key = (s: string) => `models:${s}`;
const t = (s: string) => commonText(key(s));
const newConnection = (): Omit<Provider, "id"> => ({
  name: "",
  protocol: "openai",
  credential_id: "0",
  base_url: "https://api.openai.com/v1",
  qps: 1,
  concurrency: 1,
  version: 0,
});
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
  if (draft.kind === "cli") return <CliConnection workspace={workspace} draft={draft} readOnly={readOnly} onPatch={onPatch} />;
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
    <AutoField
      label={t("defaultModel")}
      required={!readOnly && draft.kind === "api"}
    >
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
function OutputTokens({ draft, readOnly, onPatch, onValidityChange }: {
  draft: Partial<AnyRow>;
  readOnly: boolean;
  onPatch: (patch: Partial<AnyRow>) => void;
  onValidityChange: (blocked: boolean) => void;
}) {
  const value = draft.max_output_tokens;
  const invalid = value != null && (!Number.isSafeInteger(value) || value < 1);
  const validity = useRef(onValidityChange);
  validity.current = onValidityChange;
  useEffect(() => {
    validity.current(!readOnly && invalid);
    return () => validity.current(false);
  }, [invalid, readOnly]);
  return <AutoField label={t("maxOutputTokens")} help={t("maxOutputTokensHelp")}>
    <FieldControl name="max_output_tokens" field={{ name: t("maxOutputTokens"), type: "number", config: { min: 1, max: Number.MAX_SAFE_INTEGER } }}
      value={value ?? ""} readonly={readOnly} onChange={next => onPatch({ max_output_tokens: next === "" ? null : next })} />
    {invalid && !readOnly && <p role="alert" className="text-sm text-destructive">{t("maxOutputInvalid")}</p>}
  </AutoField>;
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
          render: (r) => CLI_TOOLS[r.runtime as string]?.label ?? "—",
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
  workspace,
  permissions,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    "ai-models": form<AnyRow>({
      resource: "ai-models",
      width: "full",
      heading: (r) => r.name,
      blank: {
        name: "",
        kind: "api",
        runtime: "",
        provider_id: "0",
        connection: newConnection(),
        models: [],
        default_model: "",
        max_output_tokens: null,
        tool_version: "",
        state: "enabled",
        cli_options: null,
        timezone: "",
        hosts: [],
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
                        cli_options: null,
                        connection: kind === "api" ? newConnection() : null,
                        models: [],
                        default_model: "",
                        max_output_tokens: null,
                        timezone: "",
                        hosts: [],
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
              options: cliToolOptions,
            },
            {
              key: "tool_version",
              label: "CLI version",
              labelKey: key("toolVersion"),
              type: "text",
              when: (r) => r.kind === "cli",
            },
            {
              key: "timezone",
              label: "Time zone",
              labelKey: key("timezone"),
              type: "custom",
              width: "full",
              // Only a CLI model runs on a Runner; an API model runs on the server.
              when: (r) => r.kind === "cli",
              render: ({ draft, readOnly, onPatch, onValidityChange }) => (
                <EffectiveTimezone onValidityChange={onValidityChange}><TimezoneField
                  label={t("timezone")}
                  value={draft.timezone ?? ""}
                  emptyLabel={t("timezoneEmpty")}
                  help={t("timezoneHelp")}
                  readOnly={readOnly}
                  onChange={(timezone) => onPatch({ timezone })}
                  onValidityChange={onValidityChange}
                /></EffectiveTimezone>
              ),
            },
            {
              key: "hosts",
              label: "Execution hosts",
              labelKey: key("executionHosts"),
              type: "custom",
              width: "full",
              span: 2,
              // Where the CLI runs; an API model runs on the server.
              when: (r) => r.kind === "cli",
              render: ({ workspace, draft, readOnly, onPatch }) => (
                <ModelHosts
                  workspace={workspace}
                  value={draft.hosts ?? []}
                  readOnly={readOnly}
                  onChange={(hosts) => onPatch({ hosts })}
                />
              ),
            },
            {
              key: "cli_check",
              label: "Check execution hosts",
              labelKey: key("cliCheck"),
              type: "custom",
              span: 2,
              when: (draft) => draft.kind === "cli",
              render: ({ draft }) => (
                <CliCheck
                  workspace={workspace}
                  runtime={String(draft.runtime ?? "")}
                  hosts={(draft.hosts ?? []) as string[]}
                  disabled={!permissions?.has("ai-agents.probe")}
                />
              ),
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
              // An API model calls one concrete model; the server rejects an empty one.
              required: (draft) => draft.kind === "api",
              render: (p) => <DefaultModel {...p} />,
            },
            {
              key: "max_output_tokens",
              label: "Conversation output tokens",
              labelKey: key("maxOutputTokens"),
              type: "custom",
              when: draft => draft.kind === "api",
              render: props => <OutputTokens {...props} />,
            },
            {
              key: "skip_check",
              label: "Save without checking the connection",
              labelKey: key("skipCheck"),
              type: "switch",
              helpKey: key("skipCheckHelp"),
              when: (draft) => draft.kind === "api",
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

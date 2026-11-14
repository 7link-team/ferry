import { useEffect, useMemo, useState } from "react";
import { api } from "../../shared/api/api";
import { AutoField, FieldControl } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import { CLI_TOOL_LABELS } from "../../shared/forms/cli-tools";
import type { AnyRow } from "../../shared/resources/config";
import { Button } from "../../shared/ui/button";
import { Tabs, TabsList, TabsTrigger } from "../../shared/ui/tabs";
import { Input } from "../../shared/ui/input";
import { Switch } from "../../shared/ui/switch";
import {
  ProviderPresetPicker,
  matchPreset,
  providerPresets,
  type ProviderPreset,
} from "./ProviderPresets";

const t = (s: string, values?: Record<string, unknown>) =>
  commonText(`models:${s}`, values);

export type CliFormat =
  "anthropic" | "openai_chat" | "openai_responses" | "gemini";
type Option =
  | { key: string; kind: "text" | "number"; advanced?: boolean }
  | { key: string; kind: "switch"; advanced?: boolean }
  | { key: string; kind: "select"; values: string[]; advanced?: boolean };

const efforts = ["minimal", "low", "medium", "high", "xhigh"];
/** Mirrors ferry_protocol::agent::Runtime::provider_formats and the Runner's config writers. */
const TOOLS: Record<string, { formats: CliFormat[]; options: Option[] }> = {
  claude: {
    formats: ["anthropic"],
    options: [
      {
        key: "auth_field",
        kind: "select",
        values: ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"],
        advanced: true,
      },
      { key: "sonnet_model", kind: "text", advanced: true },
      { key: "opus_model", kind: "text", advanced: true },
      { key: "haiku_model", kind: "text", advanced: true },
      { key: "fable_model", kind: "text", advanced: true },
      { key: "subagent_model", kind: "text", advanced: true },
      { key: "hide_attribution", kind: "switch", advanced: true },
      { key: "tool_search", kind: "switch", advanced: true },
    ],
  },
  codex: {
    formats: ["openai_responses"],
    options: [
      { key: "reasoning_effort", kind: "select", values: efforts },
      { key: "context_window", kind: "number", advanced: true },
      { key: "auto_compact_limit", kind: "number", advanced: true },
      { key: "subagent_model", kind: "text", advanced: true },
      {
        key: "subagent_reasoning_effort",
        kind: "select",
        values: efforts,
        advanced: true,
      },
    ],
  },
  gemini: { formats: ["gemini"], options: [] },
  qwen: { formats: ["openai_chat"], options: [] },
  opencode: {
    formats: ["openai_chat", "openai_responses", "anthropic", "gemini"],
    options: [
      { key: "context_window", kind: "number", advanced: true },
      { key: "max_output_tokens", kind: "number", advanced: true },
    ],
  },
  pi: {
    formats: ["openai_chat", "openai_responses", "anthropic", "gemini"],
    options: [
      { key: "reasoning", kind: "switch" },
      { key: "context_window", kind: "number", advanced: true },
      { key: "max_output_tokens", kind: "number", advanced: true },
    ],
  },
  kimi: {
    formats: ["openai_chat", "openai_responses", "anthropic", "gemini"],
    options: [
      { key: "reasoning", kind: "switch" },
      { key: "image_input", kind: "switch" },
      { key: "context_window", kind: "number", advanced: true },
    ],
  },
  grok: {
    formats: ["openai_responses", "openai_chat"],
    options: [{ key: "context_window", kind: "number", advanced: true }],
  },
  dsh: { formats: [], options: [] },
  copilot: { formats: [], options: [] },
  codebuddy: { formats: [], options: [] },
};
const HEADERS: Record<string, boolean> = {
  claude: true,
  codex: true,
  opencode: true,
  pi: true,
  kimi: true,
};

export const CLI_TOOLS = Object.fromEntries(
  Object.entries(TOOLS).map(([id, tool]) => [
    id,
    { ...tool, label: CLI_TOOL_LABELS[id] ?? id },
  ]),
);
export const cliToolOptions = Object.entries(CLI_TOOLS).map(
  ([value, tool]) => ({ value, label: tool.label }),
);

const presetFormat = (
  preset: ProviderPreset,
  formats: CliFormat[],
): CliFormat | undefined =>
  preset.protocol === "anthropic"
    ? formats.find((f) => f === "anthropic")
    : (formats.find((f) => f === "openai_chat") ??
      formats.find((f) => f === "openai_responses"));

export const newCliConnection = () => ({
  name: "",
  protocol: "openai",
  credential_id: "0",
  base_url: "",
  qps: 1,
  concurrency: 1,
  version: 0,
});

type ApiModel = {
  id: string;
  name: string;
  provider_id: string;
  models: string[];
  default_model: string;
};
type Source = "api" | "own";
/** Formats an existing API connection can serve to this CLI without conversion. */
const apiFormats = (
  protocol: string | undefined,
  formats: CliFormat[],
): CliFormat[] =>
  protocol === "anthropic"
    ? formats.filter((f) => f === "anthropic")
    : protocol === "openai"
      ? formats.filter((f) => f === "openai_chat" || f === "openai_responses")
      : [];

/** API models (with their connection protocol) a CLI may reference. */
function useApiModels(workspace: string, enabled: boolean) {
  const [state, setState] = useState<{
    models: (ApiModel & { protocol: string | undefined })[];
    loaded: boolean;
  }>({ models: [], loaded: false });
  useEffect(() => {
    if (!enabled || !workspace) return;
    let alive = true;
    const base = `/workspaces/${encodeURIComponent(workspace)}`;
    Promise.all([
      api<{ items: ApiModel[] }>(`${base}/ai-models?kind=api&page_size=100`),
      api<{ items: { id: string; protocol: string }[] }>(
        `${base}/ai-providers`,
      ).catch(() => ({ items: [] })),
    ])
      .then(([models, providers]) => {
        if (!alive) return;
        const protocol = new Map(
          providers.items.map((p) => [String(p.id), p.protocol]),
        );
        setState({
          models: models.items.map((m) => ({
            ...m,
            protocol: protocol.get(String(m.provider_id)),
          })),
          loaded: true,
        });
      })
      .catch(() => alive && setState({ models: [], loaded: true }));
    return () => {
      alive = false;
    };
  }, [workspace, enabled]);
  return state;
}

type Props = {
  workspace: string;
  draft: Partial<AnyRow>;
  readOnly: boolean;
  onPatch: (p: Partial<AnyRow>) => void;
};

/** Per-CLI upstream: off keeps the CLI's own login/configuration; on writes an isolated config on the Runner. */
export function CliConnection({ workspace, draft, readOnly, onPatch }: Props) {
  const tool = CLI_TOOLS[String(draft.runtime ?? "")];
  const formats = useMemo(() => tool?.formats ?? [], [tool]);
  const provider = String(draft.provider_id ?? "0");
  const custom = !!draft.connection || provider !== "0" || !!draft.cliSource;
  const options: Record<string, unknown> =
    (draft.cli_options as Record<string, unknown>) ?? {};
  const [preset, setPreset] = useState<ProviderPreset | undefined>(() =>
    matchPreset(draft.connection?.protocol, draft.connection?.base_url),
  );
  const { models: apiModels, loaded } = useApiModels(
    workspace,
    custom && formats.length > 0,
  );
  const usable = apiModels.filter(
    (m) => apiFormats(m.protocol, formats).length,
  );
  const linked = apiModels.find(
    (m) => String(m.provider_id) === provider && provider !== "0",
  );
  // A saved CLI that references an API model's connection keeps referencing it.
  const source: Source =
    (draft.cliSource as Source) ??
    (linked || (!draft.connection && provider !== "0")
      ? "api"
      : draft.connection
        ? "own"
        : usable.length
          ? "api"
          : "own");
  const allowed =
    source === "api"
      ? linked
        ? apiFormats(linked.protocol, formats)
        : formats
      : formats;
  const format = String(options.format ?? allowed[0] ?? "");
  // An existing connection loaded for editing keeps its stored key unless replaced.
  const connection =
    source === "own" ? (draft.connection ?? newCliConnection()) : null;
  useEffect(() => {
    if (readOnly) return;
    // A tool switch can leave a custom API the new CLI cannot use.
    if (custom && !formats.length)
      onPatch({
        connection: null,
        cli_options: null,
        provider_id: "0",
        models: [],
        default_model: "",
        cliSource: undefined,
      });
    // Editing a CLI linked to an API model must never resave that model's connection.
    else if (linked && !draft.cliSource)
      onPatch({ cliSource: "api", connection: null });
    else if (custom && source === "own" && !draft.connection)
      onPatch({ connection: newCliConnection() });
    else if (
      custom &&
      options.format !== undefined &&
      !allowed.includes(options.format as CliFormat) &&
      allowed.length
    )
      onPatch({ cli_options: { ...options, format: allowed[0] } });
  }, [
    custom,
    draft.connection,
    draft.cliSource,
    readOnly,
    onPatch,
    formats,
    options,
    linked,
    source,
    allowed,
  ]);
  if (!tool) return null;
  const setOption = (key: string, value: unknown) => {
    const next: Record<string, unknown> = { ...options, format };
    if (
      value === "" ||
      value === false ||
      value === undefined ||
      (typeof value === "number" && !value)
    )
      delete next[key];
    else next[key] = value;
    onPatch({ cli_options: next });
  };
  if (!formats.length)
    return (
      <AutoField label={t("cliConnection")}>
        <p className="text-sm text-muted-foreground">
          {t("cliNativeOnly", { tool: tool.label })}
        </p>
      </AutoField>
    );
  if (readOnly)
    return (
      <AutoField label={t("cliConnection")}>
        {custom ? (
          <dl className="grid min-w-0 gap-3 sm:grid-cols-2">
            {[
              [t("cliFormat"), t(`format.${format}`)],
              linked
                ? [t("apiModelSource"), linked.name]
                : [t("cliBaseUrl"), draft.connection?.base_url],
              ...Object.entries(options)
                .filter(([k]) => k !== "format" && k !== "headers")
                .map(([k, v]) => [
                  t(`opt.${k}`),
                  typeof v === "boolean" ? (v ? "✓" : "—") : String(v),
                ]),
            ].map(([k, v]) => (
              <div key={String(k)} className="min-w-0">
                <dt className="text-sm text-muted-foreground">{k}</dt>
                <dd className="break-words text-sm">{String(v ?? "—")}</dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="text-sm">{t("cliNativeConfig")}</p>
        )}
      </AutoField>
    );
  const field = (option: Option) => {
    const value = options[option.key];
    const label = t(`opt.${option.key}`);
    const help = t(`optHelp.${option.key}`);
    if (option.kind === "switch")
      return (
        <AutoField key={option.key} label={label} help={help}>
          <Switch
            aria-label={label}
            checked={value === true}
            onCheckedChange={(v) => setOption(option.key, v)}
          />
        </AutoField>
      );
    if (option.kind === "select")
      return (
        <AutoField key={option.key} label={label} help={help}>
          <FieldControl
            name={`cli-${option.key}`}
            field={{
              type: "select",
              name: label,
              width: "full",
              config: {
                placeholder: t("cliDefault"),
                options: option.values.map((v) => ({ value: v, label: v })),
              },
            }}
            value={String(value ?? "")}
            onChange={(v) => setOption(option.key, String(v ?? ""))}
          />
        </AutoField>
      );
    return (
      <AutoField key={option.key} label={label} help={help}>
        <Input
          aria-label={label}
          className="min-h-11"
          type={option.kind === "number" ? "number" : "text"}
          min={option.kind === "number" ? 1 : undefined}
          step={option.kind === "number" ? 1 : undefined}
          placeholder={t("cliDefault")}
          value={value === undefined ? "" : String(value)}
          onChange={(e) =>
            setOption(
              option.key,
              option.kind === "number"
                ? Math.max(0, Math.floor(Number(e.target.value) || 0))
                : e.target.value,
            )
          }
        />
      </AutoField>
    );
  };
  const headers = Object.entries(
    (options.headers as Record<string, string>) ?? {},
  );
  const setHeaders = (rows: [string, string][]) =>
    setOption("headers", rows.length ? Object.fromEntries(rows) : undefined);
  const presets = providerPresets.filter((p) => presetFormat(p, formats));
  const main = tool.options.filter((o) => !o.advanced);
  const advanced = tool.options.filter((o) => o.advanced);
  return (
    <div className="space-y-4">
      <AutoField label={t("customApi")} help={t("cliCustomHelp")}>
        <Switch
          aria-label={t("customApi")}
          checked={custom}
          onCheckedChange={(checked) =>
            onPatch(
              checked
                ? usable.length || !loaded
                  ? {
                      cliSource: "api",
                      connection: null,
                      cli_options: { format: formats[0] },
                      provider_id: "0",
                    }
                  : {
                      cliSource: "own",
                      connection: {
                        ...newCliConnection(),
                        protocol:
                          formats[0] === "anthropic" ? "anthropic" : "openai",
                      },
                      cli_options: { format: formats[0] },
                      provider_id: "0",
                    }
                : {
                    connection: null,
                    cli_options: null,
                    provider_id: "0",
                    models: [],
                    default_model: "",
                    cliSource: undefined,
                  },
            )
          }
        />
      </AutoField>
      {!custom ? (
        <p className="text-sm text-muted-foreground">
          {t("cliNativeHelp", { tool: tool.label })}
        </p>
      ) : (
        <>
          <Tabs
            value={source}
            onValueChange={(value) =>
              value !== source &&
              onPatch(
                value === "api"
                  ? {
                      cliSource: "api",
                      connection: null,
                      provider_id: "0",
                      models: [],
                      default_model: "",
                    }
                  : {
                      cliSource: "own",
                      connection: {
                        ...newCliConnection(),
                        protocol:
                          format === "anthropic" ? "anthropic" : "openai",
                      },
                      provider_id: "0",
                      models: [],
                      default_model: "",
                    },
              )
            }
            className="min-w-0 gap-0 overflow-hidden rounded-lg border bg-card"
          >
            <TabsList
              variant="line"
              aria-label={t("cliSource")}
              className="h-auto w-full justify-start overflow-x-auto border-b px-2"
            >
              {(["api", "own"] as const).map((value) => (
                <TabsTrigger
                  key={value}
                  value={value}
                  className="min-h-11 flex-none px-3"
                >
                  {t(value === "api" ? "cliSourceApi" : "cliSourceOwn")}
                </TabsTrigger>
              ))}
            </TabsList>
            <div className="min-w-0 space-y-4 p-3 sm:p-4">
              {source === "own" && !!presets.length && (
                <ProviderPresetPicker
                  selected={preset}
                  presets={presets}
                  onSelect={(next) => {
                    setPreset(next ?? undefined);
                    if (!next || !connection) return;
                    onPatch({
                      connection: {
                        ...connection,
                        base_url: next.base_url,
                        protocol: next.protocol,
                      },
                      cli_options: {
                        ...options,
                        format: presetFormat(next, formats),
                      },
                      ...(next.models.length
                        ? {
                            models: [...next.models],
                            default_model: next.models[0],
                          }
                        : {}),
                      ...(!String(draft.name ?? "").trim()
                        ? { name: `${tool.label} · ${next.name}` }
                        : {}),
                    });
                  }}
                />
              )}
              {source === "api" ? (
                <div className="grid min-w-0 gap-4 sm:grid-cols-2">
                  <div className="col-span-full">
                    <AutoField
                      label={t("apiModelSource")}
                      required
                      help={t("apiModelSourceHelp")}
                    >
                      <div className="space-y-2">
                        <FieldControl
                          name="cli-api-model"
                          field={{
                            type: "select",
                            name: t("apiModelSource"),
                            width: "full",
                            config: {
                              placeholder: t("apiModelPlaceholder"),
                              options: usable.map((m) => ({
                                value: m.id,
                                label: m.name,
                              })),
                            },
                          }}
                          value={linked?.id ?? ""}
                          required
                          onChange={(id) => {
                            const picked = usable.find((m) => m.id === id);
                            if (!picked) return;
                            const fits = apiFormats(picked.protocol, formats);
                            onPatch({
                              cliSource: "api",
                              provider_id: picked.provider_id,
                              connection: null,
                              models: [...picked.models],
                              default_model: picked.default_model,
                              cli_options: {
                                ...options,
                                format: fits.includes(format as CliFormat)
                                  ? format
                                  : fits[0],
                              },
                            });
                          }}
                        />
                        {loaded && !usable.length && (
                          <p className="text-sm text-muted-foreground">
                            {t("apiModelNone", { tool: tool.label })}
                          </p>
                        )}
                      </div>
                    </AutoField>
                  </div>
                  {allowed.length > 1 && (
                    <AutoField
                      label={t("cliFormat")}
                      required
                      help={t("cliFormatHelp")}
                    >
                      <FieldControl
                        name="cli-format"
                        field={{
                          type: "select",
                          name: t("cliFormat"),
                          width: "full",
                          config: {
                            options: allowed.map((f) => ({
                              value: f,
                              label: t(`format.${f}`),
                            })),
                          },
                        }}
                        value={format}
                        required
                        onChange={(v) =>
                          onPatch({
                            cli_options: { ...options, format: String(v) },
                          })
                        }
                      />
                    </AutoField>
                  )}
                  {main.map(field)}
                </div>
              ) : (
                <div className="grid min-w-0 gap-4 sm:grid-cols-2">
                  <AutoField
                    label={t("cliFormat")}
                    required
                    help={t("cliFormatHelp")}
                  >
                    <FieldControl
                      name="cli-format"
                      field={{
                        type: "select",
                        name: t("cliFormat"),
                        width: "full",
                        config: {
                          options: formats.map((f) => ({
                            value: f,
                            label: t(`format.${f}`),
                          })),
                        },
                      }}
                      value={format}
                      required
                      onChange={(v) =>
                        onPatch({
                          cli_options: { ...options, format: String(v) },
                          connection: {
                            ...connection,
                            protocol:
                              v === "anthropic" ? "anthropic" : "openai",
                          },
                        })
                      }
                    />
                  </AutoField>
                  <div className="hidden sm:block" />
                  <div className="col-span-full">
                    <AutoField
                      label={t("cliBaseUrl")}
                      required
                      help={t(`baseUrlHelp.${format}`)}
                    >
                      <Input
                        aria-label={t("cliBaseUrl")}
                        className="min-h-11"
                        required
                        type="url"
                        placeholder={t(`baseUrlPlaceholder.${format}`)}
                        value={connection?.base_url ?? ""}
                        onChange={(e) =>
                          onPatch({
                            connection: {
                              ...connection,
                              base_url: e.target.value,
                            },
                          })
                        }
                      />
                    </AutoField>
                  </div>
                  <div className="col-span-full">
                    <AutoField
                      label="API Key"
                      required={!draft.provider_id || draft.provider_id === "0"}
                      help={
                        draft.provider_id && draft.provider_id !== "0"
                          ? t("cliKeyKeep")
                          : t("cliKeyHelp")
                      }
                    >
                      <Input
                        aria-label="API Key"
                        className="min-h-11"
                        type="password"
                        autoComplete="new-password"
                        required={
                          !draft.provider_id || draft.provider_id === "0"
                        }
                        value={connection?.api_key ?? ""}
                        onChange={(e) =>
                          onPatch({
                            connection: {
                              ...connection,
                              api_key: e.target.value,
                            },
                          })
                        }
                      />
                    </AutoField>
                  </div>
                  {main.map(field)}
                </div>
              )}
            </div>
          </Tabs>
          {(!!advanced.length || HEADERS[String(draft.runtime)]) && (
            <details
              className="rounded-lg border p-3"
              open={
                advanced.some((o) => options[o.key] !== undefined) ||
                !!headers.length
              }
            >
              <summary className="min-h-11 cursor-pointer select-none py-2 text-sm font-medium">
                {t("cliAdvanced")}
              </summary>
              <div className="mt-3 grid min-w-0 gap-4 sm:grid-cols-2">
                {advanced.map(field)}
                {HEADERS[String(draft.runtime)] && (
                  <div className="col-span-full">
                    <AutoField
                      label={t("opt.headers")}
                      help={t("optHelp.headers")}
                    >
                      <div className="space-y-2">
                        {headers.map(([name, value], index) => (
                          <div
                            key={index}
                            className="flex min-w-0 flex-wrap items-center gap-2 sm:flex-nowrap"
                          >
                            <Input
                              aria-label={`${t("headerName")} ${index + 1}`}
                              placeholder={t("headerName")}
                              className="min-h-11 min-w-0 flex-1"
                              value={name}
                              onChange={(e) =>
                                setHeaders(
                                  headers.map((h, i) =>
                                    i === index ? [e.target.value, h[1]] : h,
                                  ),
                                )
                              }
                            />
                            <Input
                              aria-label={`${t("headerValue")} ${index + 1}`}
                              placeholder={t("headerValue")}
                              className="min-h-11 min-w-0 flex-1"
                              value={value}
                              onChange={(e) =>
                                setHeaders(
                                  headers.map((h, i) =>
                                    i === index ? [h[0], e.target.value] : h,
                                  ),
                                )
                              }
                            />
                            <Button
                              type="button"
                              variant="outline"
                              className="min-h-11 shrink-0"
                              aria-label={`${commonText("common.delete")} ${index + 1}`}
                              onClick={() =>
                                setHeaders(
                                  headers.filter((_, i) => i !== index),
                                )
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
                          onClick={() =>
                            setHeaders([
                              ...headers,
                              [`X-Header-${headers.length + 1}`, ""],
                            ])
                          }
                        >
                          {commonText("common.add")}
                        </Button>
                      </div>
                    </AutoField>
                  </div>
                )}
              </div>
            </details>
          )}
        </>
      )}
    </div>
  );
}

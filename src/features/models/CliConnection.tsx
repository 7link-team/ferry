import { useEffect, useState } from "react";
import { AutoField, FieldControl } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import { CLI_TOOL_LABELS } from "../../shared/forms/cli-tools";
import type { AnyRow } from "../../shared/resources/config";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { Switch } from "../../shared/ui/switch";
import { ProviderPresetPicker, matchPreset, providerPresets, type ProviderPreset } from "./ProviderPresets";

const t = (s: string, values?: Record<string, unknown>) => commonText(`models:${s}`, values);

export type CliFormat = "anthropic" | "openai_chat" | "openai_responses" | "gemini";
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
      { key: "auth_field", kind: "select", values: ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"], advanced: true },
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
      { key: "subagent_reasoning_effort", kind: "select", values: efforts, advanced: true },
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
const HEADERS: Record<string, boolean> = { claude: true, codex: true, opencode: true, pi: true, kimi: true };

export const CLI_TOOLS = Object.fromEntries(
  Object.entries(TOOLS).map(([id, tool]) => [id, { ...tool, label: CLI_TOOL_LABELS[id] ?? id }]),
);
export const cliToolOptions = Object.entries(CLI_TOOLS).map(([value, tool]) => ({ value, label: tool.label }));

const presetFormat = (preset: ProviderPreset, formats: CliFormat[]): CliFormat | undefined =>
  preset.protocol === "anthropic"
    ? formats.find((f) => f === "anthropic")
    : formats.find((f) => f === "openai_chat") ?? formats.find((f) => f === "openai_responses");

export const newCliConnection = () => ({
  name: "",
  protocol: "openai",
  credential_id: "0",
  base_url: "",
  qps: 1,
  concurrency: 1,
  version: 0,
});

type Props = {
  draft: Partial<AnyRow>;
  readOnly: boolean;
  onPatch: (p: Partial<AnyRow>) => void;
};

/** Per-CLI upstream: off keeps the CLI's own login/configuration; on writes an isolated config on the Runner. */
export function CliConnection({ draft, readOnly, onPatch }: Props) {
  const tool = CLI_TOOLS[String(draft.runtime ?? "")];
  const formats = tool?.formats ?? [];
  const custom = !!draft.connection || (!!draft.provider_id && draft.provider_id !== "0");
  const options: Record<string, unknown> = (draft.cli_options as Record<string, unknown>) ?? {};
  const format = String(options.format ?? formats[0] ?? "");
  const [preset, setPreset] = useState<ProviderPreset | undefined>(() =>
    matchPreset(draft.connection?.protocol, draft.connection?.base_url),
  );
  // An existing connection loaded for editing keeps its stored key unless replaced.
  const connection = draft.connection ?? (custom ? newCliConnection() : null);
  useEffect(() => {
    if (readOnly) return;
    if (custom && !draft.connection) onPatch({ connection: newCliConnection() });
    // A tool switch can leave a format (or a custom API) the new CLI cannot use.
    else if (custom && !formats.length)
      onPatch({ connection: null, cli_options: null, provider_id: "0", models: [], default_model: "" });
    else if (custom && options.format !== undefined && !formats.includes(options.format as CliFormat))
      onPatch({ cli_options: { ...options, format: formats[0] } });
  }, [custom, draft.connection, readOnly, onPatch, formats, options]);
  if (!tool) return null;
  const setOption = (key: string, value: unknown) => {
    const next: Record<string, unknown> = { ...options, format };
    if (value === "" || value === false || value === undefined || (typeof value === "number" && !value)) delete next[key];
    else next[key] = value;
    onPatch({ cli_options: next });
  };
  if (!formats.length)
    return (
      <AutoField label={t("cliConnection")}>
        <p className="text-sm text-muted-foreground">{t("cliNativeOnly", { tool: tool.label })}</p>
      </AutoField>
    );
  if (readOnly)
    return (
      <AutoField label={t("cliConnection")}>
        {custom ? (
          <dl className="grid min-w-0 gap-3 sm:grid-cols-2">
            {[
              [t("cliFormat"), t(`format.${format}`)],
              [t("cliBaseUrl"), connection?.base_url],
              ...Object.entries(options)
                .filter(([k]) => k !== "format" && k !== "headers")
                .map(([k, v]) => [t(`opt.${k}`), typeof v === "boolean" ? (v ? "✓" : "—") : String(v)]),
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
          <Switch aria-label={label} checked={value === true} onCheckedChange={(v) => setOption(option.key, v)} />
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
              config: { placeholder: t("cliDefault"), options: option.values.map((v) => ({ value: v, label: v })) },
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
            setOption(option.key, option.kind === "number" ? Math.max(0, Math.floor(Number(e.target.value) || 0)) : e.target.value)
          }
        />
      </AutoField>
    );
  };
  const headers = Object.entries((options.headers as Record<string, string>) ?? {});
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
                ? {
                    connection: { ...newCliConnection(), protocol: formats[0] === "anthropic" ? "anthropic" : "openai" },
                    cli_options: { format: formats[0] },
                    provider_id: "0",
                  }
                : { connection: null, cli_options: null, provider_id: "0", models: [], default_model: "" },
            )
          }
        />
      </AutoField>
      {!custom ? (
        <p className="text-sm text-muted-foreground">{t("cliNativeHelp", { tool: tool.label })}</p>
      ) : (
        <>
          {!!presets.length && (
            <ProviderPresetPicker
              selected={preset}
              presets={presets}
              onSelect={(next) => {
                setPreset(next ?? undefined);
                if (!next || !connection) return;
                onPatch({
                  connection: { ...connection, base_url: next.base_url, protocol: next.protocol },
                  cli_options: { ...options, format: presetFormat(next, formats) },
                  ...(next.models.length ? { models: [...next.models], default_model: next.models[0] } : {}),
                  ...(!String(draft.name ?? "").trim() ? { name: `${tool.label} · ${next.name}` } : {}),
                });
              }}
            />
          )}
          <div className="grid min-w-0 gap-4 sm:grid-cols-2">
            <AutoField label={t("cliFormat")} required help={t("cliFormatHelp")}>
              <FieldControl
                name="cli-format"
                field={{
                  type: "select",
                  name: t("cliFormat"),
                  width: "full",
                  config: { options: formats.map((f) => ({ value: f, label: t(`format.${f}`) })) },
                }}
                value={format}
                required
                onChange={(v) =>
                  onPatch({
                    cli_options: { ...options, format: String(v) },
                    connection: { ...connection, protocol: v === "anthropic" ? "anthropic" : "openai" },
                  })
                }
              />
            </AutoField>
            <div className="hidden sm:block" />
            <div className="col-span-full">
              <AutoField label={t("cliBaseUrl")} required help={t(`baseUrlHelp.${format}`)}>
                <Input
                  aria-label={t("cliBaseUrl")}
                  className="min-h-11"
                  required
                  type="url"
                  placeholder={t(`baseUrlPlaceholder.${format}`)}
                  value={connection?.base_url ?? ""}
                  onChange={(e) => onPatch({ connection: { ...connection, base_url: e.target.value } })}
                />
              </AutoField>
            </div>
            <div className="col-span-full">
              <AutoField
                label="API Key"
                required={!draft.provider_id || draft.provider_id === "0"}
                help={draft.provider_id && draft.provider_id !== "0" ? t("cliKeyKeep") : t("cliKeyHelp")}
              >
                <Input
                  aria-label="API Key"
                  className="min-h-11"
                  type="password"
                  autoComplete="new-password"
                  required={!draft.provider_id || draft.provider_id === "0"}
                  value={connection?.api_key ?? ""}
                  onChange={(e) => onPatch({ connection: { ...connection, api_key: e.target.value } })}
                />
              </AutoField>
            </div>
            {main.map(field)}
          </div>
          {(!!advanced.length || HEADERS[String(draft.runtime)]) && (
            <details className="rounded-lg border p-3" open={advanced.some((o) => options[o.key] !== undefined) || !!headers.length}>
              <summary className="min-h-11 cursor-pointer select-none py-2 text-sm font-medium">{t("cliAdvanced")}</summary>
              <div className="mt-3 grid min-w-0 gap-4 sm:grid-cols-2">
                {advanced.map(field)}
                {HEADERS[String(draft.runtime)] && (
                  <div className="col-span-full">
                    <AutoField label={t("opt.headers")} help={t("optHelp.headers")}>
                      <div className="space-y-2">
                        {headers.map(([name, value], index) => (
                          <div key={index} className="flex min-w-0 flex-wrap items-center gap-2 sm:flex-nowrap">
                            <Input
                              aria-label={`${t("headerName")} ${index + 1}`}
                              placeholder={t("headerName")}
                              className="min-h-11 min-w-0 flex-1"
                              value={name}
                              onChange={(e) => setHeaders(headers.map((h, i) => (i === index ? [e.target.value, h[1]] : h)))}
                            />
                            <Input
                              aria-label={`${t("headerValue")} ${index + 1}`}
                              placeholder={t("headerValue")}
                              className="min-h-11 min-w-0 flex-1"
                              value={value}
                              onChange={(e) => setHeaders(headers.map((h, i) => (i === index ? [h[0], e.target.value] : h)))}
                            />
                            <Button
                              type="button"
                              variant="outline"
                              className="min-h-11 shrink-0"
                              aria-label={`${commonText("common.delete")} ${index + 1}`}
                              onClick={() => setHeaders(headers.filter((_, i) => i !== index))}
                            >
                              {commonText("common.delete")}
                            </Button>
                          </div>
                        ))}
                        <Button
                          type="button"
                          variant="outline"
                          className="min-h-11"
                          onClick={() => setHeaders([...headers, [`X-Header-${headers.length + 1}`, ""]])}
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

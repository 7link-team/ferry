import { FieldCaption } from "./FieldHelp";
import { Input } from "../ui/input";
import { commonText, workflowText as t } from "../i18n/i18n";
const control =
  "h-11 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";
export type Price = {
  model: string;
  operation: string;
  input_micros: number;
  output_micros: number;
  unit_size: number;
};
export type Provider = {
  protocol: "openai" | "anthropic";
  id: string;
  name: string;
  credential_id: string;
  api_key?: string;
  base_url: string;
  /** Metering is not edited here; stored values round-trip unchanged. */
  currency?: string;
  timezone?: string;
  monthly_limit_micros?: number;
  qps: number;
  concurrency: number;
  prices?: Price[];
  version: number;
};
export function ProviderFields({
  draft,
  onChange,
  readOnly = false,
  hideName = false,
}: {
  draft: Provider;
  onChange: (value: Provider) => void;
  readOnly?: boolean;
  hideName?: boolean;
}) {
  if (readOnly)
    return (
      <dl className="grid min-w-0 gap-4 sm:grid-cols-2 [&_button[role=combobox]]:min-h-11">
        {(
          [
            ["aiProtocol", draft.protocol],
            ["aiBaseURL", draft.base_url],
            ["aiQPS", draft.qps],
            ["aiConcurrency", draft.concurrency],
          ] as const
        ).map(([key, value]) => (
          <div
            key={key}
            className={
              key === "aiBaseURL" ? "col-span-full min-w-0" : "min-w-0"
            }
          >
            <dt className="text-sm text-muted-foreground">{t(key)}</dt>
            <dd className="break-words text-sm">{value}</dd>
          </div>
        ))}
      </dl>
    );
  return (
    <div className="grid min-w-0 gap-4 sm:grid-cols-2 [&_button[role=combobox]]:min-h-11">
      <label className="block min-w-0 space-y-2 text-sm">
        <span>
          <FieldCaption label={<>{t("aiProtocol")}</>} required />
        </span>
        <select
          className={control}
          aria-label={t("aiProtocol")}
          required
          disabled={readOnly}
          value={draft.protocol}
          onChange={(e) =>
            onChange({
              ...draft,
              protocol: e.target.value as Provider["protocol"],
              base_url:
                e.target.value === "anthropic"
                  ? "https://api.anthropic.com/v1"
                  : "https://api.openai.com/v1",
            })
          }
        >
          <option value="openai">OpenAI</option>
          <option value="anthropic">Anthropic</option>
        </select>
      </label>
      {(
        [
          ["name", "name"],
          ["base_url", "aiBaseURL"],
          ["api_key", "API Key"],
          ["qps", "aiQPS"],
          ["concurrency", "aiConcurrency"],
        ] as const
      )
        .filter(([key]) => !hideName || key !== "name")
        .map(([key, label]) => (
          <label
            key={key}
            className={
              key === "base_url" || key === "api_key"
                ? "col-span-full block min-w-0 space-y-2 text-sm"
                : "block min-w-0 space-y-2 text-sm"
            }
          >
            <span>
              <FieldCaption
                required={key !== "api_key" || !draft.id}
                help={
                  key === "api_key" ? commonText("resource-domain:credential.secretHelp")
                    : key === "qps" ? commonText("models:qpsHelp")
                    : key === "concurrency" ? commonText("models:concurrencyHelp") : undefined
                }
                label={
                  <>{key === "name" ? commonText("common.name") : key === "api_key" ? "API Key" : t(label)}</>
                }
              />
            </span>
            <Input
                className="min-h-11"
                aria-label={
                  key === "name" ? commonText("common.name") : key === "api_key" ? "API Key" : t(label)
                }
                disabled={readOnly}
                type={key === "api_key" ? "password" : typeof draft[key] === "number" ? "number" : "text"}
                autoComplete={key === "api_key" ? "new-password" : undefined}
                required={key !== "api_key" || !draft.id}
                min={key === "qps" || key === "concurrency" ? 1 : typeof draft[key] === "number" ? 0 : undefined}
                max={key === "qps" ? 1000 : key === "concurrency" ? 100 : undefined}
                step={typeof draft[key] === "number" ? 1 : undefined}
                value={draft[key] ?? ""}
                onChange={(e) =>
                  onChange({
                    ...draft,
                    [key]:
                      typeof draft[key] === "number"
                        ? Number(e.target.value)
                        : e.target.value,
                  })
                }
              />
          </label>
        ))}
    </div>
  );
}

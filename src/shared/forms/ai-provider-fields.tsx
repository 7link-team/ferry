import { FieldCaption } from "./FieldHelp";
import { FieldControl } from "./auto-form";
import { Input } from "../ui/input";
import { Button } from "../ui/button";
import { commonText, workflowText as t } from "../i18n/i18n";
const control =
  "h-11 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";
const operations = [
  "ai.llm.chat",
  "ai.image.generate",
  "ai.audio.speech",
  "ai.audio.transcribe",
];
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
  currency: string;
  timezone: string;
  monthly_limit_micros: number;
  qps: number;
  concurrency: number;
  prices: Price[];
  version: number;
};
export function ProviderFields({
  draft,
  onChange,
  readOnly = false,
  hideName = false,
  type = "ai.llm.chat",
}: {
  draft: Provider;
  onChange: (value: Provider) => void;
  readOnly?: boolean;
  hideName?: boolean;
  type?: string;
}) {
  if (readOnly)
    return (
      <dl className="grid min-w-0 gap-4 sm:grid-cols-2 [&_button[role=combobox]]:min-h-11">
        {(
          [
            ["aiProtocol", draft.protocol],
            ["aiBaseURL", draft.base_url],
            ["aiCurrency", draft.currency],
            ["aiTimezone", draft.timezone],
            ["aiMonthlyBudget", draft.monthly_limit_micros],
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
        {draft.prices.map((price, index) => (
          <div
            key={index}
            className="col-span-full grid min-w-0 gap-3 rounded-lg border bg-muted/30 p-3 sm:grid-cols-2"
          >
            {(
              [
                ["aiOperation", price.operation],
                ["aiModel", price.model],
                ["aiInputPrice", price.input_micros],
                ["aiOutputPrice", price.output_micros],
                ["aiPriceUnit", price.unit_size],
              ] as const
            ).map(([key, value]) => (
              <div key={key} className="min-w-0">
                <dt className="text-sm text-muted-foreground">{t(key)}</dt>
                <dd className="break-words text-sm">{value}</dd>
              </div>
            ))}
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
          ["currency", "aiCurrency"],
          ["timezone", "aiTimezone"],
          ["monthly_limit_micros", "aiMonthlyBudget"],
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
                  key === "timezone"
                    ? commonText("models:detectedTimezone", {
                        timezone:
                          Intl.DateTimeFormat().resolvedOptions().timeZone,
                      })
                    : key === "api_key" ? commonText("resource-domain:credential.secretHelp") : undefined
                }
                label={
                  <>{key === "name" ? commonText("common.name") : key === "api_key" ? "API Key" : t(label)}</>
                }
              />
            </span>
            {key === "timezone" ? (
              <FieldControl
                field={{
                  type: "select",
                  name: t("aiTimezone"),
                  required: true,
                  config: {
                    width: "full",
                    options: [
                      ...new Set([
                        draft.timezone,
                        "UTC",
                        ...Intl.supportedValuesOf("timeZone"),
                      ]),
                    ]
                      .filter(Boolean)
                      .map((value) => ({ value, label: value })),
                  },
                }}
                name="connection-timezone"
                required
                value={draft.timezone}
                disabled={readOnly || !!draft.id}
                onChange={(value) =>
                  onChange({ ...draft, timezone: String(value) })
                }
              />
            ) : (
              <Input
                className="min-h-11"
                aria-label={
                  key === "name" ? commonText("common.name") : key === "api_key" ? "API Key" : t(label)
                }
                disabled={readOnly || (!!draft.id && key === "currency")}
                type={key === "api_key" ? "password" : typeof draft[key] === "number" ? "number" : "text"}
                autoComplete={key === "api_key" ? "new-password" : undefined}
                required={key !== "api_key" || !draft.id}
                min={0}
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
            )}
          </label>
        ))}
      {draft.prices.map((price, index) => (
        <div
          key={index}
          className="col-span-full grid min-w-0 gap-3 rounded-lg border bg-muted/30 p-3 sm:grid-cols-2"
        >
          <label className="block min-w-0 space-y-2 text-sm">
            <span>
              <FieldCaption label={<>{t("aiOperation")}</>} required />
            </span>
            <select
              className={control}
              aria-label={t("aiOperation")}
              required
              disabled={readOnly}
              value={price.operation}
              onChange={(e) =>
                onChange({
                  ...draft,
                  prices: draft.prices.map((p, i) =>
                    i === index ? { ...p, operation: e.target.value } : p,
                  ),
                })
              }
            >
              {operations.map((op) => (
                <option key={op}>{op}</option>
              ))}
            </select>
          </label>
          {(
            [
              ["model", "aiModel"],
              ["input_micros", "aiInputPrice"],
              ["output_micros", "aiOutputPrice"],
              ["unit_size", "aiPriceUnit"],
            ] as const
          ).map(([key, label]) => (
            <label key={key} className="block space-y-2 text-sm">
              <span>
                <FieldCaption label={<>{t(label)}</>} help={t("aiPriceHelp")} required />
              </span>
              <Input
                className="min-h-11"
                aria-label={t(label)}
                required
                disabled={readOnly}
                type={key === "model" ? "text" : "number"}
                min={key === "unit_size" ? 1 : 0}
                value={price[key]}
                onChange={(e) =>
                  onChange({
                    ...draft,
                    prices: draft.prices.map((p, i) =>
                      i === index
                        ? {
                            ...p,
                            [key]:
                              key === "model"
                                ? e.target.value
                                : Number(e.target.value),
                          }
                        : p,
                    ),
                  })
                }
              />
            </label>
          ))}
          <Button
            type="button"
            variant="ghost"
            disabled={readOnly || draft.prices.length === 1}
            onClick={() =>
              onChange({
                ...draft,
                prices: draft.prices.filter((_, i) => i !== index),
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
        disabled={readOnly}
        onClick={() =>
          onChange({
            ...draft,
            prices: [
              ...draft.prices,
              {
                model: "",
                operation: type,
                input_micros: 0,
                output_micros: 0,
                unit_size: 1,
              },
            ],
          })
        }
      >
        {t("aiAddPrice")}
      </Button>
    </div>
  );
}

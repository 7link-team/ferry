import { FieldCaption } from "../../shared/forms/FieldHelp";
import { useContext, useEffect, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { commonText, workflowText as t } from "../../shared/i18n/i18n";
import { Input } from "../../shared/ui/input";
import { Button } from "../../shared/ui/button";
import { WorkflowAssetContext } from "./WorkflowAssets";
import { AutoForm } from "../../shared/forms/auto-form";

type Price = {
  model: string;
  operation: string;
  input_micros: number;
  output_micros: number;
  unit_size: number;
};
type Provider = {
  protocol: "openai" | "anthropic";
  id: string;
  name: string;
  credential_id: string;
  base_url: string;
  currency: string;
  timezone: string;
  monthly_limit_micros: number;
  qps: number;
  concurrency: number;
  prices: Price[];
  version: number;
};
const control =
  "h-10 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";
const operations = [
  "ai.llm.chat",
  "ai.image.generate",
  "ai.audio.speech",
  "ai.audio.transcribe",
];
function CallLedger() {
  const { workspace } = useContext(WorkflowAssetContext);
  const [rows, setRows] = useState<
    {
      id: string;
      state: string;
      currency: string;
      reserved_micros: number;
      actual_micros: number;
      execution_id: string;
    }[]
  >();
  const [error, setError] = useState("");
  const load = async () => {
    try {
      setRows(
        (
          await api<{ items: NonNullable<typeof rows> }>(
            `/workspaces/${workspace}/ai-calls`,
          )
        ).items,
      );
      setError("");
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <section className="space-y-3 border-t pt-3">
      <Button variant="outline" onClick={load}>
        {t("aiLedger")}
      </Button>
      {rows?.map((row) => (
        <div key={row.id} className="space-y-2 border-b py-3 text-xs">
          <p className="break-all">
            {row.execution_id} · {row.state} · {row.currency}
          </p>
          <p>
            {t("aiCallBudget")}: {row.reserved_micros} · {t("aiActualCost")}:{" "}
            {row.actual_micros}
          </p>
          {row.state === "unknown" && (
            <AutoForm
              className="space-y-2"
              onSubmit={async (e) => {
                e.preventDefault();
                const data = new FormData(e.currentTarget);
                try {
                  await api(
                    `/workspaces/${workspace}/ai-calls/${row.id}/reconcile`,
                    {
                      method: "POST",
                      body: JSON.stringify({
                        actual_micros: Number(data.get("actual")),
                        reason: String(data.get("reason")),
                      }),
                    },
                  );
                  await load();
                } catch (e) {
                  setError(errorText(e));
                }
              }}
            >
              <label className="block space-y-1">
                <span><FieldCaption label={<>{t("aiActualCost")}</>} /></span>
                <Input
                  type="number"
                  name="actual"
                  min={0}
                  max={1000000000000}
                  required
                />
              </label>
              <label className="block space-y-1">
                <span><FieldCaption label={<>{t("aiReconcileReason")}</>} /></span>
                <Input name="reason" required maxLength={2000} />
              </label>
              <Button type="submit" variant="outline">
                {t("aiReconcile")}
              </Button>
            </AutoForm>
          )}
        </div>
      ))}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}
export function WorkflowAIParameters({
  type,
  value,
  readOnly,
  change,
}: {
  type: string;
  value: Record<string, unknown>;
  readOnly: boolean;
  change: (value: Record<string, unknown>) => void;
}) {
  const { workspace } = useContext(WorkflowAssetContext);
  const [providers, setProviders] = useState<Provider[]>([]),
    [credentials, setCredentials] = useState<
      { id: string; name: string; kind: string }[]
    >([]);
  const [draft, setDraft] = useState<Provider>(),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  const load = () =>
    api<{ items: Provider[] }>(`/workspaces/${workspace}/ai-providers`).then(
      (result) => setProviders(result.items),
    );
  useEffect(() => {
    let active = true;
    api<{ items: Provider[] }>(`/workspaces/${workspace}/ai-providers`)
      .then((result) => {
        if (active) setProviders(result.items);
      })
      .catch((e) => {
        if (active) setError(errorText(e));
      });
    return () => {
      active = false;
    };
  }, [workspace]);
  const patch = (key: string, next: unknown) =>
    change({ ...value, [key]: next });
  const field = (
    key: string,
    label: Parameters<typeof t>[0],
    numeric = false,
  ) => (
    <label className="block space-y-2 text-sm">
      <span><FieldCaption label={<>{t(label)}</>} /></span>
      <Input
        aria-label={t(label)}
        disabled={readOnly}
        type={numeric ? "number" : "text"}
        min={0}
        value={String(value[key] ?? "")}
        onChange={(e) =>
          patch(key, numeric ? Number(e.target.value) : e.target.value)
        }
      />
    </label>
  );
  const selected = providers.find((p) => p.id === value.provider_id);
  return (
    <div className="space-y-4">
      <label className="block space-y-2 text-sm">
        <span><FieldCaption label={<>{t("aiProvider")}</>} /></span>
        <select
          aria-label={t("aiProvider")}
          className={control}
          disabled={readOnly}
          value={String(value.provider_id ?? "0")}
          onChange={(e) => patch("provider_id", e.target.value)}
        >
          <option value="0">{t("selectValue")}</option>
          {providers
            .filter(
              (provider) =>
                type === "ai.llm.chat" || provider.protocol === "openai",
            )
            .map((provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.name}
              </option>
            ))}
        </select>
      </label>
      {!readOnly && (
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            onClick={async () => {
              try {
                const result = await api<{
                  items: { id: string; name: string; kind: string }[];
                }>(
                  `/workspaces/${workspace}/credentials?scope=ai&page_size=200`,
                );
                setCredentials(result.items.filter((c) => c.kind === "token"));
                setDraft({
                  id: "",
                  name: "",
                  credential_id: "",
                  base_url: "https://api.openai.com/v1",
                  protocol: "openai",
                  currency: "USD",
                  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
                  monthly_limit_micros: 0,
                  qps: 1,
                  concurrency: 1,
                  prices: [
                    {
                      model: String(value.model),
                      operation: type,
                      input_micros: 0,
                      output_micros: 0,
                      unit_size: 1,
                    },
                  ],
                  version: 0,
                });
              } catch (e) {
                setError(errorText(e));
              }
            }}
          >
            {t("aiAddProvider")}
          </Button>
          {selected && (
            <Button
              variant="ghost"
              onClick={async () => {
                try {
                  const result = await api<{
                    items: { id: string; name: string; kind: string }[];
                  }>(
                    `/workspaces/${workspace}/credentials?scope=ai&page_size=200`,
                  );
                  setCredentials(
                    result.items.filter((c) => c.kind === "token"),
                  );
                  setDraft(structuredClone(selected));
                } catch (e) {
                  setError(errorText(e));
                }
              }}
            >
              {commonText("common.edit")}
            </Button>
          )}
        </div>
      )}
      {draft && (
        <section
          aria-label={t("aiAddProvider")}
          className="space-y-3 rounded-lg border p-3"
        >
          <p className="text-xs leading-5 text-muted-foreground">
            {t("aiPriceHelp")}
          </p>
          <label className="block space-y-2 text-sm">
            <span><FieldCaption label={<>{t("aiProtocol")}</>} /></span>
            <select
              className={control}
              aria-label={t("aiProtocol")}
              value={draft.protocol}
              onChange={(e) =>
                setDraft({
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
          <a
            className="block text-sm text-primary underline"
            href={`/${document.documentElement.lang || "zh-CN"}/workspaces/${workspace}/credentials?scope=ai`}
            target="_blank"
            rel="noreferrer"
          >
            {t("aiCredentials")}
          </a>
          <label className="block space-y-2 text-sm">
            <span><FieldCaption label={<>{t("aiCredential")}</>} /></span>
            <select
              className={control}
              aria-label={t("aiCredential")}
              disabled={!!draft.id}
              value={draft.credential_id}
              onChange={(e) =>
                setDraft({ ...draft, credential_id: e.target.value })
              }
            >
              <option value="">{t("selectValue")}</option>
              {credentials.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </label>
          {(
            [
              ["name", "name"],
              ["base_url", "aiBaseURL"],
              ["currency", "aiCurrency"],
              ["timezone", "aiTimezone"],
              ["monthly_limit_micros", "aiMonthlyBudget"],
              ["qps", "aiQPS"],
              ["concurrency", "aiConcurrency"],
            ] as const
          ).map(([key, label]) => (
            <label key={key} className="block space-y-2 text-sm">
              <span><FieldCaption label={<>
                {key === "name" ? commonText("common.name") : t(label)}
              </>} /></span>
              <Input
                aria-label={t(label)}
                disabled={
                  !!draft.id && (key === "currency" || key === "timezone")
                }
                type={typeof draft[key] === "number" ? "number" : "text"}
                min={0}
                value={draft[key]}
                onChange={(e) =>
                  setDraft({
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
          {draft.prices.map((price, index) => (
            <div key={index} className="space-y-2 border-t pt-3">
              <label className="block space-y-2 text-sm">
                <span><FieldCaption label={<>{t("aiOperation")}</>} /></span>
                <select
                  className={control}
                  value={price.operation}
                  onChange={(e) =>
                    setDraft({
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
                  <span><FieldCaption label={<>{t(label)}</>} /></span>
                  <Input
                    aria-label={t(label)}
                    type={key === "model" ? "text" : "number"}
                    min={key === "unit_size" ? 1 : 0}
                    value={price[key]}
                    onChange={(e) =>
                      setDraft({
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
                variant="ghost"
                disabled={draft.prices.length === 1}
                onClick={() =>
                  setDraft({
                    ...draft,
                    prices: draft.prices.filter((_, i) => i !== index),
                  })
                }
              >
                {t("delete")}
              </Button>
            </div>
          ))}
          <Button
            variant="outline"
            onClick={() =>
              setDraft({
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
          <div className="flex gap-2">
            <Button
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                setError("");
                try {
                  const { id, ...body } = draft;
                  const saved = await api<Provider>(
                    `/workspaces/${workspace}/ai-providers${id ? `/${id}` : ""}`,
                    { method: id ? "PUT" : "POST", body: JSON.stringify(body) },
                  );
                  await load();
                  patch("provider_id", saved.id);
                  setDraft(undefined);
                } catch (e) {
                  setError(errorText(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              {commonText("common.save")}
            </Button>
            <Button variant="ghost" onClick={() => setDraft(undefined)}>
              {t("cancel")}
            </Button>
          </div>
        </section>
      )}
      {field("model", "aiModel")}
      {type !== "ai.audio.transcribe" && (
        <label className="block space-y-2 text-sm">
          <span><FieldCaption label={<>{t("aiPrompt")}</>} /></span>
          <textarea
            className="min-h-28 w-full rounded-md border bg-background p-3 text-sm"
            aria-label={t("aiPrompt")}
            disabled={readOnly}
            value={String(value.prompt ?? "")}
            onChange={(e) => patch("prompt", e.target.value)}
          />
        </label>
      )}
      {type === "ai.llm.chat" && (
        <>
          {field("image_field", "aiImageField")}
          {field("max_tokens", "aiMaxTokens", true)}
        </>
      )}
      {type === "ai.image.generate" && (
        <label className="block space-y-2 text-sm">
          <span><FieldCaption label={<>{t("aiImageSize")}</>} /></span>
          <select
            aria-label={t("aiImageSize")}
            className={control}
            disabled={readOnly}
            value={String(value.size)}
            onChange={(e) => patch("size", e.target.value)}
          >
            {["1024x1024", "1024x1536", "1536x1024", "auto"].map((size) => (
              <option key={size}>{size}</option>
            ))}
          </select>
        </label>
      )}
      {type === "ai.audio.speech" && field("voice", "aiVoice")}
      {type === "ai.audio.transcribe" && field("file_field", "binaryField")}
      {field("output_field", "outputField")}
      {field("max_cost_micros", "aiCallBudget", true)}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!readOnly && <CallLedger />}
    </div>
  );
}

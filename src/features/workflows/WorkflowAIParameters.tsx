import { FieldCaption } from "../../shared/forms/FieldHelp";
import { useContext, useEffect, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { commonText, workflowText as t } from "../../shared/i18n/i18n";
import { Input } from "../../shared/ui/input";
import { Button } from "../../shared/ui/button";
import { WorkflowAssetContext } from "./WorkflowAssets";
import { AutoForm } from "../../shared/forms/auto-form";

import {
  ProviderFields,
  type Provider,
} from "../../shared/forms/ai-provider-fields";
export {
  ProviderFields,
  type Provider,
  type Price,
} from "../../shared/forms/ai-provider-fields";
const control =
  "h-11 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";
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
                <span>
                  <FieldCaption label={<>{t("aiActualCost")}</>} />
                </span>
                <Input
                  type="number"
                  name="actual"
                  min={0}
                  max={1000000000000}
                  required
                />
              </label>
              <label className="block space-y-1">
                <span>
                  <FieldCaption label={<>{t("aiReconcileReason")}</>} />
                </span>
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
  providerOnly = false,
}: {
  type: string;
  value: Record<string, unknown>;
  readOnly: boolean;
  change: (value: Record<string, unknown>) => void;
  providerOnly?: boolean;
}) {
  const { workspace } = useContext(WorkflowAssetContext);
  const [providers, setProviders] = useState<Provider[]>([]);
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
      <span>
        <FieldCaption label={<>{t(label)}</>} />
      </span>
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
        <span>
          <FieldCaption label={<>{t("aiProvider")}</>} />
        </span>
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
            type="button"
            variant="outline"
            onClick={async () => {
              try {
                setDraft({
                  id: "",
                  name: "",
                  credential_id: "0",
                  base_url: "https://api.openai.com/v1",
                  protocol: "openai",
                  qps: 1,
                  concurrency: 1,
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
              type="button"
              variant="ghost"
              onClick={async () => {
                try {
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
          <ProviderFields draft={draft} onChange={setDraft} />
          <div className="flex gap-2">
            <Button
              type="button"
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
            <Button
              type="button"
              variant="ghost"
              onClick={() => setDraft(undefined)}
            >
              {t("cancel")}
            </Button>
          </div>
        </section>
      )}
      {!providerOnly && (
        <>
          {field("model", "aiModel")}
          {type !== "ai.audio.transcribe" && (
            <label className="block space-y-2 text-sm">
              <span>
                <FieldCaption label={<>{t("aiPrompt")}</>} />
              </span>
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
              <span>
                <FieldCaption label={<>{t("aiImageSize")}</>} />
              </span>
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
        </>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!readOnly && !providerOnly && <CallLedger />}
    </div>
  );
}

import { useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText as t } from "../../shared/i18n/i18n";
import type { AnyRow } from "../../shared/resources/config";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";

const key = (name: string) => `notifications:${name}`;

/**
 * A saved address is echoed with its secret parts hidden; editors can reveal the
 * plain address (audited on the server). Leaving the input empty keeps it.
 */
export function WebhookAddress({ workspace, draft, readOnly, onPatch, canReveal }: {
  workspace: string;
  draft: Partial<AnyRow>;
  readOnly: boolean;
  onPatch: (patch: Partial<AnyRow>) => void;
  canReveal: boolean;
}) {
  const [revealed, setRevealed] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const saved = !!draft.id && !!draft.has_webhook;
  const masked = String(draft.webhook_masked ?? "");
  const label = t(key("webhook"));
  async function toggle() {
    if (revealed !== undefined) { setRevealed(undefined); return; }
    setBusy(true); setError("");
    try {
      const r = await api<{ webhook: string }>(`/workspaces/${workspace}/notification-channels/${draft.id}/webhook-reveals`, { method: "POST" });
      setRevealed(r.webhook);
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); }
  }
  const current = saved && (
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      <code aria-label={t(key("webhookCurrent"))} className="min-w-0 break-all rounded bg-muted px-2 py-1 font-mono text-xs">{revealed ?? masked}</code>
      {canReveal && <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => void toggle()}>
        {busy ? t("form.reading") : revealed === undefined ? t("form.reveal") : t("form.hidePlaintext")}
      </Button>}
    </div>
  );
  if (readOnly) return (
    <AutoField label={label}>
      <div className="space-y-1">
        {current || <p className="text-sm text-muted-foreground">{t("common.notSet")}</p>}
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      </div>
    </AutoField>
  );
  return (
    <AutoField label={label}>
      <div className="space-y-2">
        {current}
        <Input
          aria-label={label}
          type="url"
          autoComplete="off"
          spellCheck={false}
          value={String(draft.webhook ?? "")}
          placeholder={saved ? t(key("webhookKeep")) : "https://"}
          onChange={(event) => onPatch({ webhook: event.target.value })}
        />
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      </div>
    </AutoField>
  );
}

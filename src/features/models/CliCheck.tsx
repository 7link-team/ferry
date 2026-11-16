import { useState } from "react";
import { CheckCircle2, XCircle } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { InlineAlert } from "../../shared/ui/inline-alert";

const t = (s: string, values?: Record<string, unknown>) => commonText(`models:${s}`, values);

interface HostCheck {
  host_id: string;
  host_name: string;
  ok: boolean;
  executable?: string | null;
  user?: string;
  version?: string | null;
  authentication?: string;
  reason?: string;
}

/**
 * Checks, without saving, whether each execution host can run this CLI: where it
 * was found and for which user, or why not. Saving never depends on it, since
 * hosts can be offline at the time.
 */
export function CliCheck({ workspace, runtime, hosts, disabled }: {
  workspace: string;
  runtime: string;
  hosts: string[];
  disabled?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [items, setItems] = useState<HostCheck[]>();
  async function check() {
    setBusy(true); setError(""); setItems(undefined);
    try {
      const result = await api<{ items: HostCheck[] }>(`/workspaces/${workspace}/ai-models/cli-checks`, {
        method: "POST", body: JSON.stringify({ runtime, hosts }),
      });
      setItems(result.items);
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); }
  }
  return (
    <AutoField label={t("cliCheck")} help={t("cliCheckHelp")}>
      <div className="space-y-2">
        <Button type="button" variant="outline" disabled={disabled || busy || !runtime} onClick={() => void check()}>
          {busy ? t("cliChecking") : t("cliCheckRun")}
        </Button>
        {error && <InlineAlert onDismiss={() => setError("")}>{error}</InlineAlert>}
        {items && !items.length && <p role="status" className="text-sm text-muted-foreground">{t("cliCheckNoHosts")}</p>}
        {items && !!items.length && (
          <ul aria-label={t("cliCheck")} className="space-y-1.5">
            {items.map((item) => (
              <li key={item.host_id} className="flex items-start gap-2 rounded-md border px-3 py-2 text-sm">
                {item.ok
                  ? <CheckCircle2 aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-primary" />
                  : <XCircle aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-destructive" />}
                <div className="min-w-0 flex-1">
                  <p className="font-medium">{item.host_name}</p>
                  <p className={`break-words text-xs ${item.ok ? "text-muted-foreground" : "text-destructive"}`}>
                    {item.ok
                      ? t("cliCheckFound", { path: item.executable ?? "", user: item.user ?? "", version: item.version ?? "—", login: t(`login_${item.authentication ?? "unknown"}`) })
                      : item.reason}
                  </p>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </AutoField>
  );
}

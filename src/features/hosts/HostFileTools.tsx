import { useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { InlineAlert } from "../../shared/ui/inline-alert";

const t = (key: string) => commonText(`resource-domain:fileTools.${key}`);

/** What the Runner's user can already use to produce files; checked on request. */
export function HostFileTools({ workspace, hostId }: { workspace: string; hostId: string }) {
  const [result, setResult] = useState<{ tools: string[]; write_workdir: boolean }>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function check() {
    setBusy(true);
    setError("");
    try {
      setResult(await api<{ tools: string[]; write_workdir: boolean }>(`${base(workspace, "hosts")}/${encodeURIComponent(hostId)}/file-tools`));
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="space-y-2 text-sm" data-testid="host-file-tools">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{t("label")}</span>
        <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => void check()}>
          {busy ? t("checking") : t("check")}
        </Button>
      </div>
      <p className="text-xs leading-5 text-muted-foreground">{t("help")}</p>
      {error && <InlineAlert onDismiss={() => setError("")}>{error}</InlineAlert>}
      {result && !result.write_workdir && (
        <p role="status" className="text-amber-700 dark:text-amber-400">{t("upgrade")}</p>
      )}
      {result && (result.tools.length ? (
        <ul className="list-disc space-y-1 pl-5" aria-label={t("label")}>
          {result.tools.map((tool) => <li key={tool} className="break-words">{tool}</li>)}
        </ul>
      ) : (
        <p role="status" className="text-muted-foreground">{t("none")}</p>
      ))}
    </div>
  );
}

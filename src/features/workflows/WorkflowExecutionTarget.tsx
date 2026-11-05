import { useEffect, useState } from "react";
import { listResource } from "../../shared/api/resource";
import { errorText } from "../../shared/api/api";
import { FieldCaption } from "../../shared/forms/FieldHelp";
import { workflowText as t } from "../../shared/i18n/i18n";

type Settings = Record<string, unknown>;
const control = "h-10 w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";

export function WorkflowExecutionTarget({ workspace, value, change, readOnly, hostOnly = false, inherit = false }: {
  workspace: string; value: Settings; change: (value: Settings) => void; readOnly: boolean; hostOnly?: boolean; inherit?: boolean;
}) {
  const [hosts, setHosts] = useState<{id: string; name: string; connection?: string}[]>([]);
  const [error, setError] = useState("");
  const runtime = String(value.runtime ?? (inherit ? "" : hostOnly ? "host" : "server"));
  useEffect(() => {
    if (runtime !== "host") return;
    const abort = new AbortController();
    setError(""); setHosts([]);
    void (async () => {
      const rows: typeof hosts = [];
      for (let page=1;;page++) {
        const result = await listResource<typeof hosts[number]>(workspace, "hosts", {page, page_size:200}, abort.signal);
        rows.push(...result.items);
        if (page * result.page_size >= result.total) break;
      }
      if (!abort.signal.aborted) setHosts(rows);
    })().catch(cause => { if (!abort.signal.aborted) setError(errorText(cause)); });
    return () => abort.abort();
  }, [workspace, runtime]);
  return <div className="space-y-3">
    <label className="block space-y-2 text-sm">
      <FieldCaption label={t("execution.location")} help={t("execution.help")} />
      <select aria-label={t("execution.location")} className={control} disabled={readOnly} value={runtime}
        onChange={e => {
          const next = {...value};
          if (e.target.value) next.runtime=e.target.value; else delete next.runtime;
          if (e.target.value !== "host") { delete next.target_ref; delete next.target_kind; }
          change(next);
        }}>
        {inherit && <option value="">{t("execution.inherit")}</option>}
        {!hostOnly && <option value="server">{t("execution.server")}</option>}
        <option value="host">{t("execution.host")}</option>
      </select>
    </label>
    {runtime === "host" && <label className="block space-y-2 text-sm">
      <FieldCaption label={t("execution.target")} help={t("execution.targetHelp")} />
      <select aria-label={t("execution.target")} className={control} disabled={readOnly} value={String(value.target_ref ?? "")}
        onChange={e => change({...value, target_kind:"host",target_ref:e.target.value})}>
        <option value="">{t("selectValue")}</option>
        {!!value.target_ref && !hosts.some(h=>h.id===value.target_ref) && <option value={String(value.target_ref)}>{String(value.target_ref)}</option>}
        {hosts.map(h=><option key={h.id} value={h.id}>{h.name} · {h.connection === "runner" ? "Agent" : "SSH"}</option>)}
      </select>
    </label>}
    <p className="text-xs leading-5 text-muted-foreground">{t("execution.help")}</p>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </div>;
}

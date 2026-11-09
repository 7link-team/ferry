import { useEffect, useState } from "react";
import { ExternalLink } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { listResource } from "../../shared/api/resource";
import { commonText, workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { previewLink, type Hole } from "../holes/resource-config";

type Preview = Hole & { review_state: string };
const reviewText = (state: string) => {
  const key = `preview.review.${state}`;
  const text = t(key as Parameters<typeof t>[0]);
  return text === key ? state : text;
};

/** The previews a host.preview node opened, and its review while one is open. */
export function WorkflowPreviewReview({ workspace, execution, node, canReview, refresh }: {
  workspace: string; execution: string; node: string; canReview: boolean; refresh: unknown;
}) {
  const [rows, setRows] = useState<Preview[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  useEffect(() => {
    const abort = new AbortController();
    listResource<Preview>(workspace, "holes", { source_kind: "workflow", source_id: execution, source_ref: node, sort: "-created_at", page_size: 20 }, abort.signal)
      .then(result => { if (!abort.signal.aborted) { setRows(result.items); setError(""); } })
      .catch(cause => { if (!abort.signal.aborted) setError(errorText(cause)); });
    return () => abort.abort();
  }, [workspace, execution, node, refresh, reload]);
  const decide = async (row: Preview, decision: "approved" | "rejected") => {
    setBusy(true); setError("");
    try {
      await api(`/workspaces/${workspace}/holes/${row.id}/reviews`, { method: "POST", body: JSON.stringify({ decision }) });
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); setReload(value => value + 1); }
  };
  return <section className="min-w-0 space-y-2 rounded-md border p-2" aria-label={t("preview.title")}>
    <h4 className="text-sm font-medium">{t("preview.title")}</h4>
    {!rows.length && !error && <p className="text-xs text-muted-foreground">{t("preview.empty")}</p>}
    {rows.map(row => {
      const waiting = row.review_state === "waiting" && row.state === "active";
      return <div key={row.id} className="min-w-0 space-y-2 text-sm">
        <p className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <span className="min-w-0 break-all font-medium">{row.name}</span>
          <span className="text-xs text-muted-foreground">{commonText(`holes:state.${row.state}`)} · {reviewText(row.review_state)}</span>
        </p>
        {row.state === "active" && row.url && <a className="inline-flex min-h-11 max-w-full items-center gap-1 break-all text-primary underline-offset-4 hover:underline" href={previewLink(row)} target="_blank" rel="noopener noreferrer"><ExternalLink className="size-4 shrink-0" aria-hidden="true" />{commonText("holes:open")}</a>}
        {waiting && canReview && <div className="flex flex-wrap gap-2">
          <Button type="button" size="sm" disabled={busy} onClick={() => void decide(row, "approved")}>{t("preview.approve")}</Button>
          <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => void decide(row, "rejected")}>{t("preview.reject")}</Button>
        </div>}
      </div>;
    })}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </section>;
}

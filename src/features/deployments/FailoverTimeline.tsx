import { useEffect, useId, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { named } from "../../shared/api/resource";
import { commonText, useCommonTranslation } from "../../shared/i18n/i18n";
import { currentLocale, localizedPath } from "../../shared/i18n/locale";
import { Badge } from "../../shared/ui/badge";
import { Button } from "../../shared/ui/button";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { useFailoverEvents } from "./blueprint-hooks";
import { instanceResource, type Deployment } from "./deployment";

const t = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:failover.${key}`, values);
const touch = "min-h-11 lg:min-h-9";
const date = (value: number) => value ? new Date(value).toLocaleString(currentLocale()) : "—";
const kinds = new Set(["hostSuspected", "hostConfirmedDown", "peerVeto", "fenced", "replaceStarted", "revisionQueued", "replaceSucceeded", "replaceFailed", "hookOutput", "needsAttention", "unfenced", "alertOnly", "acknowledged"]);

export function FailoverTimeline({ workspace, deployment }: { workspace: string; deployment: Deployment }) {
  useCommonTranslation();
  const title = useId();
  const [page, setPage] = useState(1);
  const events = useFailoverEvents(workspace, deployment.id, page, deployment.version);
  const pages = Math.max(1, Math.ceil(events.total / 20));
  useEffect(() => { if (!events.loading && !events.error && page > pages) setPage(pages); }, [events.loading, events.error, page, pages]);
  return <section aria-labelledby={title} className="min-w-0 space-y-3">
    <h3 id={title} className="font-semibold">{t("timeline")}</h3>
    {events.loading ? <LoadingSkeleton variant="page" label={t("loading")} /> : events.error ? <div role="alert" className="space-y-2 break-words text-sm text-destructive">
      <p>{events.error}</p><Button className={touch} variant="outline" onClick={events.retry}>{commonText("common.retry")}</Button>
    </div> : <>
      {!events.items.length ? <p className="text-sm text-muted-foreground">{t("empty")}</p> : <ol className="min-w-0 divide-y rounded-md border px-3">{events.items.map(event => <li key={event.id} className="min-w-0 space-y-2 py-3 text-sm">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <time dateTime={new Date(event.created_at).toISOString()} className="text-muted-foreground">{date(event.created_at)}</time>
          <Badge variant="outline" className="max-w-full whitespace-normal break-all">{kinds.has(event.kind) ? t(`kind.${event.kind}`) : commonText("state.unknown", { value: event.kind })}</Badge>
          <span className="min-w-0 break-all font-medium">{event.instance}</span>
          <span className="min-w-0 break-all">{named(event.host_name, event.host_id)}</span>
          {event.revision_id && event.revision_id !== "0" && <a className="inline-flex min-h-11 items-center break-all text-primary underline lg:min-h-9" href={localizedPath(`/workspaces/${encodeURIComponent(workspace)}/deployment-revisions/${encodeURIComponent(event.revision_id)}`, currentLocale())}>{t("revision", { id: event.revision_id })}</a>}
        </div>
        <p className="whitespace-pre-wrap break-words [overflow-wrap:anywhere]">{event.message}</p>
      </li>)}</ol>}
      <nav aria-label={t("pagination")} className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
        <Button variant="outline" className={touch} disabled={page <= 1} onClick={() => setPage(n => n - 1)}>{t("previous")}</Button>
        <span>{t("page", { page, pages })}</span>
        <Button variant="outline" className={touch} disabled={page >= pages} onClick={() => setPage(n => n + 1)}>{t("next")}</Button>
      </nav>
    </>}
  </section>;
}

export function FailoverAttention({ workspace, deployment, permissions, onDone }: { workspace: string; deployment: Deployment; permissions: Set<string>; onDone: () => void }) {
  useCommonTranslation();
  const title = useId();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const attention = deployment.failover?.attention ?? [];
  if (!attention.length) return null;
  const acknowledge = async (instance: string) => {
    const path = instanceResource(workspace, deployment.id, instance);
    if (!path || busy) return;
    setBusy(true); setError("");
    try { await api(`${path}/acknowledgements`, { method: "POST", body: JSON.stringify({ version: deployment.version }) }); onDone(); }
    catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); }
  };
  return <section aria-labelledby={title} className="min-w-0 space-y-3 rounded-md border border-destructive/40 bg-destructive/5 p-3">
    <h3 id={title} className="font-semibold">{t("attention")}</h3>
    <p className="text-sm text-muted-foreground">{t("attentionHelp")}</p>
    <ul className="space-y-2">{attention.map(item => <li key={item.instance} className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
      <span className="min-w-0 break-all font-medium">{item.instance}</span><span className="text-muted-foreground">{date(item.since)}</span>
      {permissions.has("deployments.apply") && instanceResource(workspace, deployment.id, item.instance) && <Button className={touch} variant="outline" disabled={busy} onClick={() => void acknowledge(item.instance)}>{t("acknowledge")}</Button>}
    </li>)}</ul>
    {error && <p role="alert" className="break-words text-sm text-destructive">{error}</p>}
  </section>;
}

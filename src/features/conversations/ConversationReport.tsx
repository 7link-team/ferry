import { useEffect, useState, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { Download, FileText, X } from "lucide-react";
import { localizedFetch, errorText } from "../../shared/api/api";
import { Button } from "../../shared/ui/button";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { conversationText as t } from "./resource-config";

/** Only same-room authenticated Markdown files are upgraded into report cards. */
export function ConversationReport({ href, name, renderContent }: { href: string; name: string; renderContent: (text: string) => ReactNode }) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState<string>();
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!open) return;
    const controller = new AbortController(); setError(""); setText(undefined);
    void (async () => {
      try {
        const response = await localizedFetch(href, { signal: controller.signal });
        if (!response.ok) throw new Error(t("reportUnavailable"));
        const reader = response.body!.getReader(); let size = 0; const chunks: Uint8Array[] = [];
        try { for (;;) { const {done,value}=await reader.read(); if(done) break; size+=value.length; if(size>1024*1024) throw new Error(t("reportPreviewLimit")); chunks.push(value); } }
        finally { await reader.cancel(); }
        const all = new Uint8Array(size); let offset=0; for(const chunk of chunks) { all.set(chunk,offset);offset+=chunk.length; }
        if (!controller.signal.aborted) setText(new TextDecoder().decode(all));
      } catch (cause) { if (!controller.signal.aborted) setError(errorText(cause)); }
    })();
    return () => controller.abort();
  }, [open,href,retry]);
  return <Dialog.Root open={open} onOpenChange={setOpen}>
    <span className="my-3 flex min-w-0 flex-wrap items-center gap-3 rounded-xl border border-primary/25 bg-muted/60 p-3 text-foreground" data-report-card>
      <FileText aria-hidden className="size-8 shrink-0 text-primary" /><span className="min-w-0 flex-1"><span className="block text-xs text-muted-foreground">{t("reportFile")}</span><span className="block break-words font-medium">{name}</span></span>
      <span className="flex gap-2"><Dialog.Trigger asChild><Button variant="outline" className="min-h-11">{t("preview")}</Button></Dialog.Trigger><Button asChild className="min-h-11"><a href={`${href}?download=true`} download={name}><Download className="size-4" />{t("download")}</a></Button></span>
    </span>
    <Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" /><Dialog.Content aria-describedby={undefined} className="fixed inset-3 z-50 flex min-w-0 flex-col overflow-hidden rounded-xl border bg-card text-card-foreground shadow-lg sm:inset-x-[8vw] sm:inset-y-8">
      <div className="flex shrink-0 items-center gap-3 border-b px-4 py-3"><Dialog.Title className="min-w-0 flex-1 truncate font-semibold">{name}</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" aria-label={t("close")}><X /></Button></Dialog.Close></div>
      <div className="min-h-0 flex-1 overflow-auto p-4 text-sm leading-relaxed sm:p-6">{error ? <div role="alert" className="space-y-3"><p>{error}</p><Button variant="outline" onClick={() => setRetry(retry+1)}>{t("retryReport")}</Button></div> : text === undefined ? <LoadingSkeleton variant="form" /> : renderContent(text)}</div>
      <div className="flex shrink-0 justify-end border-t p-3 pb-[max(.75rem,env(safe-area-inset-bottom))]"><Button asChild className="min-h-11"><a href={`${href}?download=true`} download={name}><Download className="size-4" />{t("download")}</a></Button></div>
    </Dialog.Content></Dialog.Portal>
  </Dialog.Root>;
}

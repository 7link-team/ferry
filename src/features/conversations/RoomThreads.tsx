import { useState } from "react";
import { Dialog } from "radix-ui";
import { GitBranch, Workflow, X } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { currentLocale, localizedPath } from "../../shared/i18n/locale";
import { Button } from "../../shared/ui/button";
import { InlineAlert } from "../../shared/ui/inline-alert";
import { conversationText as t } from "./resource-config";

export interface RoomThread {
  id: string;
  kind: "pipeline" | "workflow";
  resource_id: string;
  name: string | null;
  deleted: boolean;
  linked: boolean;
  latest_run: { id: string; kind: string; state: string | null } | null;
}
export interface ThreadMessage { id: string; role: string; prompt: string; sender_name: string; created_at: number; thread_id?: string | null }

function runHref(workspace: string, thread: RoomThread) {
  const run = thread.latest_run;
  if (!run) return null;
  return run.kind === "pipeline_run"
    ? localizedPath(`/workspaces/${encodeURIComponent(workspace)}/pipeline-runs/${run.id}`, currentLocale())
    : `${localizedPath(`/workspaces/${encodeURIComponent(workspace)}/workflows/${thread.resource_id}`, currentLocale())}?execution=${run.id}`;
}
const title = (thread: RoomThread) => thread.deleted ? t("threadResourceDeleted") : thread.name ?? "";

/** The room's pipeline and workflow threads, above the message feed. */
export function RoomThreadList({ threads, onOpen }: { threads: RoomThread[]; onOpen: (id: string) => void }) {
  if (!threads.length) return null;
  return <nav aria-label={t("threads")} data-room-threads className="flex shrink-0 gap-2 overflow-x-auto border-b px-3 py-2 sm:px-6">
    {threads.map(thread => {
      const Icon = thread.kind === "pipeline" ? GitBranch : Workflow;
      return <button key={thread.id} type="button" data-room-thread={thread.id} onClick={() => onOpen(thread.id)}
        className="flex min-h-11 max-w-64 shrink-0 items-center gap-2 rounded-full border bg-card px-3 text-left text-sm hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring">
        <Icon className="size-4 shrink-0" aria-hidden />
        <span className="min-w-0 truncate">{title(thread)}</span>
        {thread.latest_run?.state && <span className="shrink-0 rounded-full bg-muted px-2 text-xs text-muted-foreground">{t(thread.latest_run.state)}</span>}
      </button>;
    })}
  </nav>;
}

/** One thread: its run results and discussion, with a composer that posts into it. */
export function RoomThreadPanel({ thread, messages, workspace, endpoint, onOpenChange, onSent }: {
  thread: RoomThread | undefined; messages: ThreadMessage[]; workspace: string; endpoint: string;
  onOpenChange: (open: boolean) => void; onSent: () => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const link = thread ? runHref(workspace, thread) : null;
  async function send() {
    if (!thread || !text.trim()) return;
    setBusy(true); setError("");
    try {
      await api(`${endpoint}/messages`, { method: "POST", body: JSON.stringify({ prompt: text, client_id: crypto.randomUUID(), thread_id: thread.id }) });
      setText(""); await onSent();
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); }
  }
  return <Dialog.Root open={!!thread} onOpenChange={value => { if (!busy) onOpenChange(value); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
      <Dialog.Content aria-describedby={undefined} data-thread-panel className="fixed inset-y-0 right-0 z-50 flex w-full max-w-lg flex-col border-l bg-background text-foreground shadow-xl">
        <header className="flex shrink-0 items-center gap-2 border-b px-4 py-3">
          <div className="min-w-0 flex-1">
            <Dialog.Title className="truncate font-semibold">{thread ? title(thread) : ""}</Dialog.Title>
            <p className="text-xs text-muted-foreground">{t(thread?.kind === "workflow" ? "threadWorkflow" : "threadPipeline")}{thread?.linked ? ` · ${t("threadLinked")}` : ""}</p>
          </div>
          <Dialog.Close asChild><Button variant="ghost" size="icon" className="size-11" aria-label={t("close")}><X /></Button></Dialog.Close>
        </header>
        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto overscroll-contain p-4">
          {thread?.latest_run && <div className="flex flex-wrap items-center gap-2 rounded-xl border p-3 text-sm">
            <span className="min-w-0 flex-1">{t("threadLatestRun")}: {thread.latest_run.state ? t(thread.latest_run.state) : "—"}</span>
            {link && <a className="text-primary underline" href={link}>{t("threadOpenRun")}</a>}
          </div>}
          {messages.map(message => <article key={message.id} className="rounded-xl border bg-card px-3 py-2 text-sm">
            {message.role !== "notice" && <p className="text-xs font-medium">{message.sender_name}</p>}
            <p className={`whitespace-pre-wrap break-words ${message.role === "notice" ? "text-muted-foreground" : ""}`}>{message.prompt}</p>
          </article>)}
          {!messages.length && <p className="text-sm text-muted-foreground">{t("threadEmpty")}</p>}
        </div>
        <footer className="shrink-0 space-y-2 border-t p-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
          {error && <InlineAlert>{error}</InlineAlert>}
          <div className="flex items-end gap-2">
            <textarea aria-label={t("threadReply")} rows={1} value={text} onChange={event => setText(event.target.value)}
              className="min-h-11 min-w-0 flex-1 resize-none rounded-md border bg-background px-3 py-2 text-base sm:text-sm" placeholder={t("threadReply")} />
            <Button className="min-h-11" disabled={busy || !text.trim()} onClick={() => void send()}>{t("send")}</Button>
          </div>
        </footer>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

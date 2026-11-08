import { Download, FileText, X } from "lucide-react";
import { Dialog } from "radix-ui";
import { Button } from "../../shared/ui/button";
import { conversationText as t } from "./resource-config";
export interface Attachment {
  name: string;
  text?: string;
  file_id?: string;
  mime?: string;
  size?: number;
}
export function AttachmentView({
  attachment: a,
  endpoint,
}: {
  attachment: Attachment;
  endpoint: string;
}) {
  const url = a.file_id
    ? `/api${endpoint}/files/${encodeURIComponent(a.file_id)}`
    : undefined;
  const download = url
    ? `${url}?download=true`
    : `data:text/plain;charset=utf-8,${encodeURIComponent(a.text ?? "")}`;
  const size =
    a.size !== undefined
      ? a.size >= 1024 * 1024
        ? `${(a.size / 1024 / 1024).toFixed(1)} MB`
        : `${Math.ceil(a.size / 1024)} KB`
      : "";
  const image = url && /^image\/(png|jpeg|gif|webp|avif)$/.test(a.mime ?? "");
  return (
    <div className="mt-2 min-w-0 overflow-hidden rounded-lg border bg-background/60 text-foreground">
      {image && (
        <Dialog.Root>
          <Dialog.Trigger asChild>
            <button
              type="button"
              aria-label={`${t("preview")} ${a.name}`}
              className="block w-full cursor-zoom-in focus-visible:outline-2 focus-visible:outline-ring"
            >
              <img
                src={url}
                alt={a.name}
                loading="lazy"
                className="max-h-64 w-full object-contain"
              />
            </button>
          </Dialog.Trigger>
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 z-50 bg-black/70" />
            <Dialog.Content
              aria-describedby={undefined}
              className="fixed inset-4 z-50 flex flex-col rounded-lg bg-background p-4 sm:inset-10"
            >
              <div className="mb-3 flex items-center justify-between gap-3">
                <Dialog.Title className="truncate font-medium">
                  {a.name}
                </Dialog.Title>
                <Dialog.Close asChild>
                  <Button size="icon" variant="ghost" aria-label={t("close")}>
                    <X />
                  </Button>
                </Dialog.Close>
              </div>
              <img
                src={url}
                alt={a.name}
                className="min-h-0 flex-1 object-contain"
              />
            </Dialog.Content>
          </Dialog.Portal>
        </Dialog.Root>
      )}
      {url && a.mime?.startsWith("audio/") && (
        <audio
          src={url}
          controls
          preload="metadata"
          className="max-w-full p-2"
        />
      )}
      {url && a.mime?.startsWith("video/") && (
        <video
          src={url}
          controls
          playsInline
          preload="metadata"
          className="max-h-80 w-full"
        />
      )}
      <div className="flex min-w-0 items-center gap-2 px-3 py-2 text-sm">
        <FileText className="size-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate" title={a.name}>
          {a.name}
        </span>
        <span className="shrink-0 text-xs text-muted-foreground">{size}</span>
        <a
          href={download}
          download={a.name}
          aria-label={`${t("download")} ${a.name}`}
          className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
        >
          <Download className="size-4" />
        </a>
      </div>
      {!!a.text && (
        <details className="border-t px-3 py-2 text-sm">
          <summary className="cursor-pointer">{t("preview")}</summary>
          <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words text-xs">
            {a.text}
          </pre>
        </details>
      )}
    </div>
  );
}

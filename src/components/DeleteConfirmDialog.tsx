import { commonText, useCommonTranslation } from "../i18n";
import { useState } from "react";
import { Dialog } from "radix-ui";
import { Button } from "./ui/button";
import { errorText } from "../api";

export function DeleteConfirmDialog({ action = commonText("common.delete"), target, description = commonText("delete.help"), onClose, onConfirm }: {
  action?: string;
  target: string;
  description?: string;
  onClose: () => void;
  onConfirm: () => Promise<void> | void;
}) {
  useCommonTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return <Dialog.Root open onOpenChange={open => { if (!open && !busy) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
      <Dialog.Content className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-md border bg-background p-5 shadow-lg">
        <Dialog.Title className="break-all text-base font-semibold">{commonText("common.confirm")}{action}“{target}”？</Dialog.Title>
        <Dialog.Description className="mt-2 text-sm text-muted-foreground">{description}</Dialog.Description>
        {error && <p role="alert" className="mt-3 text-sm text-destructive">{error}</p>}
        <div className="mt-5 flex justify-end gap-2">
          <Button variant="outline" disabled={busy} onClick={onClose}>{commonText("common.cancel")}</Button>
          <Button variant="destructive" disabled={busy} onClick={async () => {
            setBusy(true); setError("");
            try { await onConfirm(); onClose(); }
            catch (cause) { setError(errorText(cause)); }
            finally { setBusy(false); }
          }}>{busy ? commonText("common.processing") : commonText("delete.confirm", { action })}</Button>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

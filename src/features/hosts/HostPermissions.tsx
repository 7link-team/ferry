import { useState } from "react";
import { Dialog } from "radix-ui";
import { AutoForm } from "../../shared/forms/auto-form";
import { Button } from "../../shared/ui/button";
import { Checkbox } from "../../shared/ui/checkbox";
import { api, errorText } from "../../shared/api/api";
import { hostRepositoryText as t } from "../../shared/i18n/i18n";

export function HostPermissions({
  endpoint,
  path,
  initialMode,
  onClose,
  onSaved,
}: {
  endpoint: string;
  path: string;
  initialMode: number;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [mode, setMode] = useState(initialMode & 0o777),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
        <Dialog.Content
          className="dialog-shell fixed left-1/2 top-1/2 z-50 flex max-h-[90dvh] w-[min(32rem,calc(100vw-1.5rem))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border bg-background shadow-xl"
          onEscapeKeyDown={(e) => {
            if (busy) e.preventDefault();
          }}
          onPointerDownOutside={(e) => {
            if (busy) e.preventDefault();
          }}
        >
          <div className="shrink-0 border-b p-4">
            <Dialog.Title className="font-semibold">
              {t("host-files.permissions")}
            </Dialog.Title>
            <Dialog.Description className="mt-1 break-all text-sm text-muted-foreground">
              {path}
            </Dialog.Description>
          </div>
          <AutoForm
            className="dialog-form min-h-0"
            onSubmit={async (e) => {
              e.preventDefault();
              setBusy(true);
              setError("");
              try {
                await api(endpoint.replace(/^\/api/, "") + "/permissions", {
                  method: "PATCH",
                  body: JSON.stringify({ path, mode }),
                });
                onSaved();
              } catch (cause) {
                setError(errorText(cause));
              } finally {
                setBusy(false);
              }
            }}
          >
            <div className="min-h-0 space-y-4 overflow-y-auto p-4">
              <p className="text-sm text-muted-foreground">
                {t("host-files.permissionsHelp")}
              </p>
              <div className="grid grid-cols-[minmax(0,1fr)_repeat(3,3.5rem)] items-center text-sm">
                <span />
                {["read", "write", "execute"].map((key) => (
                  <span key={key} className="text-center font-medium">
                    {t(`host-files.permission.${key}`)}
                  </span>
                ))}
                {["owner", "group", "others"].map((name, index) => (
                  <div key={name} className="contents">
                    <span>{t(`host-files.permission.${name}`)}</span>
                    {[4, 2, 1].map((bit, column) => {
                      const flag = bit << ((2 - index) * 3);
                      return (
                        <label
                          key={bit}
                          className="flex min-h-11 items-center justify-center"
                        >
                          <Checkbox
                            disabled={busy}
                            aria-label={`${t(`host-files.permission.${name}`)} · ${t(`host-files.permission.${["read", "write", "execute"][column]}`)}`}
                            checked={!!(mode & flag)}
                            onCheckedChange={(checked) =>
                              setMode((current) =>
                                checked ? current | flag : current & ~flag,
                              )
                            }
                          />
                        </label>
                      );
                    })}
                  </div>
                ))}
              </div>
              <p className="font-mono text-sm">
                {mode.toString(8).padStart(3, "0")}
              </p>
              {error && (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              )}
            </div>
            <div className="dialog-footer flex shrink-0 gap-3 border-t p-4">
              <Button
                type="button"
                variant="outline"
                className="min-h-11 flex-1"
                disabled={busy}
                onClick={onClose}
              >
                {t("host-files.uploadClose")}
              </Button>
              <Button type="submit" className="min-h-11 flex-1" disabled={busy}>
                {t("host-files.permissionsSave")}
              </Button>
            </div>
          </AutoForm>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

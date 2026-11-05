import { Check, GitBranch, Workflow } from "lucide-react";
import { Dialog } from "radix-ui";
import { useState } from "react";
import { workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";

type Kind = "actions" | "nodes";
export function WorkflowKindChoice({
  value,
  creating,
  dirty,
  allowed,
  choose,
}: {
  value: Kind;
  creating: boolean;
  dirty: boolean;
  allowed: (kind: Kind) => boolean;
  choose: (kind: Kind) => void;
}) {
  const [pending, setPending] = useState<Kind>();
  if (!creating)
    return (
      <div
        className="flex items-center gap-2 border-b px-4 py-2 text-xs text-muted-foreground"
        aria-label={t("workflowKind")}
      >
        {value === "actions" ? (
          <GitBranch className="size-4" />
        ) : (
          <Workflow className="size-4" />
        )}
        <span className="font-medium text-foreground">
          {t(`kind.${value}`)}
        </span>
        <span className="hidden sm:inline">· {t(`kindHelp.${value}`)}</span>
      </div>
    );
  return (
    <section
      className="border-b bg-muted/10 px-4 py-3"
      aria-label={t("workflowKind")}
    >
      <h2 className="mb-2 text-xs font-medium text-muted-foreground">
        {t("workflowKind")}
      </h2>
      <div
        className="grid gap-2 sm:grid-cols-2"
        role="group"
        aria-label={t("workflowKind")}
      >
        {(["actions", "nodes"] as const).map((kind) => {
          const Icon = kind === "actions" ? GitBranch : Workflow;
          return (
            <button
              key={kind}
              type="button"
              aria-pressed={kind === value}
              aria-label={t(`kind.${kind}`)}
              disabled={!allowed(kind)}
              className={`flex min-h-16 items-center gap-3 rounded-lg border px-4 py-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 ${kind === value ? "border-primary bg-background ring-1 ring-primary" : "bg-background/70 hover:border-foreground/30"}`}
              onClick={() => {
                if (kind !== value) {
                  if (dirty) setPending(kind);
                  else choose(kind);
                }
              }}
            >
              <Icon className="size-5 shrink-0" />
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold">
                  {t(`kind.${kind}`)}
                </span>
                <span className="mt-1 block text-xs leading-5 text-muted-foreground">
                  {t(`kindHelp.${kind}`)}
                </span>
              </span>
              {kind === value && <Check className="size-4 shrink-0" />}
            </button>
          );
        })}
      </div>
      <Dialog.Root
        open={!!pending}
        onOpenChange={(open) => {
          if (!open) setPending(undefined);
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
          <Dialog.Content className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-xl border bg-background p-6 shadow-xl">
            <Dialog.Title className="text-base font-semibold">
              {t("switchKind")}
            </Dialog.Title>
            <Dialog.Description className="mt-3 text-sm leading-6 text-muted-foreground">
              {t("switchKindHelp")}
            </Dialog.Description>
            <div className="mt-5 flex justify-end gap-2">
              <Dialog.Close asChild>
                <Button type="button" variant="outline">
                  {t("cancel")}
                </Button>
              </Dialog.Close>
              <Button
                type="button"
                onClick={() => {
                  if (pending) choose(pending);
                  setPending(undefined);
                }}
              >
                {t("confirm")}
              </Button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </section>
  );
}

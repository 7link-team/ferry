import { CircleAlert, X } from "lucide-react";
import type { ReactNode } from "react";
import { commonText, i18n } from "../i18n/i18n";
import { cn } from "cn";

/**
 * An error shown next to the page, section or control it belongs to.
 * `action` holds a recovery control such as Retry; `detail` keeps technical text
 * (e.g. a parser's own message) out of the main sentence but still available.
 */
export function InlineAlert({ children, onDismiss, action, detail, className }: {
  children: ReactNode;
  onDismiss?: (() => void) | undefined;
  action?: ReactNode;
  detail?: string | undefined;
  className?: string | undefined;
}) {
  return (
    <div
      role="alert"
      className={cn(
        "flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive",
        className,
      )}
    >
      <CircleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
      <div className="min-w-0 flex-1 space-y-1">
        <div className="break-words">{children}</div>
        {detail && (
          <details className="text-xs text-destructive/80">
            <summary className="cursor-pointer select-none">{i18n.t("errors.detail")}</summary>
            <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-all font-mono">{detail}</pre>
          </details>
        )}
      </div>
      {action && <div className="shrink-0 self-center">{action}</div>}
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label={commonText("common.close")}
          title={commonText("common.close")}
          className="-m-1 shrink-0 rounded p-1 hover:bg-destructive/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <X aria-hidden="true" className="size-4" />
        </button>
      )}
    </div>
  );
}

/** Retry control for an InlineAlert action slot. */
export function RetryButton({ onClick, disabled }: { onClick: () => void; disabled?: boolean | undefined }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="rounded-md border border-destructive/40 px-2.5 py-1 text-xs font-medium hover:bg-destructive/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
    >
      {commonText("common.retry")}
    </button>
  );
}

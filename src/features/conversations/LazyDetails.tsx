import { useState, type ReactNode } from "react";
import { toast } from "sonner";
import { Button } from "../../shared/ui/button";
import { commonText } from "../../shared/i18n/i18n";
import { conversationText as t } from "./resource-config";

/** A <details> whose content is built only once it is opened. */
export function LazyDetails({
  summary,
  children,
  className,
  summaryClassName,
  ...rest
}: {
  summary: ReactNode;
  children: () => ReactNode;
  className?: string;
  summaryClassName?: string;
  [data: `data-${string}`]: unknown;
}) {
  const [open, setOpen] = useState(false);
  return (
    <details {...rest} className={className} onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary className={summaryClassName}>{summary}</summary>
      {open && children()}
    </details>
  );
}

/** First part shown of very long text, so a large result cannot freeze layout. */
export const LONG_TEXT_LIMIT = 16 * 1024;

/** A JSON value or text, formatted on demand and capped until "show all". */
export function LongText({ value, className = "" }: { value: unknown; className?: string }) {
  const [all, setAll] = useState(false);
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2) ?? "";
  const long = text.length > LONG_TEXT_LIMIT;
  return (
    <div data-long-text>
      <pre className={`${className} overflow-auto whitespace-pre-wrap break-all`}>
        {long && !all ? text.slice(0, LONG_TEXT_LIMIT) : text}
      </pre>
      {long && (
        <div className="mt-1 flex flex-wrap items-center gap-2">
          {!all && <span>{t("truncatedNote")}</span>}
          {!all && (
            <Button type="button" size="sm" variant="outline" onClick={() => setAll(true)}>
              {t("showAll")}
            </Button>
          )}
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => {
              void navigator.clipboard?.writeText(text).then(
                () => toast.success(commonText("common.copied")),
                () => undefined,
              );
            }}
          >
            {t("copyAll")}
          </Button>
        </div>
      )}
    </div>
  );
}

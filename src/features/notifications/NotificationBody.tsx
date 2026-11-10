import { lazy, Suspense, useState } from "react";
import { commonText as t } from "../../shared/i18n/i18n";
import { Tabs, TabsList, TabsTrigger } from "../../shared/ui/tabs";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { sanitizeRichText } from "../maintenance/value-content";

const RichTextEditor = lazy(() => import("../maintenance/RichTextValueEditor"));

export function NotificationBody({ value, format, readOnly, onChange }: {
  value: string; format: string; readOnly: boolean; onChange: (value: string) => void;
}) {
  const [advanced, setAdvanced] = useState(false);
  if (readOnly) return format === "html"
    ? <div className="prose prose-sm dark:prose-invert max-w-none break-words [&_img]:max-w-full" dangerouslySetInnerHTML={{ __html: sanitizeRichText(value) }} />
    : <div className="whitespace-pre-wrap break-words text-sm">{value || "—"}</div>;
  return <div className="min-w-0 space-y-3">
    {format === "html" && <Tabs value={advanced ? "html" : "rich"} onValueChange={v => setAdvanced(v === "html")}>
      <TabsList><TabsTrigger value="rich">{t("notifications:rich")}</TabsTrigger><TabsTrigger value="html">HTML</TabsTrigger></TabsList>
    </Tabs>}
    {format === "html" && !advanced
      ? <Suspense fallback={<LoadingSkeleton variant="editor" />}><RichTextEditor value={value} readOnly={false} onChange={onChange} /></Suspense>
      : <textarea aria-label={t("notifications:body")} className="min-h-48 w-full rounded-md border bg-background p-3 text-sm" maxLength={48000} value={value} onChange={e => onChange(e.target.value)} />}
  </div>;
}

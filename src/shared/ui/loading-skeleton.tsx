import { cn } from "cn";
import { commonText } from "../i18n/i18n";
import { Skeleton } from "./skeleton";

/** Shared loading geometry for routes, forms, editors and small data fields. */
export function LoadingSkeleton({ variant = "form", label = commonText("common.loading"), className }: {
  variant?: "page" | "form" | "editor" | "inline";
  label?: string;
  className?: string;
}) {
  return <div role="status" aria-busy="true" aria-label={label} data-slot="loading-skeleton" className={cn("min-w-0", className)}>
    <span className="sr-only">{label}</span>
    <div aria-hidden="true" className={variant === "inline" ? "space-y-2" : "space-y-6 p-4 sm:p-6"}>
      {variant === "inline" ? <Skeleton className="h-9 w-full max-w-md" /> : <>
        <div className="flex items-center justify-between gap-4"><div className="min-w-0 flex-1 space-y-2"><Skeleton className="h-5 w-36 max-w-full" /><Skeleton className="h-3 w-56 max-w-full" /></div><Skeleton className="h-8 w-20 shrink-0" /></div>
        {variant === "page" && <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">{[0, 1, 2, 3].map(i => <div key={i} className="space-y-3 rounded-md border p-4"><Skeleton className="h-3 w-20 max-w-full" /><Skeleton className="h-7 w-16" /></div>)}</div>}
        {variant === "editor" ? <div className="min-h-48 space-y-3 rounded-md border p-4">{[76, 54, 88, 63, 44, 72, 58, 82].map((width, i) => <Skeleton key={i} className="h-3" style={{ width: `${width}%`, marginLeft: i % 3 ? "5%" : 0 }} />)}</div>
          : variant === "page" ? <div className="space-y-4 rounded-md border p-4">{[0, 1, 2, 3, 4, 5].map(row => <div key={row} className="grid grid-cols-3 gap-6"><Skeleton className="h-4 w-3/4" /><Skeleton className="h-4 w-full" /><Skeleton className="h-4 w-2/3" /></div>)}</div>
          : <div className="grid gap-6 sm:grid-cols-2">{[0, 1, 2, 3, 4, 5].map(i => <div key={i} className="space-y-2"><Skeleton className="h-3 w-24" /><Skeleton className="h-10 w-full" /></div>)}</div>}
      </>}
    </div>
  </div>;
}

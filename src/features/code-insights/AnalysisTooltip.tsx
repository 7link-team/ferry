import { Tooltip } from "radix-ui";
import type { ReactElement } from "react";
import { insightText } from "../../shared/i18n/i18n";

export const metricClass = (metric: string) => ({ additions: "text-[var(--contribution-additions)]", deletions: "text-[var(--contribution-deletions)]", effectives: "text-[var(--contribution-effective)]" })[metric as "additions" | "deletions" | "effectives"] ?? "";

export function AnalysisTooltip({ children, date, row, author }: { children: ReactElement; date: string; row: Record<string, unknown>; author?: string }) {
  const authors = Array.isArray(row.authors) ? row.authors as Record<string, unknown>[] : [];
  return <Tooltip.Root><Tooltip.Trigger asChild>{children}</Tooltip.Trigger><Tooltip.Portal><Tooltip.Content sideOffset={8} className="z-50 max-h-72 w-72 max-w-[calc(100vw-2rem)] overflow-y-auto rounded-md border bg-popover p-3 text-xs text-popover-foreground shadow-md">
    <p className="font-semibold">{date}{author ? ` · ${author}` : ""}</p>
    <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1 tabular-nums">{[["commits", "提交数"], ["additions", "新增行"], ["deletions", "删除行"], ["effectives", "有效变更"], ...(!author ? [["contributors", "贡献者"]] : [])].map(([key, label]) => <div key={key} className="flex justify-between gap-2"><dt>{insightText(label!)}</dt><dd className={metricClass(key!)}>{Number(row[key!] ?? 0).toLocaleString()}</dd></div>)}</dl>
    {!!authors.length && <ul className="mt-2 space-y-1 border-t pt-2">{authors.map((person, index) => <li key={String(person.authorKey ?? index)} className="flex justify-between gap-3"><span className="break-all">{String(person.nickname || person.name || person.email || "—")}</span><span className="shrink-0 tabular-nums">{Number(person.commits ?? 0)} · <span className={metricClass("effectives")}>{Number(person.effectives ?? 0)}</span></span></li>)}</ul>}
    <Tooltip.Arrow className="fill-popover" />
  </Tooltip.Content></Tooltip.Portal></Tooltip.Root>;
}

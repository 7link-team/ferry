import { ChevronDown, RefreshCw } from "lucide-react";
import { REFRESH_INTERVALS } from "../api/resource";
import { commonText } from "../i18n/i18n";
import { Button } from "./button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./dropdown-menu";

export function RefreshControl({ loading, interval, remaining, onRefresh, onIntervalChange }: {
  loading: boolean; interval: number; remaining: number; onRefresh: () => void; onIntervalChange: (seconds: number) => void;
}) {
  return <div data-slot="refresh-control" className="inline-flex shrink-0 items-center overflow-hidden whitespace-nowrap rounded-full border bg-background shadow-xs">
    <Button size="sm" variant="ghost" aria-label={commonText("common.refresh")} className="rounded-none! border-0! shadow-none! ring-inset" onClick={onRefresh} disabled={loading}>
      <RefreshCw className={loading ? "animate-spin" : ""} />{commonText("common.refresh")}
    </Button>
    <span aria-hidden="true" className="h-4 w-px shrink-0 bg-border" />
    <DropdownMenu><DropdownMenuTrigger asChild>
      <Button size="sm" variant="ghost" className="min-w-10 rounded-none! border-0! px-2 shadow-none! ring-inset tabular-nums" aria-label={commonText("table.autoRefresh")}>
        {interval ? <span role="timer" aria-live="off" aria-busy={loading}>{loading ? "0s" : `${remaining}s`}</span> : <ChevronDown aria-hidden="true" className="size-3.5" />}
      </Button>
    </DropdownMenuTrigger><DropdownMenuContent align="start">
      {REFRESH_INTERVALS.map(seconds => <DropdownMenuItem key={seconds} onClick={() => onIntervalChange(seconds)}>{seconds ? commonText("table.refreshEvery", {seconds}) : commonText("table.disableRefresh")}</DropdownMenuItem>)}
    </DropdownMenuContent></DropdownMenu>
  </div>;
}
